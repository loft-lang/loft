#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN112 phase 3 — the per-context OVERLAY: show one library's API across every source
# that exists ON THIS MACHINE / in THIS project, labelled by provenance. On demand, to
# stdout — NEVER committed (git carries only the shared `published`+`unreleased` catalogue;
# a dev's working checkout and a project's pin are machine-/context-specific).
#
#   scripts/lib-overlay.py <libname> [--local DIR] [--lock PATH|--project DIR]
#                                    [--dev-root DIR] [--no-local] [--no-pinned]
#
# Sources unioned (each shown only when present):
#   ✓ published   — registry latest         (committed snapshot; the installable API)
#   🟢 unreleased — the lib's origin/main    (committed snapshot; the correct CURRENT state)
#   🔶 local      — a dev WORKING CHECKOUT   (discovered under --dev-root, or --local DIR)
#   📌 pinned     — what THIS project's loft.lock resolves to (its callable-here API)
#
# published + unreleased come from the committed snapshots (shared truth, same data the
# catalogue renders); local + pinned are extracted live the SAME way every source is
# (`loft api <dir> --json` = pkg_api_items). Functions are keyed + diffed by TYPE signature
# (a param rename is not a change; `&T`->`T` is) — matching api_diff / phase 4/5 — and each
# source pair carries the compatibility verdict (⚠ BREAKING where a published fn is
# removed/changed). Nothing is auto-deleted; this only reports.
#
# The `proposed` source is the sibling overlay tool: scripts/proposal-review.py.
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
SNAPSHOT = REPO / "doc" / "claude" / "registry-index-snapshot.json"
UNRELEASED = REPO / "doc" / "claude" / "unreleased-snapshot.json"
LOFT = REPO / "target" / "release" / "loft"
INSTALLED = Path.home() / ".loft" / "registry"
DEV_ROOT = Path(os.environ.get("LOFT_DEV_ROOT", str(Path.home() / "workspace")))

TAG = {"published": "✓ published", "unreleased": "🟢 unreleased", "local": "🔶 local", "pinned": "📌 pinned"}
ORDER = ["published", "unreleased", "local", "pinned"]


def type_sig(sig: str) -> str:
    """Comparison key ignoring param NAMES (matches api_diff / proposal-review): a param
    rename is not a change, a real type change (`&T`->`T`) is."""
    s = " ".join((sig or "").split())
    m = re.match(r"pub (?:fn|operator) (\w+)\s*\((.*)\)(.*)$", s)
    if not m:
        return s
    name, params, ret = m.group(1), m.group(2), m.group(3).strip()
    parts, depth, cur = [], 0, ""
    for ch in params:
        if ch in "<([":
            depth += 1
        elif ch in ">)]":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append(cur)
            cur = ""
        else:
            cur += ch
    if cur.strip():
        parts.append(cur)
    typ = lambda p: (p.split(":", 1)[1] if ":" in p else p).split("=", 1)[0].strip()
    return f"{name}(" + ",".join(typ(p) for p in parts) + f"){ret}"


def semver_key(v: str) -> list[int]:
    return [int(x) for x in re.split(r"[.\-]", v) if x.isdigit()]


def extract_api(pkg_root: str) -> list[dict]:
    """Extract a package's public API the same way every source is — `loft api <root> --json`
    (root = the dir holding loft.toml + src/)."""
    env = {**os.environ, "LOFT_TIMEOUT": os.environ.get("LOFT_TIMEOUT", "60")}
    r = subprocess.run([str(LOFT), "api", pkg_root, "--json"], capture_output=True, text=True, env=env)
    try:
        return json.loads(r.stdout)
    except json.JSONDecodeError:
        return []


# --- source resolvers -------------------------------------------------------------------

def published_api(libname: str, index: dict) -> tuple[list[dict], str]:
    try:
        vs = index["packages"][libname]["versions"]
        latest = max(vs, key=semver_key)
        return vs[latest].get("api") or [], latest
    except (KeyError, ValueError):
        return [], ""


def repo_subpath(homepage: str) -> tuple[str, str]:
    """github.com/<owner>/<repo>/tree/main/<subpath> -> (repo, subpath)."""
    rest = (homepage or "").removeprefix("https://github.com/")
    owner_repo = rest.split("/tree/main/", 1)[0].rstrip("/")
    subpath = rest.split("/tree/main/", 1)[1] if "/tree/main/" in rest else ""
    repo = owner_repo.split("/", 1)[1] if "/" in owner_repo else owner_repo
    return repo, subpath


def find_local(libname: str, index: dict, explicit: str | None) -> tuple[str, str] | None:
    """Locate a dev WORKING CHECKOUT for the lib (a package root with loft.toml + src/).
    --local DIR wins; else derive candidates from the registry homepage (repo/subpath under
    --dev-root) and the per-lib convention loft-lib-<name>."""
    def ok(d: Path) -> bool:
        return (d / "loft.toml").is_file() and (d / "src").is_dir()

    if explicit:
        d = Path(explicit).expanduser()
        return (str(d), str(d)) if ok(d) else None

    cands: list[Path] = []
    hp = ((index.get("packages", {}).get(libname) or {}).get("homepage") or "").strip()
    if hp:
        repo, subpath = repo_subpath(hp)
        base = DEV_ROOT / repo
        cands += [base / subpath, base]  # multi-lib repo (repo/<subpath>) or single-lib repo
    cands += [DEV_ROOT / f"loft-lib-{libname}", DEV_ROOT / libname]
    for d in cands:
        if ok(d):
            return str(d), str(d)
    return None


def find_lock(lock: str | None, project: str | None) -> Path | None:
    if lock:
        p = Path(lock).expanduser()
        return p if p.is_file() else None
    start = Path(project).expanduser() if project else Path.cwd()
    for d in [start, *start.parents]:
        p = d / "loft.lock"
        if p.is_file():
            return p
    return None


def pinned_api(libname: str, lockpath: Path) -> tuple[list[dict], str, str]:
    """Read loft.lock, find the lib's pinned version, extract the INSTALLED copy's API.
    Returns (api, version, note). note != '' means pinned-but-not-installed."""
    ver, name = "", ""
    for raw in lockpath.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if line == "[[package]]":
            name, ver = "", ""
        elif line.startswith("name"):
            name = line.split("=", 1)[1].strip().strip('"')
        elif line.startswith("version"):
            v = line.split("=", 1)[1].strip().strip('"')
            if name == libname:
                ver = v
                break
    if not ver:
        return [], "", "not pinned in this lockfile"
    inst = INSTALLED / f"{libname}-{ver}"
    if not (inst / "loft.toml").is_file():
        return [], ver, f"pinned {ver} but not installed at {inst}"
    return extract_api(str(inst)), ver, ""


# --- rendering --------------------------------------------------------------------------

def compat(old: dict[str, dict], new: dict[str, dict]) -> list[str]:
    """api_diff rule: broken = published/old type-sigs absent from new. Returns broken keys."""
    return [k for k in old if k not in new]


def main() -> int:
    global DEV_ROOT
    ap = argparse.ArgumentParser(description="@PLN112 phase 3 — per-context library-API overlay")
    ap.add_argument("libname")
    ap.add_argument("--local", help="explicit dev-checkout package root (overrides discovery)")
    ap.add_argument("--lock", help="explicit loft.lock path (else auto-discover from --project/cwd)")
    ap.add_argument("--project", help="project dir to auto-discover loft.lock from (default: cwd)")
    ap.add_argument("--dev-root", help=f"root to scan for a dev checkout (default: {DEV_ROOT})")
    ap.add_argument("--no-local", action="store_true")
    ap.add_argument("--no-pinned", action="store_true")
    args = ap.parse_args()

    if args.dev_root:
        DEV_ROOT = Path(args.dev_root).expanduser()

    lib = args.libname
    index = json.loads(SNAPSHOT.read_text(encoding="utf-8")) if SNAPSHOT.exists() else {"packages": {}}
    unrel = json.loads(UNRELEASED.read_text(encoding="utf-8")) if UNRELEASED.exists() else {}

    # assemble present sources -> {type_sig: item}
    sources: dict[str, dict[str, dict]] = {}
    meta: dict[str, str] = {}

    data_gap = ""
    pub, pver = published_api(lib, index)
    if pub:
        sources["published"] = {type_sig(i.get("sig", "")): i for i in pub}
        meta["published"] = f"v{pver}"
    elif lib in index.get("packages", {}) and pver:
        # In the registry but its `api` field is empty — the known phase-2 registry data
        # gap. Say so, rather than silently omitting `published`.
        data_gap = f"published v{pver} exists but records no API (registry data gap — see @PLN112 phase 2)"
    unrel_api = (unrel.get(lib) or {}).get("api") or []
    if unrel_api:
        sources["unreleased"] = {type_sig(i.get("sig", "")): i for i in unrel_api}
        meta["unreleased"] = "origin/main"

    if not args.no_local:
        loc = find_local(lib, index, args.local)
        if loc:
            api = extract_api(loc[0])
            if api:
                sources["local"] = {type_sig(i.get("sig", "")): i for i in api}
                meta["local"] = loc[1].replace(str(Path.home()), "~")

    pin_note = ""
    if not args.no_pinned:
        lockpath = find_lock(args.lock, args.project)
        if lockpath:
            api, pinver, note = pinned_api(lib, lockpath)
            if api:
                sources["pinned"] = {type_sig(i.get("sig", "")): i for i in api}
                meta["pinned"] = f"v{pinver} ({lockpath.parent.name}/loft.lock)"
            elif note:
                pin_note = f"{note} (lock: {lockpath})"

    present = [s for s in ORDER if s in sources]
    print(f"# {lib} — provenance overlay (@PLN112 phase 3, on-demand — not committed)")
    if not present:
        print(f"  no sources found for '{lib}'." + (f"  {pin_note}" if pin_note else ""))
        print("  (published/unreleased come from the committed snapshots; local needs a dev")
        print(f"   checkout under {str(DEV_ROOT).replace(str(Path.home()), '~')}; pinned needs a loft.lock.)")
        return 0
    print("  sources: " + " · ".join(f"{TAG[s]} {meta.get(s, '')}".strip() for s in present))
    for n in (data_gap, pin_note):
        if n:
            print(f"  note: {n}")

    # union of every function, keyed by type-sig, tagged by membership
    all_keys: dict[str, dict] = {}
    for s in present:
        for k, item in sources[s].items():
            all_keys.setdefault(k, item)

    identical = all(set(sources[s]) == set(all_keys) for s in present)
    if len(present) == 1 or identical:
        # PLAIN list — a single source, or every source agrees.
        note = "single source" if len(present) == 1 else f"{len(present)} sources agree"
        print(f"\n  API ({len(all_keys)} fns · {note}):")
        for k in sorted(all_keys):
            print(f"    - `{all_keys[k].get('sig', '').strip()}`")
    else:
        # TAGGED interleave — show each fn's source membership; mark the divergent ones.
        print(f"\n  API ({len(all_keys)} fns · sources diverge — tagged by provenance):")
        for k in sorted(all_keys):
            member = [s for s in present if k in sources[s]]
            sig = all_keys[k].get("sig", "").strip()
            if len(member) == len(present):
                print(f"    - `{sig}`")
            else:
                badges = " ".join(TAG[s] for s in member)
                miss = ", ".join(s for s in present if s not in member)
                print(f"    - `{sig}`   [{badges}]  (not in: {miss})")

    # api-compat verdicts on the meaningful pairs (old -> new).
    pairs = [("published", "unreleased"), ("published", "local"), ("pinned", "published")]
    lines = []
    for old, new in pairs:
        if old in sources and new in sources:
            broken = compat(sources[old], sources[new])
            added = [k for k in sources[new] if k not in sources[old]]
            verdict = f"⚠ {len(broken)} BREAKING" if broken else "OK (additive)"
            extra = f", +{len(added)} added" if added else ""
            lines.append(f"    {TAG[old]} → {TAG[new]}: {verdict}{extra}")
            for k in broken:
                lines.append(f"        ⚠ `{sources[old][k].get('sig', '').strip()}` removed/changed")
    if lines:
        print("\n  api-compat (api_diff rule — identical-or-added is compatible):")
        print("\n".join(lines))
    print("\n  (overlay only — nothing deleted; `local`/`pinned` are never committed. For an")
    print("   external candidate use `scripts/proposal-review.py <lib> <ref>`.)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
