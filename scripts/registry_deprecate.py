#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Mark the registry releases this loft cannot build as DEPRECATED.

A deprecated release is held — an exact pin or a lock still resolves it, and it stays
downloadable — but no range lands on it, and building it is refused with the cure
(`registry_index::Version::deprecated`; PKG_REGISTRY.md § Deprecation).  The reason this script
reads from the release itself: its native part is Rust only (`[native] crate` and no C part: no
`[c]` shim, no prebuilt library, no component) — loft builds C, and the rustc writer is leaving.

A DEPENDANT of a deprecated release is not inferred deprecated: when nothing in its range's API
line is left, the range resolves to the newest release (@C166), and whether it works there is a
test result (registry validation), not something the line can say — drawing 0.5.0, whose graphics
line is all Rust-only, passes on graphics 0.12.1.

Usage:  registry_deprecate.py <index.json> [--write] [--manifests DIR]
        registry_deprecate.py --self-test

Reads each release's `loft.toml` from the local registry cache (`~/.loft/registry`) or from
its tarball (downloaded once into `--manifests`, default a temp directory), prints what it
would deprecate and why, and with `--write` writes the `deprecated` reasons into `index.json`
(two-space JSON, as the registry keeps it).  The registry PR it becomes is signed by the
maintainer: `scripts/registry-sign.sh --pr <N>`.
"""

import io
import json
import os
import re
import sys
import tarfile
import tempfile
import urllib.request
import zipfile

RUST_ONLY = "its native part is Rust only, which loft does not build"
UNREAD = []  # releases whose manifest could not be read: reported, never deprecated


def _key(v):
    return tuple(int(n) for n in re.findall(r"\d+", v))


def satisfies(version, constraint):
    """`registry_index::satisfies`, for the forms the index uses."""
    c = constraint.strip()
    if not c or c == "*":
        return True
    if "," in c:
        return all(satisfies(version, p) for p in c.split(","))
    v = _key(version)
    for op in (">=", "<=", ">", "<"):
        if c.startswith(op):
            b = _key(c[len(op):])
            return {">=": v >= b, "<=": v <= b, ">": v > b, "<": v < b}[op]
    if c[0] in "^~":
        parts = _key(c[1:]) + (0, 0)
        lo, hi = parts[:3], (parts[0], parts[1] + 1, 0)
        return lo <= v + (0,) * (3 - len(v)) < hi
    return v == _key(c.lstrip("="))


def line_floor(constraint):
    """`registry_index::api_line_floor`: the lower bound of `>=`, `^` or `~`."""
    if re.fullmatch(r"\s*=?\s*[\d.]+\s*", constraint):
        return None
    for part in constraint.split(","):
        m = re.fullmatch(r"\s*(?:>=|\^|~)\s*([\d.]+)\s*", part)
        if m:
            return _key(m.group(1))
    return None


def rust_only(manifest):
    """A `[native] crate` with no C part beside it."""
    crate = re.search(r"(?ms)^\[native\][^\[]*?^\s*crate\s*=", manifest)
    c_part = re.search(r"(?m)^\[c\]|^\[\[?components|^\s*prebuilt\s*=", manifest)
    return bool(crate) and not c_part


def manifest_of(name, version, entry, cache_dir):
    """The release's loft.toml: the local registry cache first, else its tarball."""
    local = os.path.expanduser(f"~/.loft/registry/{name}-{version}/loft.toml")
    if os.path.exists(local):
        return open(local, encoding="utf-8", errors="replace").read()
    saved = os.path.join(cache_dir, f"{name}-{version}.loft.toml")
    if os.path.exists(saved):
        return open(saved, encoding="utf-8", errors="replace").read()
    try:
        with urllib.request.urlopen(entry["url"], timeout=120) as r:
            data = r.read()
    except OSError:
        return None
    sub = (entry.get("subpath") or "").strip("/")
    want = lambda path: path.endswith("loft.toml") and (not sub or f"{sub}/loft.toml" in path)
    text = None
    try:
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:*") as t:
            tomls = [m for m in t.getmembers() if m.isfile() and want(m.name)]
            if tomls:
                best = min(tomls, key=lambda m: m.name.count("/"))
                text = t.extractfile(best).read().decode("utf-8", "replace")
    except tarfile.TarError:
        try:
            with zipfile.ZipFile(io.BytesIO(data)) as z:
                tomls = [n for n in z.namelist() if want(n)]
                if tomls:
                    text = z.read(min(tomls, key=lambda n: n.count("/"))).decode("utf-8", "replace")
        except zipfile.BadZipFile:
            return None
    if text is None:
        return None
    open(saved, "w", encoding="utf-8").write(text)
    return text


def deprecations(packages, manifest):
    """{(name, version): reason} for every release to deprecate.  `manifest(name, version)`
    answers a release's loft.toml text."""
    out = {}
    for name, pkg in packages.items():
        yanked = set(pkg.get("yanked") or []) if isinstance(pkg.get("yanked"), list) else set()
        for v in pkg["versions"]:
            if v in yanked or "-" in v:
                continue
            text = manifest(name, v)
            if text is None:
                UNREAD.append(f"{name}@{v}")
            elif rust_only(text):
                out[(name, v)] = RUST_ONLY
    return out


def _self_test():
    pkgs = {
        "g": {"yanked": [], "versions": {
            "0.9.0": {}, "0.9.12": {"api_compatible_with": "0.4.2"},
            "0.12.1": {"api_compatible_with": "0.12.0"}}},
        "a": {"yanked": [], "versions": {
            "0.1.3": {"deps": {"g": ">=0.9.0"}}, "0.2.0": {"deps": {"g": ">=0.12.0"}}}},
        "p": {"yanked": [], "versions": {"1.0.0": {"deps": {"g": "0.9.12"}}}},
    }
    texts = {("g", "0.9.0"): '[native]\ncrate = "x"\n', ("g", "0.9.12"): '[native]\ncrate = "x"\n',
             ("g", "0.12.1"): '[c]\nshim = "c/g.c"\n'}
    got = deprecations(pkgs, lambda n, v: texts.get((n, v), "[package]\n"))
    cases = [
        ("a Rust-only release is deprecated", ("g", "0.9.12") in got),
        ("a release with a C part is not", ("g", "0.12.1") not in got),
        ("a dependant is not inferred deprecated", ("a", "0.1.3") not in got),
        ("nor one pinned onto a deprecated release", ("p", "1.0.0") not in got),
        ("a crate beside a [c] shim is not Rust only",
         not rust_only('[native]\ncrate = "x"\n[c]\nshim = "s.c"\n')),
        ("a [native] table with no crate is not Rust only", not rust_only('[native]\n')),
        ("^ bounds the minor", satisfies("0.1.9", "^0.1") and not satisfies("0.2.0", "^0.1")),
    ]
    bad = [label for label, ok in cases if not ok]
    for label in bad:
        print(f"  FAIL {label}")
    print(f"  registry_deprecate self-test: {len(cases) - len(bad)}/{len(cases)} cases pass")
    return 1 if bad else 0


def main(argv):
    if argv[1:2] == ["--self-test"]:
        return _self_test()
    if len(argv) < 2:
        sys.stderr.write(__doc__)
        return 2
    path = argv[1]
    write = "--write" in argv
    cache = (argv[argv.index("--manifests") + 1] if "--manifests" in argv
             else tempfile.mkdtemp(prefix="loft_manifests_"))
    os.makedirs(cache, exist_ok=True)
    index = json.load(open(path, encoding="utf-8"))
    packages = index["packages"]
    found = deprecations(
        packages, lambda n, v: manifest_of(n, v, packages[n]["versions"][v], cache))
    by_reason = {}
    for (n, v), why in sorted(found.items(), key=lambda kv: (kv[0][0], _key(kv[0][1]))):
        by_reason.setdefault(why, []).append(f"{n}@{v}")
    for why, rels in by_reason.items():
        print(f"{len(rels)} deprecated — {why}:")
        print("  " + ", ".join(rels))
    if UNREAD:
        print(f"{len(UNREAD)} release(s) whose manifest could not be read (not deprecated): "
              + ", ".join(UNREAD))
    for n, pkg in sorted(packages.items()):
        live = [v for v in pkg["versions"] if "-" not in v and (n, v) not in found]
        if pkg["versions"] and not live:
            print(f"warning: every release of {n} would be deprecated — it has nothing to install")
    if write:
        for (n, v), why in found.items():
            packages[n]["versions"][v]["deprecated"] = why
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(json.dumps(index, indent=2, ensure_ascii=False) + "\n")
        print(f"wrote {len(found)} deprecation(s) into {path}")
        expects = " ".join(f"--expect {n}@{v}" for n, v in sorted(found, key=lambda k: (k[0], _key(k[1]))))
        print(f"sign it, once it is a registry PR: scripts/registry-sign.sh --pr <N> {expects}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
