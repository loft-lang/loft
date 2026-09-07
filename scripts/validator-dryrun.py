#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Run loft-lang/registry's OWN validator against this release's entry, before tag day
decides anything.

@PLN156 phase 3.  The 2026.8.0 submission was rejected by the registry's validator
because nobody had ever run it against what a loft release generates -- its gate 3
re-packages a source tree with `loft package`, and loft's repo root has no `loft.toml`
(fixed downstream as registry#22: gate 2b + a narrow gate-3 exemption).  That
incompatibility was discoverable months before the release; this script is the asking.

Two modes, picked by what exists (the mode is always PRINTED -- a structural pass and a
full pass are different claims and must never read as one):

  full        the release's own `loft-<v>-registry-entry.json` (downloaded from the
              GitHub release via `gh`, so a still-draft release works) is spliced into
              a fresh clone of the live index, and the registry's `tools/validate.py`
              runs ALL its gates -- including gate 2/2b's real downloads, which need
              the assets PUBLISHED.  This is the rehearsal of M-registry-splice.
  structural  no release exists yet: a synthetic entry is generated from dummy zips by
              scripts/gen-toolchain-entry.py (exercising the real generator and the
              real splice code), and only the gates that need no published artifact run
              (schema/docs, trigger uniqueness).  Gates 2/2b/3 are reported SKIPPED,
              loudly.  Catches shape, docs and trigger drift months early; it is NOT
              the full verdict, and the exit code says so.

`--replay` re-runs the full gates for a version ALREADY in the index by pointing the
clone's origin/main ref at the commit before its splice, so the validator sees it as
new again.  That is how this instrument is falsified against a known-good release --
and, with `--corrupt`, how it is shown able to go red (one sha256 flipped in the
spliced entry must fail gate 2b).

Exit codes (scripts/release-checklist.py reads them apart):
  0  full validation passed (or --replay passed)
  3  structural-only pass -- the full run still owes its answer
  4  the version is already in the live index and --replay was not asked: nothing to
     dry-run; the live validator already covered it
  1  a gate that could run answered wrong
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
REGISTRY_URL = os.environ.get(
    "LOFT_REGISTRY_REPO", "https://github.com/loft-lang/registry"
)


def sh(*args: str, cwd: pathlib.Path | None = None, timeout: int = 600) -> tuple[int, str]:
    try:
        p = subprocess.run(
            list(args), cwd=cwd, capture_output=True, text=True, timeout=timeout
        )
        return p.returncode, (p.stdout + p.stderr).strip()
    except FileNotFoundError:
        return 127, f"{args[0]}: not installed"
    except subprocess.TimeoutExpired:
        return 124, f"{args[0]}: timed out after {timeout}s"


def clone_registry(work: pathlib.Path) -> pathlib.Path:
    dest = work / "registry"
    code, out = sh("git", "clone", "--quiet", REGISTRY_URL, str(dest), timeout=300)
    if code != 0:
        sys.exit(f"validator-dryrun: cannot clone {REGISTRY_URL}: {out}")
    return dest


def merge_entry(index_path: pathlib.Path, entry_file: pathlib.Path, ver: str) -> None:
    """The same merge M-registry-splice performs, on the clone: versions EXTEND, and an
    existing version refuses -- mirroring gen-toolchain-entry.py's splice()."""
    index = json.loads(index_path.read_text())
    wrapper = json.loads(entry_file.read_text())
    entry = wrapper.get("loft") or wrapper  # the release attaches {"loft": {...}}
    packages = index.setdefault("packages", {})
    existing = packages.get("loft")
    if existing:
        prior = existing.get("versions", {})
        if ver in prior:
            sys.exit(f"validator-dryrun: `loft` {ver} is already in the cloned index")
        entry["versions"] = {**prior, **entry["versions"]}
        entry["yanked"] = existing.get("yanked", [])
    packages["loft"] = entry
    published = entry["versions"][ver]["published"]
    index["updated"] = max(index.get("updated", ""), published)
    index_path.write_text(json.dumps(index, indent=2, ensure_ascii=False) + "\n")


def dummy_release_dir(work: pathlib.Path, ver: str) -> pathlib.Path:
    """Release-shaped zips with throwaway content, so gen-toolchain-entry.py produces a
    structurally REAL entry (real generator, real splice) whose hashes name nothing."""
    sys.path.insert(0, str(ROOT / "scripts"))
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "gen_toolchain_entry", ROOT / "scripts" / "gen-toolchain-entry.py"
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    art = work / "artifacts"
    art.mkdir()
    for triple in mod.published_triples():
        zp = art / f"loft-{ver}-{triple}.zip"
        with zipfile.ZipFile(zp, "w") as z:
            z.writestr(f"loft-{ver}-{triple}/bin/loft", "#!/bin/sh\n")
            z.writestr(f"loft-{ver}-{triple}/SHA256SUMS", "00  bin/loft\n")
    with zipfile.ZipFile(art / f"loft-{ver}-src.zip", "w") as z:
        z.writestr("loft/README.md", "dry-run source archive\n")
    return art


def run_structural_gates(reg: pathlib.Path) -> None:
    """The registry's own gate functions, for the gates that need no published asset.
    Imported from the CLONE so a validator change there is what runs here."""
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "registry_validate", reg / "tools" / "validate.py"
    )
    mod = importlib.util.module_from_spec(spec)
    cwd = os.getcwd()
    os.chdir(reg)  # validate.py resolves index.json relative to the repo root
    try:
        spec.loader.exec_module(mod)
        idx = mod.load_index()
        print("[gate 1] schema lint (registry's own)")
        mod.gate_schema(idx)
        print("[gate 4] trigger uniqueness (registry's own)")
        mod.gate_trigger_uniqueness(idx)
    finally:
        os.chdir(cwd)
    print("[gate 2] tarball verify — SKIPPED (no published assets yet; full mode runs it)")
    print("[gate 2b] binary verify — SKIPPED (no published assets yet; full mode runs it)")
    print("[gate 3] reproducible build — SKIPPED (no published assets yet; full mode runs it)")


def splice_commit_parent(reg: pathlib.Path, ver: str) -> str:
    """For --replay: the commit BEFORE the one that added `loft` <ver> to the index."""
    code, out = sh(
        "git", "log", "--format=%H", "-S", f'"{ver}"', "--", "index.json", cwd=reg
    )
    if code != 0 or not out.strip():
        sys.exit(f"validator-dryrun: cannot find the commit that spliced {ver}")
    splice_sha = out.splitlines()[0].strip()
    code, parent = sh("git", "rev-parse", f"{splice_sha}^", cwd=reg)
    if code != 0:
        sys.exit(f"validator-dryrun: cannot resolve the parent of {splice_sha}")
    return parent.strip()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--version", required=True)
    ap.add_argument("--replay", action="store_true",
                    help="re-validate a version already in the index, as if new")
    ap.add_argument("--corrupt", action="store_true",
                    help="with --replay: flip one binary sha256 first — the run MUST go red "
                         "(the instrument's own falsification)")
    ap.add_argument("--keep", action="store_true", help="keep the work directory")
    args = ap.parse_args()
    ver = args.version

    work = pathlib.Path(tempfile.mkdtemp(prefix="loft-validator-dryrun-"))
    try:
        reg = clone_registry(work)
        index_path = reg / "index.json"
        index = json.loads(index_path.read_text())
        live = index.get("packages", {}).get("loft", {}).get("versions", {})

        if ver in live and not args.replay:
            print(f"validator-dryrun: loft {ver} is ALREADY in the signed index — the "
                  "live validator covered it; nothing to dry-run (--replay re-runs it)")
            return 4

        if args.replay:
            if ver not in live:
                sys.exit(f"validator-dryrun: --replay needs {ver} in the live index")
            parent = splice_commit_parent(reg, ver)
            sh("git", "update-ref", "refs/remotes/origin/main", parent, cwd=reg)
            print(f"mode: full (replay — origin/main rewound to {parent[:12]} so "
                  f"{ver} reads as new)")
            if args.corrupt:
                idx = json.loads(index_path.read_text())
                vobj = idx["packages"]["loft"]["versions"][ver]
                triple = sorted(vobj["binaries"])[0]
                vobj["binaries"][triple]["sha256"] = "0" * 64
                index_path.write_text(json.dumps(idx, indent=2, ensure_ascii=False) + "\n")
                print(f"corrupted: binaries[{triple}].sha256 := 0*64 — gate 2b must refuse")
        else:
            # Is the release's own entry downloadable?  `gh` reaches a draft too.
            code, out = sh(
                "gh", "release", "download", f"v{ver}",
                "-R", "loft-lang/loft",
                "-p", f"loft-{ver}-registry-entry.json",
                "-D", str(work), timeout=120,
            )
            entry_file = work / f"loft-{ver}-registry-entry.json"
            if code == 0 and entry_file.is_file():
                print("mode: full (the release's own generated entry)")
                merge_entry(index_path, entry_file, ver)
            else:
                print(f"mode: structural (no release entry for v{ver} yet: {out.splitlines()[-1] if out else code})")
                print("      a structural pass is NOT the full verdict — rerun after the draft exists")
                art = dummy_release_dir(work, ver)
                code, out = sh(
                    sys.executable, str(ROOT / "scripts" / "gen-toolchain-entry.py"),
                    "--version", ver, "--dir", str(art),
                    "--published", "2026-01-01T00:00:00Z",
                    "--splice-into", str(index_path),
                )
                if code != 0:
                    print(out)
                    print("validator-dryrun: FAIL — gen-toolchain-entry.py refused the splice")
                    return 1
                run_structural_gates(reg)
                print("validator-dryrun: structural gates PASSED (exit 3 — not the full verdict)")
                return 3

        # Full validation: the registry's entrypoint, unmodified, in its own repo.
        code, out = sh(sys.executable, str(reg / "tools" / "validate.py"), cwd=reg,
                       timeout=1800)
        print(out)
        if code != 0:
            print(f"validator-dryrun: FAIL — the registry's validator refused loft {ver}")
            return 1
        print(f"validator-dryrun: PASS — the registry's validator accepts loft {ver} (all gates)")
        return 0
    finally:
        if args.keep:
            print(f"work directory kept: {work}")
        else:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
