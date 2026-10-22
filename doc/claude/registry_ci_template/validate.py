#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later

"""PKG.REG R9 — `loft-lang/registry` PR validation.

Drop this file at `tools/validate.py` in the `loft-lang/registry`
repo.  Wired in by `.github/workflows/pr-validate.yml` (also in this
template directory).

Four gates per PR:

1. **Schema lint** — every package + version row has the required
   fields, types match, schema_version is unchanged; AND every package
   carries correct docs (non-empty description, non-empty categories,
   http(s) homepage) so the auto-generated library catalogue can never be
   incomplete — a doc-less library is invisible and gets reimplemented.
2. **Tarball verify** — download every newly-added `versions.<v>.url`,
   hash it, compare to the PR's declared sha256.  Reject on
   mismatch.  Caught: publisher pasted wrong hash, tarball was
   re-uploaded after PR opened, opportunistic supply-chain swap.
3. **Reproducible-build re-check** — for each newly-added version
   whose homepage points at a public GitHub repo, clone the tag,
   run `loft package`, compare the produced sha256 to the PR's
   claim.  Caught: source repo's tag points at different bytes
   than the uploaded release tarball.
   Also re-derives the `api` field from the cloned source and
   rejects a pasted one that disagrees, so `loft search` can never
   point at a function the source does not have.

4. **Trigger uniqueness** — every Tier-1 `method:receiver` trigger
   must be owned by at most one package across the whole registry.
   A consumer auto-loads a library from a bare `obj.method()` call,
   so two packages claiming `text.matches` would make the auto-load
   ambiguous.  Caught: a new library claiming a method-on-type
   trigger another package already owns.

Exits 0 on all-pass; non-zero with line-prefixed errors on any
failure.  The workflow surfaces those lines as PR comments.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

INDEX_PATH = Path("index.json")
SCHEMA_VERSION = 1


def fail(msg: str) -> None:
    print(f"::error::{msg}")
    sys.exit(1)


def load_index() -> dict:
    if not INDEX_PATH.exists():
        fail(f"{INDEX_PATH} not found")
    with INDEX_PATH.open(encoding="utf-8") as f:
        try:
            return json.load(f)
        except json.JSONDecodeError as e:
            fail(f"{INDEX_PATH}: invalid JSON: {e}")


def load_previous_index() -> dict:
    """Read the version of index.json on `main` so we can find the
    NEW entries (ones the PR adds)."""
    try:
        out = subprocess.check_output(
            ["git", "show", "origin/main:index.json"], text=True
        )
        return json.loads(out)
    except subprocess.CalledProcessError:
        # First publish — main may not have index.json yet.
        return {"schema_version": SCHEMA_VERSION, "packages": {}}


def gate_schema(idx: dict) -> None:
    if idx.get("schema_version") != SCHEMA_VERSION:
        fail(
            f"schema_version must be {SCHEMA_VERSION}, "
            f"got {idx.get('schema_version')!r}"
        )
    if "packages" not in idx or not isinstance(idx["packages"], dict):
        fail("`packages` must be an object")
    for name, pkg in idx["packages"].items():
        if "versions" not in pkg or not isinstance(pkg["versions"], dict):
            fail(f"package `{name}`: missing or non-object `versions`")
        # Docs gate — a library whose docs are missing or wrong is invisible in
        # the auto-generated catalogue (loft `doc/claude/LIBRARIES.md`), so agents
        # never find it and reimplement it (duplicate code, new bugs).  Reject a
        # package that does not carry correct per-package docs.
        desc = pkg.get("description")
        if not isinstance(desc, str) or len(desc.strip()) < 10:
            fail(
                f"package `{name}`: `description` missing or too short — it is the "
                f"library's one-line docs in the catalogue; write a real summary"
            )
        cats = pkg.get("categories")
        if not isinstance(cats, list) or not cats or not all(
            isinstance(c, str) and c.strip() for c in cats
        ):
            fail(
                f"package `{name}`: `categories` must be a non-empty list of "
                f"non-empty strings (it groups the library in the catalogue)"
            )
        hp = pkg.get("homepage")
        if not isinstance(hp, str) or not hp.strip().lower().startswith("http"):
            fail(
                f"package `{name}`: `homepage` must be an http(s) URL "
                f"(the catalogue links to it for the full API docs)"
            )
        # `yanked` is a LIST of yanked version strings — never a bool.  A hand
        # publish wrote `false` into eleven hex_* packages, and loft's nightly
        # registry-validation does `.yanked | length` to pick what to test: jq
        # dies on a boolean, so `discover` failed in 3s and the whole sweep was
        # SKIPPED.  Every published package went unvalidated for days while the
        # run merely looked red.  Typed here so bad data can never reach it.
        yanked = pkg.get("yanked", [])
        if not isinstance(yanked, list) or not all(
            isinstance(v, str) for v in yanked
        ):
            fail(
                f"package `{name}`: `yanked` must be a list of version strings "
                f"(got {yanked!r}) — use [] when nothing is yanked"
            )
        # Deliberately NOT checked: that each yanked version still has a
        # `versions` row.  Yanking REMOVES the row and records the string here
        # (PKG_REGISTRY.md § Yanking), so requiring one would reject correct data
        # — `web` 0.2.2 is exactly that shape.
        for ver, vobj in pkg["versions"].items():
            for required in ("url", "sha256", "size", "loft", "published"):
                if required not in vobj:
                    fail(f"`{name}` v{ver}: missing required field `{required}`")
            if not isinstance(vobj["sha256"], str) or len(vobj["sha256"]) != 64:
                fail(f"`{name}` v{ver}: sha256 must be 64-char hex")
            if not isinstance(vobj["size"], int) or vobj["size"] <= 0:
                fail(f"`{name}` v{ver}: size must be a positive integer")


def gate_tarball_verify(idx: dict, prev: dict) -> None:
    new_entries = _new_entries(idx, prev)
    for name, ver, vobj in new_entries:
        print(f"[verify] downloading {name} v{ver} from {vobj['url']}")
        try:
            with urllib.request.urlopen(vobj["url"], timeout=60) as resp:
                data = resp.read()
        except Exception as e:  # noqa: BLE001 — surface any failure
            fail(f"`{name}` v{ver}: download failed: {e}")
        actual_sha = hashlib.sha256(data).hexdigest()
        if actual_sha.lower() != vobj["sha256"].lower():
            fail(
                f"`{name}` v{ver}: sha256 MISMATCH\n"
                f"  PR claims: {vobj['sha256']}\n"
                f"  actual:    {actual_sha}"
            )
        if len(data) != vobj["size"]:
            fail(
                f"`{name}` v{ver}: size MISMATCH\n"
                f"  PR claims: {vobj['size']} bytes\n"
                f"  actual:    {len(data)} bytes"
            )
        print(f"[verify] {name} v{ver} sha256 + size match")


def gate_binaries_verify(idx: dict, prev: dict) -> None:
    """Gate 2b — every `binaries[triple].sha256` re-checked by download.

    A version may carry prebuilt platform artifacts alongside its source
    archive (today: the `loft` toolchain itself, so `loft self-update` can find
    a release).  Those zips are what users actually RUN, and `index.json` is the
    only signed thing the registry publishes -- the `.zip.sha256` sidecar beside
    each artifact rides the same transport as the artifact, so it catches a
    corrupted download, never a substituted one.  Naming the binaries from the
    signed index is what puts them under a signature, and this gate is what makes
    that signature mean something: the hash is verified against the bytes here,
    at review time, rather than trusted from the submission.

    It is also the reason gate 3 can exempt the toolchain (see
    `gate_reproducible_build`).  Without this gate that exemption would leave the
    binaries unchecked entirely, so the two belong together: if this gate is ever
    removed, the exemption below must go with it.

    `manifest_sha256` is NOT downloaded here.  It names `SHA256SUMS` *inside* the
    zip, which the client checks against what it INSTALLED; verifying the zip's
    own hash is what this gate is for.  Its presence is checked by the schema gate.
    """
    for name, ver, vobj in _new_entries(idx, prev):
        binaries = vobj.get("binaries")
        if not binaries:
            continue
        for triple, b in sorted(binaries.items()):
            print(f"[verify] downloading {name} v{ver} {triple} from {b['url']}")
            try:
                with urllib.request.urlopen(b["url"], timeout=120) as resp:
                    data = resp.read()
            except Exception as e:  # noqa: BLE001 -- surface any failure
                fail(f"`{name}` v{ver} [{triple}]: download failed: {e}")
            actual_sha = hashlib.sha256(data).hexdigest()
            if actual_sha.lower() != b["sha256"].lower():
                fail(
                    f"`{name}` v{ver} [{triple}]: sha256 MISMATCH\n"
                    f"  PR claims: {b['sha256']}\n"
                    f"  actual:    {actual_sha}"
                )
            print(f"[verify] {name} v{ver} {triple} sha256 match")


def gate_reproducible_build(idx: dict, prev: dict) -> None:
    """Clone the homepage repo at the version tag, run `loft package`,
    compare sha256 to the PR's claim.

    Caught here: PR claims hash X, but the source tree at the tag
    produces hash Y when re-packaged.  Either publisher tampered or
    upstream history rewrote the tag.

    Skipped when the package has no `homepage` (private deps,
    third-party-hosted tarballs).  Schema lint always runs; this gate
    is the additional reproducibility check.

    ## Single-package vs multi-package chunk-repo homepages

    For a **single-package repo**, `homepage` is the repo root
    (`https://github.com/<owner>/<repo>`), the tag is `v<version>`,
    and `loft package` runs at the repo root.

    For a **multi-package chunk repo** (e.g. `loft-libs-core` hosts
    `arguments`, `crypto`, `random` in separate subdirectories), the
    convention is:

      - `homepage` = `https://github.com/<owner>/<repo>/tree/<branch>/<pkg>`
      - tag = `<pkg>-v<version>` (so `arguments-v0.1.1` not `v0.1.1`)
      - `subpath` field carries `<pkg>` (also extractable from the homepage)

    Both shapes coexist in the registry today: `time`, `markdown`,
    `html` (planned) are single-package; `arguments`, `crypto`,
    `random`, `shapes`, `gridmesh`, `web`, `server`, `game_protocol`
    are multi-package chunk-repo members.
    """
    # Matches `https://github.com/<owner>/<repo>/tree/<branch>/<subpath>`.
    # The subpath captures any subdirectory path (may contain slashes
    # for nested packages, though current convention is single-segment).
    chunk_homepage = re.compile(
        r"^(https://github\.com/[^/]+/[^/]+)/tree/[^/]+/(.+?)/?$"
    )

    new_entries = _new_entries(idx, prev)
    for name, ver, vobj in new_entries:
        pkg_meta = idx["packages"][name]
        homepage = pkg_meta.get("homepage", "")
        if not homepage or "github.com" not in homepage:
            print(f"[repro] {name} v{ver} — no GitHub homepage, skipping")
            continue

        # The TOOLCHAIN is exempt, and the exemption is narrow: it applies to a
        # version whose artifact is a source archive of the compiler itself, not
        # to anything a publisher can opt into.
        #
        # This gate re-packages a source tree with `loft package` and compares the
        # hash.  Neither half applies here.  loft's repo root has no `loft.toml` --
        # it is the compiler, not a loft package -- so there is nothing to
        # re-package, and its version artifact is a `git archive` zip produced by
        # the release workflow, not a `loft package` tarball.  Running the gate
        # anyway fails on every toolchain submission for a reason that says nothing
        # about the bytes ("`loft package` failed: exit status 1").
        #
        # What keeps the exemption honest is gate 2b above, which downloads every
        # `binaries[triple]` zip and re-checks its sha256 -- so the artifacts users
        # actually run are verified against their hashes here, at review time.  The
        # source archive itself is still covered by gate 2. If gate 2b is ever
        # removed, this exemption must be removed with it.
        if vobj.get("binaries") and name == "loft":
            print(f"[repro] {name} v{ver} — toolchain, verified by gate 2b instead")
            continue

        m = chunk_homepage.match(homepage)
        if m:
            # Multi-package chunk repo: clone the parent + cd subpath.
            clone_url = m.group(1)
            url_subpath = m.group(2)
            # Prefer the explicit `subpath` field on the version object
            # when present; fall back to the homepage-derived value.  If
            # both exist and disagree, the explicit subpath wins (it's
            # the canonical declaration; the homepage URL is a hint).
            subpath = vobj.get("subpath", url_subpath)
            tag = f"{name}-v{ver}"
        else:
            # Single-package repo: clone the homepage directly, repo
            # root is the package root.
            clone_url = homepage
            subpath = ""
            tag = f"v{ver}"

        with tempfile.TemporaryDirectory() as tmp:
            print(f"[repro] cloning {clone_url} @ {tag}"
                  + (f" (subpath: {subpath})" if subpath else ""))
            try:
                subprocess.check_call(
                    ["git", "clone", "--depth", "1", "--branch", tag, clone_url, tmp],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.PIPE,
                )
            except subprocess.CalledProcessError as e:
                fail(
                    f"`{name}` v{ver}: git clone of {clone_url}@{tag} failed: "
                    f"{e.stderr.decode(errors='replace') if e.stderr else e}"
                )
            pkg_dir = Path(tmp) / subpath if subpath else Path(tmp)
            if not pkg_dir.is_dir():
                fail(
                    f"`{name}` v{ver}: subpath `{subpath}` does not exist "
                    f"in the cloned tree at {clone_url}@{tag}"
                )
            try:
                subprocess.check_call(
                    ["loft", "package"], cwd=pkg_dir, stdout=subprocess.DEVNULL,
                )
            except subprocess.CalledProcessError as e:
                fail(f"`{name}` v{ver}: `loft package` failed: {e}")
            artefact = pkg_dir / f"{name}-{ver}.tar.gz"
            if not artefact.exists():
                fail(f"`{name}` v{ver}: `loft package` produced no artefact")
            actual = hashlib.sha256(artefact.read_bytes()).hexdigest()
            if actual.lower() != vobj["sha256"].lower():
                fail(
                    f"`{name}` v{ver}: REPRODUCIBLE-BUILD MISMATCH\n"
                    f"  PR claims sha256: {vobj['sha256']}\n"
                    f"  rebuilt from src: {actual}\n"
                    f"  clone url:        {clone_url}@{tag}"
                    + (f" (subpath: {subpath})" if subpath else "") + "\n"
                    f"  The source repo at {tag} produces a different "
                    f"tarball than the one uploaded to releases.  Either:\n"
                    f"    (a) the GitHub release tarball is stale — re-upload, OR\n"
                    f"    (b) the git tag was force-pushed — investigate.\n"
                )
            # S7-CI: re-derive the function-level `api` from the cloned source and
            # reject a pasted field that disagrees, so `loft search`'s function
            # discovery can never point at a function the source does not have.
            # The `api` field is AUTO-DERIVED by `loft publish` — it must equal
            # the source, never hand-edited.  (Only checkable where the source is
            # cloned; a package without a GitHub homepage has its `api` trusted-
            # as-pasted, exactly like its sha256.)
            # ⚠ Derive from `pkg_dir`, NOT the clone root: in a multi-package
            # chunk repo the package is at `<tmp>/<subpath>`, and deriving from
            # the root answers for the wrong tree — which rejects every
            # chunk-repo submission.
            submitted_api = vobj.get("api")
            if submitted_api is not None:
                try:
                    derived = json.loads(
                        subprocess.check_output(
                            ["loft", "api", "--json", str(pkg_dir)],
                            stderr=subprocess.PIPE,
                        )
                    )
                except (subprocess.CalledProcessError, json.JSONDecodeError) as e:
                    fail(f"`{name}` v{ver}: `loft api --json` re-derive failed: {e}")
                if derived != submitted_api:
                    fail(
                        f"`{name}` v{ver}: `api` MISMATCH — the pasted `api` field "
                        f"does not match the source at {tag}.\n"
                        f"  The `api` field is auto-derived; re-run `loft publish` "
                        f"and paste the regenerated entry rather than hand-editing it."
                    )
                print(f"[verify] {name} v{ver} api ({len(derived)} fns) matches source")
            print(f"[repro] {name} v{ver} reproduces from source")


def gate_trigger_uniqueness(idx: dict) -> None:
    """Every `method:receiver` Tier-1 trigger must be owned by at most one
    package across the whole registry.

    A consumer auto-loads a library from a bare `obj.method()` call, so the
    trigger that maps `matches` -> the providing package must be globally
    unique; two packages claiming `text.matches` would make the auto-load
    ambiguous.  A package re-declaring its own trigger across versions is fine
    — only a *cross-package* collision is rejected.

    Runs over the full index (not just new rows): `main` is assumed clean, so
    any collision present here was introduced by this PR.
    """
    owner: dict[str, str] = {}
    for name, pkg in idx.get("packages", {}).items():
        for vobj in pkg.get("versions", {}).values():
            for trig in vobj.get("triggers", []) or []:
                if not trig:
                    continue
                prior = owner.get(trig)
                if prior is not None and prior != name:
                    fail(
                        f"trigger `{trig}` is claimed by both `{prior}` and "
                        f"`{name}`; a method-on-type trigger must be unique "
                        f"across the registry — rename the method in `{name}` "
                        f"or drop its `[triggers]` opt-in"
                    )
                owner.setdefault(trig, name)
    print("[triggers] all method-on-type triggers are uniquely owned")



def _new_entries(idx: dict, prev: dict) -> list[tuple[str, str, dict]]:
    """Return list of (name, version, version_object) for rows that
    are present in `idx` but not in `prev`."""
    out: list[tuple[str, str, dict]] = []
    for name, pkg in idx.get("packages", {}).items():
        prev_versions: dict = (
            prev.get("packages", {}).get(name, {}).get("versions", {})
        )
        for ver, vobj in pkg.get("versions", {}).items():
            if ver not in prev_versions:
                out.append((name, ver, vobj))
    return out


def main() -> None:
    idx = load_index()
    prev = load_previous_index()
    print("[gate 1] schema lint")
    gate_schema(idx)
    print("[gate 2] tarball sha256 + size verify")
    gate_tarball_verify(idx, prev)
    print("[gate 2b] prebuilt-binary sha256 verify")
    gate_binaries_verify(idx, prev)
    skip_repro = os.environ.get("LOFT_VALIDATE_SKIP_REPRO") == "1"
    if skip_repro:
        print("[gate 3] reproducible-build re-check — SKIPPED (env)")
    else:
        print("[gate 3] reproducible-build re-check")
        gate_reproducible_build(idx, prev)
    print("[gate 4] trigger uniqueness")
    gate_trigger_uniqueness(idx)
    print("All gates passed.")


if __name__ == "__main__":
    main()
