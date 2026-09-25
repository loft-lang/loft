#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The revalidate-libs matrix: which published packages the gate checks, at which version.

One home for a policy that has two readers — `.github/workflows/revalidate-libs.yml` (the
gate that runs on `pull_request` and `push` to `main`) and `scripts/revalidate_libs_local.sh`
(the same gate on a work branch, where the workflow never fires).  The local script's header
says it re-classifies "exactly as the workflow does", and for the matrix itself it did not:
the two grew four separate answers to the same questions, and each difference changed what
the run measured.

  * **the compiler's own package.** `loft` is published, and it is the COMPILER rather than a
    library, so the question this gate asks — *does a language change retro-break a shipped
    lib?* — is not one it can answer about itself.  Its `tests/` is the repo's own suite,
    which `make ci` runs against the CURRENT corpus, and it holds fixtures that are
    deliberately not standalone programs (`tests/data/script_hello.loft` says so in its own
    header).  The re-classifier compiles every `tests/**/*.loft` with `--dump`, read those as
    COMPILE-BREAKs, and reported the compiler as retro-breaking itself.  The workflow has
    skipped it since that was measured; the local script did not, so its summary line read
    `1 COMPILE-BREAK` on an unchanged tree — an instrument whose zero is not zero, and its
    exit status was therefore 1 on every run (loft#1315).
  * **known-broken.** The workflow carries a map of libs a language change ALREADY broke
    before the freeze, so a red means a NEW break.  The local script had no such map, so the
    same red meant different things in the two places.
  * **`subpath`.** A package that declares none lives at its repo ROOT.  The workflow
    defaults to `"."`; the local script defaulted to the package NAME, which names a
    directory that does not exist.  It survived only because the extractor falls back to the
    archive root when the subpath is missing — a wrong default cancelled by a lenient reader.
  * **yanked versions.** The local script dropped them and the workflow did not, so the gate
    could validate a version the registry has withdrawn and report on a package nobody can
    install.

Usage:  revalidate_matrix.py <index.json> [--format tsv|github]
        revalidate_matrix.py --self-test
        revalidate_matrix.py --native-policy <loft.toml>

  --native-policy  `gate` or `best-effort<TAB><build-deps>` for one library: does a failing
                   `--native` suite fail the gate?  See `native_policy`.

  tsv     (default) one leg per line: name, version, repo, tag, subpath — for the shell.
  github  `matrix=<json>` + `count=<n>` on stdout for `$GITHUB_OUTPUT`.

Exclusions are announced on stderr in both formats, so a run says what it did NOT look at.
A skip that is not printed is indistinguishable from a package that passed.
"""

import json
import re
import subprocess
import sys
import tomllib

# A published lib a language change ALREADY retro-broke (pre-freeze migration debt), skipped
# so the gate reflects NEW breaks and future reds are real signals.  Every entry MUST cite a
# tracking issue and be removed the moment the lib republishes migrated.
KNOWN_BROKEN: dict[str, str] = {
    # (empty) — hex_terrain 0.1.1 republished migrated (loft-lang/loft#579).
    # Add an entry ONLY with a tracking issue + a remove-on-republish note.
}

# The compiler is not a library revalidated against itself; see the module docstring.
NOT_A_LIBRARY = {
    "loft": "the compiler is not a library; its suite is CI's job",
}

_RELEASE_URL = re.compile(r"https://github\.com/([^/]+/[^/]+)/releases/download/([^/]+)/")


def _version_key(v: str) -> list[int]:
    """Order versions numerically, on the first three components.

    Truncating at three is the workflow's rule and is the one kept: a build/suffix number
    beyond `major.minor.patch` must not outrank a higher patch.
    """
    return [int(x) for x in re.findall(r"\d+", v)][:3]


def legs(index_path: str, note) -> list[dict]:
    """The packages to check, each at its latest installable version."""
    packages = json.load(open(index_path))["packages"]
    out = []
    for name, pkg in sorted(packages.items()):
        if name in NOT_A_LIBRARY:
            note(f"SKIP '{name}': {NOT_A_LIBRARY[name]}")
            continue
        if name in KNOWN_BROKEN:
            note(f"SKIP known-broken '{name}': {KNOWN_BROKEN[name]}")
            continue
        # A yanked version is one the registry has withdrawn — validating it reports on
        # something nobody can install, and a break in it is not a break anyone can hit.
        yanked = set(pkg.get("yanked", []))
        versions = {k: v for k, v in pkg.get("versions", {}).items() if k not in yanked}
        if not versions:
            continue
        latest = max(versions, key=_version_key)
        m = _RELEASE_URL.match(versions[latest].get("url", ""))
        if not m:  # non-GitHub / malformed URL — nothing to check out
            continue
        out.append(
            {
                "name": name,
                "version": latest,
                "repo": m.group(1),
                "tag": m.group(2),
                # No `subpath` means the repo IS the package, so its root is ".".
                "sub": versions[latest].get("subpath") or ".",
            }
        )
    return out


def native_policy(manifest_text: str) -> tuple[str, str]:
    """Does a failing `--native` suite fail the gate for this library?  (loft#1653)

    Native is the backend a user gets by default, so a native-only break in a shipped
    library is a retro-break like any other.  The one honest reason not to gate on it is
    the RUNNER: a library whose native crate needs system packages (`graphics` → GL and
    ALSA) can fail there for want of a package, which says nothing about the language.  The
    manifest names those packages in `[native] build-deps`, so that field decides:

    * no `build-deps` (pure loft, or a crate with only crate dependencies) → `gate`;
    * `build-deps` declared → `best-effort`, carrying the list so the report can say why.

    One home for the answer, read by `revalidate-libs.yml` and by
    `revalidate_libs_local.sh --native`, so the two cannot drift the way the matrix policy
    did (loft#1315).
    """
    cfg = tomllib.loads(manifest_text)
    deps = (cfg.get("native") or {}).get("build-deps", "")
    if isinstance(deps, list):
        deps = ", ".join(str(d) for d in deps)
    deps = str(deps).strip()
    return ("best-effort", deps) if deps else ("gate", "")


def _self_test() -> int:
    """Prove the policy DECIDES rather than merely passing everything through.

    A matrix that returns every package looks identical to a correct one on a registry with
    nothing to exclude, and that is the state the index is usually in — `KNOWN_BROKEN` is
    empty and `loft` is one row in forty-three.  So each rule is given an input it must act
    on, and the run asserts the row is gone AND that the skip was announced: an exclusion
    nobody is told about is indistinguishable from a package that passed, which is the shape
    loft#1315 was.
    """
    import tempfile

    def index(packages):
        fh = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False)
        json.dump({"packages": packages}, fh)
        fh.close()
        return fh.name

    def run(packages):
        said = []
        rows = legs(index(packages), said.append)
        return {r["name"]: r for r in rows}, " ".join(said)

    url = "https://github.com/o/r/releases/download/t-v1.0.0/x.zip"
    ok = {"versions": {"1.0.0": {"url": url, "subpath": "ok"}}}
    failures = []

    def check(cond, what):
        if not cond:
            failures.append(what)
        print(f"  {'ok  ' if cond else 'FAIL'}  {what}")

    names = json.loads(
        subprocess.run(
            [sys.executable, __file__, "--not-a-library"], capture_output=True, text=True, check=True
        ).stdout
    )
    check(names == sorted(NOT_A_LIBRARY), "--not-a-library prints the policy set")
    check("loft" in names, "--not-a-library names the compiler's own package")
    rows, said = run({"ok": ok, "loft": {"versions": {"1.0.0": {"url": url}}}})
    check("ok" in rows, "an ordinary package is checked")
    check("loft" not in rows, "the compiler's own package is not checked")
    check("loft" in said, "and the run SAYS it skipped it")

    # A yanked latest must fall back to the newest version still installable, not be picked.
    two = {
        "versions": {"1.0.0": {"url": url, "subpath": "s"}, "2.0.0": {"url": url, "subpath": "s"}},
        "yanked": ["2.0.0"],
    }
    rows, _ = run({"two": two})
    check(rows["two"]["version"] == "1.0.0", "a yanked version is never the one validated")

    # Every version yanked: the package drops out rather than being validated at a withdrawn one.
    rows, _ = run({"gone": {"versions": {"1.0.0": {"url": url}}, "yanked": ["1.0.0"]}})
    check("gone" not in rows, "a fully yanked package drops out")

    # No `subpath` means the repo IS the package, so its root.
    rows, _ = run({"root": {"versions": {"1.0.0": {"url": url}}}})
    check(rows["root"]["sub"] == ".", "a package with no subpath sits at its repo root")

    # Versions order numerically on three components, so a longer suffix cannot outrank a
    # higher patch — the two readers used to disagree about exactly this.
    many = {"versions": {"1.2.10": {"url": url}, "1.2.9.9": {"url": url}}}
    rows, _ = run({"many": many})
    check(rows["many"]["version"] == "1.2.10", "1.2.10 outranks 1.2.9.9")

    # A non-GitHub URL cannot be checked out, so it is not reported as checked.
    rows, _ = run({"weird": {"versions": {"1.0.0": {"url": "https://elsewhere/x.zip"}}}})
    check("weird" not in rows, "a package with no checkout-able URL is not a leg")

    # The native policy: only a DECLARED system dependency excuses a native failure.
    check(native_policy('[package]\nname = "p"\n') == ("gate", ""), "pure loft gates on native")
    check(
        native_policy('[native]\ncrate = "c"\n') == ("gate", ""),
        "a native crate with no build-deps gates on native",
    )
    check(
        native_policy('[native]\ncrate = "c"\nbuild-deps = "libgl-dev, libasound2-dev"\n')
        == ("best-effort", "libgl-dev, libasound2-dev"),
        "declared build-deps make native best-effort, and say which",
    )
    check(native_policy('[native]\nbuild-deps = "  "\n') == ("gate", ""), "an empty build-deps is none")
    check(
        native_policy('[native]\nbuild-deps = ["a-dev", "b-dev"]\n') == ("best-effort", "a-dev, b-dev"),
        "a build-deps list reads like the string form",
    )

    if failures:
        print(f"self-test FAILED: {len(failures)} of the policy's rules did not hold")
        return 1
    print("self-test: every matrix rule acts on an input that needs it")
    return 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv[1:]:
        return _self_test()
    if "--native-policy" in argv[1:]:
        path = argv[argv.index("--native-policy") + 1]
        with open(path, encoding="utf-8") as fh:
            verdict, deps = native_policy(fh.read())
        print(verdict if verdict == "gate" else f"{verdict}\t{deps}")
        return 0
    if "--not-a-library" in argv[1:]:
        # The exclusion set as DATA, for a consumer that builds its own matrix and needs the
        # policy rather than the rows — `registry-validation.yml` installs from the registry
        # and needs no repo or tag.  It restated the set in `jq` and therefore did not have
        # it: the nightly tried `loft install loft@2026.8.0` every night, the installer
        # answered "`loft` is the toolchain, not a library", and the sweep was red from
        # 2026-08-31 on.  That is loft#1315's finding — one policy written twice — in the
        # workflow the fix did not reach.
        print(json.dumps(sorted(NOT_A_LIBRARY)))
        return 0
    args = [a for a in argv[1:] if not a.startswith("--")]
    fmt = "tsv"
    for a in argv[1:]:
        if a.startswith("--format"):
            fmt = a.split("=", 1)[1] if "=" in a else argv[argv.index(a) + 1]
    if len(args) < 1:
        print(__doc__, file=sys.stderr)
        return 2
    if fmt == "github":
        rows = legs(args[0], lambda m: print(f"::notice::revalidate-libs {m}", file=sys.stderr))
        print("matrix=" + json.dumps({"include": rows}))
        print("count=" + str(len(rows)))
        return 0
    rows = legs(args[0], lambda m: print(f"  {m}", file=sys.stderr))
    for r in rows:
        print("\t".join([r["name"], r["version"], r["repo"], r["tag"], r["sub"]]))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
