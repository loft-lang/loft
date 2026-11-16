#!/usr/bin/env python3
"""Expand a registry package list into the VERSIONS one sweep should validate.

`registry-validation.yml` used to run one leg per PACKAGE, and each leg validated that
package's newest stable version only — so every older published version was validated by
nothing (121 of 164 when this was written).  That is the gate hole behind loft#1448: `imaging`
0.1.0 shipped a `native/Cargo.toml` requiring `loft-ffi-build = "0.1"` for a `build.rs` calling
the 0.2 API, and nothing ever noticed, because by then the newest stable was 0.3.2 and 0.3.2
builds fine.

Three scopes, because the questions have different urgencies (loft#1462):

``tip``   the newest stable of each package — what a `loft install` gets today, so a rot here
          is somebody's build breaking tonight.  This is the nightly, and it emits BARE package
          names on purpose: `registry_validate.sh` resolves the tip through `loft api
          --registry`, which is loft's own resolution and the authoritative one.  Naming a
          version here instead would make this file a SECOND resolver, and the nightly would
          silently change target the day the two disagreed.  The scope that needs explicit
          versions is the one that cannot ask — `full`.
``contract``  the weekly sweep: from the contract boundary on (a release whose `loft` floor is
          at or past ``CONTRACT_FLOOR``), the newest release of each API line — the releases that
          declare one `api_compatible_with` share an API, so the newest of them stands for the
          line.  Under the contract loft does not break a release that works, so this sweep is
          the check that it kept that promise, one release per API rather than every one.
          A release published before the boundary is HELD — an exact pin or a lock still
          resolves it — but it is neither tested nor offered (`.github/LABELS.md` §
          `contract:broken`; the owner's soft-freeze ruling).
``full``  every non-yanked, non-prerelease version, by hand only (`workflow_dispatch`).  Most
          releases from before the boundary fail it: the language changed under them, which the
          contract now forbids, not loft breaking them since.

**Why not the cheaper middles.**  Validating only versions some published `loft.toml` depends
on is cheapest and would have MISSED loft#1448 outright: nothing depended on imaging 0.1, and
"nobody uses it" is exactly why nobody noticed.  Validating the newest of each MINOR line is
about half the cost of ``full`` and covers the `^0.1`-pin shape, but it leaves 82 of 164
versions unchecked — it narrows the hole rather than closing it, and the report's own headline
is the count.  A cadence split closes it and keeps the nightly's cost and signal intact;
`repro-build` is the existing precedent for a weekly gate in this repo.

Usage:  registry_matrix_versions.py <index.json> <tip|contract|full> [packages.json]
        registry_matrix_versions.py --self-test

`packages.json` is the workflow's already-computed package list (a JSON array of names); when
omitted every package in the index is considered.  Output is a JSON array of entries
`scripts/registry_validate.sh` takes directly: a bare `pkg` under `tip`, and `pkg@version`
under `contract` and `full`.
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


# The contract boundary: the `loft` floor from which a release is under the contract.  Releases
# whose floor is older are held but not tested.  It moves once, to the contract=1 release.
CONTRACT_FLOOR = "2026.11"


def _key(version: str) -> tuple:
    """Sort key: the numeric components, so `0.10.0` sorts above `0.9.0` (a string sort does not)."""
    return tuple(int(n) for n in re.findall(r"\d+", version))


def _floor(entry: dict) -> tuple:
    """The `loft` floor a release declares (`>=2026.11.20261114`), as a sort key; `()` when it
    names none or one that is not a plain `>=`, which is older than any boundary."""
    m = re.match(r"\s*>=\s*([\d.]+)\s*$", entry.get("loft") or "")
    return _key(m.group(1)) if m else ()


def _api_line_tips(pkg: dict, usable: list) -> list:
    """The newest release of each API line among `usable` (oldest first), the line being the
    `api_compatible_with` a release declares — or the release itself when it declares none."""
    tips = {}
    for v in usable:
        line = pkg["versions"][v].get("api_compatible_with") or v
        tips[line] = v
    return sorted(tips.values(), key=_key)


def versions_for(pkg: dict, scope: str) -> list:
    """The versions of one package this scope validates, oldest first.

    Returns `[None]` for `tip`, meaning "the bare name, and let the validator resolve it".
    """
    yanked = _yanked(pkg)
    # A PRERELEASE is excluded from both scopes: `loft install` will not resolve one without an
    # exact pin, so it is not what a user gets and not what a pin lands on either.
    # A DEPRECATED release (this loft cannot build it — scripts/registry_deprecate.py) is held
    # for a pin, but no scope tests it.
    usable = sorted(
        (v for v, e in pkg.get("versions", {}).items()
         if v not in yanked and "-" not in v and not (e or {}).get("deprecated")),
        key=_key,
    )
    if not usable:
        return []
    if scope == "contract":
        boundary = _key(CONTRACT_FLOOR)
        return [v for v in _api_line_tips(pkg, usable)
                if _floor(pkg["versions"][v])[:len(boundary)] >= boundary]
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
    # The contract scope: one release per API line, from the boundary on.
    ("contract takes the newest release of each API line",
     {"versions": {"0.3.1": {"loft": ">=2026.11.1"},
                   "0.3.2": {"loft": ">=2026.11.1", "api_compatible_with": "0.3.1"},
                   "0.3.3": {"loft": ">=2026.11.2", "api_compatible_with": "0.3.1"},
                   "0.4.0": {"loft": ">=2026.11.2", "api_compatible_with": "0.4.0"}}},
     "contract", ["0.3.3", "0.4.0"]),
    # A patch release may declare a break inside its minor line (cbor 0.1.3 is compatible back
    # to 0.1.1, not 0.1.0), so the line is the API line, not the minor number.
    ("a break inside a minor line is its own API line",
     {"versions": {"0.1.0": {"loft": ">=2026.11.1"},
                   "0.1.1": {"loft": ">=2026.11.1", "api_compatible_with": "0.1.1"},
                   "0.1.2": {"loft": ">=2026.11.1", "api_compatible_with": "0.1.1"}}},
     "contract", ["0.1.0", "0.1.2"]),
    ("a release from before the boundary is held, not tested",
     {"versions": {"0.1.0": {"loft": ">=0.8"},
                   "0.2.0": {"loft": ">=2026.10.0"},
                   "0.3.0": {"loft": ">=2026.11.20261114"}}},
     "contract", ["0.3.0"]),
    ("a release with no floor is from before the boundary",
     {"versions": {"0.1.0": {}}}, "contract", []),
    # The boundary reads the floor as a version, so 2026.12 is past 2026.11 and 2026.9 is not.
    ("a deprecated release is in no scope",
     {"versions": {"0.1.0": {"deprecated": "Rust only"}, "0.2.0": {}}}, "full", ["0.2.0"]),
    ("the floor compares as a version, not a string",
     {"versions": {"0.1.0": {"loft": ">=2026.9.0"}, "0.2.0": {"loft": ">=2026.12.0"}}},
     "contract", ["0.2.0"]),
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
    if scope not in ("tip", "contract", "full"):
        sys.stderr.write(f"unknown scope {scope!r} — expected 'tip', 'contract' or 'full'\n")
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
