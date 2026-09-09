#!/usr/bin/env python3
"""Expand a registry package list into the VERSIONS one sweep should validate.

`registry-validation.yml` used to run one leg per PACKAGE, and each leg validated that
package's newest stable version only — so every older published version was validated by
nothing (121 of 164 when this was written).  That is the gate hole behind loft#1448: `imaging`
0.1.0 shipped a `native/Cargo.toml` requiring `loft-ffi-build = "0.1"` for a `build.rs` calling
the 0.2 API, and nothing ever noticed, because by then the newest stable was 0.3.2 and 0.3.2
builds fine.

Two scopes, because the two questions have different urgencies (loft#1462):

``tip``   the newest stable of each package — what a `loft install` gets today, so a rot here
          is somebody's build breaking tonight.  This is the nightly, and it emits BARE package
          names on purpose: `registry_validate.sh` resolves the tip through `loft api
          --registry`, which is loft's own resolution and the authoritative one.  Naming a
          version here instead would make this file a SECOND resolver, and the nightly would
          silently change target the day the two disagreed.  The scope that needs explicit
          versions is the one that cannot ask — `full`.
``full``  every non-yanked, non-prerelease version.  A rotted OLD version breaks nobody until
          somebody pins it, so a week's latency is proportionate — and it is the only scope
          under which "checked by nothing" is false for every published version, which is the
          whole of loft#1462.

**Why not the cheaper middles.**  Validating only versions some published `loft.toml` depends
on is cheapest and would have MISSED loft#1448 outright: nothing depended on imaging 0.1, and
"nobody uses it" is exactly why nobody noticed.  Validating the newest of each MINOR line is
about half the cost of ``full`` and covers the `^0.1`-pin shape, but it leaves 82 of 164
versions unchecked — it narrows the hole rather than closing it, and the report's own headline
is the count.  A cadence split closes it and keeps the nightly's cost and signal intact;
`repro-build` is the existing precedent for a weekly gate in this repo.

Usage:  registry_matrix_versions.py <index.json> <tip|full> [packages.json]
        registry_matrix_versions.py --self-test

`packages.json` is the workflow's already-computed package list (a JSON array of names); when
omitted every package in the index is considered.  Output is a JSON array of entries
`scripts/registry_validate.sh` takes directly: a bare `pkg` under `tip`, and `pkg@version`
under `full`.
"""

import json
import re
import sys


def _yanked(pkg: dict) -> set:
    """The yanked set, TYPE-COERCED rather than trusted.

    The schema says `yanked` is a list, and eleven hand-published packages carried `false`.
    The workflow's own `jq` learned this the hard way — a bare `.yanked | length` died with
    "boolean (false) has no length", which failed `discover` in 3 seconds and skipped the
    ENTIRE validate matrix, so every published package went unvalidated for days while the run
    merely looked red.  A malformed field must cost that one package, never the whole sweep.
    """
    y = pkg.get("yanked")
    return set(y) if isinstance(y, list) else set()


def _key(version: str) -> tuple:
    """Sort key: the numeric components, so `0.10.0` sorts above `0.9.0` (a string sort does not)."""
    return tuple(int(n) for n in re.findall(r"\d+", version))


def versions_for(pkg: dict, scope: str) -> list:
    """The versions of one package this scope validates, oldest first.

    Returns `[None]` for `tip`, meaning "the bare name, and let the validator resolve it".
    """
    yanked = _yanked(pkg)
    # A PRERELEASE is excluded from both scopes: `loft install` will not resolve one without an
    # exact pin, so it is not what a user gets and not what a pin lands on either.
    usable = sorted(
        (v for v in pkg.get("versions", {}) if v not in yanked and "-" not in v),
        key=_key,
    )
    if not usable:
        return []
    return usable if scope == "full" else [None]


# One case per thing that has actually gone wrong here or next door, so a reader can see what
# the selection is defending against rather than inferring it from the code.
_CASES = [
    # (label, package json, scope, expected versions)
    ("tip is the BARE name — the validator resolves it",
     {"versions": {"0.1.0": {}, "0.2.0": {}}}, "tip", [None]),
    ("full takes every non-yanked version, oldest first",
     {"versions": {"0.1.0": {}, "0.2.0": {}}}, "full", ["0.1.0", "0.2.0"]),
    # Eleven hand-published packages carried `yanked: false`, and a bare `.yanked | length`
    # over that killed the whole sweep for days.  Coerced, never trusted.
    ("a non-list `yanked` is coerced, not fatal",
     {"versions": {"0.1.0": {}}, "yanked": False}, "full", ["0.1.0"]),
    ("a yanked version is not validated",
     {"versions": {"0.1.0": {}, "0.2.0": {}}, "yanked": ["0.1.0"]}, "full", ["0.2.0"]),
    ("a package whose every version is yanked contributes no leg",
     {"versions": {"0.1.0": {}}, "yanked": ["0.1.0"]}, "full", []),
    ("…and contributes none under tip either, rather than a name with nothing behind it",
     {"versions": {"0.1.0": {}}, "yanked": ["0.1.0"]}, "tip", []),
    # A string sort puts 0.9.0 above 0.10.0, which would validate the wrong tip the day a
    # package reaches a two-digit component.  `stage` is already at 0.18.
    ("versions sort NUMERICALLY, so 0.10.0 is newer than 0.9.0",
     {"versions": {"0.9.0": {}, "0.10.0": {}}}, "full", ["0.9.0", "0.10.0"]),
    # `loft install` will not resolve a prerelease without an exact pin, so it is neither what
    # a user gets nor what a `^` pin lands on.
    ("a prerelease is excluded from both scopes",
     {"versions": {"0.1.0": {}, "0.2.0-rc1": {}}}, "full", ["0.1.0"]),
]


def _self_test() -> int:
    bad = 0
    for label, pkg, scope, want in _CASES:
        got = versions_for(pkg, scope)
        if got != want:
            bad += 1
            print(f"  FAIL {label}\n       want {want!r}, got {got!r}")
    print(f"  registry_matrix_versions self-test: {len(_CASES) - bad}/{len(_CASES)} cases pass")
    return 1 if bad else 0


def main(argv: list) -> int:
    if len(argv) == 2 and argv[1] == "--self-test":
        return _self_test()
    if len(argv) < 3:
        sys.stderr.write(__doc__)
        return 2
    index = json.load(open(argv[1], encoding="utf-8"))
    scope = argv[2]
    if scope not in ("tip", "full"):
        sys.stderr.write(f"unknown scope {scope!r} — expected 'tip' or 'full'\n")
        return 2
    wanted = None
    if len(argv) > 3:
        with open(argv[3], encoding="utf-8") as fh:
            wanted = set(json.load(fh))

    entries = []
    for name, pkg in sorted(index.get("packages", {}).items()):
        if wanted is not None and name not in wanted:
            continue
        entries.extend(name if v is None else f"{name}@{v}" for v in versions_for(pkg, scope))
    json.dump(entries, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
