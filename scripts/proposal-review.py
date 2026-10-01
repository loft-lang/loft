#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN112 phase 4 — review an external PROPOSAL for a library against its published API,
# on demand (the machine/context overlay tier — never committed).
#
#   scripts/proposal-review.py <libname> <ref>
#
# <ref> — how a contributor hands us the proposal:
#   /a/local/dir          a checked-out lib dir (e.g. ~/workspace/loft-lib-mariadb)
#   owner/repo@branch      fetch the lib's src/ from that ref via `gh`
#   (registry PR# / issue# resolve to one of the above — TODO, phase-4 follow-on:
#    a registry PR's added index entry -> repo@tag; an issue -> its proposed-API sigs.)
#
# The proposal's API is extracted the SAME way every source is (`loft api --json`), diffed
# against `published` (from the committed registry snapshot) by TYPE signature, and printed
# as a `🌱 proposed` overlay with the api-compat verdict + a delta-vs-rewrite summary.
# Fit-to-direction is a HUMAN judgment the tool surfaces but never makes (see the footer).
from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
SNAPSHOT = REPO / "doc" / "claude" / "registry-index-snapshot.json"
LOFT = REPO / "target" / "release" / "loft"


def gh(args: list[str]) -> str:
    r = subprocess.run(["gh", "api", *args], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else ""


def type_sig(sig: str) -> str:
    """Comparison key ignoring param NAMES (matches api_diff): a param rename is not a change,
    a real type change (`&T`->`T`) is."""
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


def extract_api(src_dir: str) -> list[dict]:
    r = subprocess.run([str(LOFT), "api", src_dir, "--json"], capture_output=True, text=True)
    try:
        return json.loads(r.stdout)
    except json.JSONDecodeError:
        return []


def resolve_ref(ref: str) -> tuple[str, str] | None:
    """Return (source_dir, label). A local dir is used in place; `owner/repo@branch` is
    fetched into a temp dir (the caller keeps it alive)."""
    if os.path.isdir(ref):
        return ref, f"dir:{ref}"
    m = re.match(r"^([^/]+/[^/@]+)(?:/(.+))?@(.+)$", ref)  # owner/repo[/subpath]@branch
    if m:
        owner_repo, subpath, branch = m.group(1), m.group(2) or "", m.group(3)
        srcdir = f"{subpath}/src" if subpath else "src"
        names = [
            n
            for n in gh([f"repos/{owner_repo}/contents/{srcdir}?ref={branch}", "--jq", ".[].name"]).splitlines()
            if n.endswith(".loft")
        ]
        if not names:
            return None
        td = tempfile.mkdtemp(prefix="pln112-proposal-")
        os.makedirs(f"{td}/src", exist_ok=True)
        for n in names:
            raw = gh([f"repos/{owner_repo}/contents/{srcdir}/{n}?ref={branch}", "-H", "Accept: application/vnd.github.raw"])
            Path(f"{td}/src/{n}").write_text(raw, encoding="utf-8")
        Path(f"{td}/loft.toml").write_text('name = "probe"\nversion = "0.0.0"\n', encoding="utf-8")
        return td, f"{owner_repo}@{branch}"
    return None


def published_api(libname: str) -> list[dict]:
    try:
        idx = json.loads(SNAPSHOT.read_text(encoding="utf-8"))
        vs = idx["packages"][libname]["versions"]
        latest = max(vs, key=lambda s: [int(x) for x in re.split(r"[.\-]", s) if x.isdigit()])
        return vs[latest].get("api") or []
    except (KeyError, ValueError):
        return []


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: proposal-review.py <libname> <ref>", file=sys.stderr)
        return 2
    libname, ref = sys.argv[1], sys.argv[2]
    resolved = resolve_ref(ref)
    if not resolved:
        print(f"proposal-review: could not resolve <ref> '{ref}' (want a lib dir or owner/repo@branch)", file=sys.stderr)
        return 1
    src_dir, label = resolved

    proposed = extract_api(src_dir)
    published = published_api(libname)
    pub_t = {type_sig(i.get("sig", "")): i for i in published}
    prop_t = {type_sig(i.get("sig", "")): i for i in proposed}

    new = [i for k, i in prop_t.items() if k not in pub_t]        # 🌱 proposed additions
    kept = [i for k, i in prop_t.items() if k in pub_t]           # unchanged
    broken = [i for k, i in pub_t.items() if k not in prop_t]     # ⚠ removed/changed by the proposal

    # rewrite vs delta: a proposal that shares little with published (and is not a marked
    # rename) reads as a rewrite; otherwise a delta. (Marker linkage is a phase-5-adjacent
    # refinement; here we use overlap when there IS a published API.)
    is_new_lib = not published
    shape = "NEW LIBRARY" if is_new_lib else ("rewrite" if kept and len(kept) < len(broken) else "delta")

    print(f"{libname}  —  proposal {label}  vs  published ({'none — new lib' if is_new_lib else f'{len(published)} fns'})")
    verdict = "OK (additive)" if not broken else f"⚠ BREAK — {len(broken)} published fn(s) removed/changed"
    print(f"  api-compat: {verdict}")
    print(f"  delta: +{len(new)} proposed · {len(kept)} kept · {len(broken)} removed   [{shape}]")
    print("  API:")
    for i in kept:
        print(f"    - `{i.get('sig','').strip()}`")
    for i in new:
        print(f"    - `{i.get('sig','').strip()}`  🌱 proposed")
    for i in broken:
        print(f"    - `{i.get('sig','').strip()}`  ⚠ BREAKING (published, removed/changed by the proposal)")
    print()
    print("  ⚠ Fit-to-direction is a HUMAN judgment the tool does NOT make: does this serve the")
    print("     ENVISIONED use case (e.g. @PLN23 for a DB client), or only work for many? The view")
    print("     shows the API + compat verdict; you decide fit, and never auto-adopt (@PLN112).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
