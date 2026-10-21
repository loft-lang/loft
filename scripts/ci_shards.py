#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which test BINARIES a CI test leg builds and runs — so a leg builds only what it runs.

    scripts/ci_shards.py args <shard>     # the cargo target flags of one leg: `--test a … [--lib --bin x]`
    scripts/ci_shards.py html <shard>     # `true` when a binary of the leg runs a `--html` page
    scripts/ci_shards.py wasi <shard>     # `true` when a binary of the leg builds for wasi
    scripts/ci_shards.py pack [--legs N]  # the rest legs' packs with their weights (a report)
    scripts/ci_shards.py proof            # every test binary is in exactly one leg, or exit 1
    scripts/ci_shards.py measure <junit>… # rewrite scripts/ci_shards.tsv from nextest junit reports

Why
---
Every ubuntu test leg ran `cargo build --all-targets` — all 445 test binaries — and then
nextest ran its third of the TESTS: 7.6 to 8.5 minutes of build on each of five legs for a
test phase of 6 to 8, and a cache restore of the whole `target/` (4 to 6 minutes for 11 GB)
that made the build no faster (CI_BUDGET.md § Each leg builds only the binaries it runs).
A leg that builds only its own binaries starts its tests in a fraction of that, and the
runner-minutes stop scaling with the number of legs.

How
---
The LEG a binary runs on is decided here and nowhere else (`scripts/ci_test_filter.py`
composes one filterset for every leg; the shard argument it takes no longer selects):

- `corpus` runs the `native` binary whole — the script corpus (`native_scripts_N`, each
  taking the whole runner under `threads-required`) and its other tests, all in the
  single-slot `heavy-serial` group.
- `heavy` runs the other single-slot binaries that drive a `--html` page in a browser:
  `html_wasm` (the `html-wasm-serial` group) and the `heavy-serial` members that need the
  html runtime.  One leg installs the wasm toolchain for them.
- the REST — every other `tests/*.rs`, the crate's unit tests (`lib`) and the bin targets'
  (`bin/loft`, `bin/gendoc`) — is packed into `REST_LEGS` legs by MEASURED weight:
  `scripts/ci_shards.tsv` holds each binary's test seconds from one run, a test that takes
  the whole runner (`threads-required` in `.config/nextest.toml`) counted `TEST_THREADS`
  times, since nothing runs beside it.  A binary the table does not know packs at the
  table's median; the greedy pack (heaviest first, into the lightest leg) is deterministic,
  so every leg of one run agrees.  The binaries that need a wasm runtime rlib (`--html`
  pages, wasi builds) are packed together into `rest-a`, so the runtime rlib builds happen
  on that one leg and on no other; `html` and `wasi` say which runtime a leg's binaries
  need, so it builds no other.

The partition is checked, not trusted: `proof` lists every test binary and fails when one is
in no leg or in two.  A binary added to the repo lands in a rest leg by construction (the
table only weights; the file list is read from `tests/`), so nothing can run nowhere.
"""

import re
import statistics
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # python < 3.11
    import tomli as tomllib  # type: ignore[no-redef]

ROOT = Path(__file__).resolve().parent.parent
TABLE = ROOT / "scripts" / "ci_shards.tsv"
TESTS = ROOT / "tests"
NEXTEST_TOML = ROOT / ".config" / "nextest.toml"
CARGO_TOML = ROOT / "Cargo.toml"
sys.path.insert(0, str(ROOT / "scripts"))
import ci_test_filter  # noqa: E402

REST_LEGS = 3
REST_NAMES = ["rest-a", "rest-b", "rest-c"]
CORPUS_BINARY = "native"
# A GitHub-hosted runner's cores: nextest's default `test-threads` there, and what a test
# with `threads-required` at or over it costs — the whole runner, for its duration.
TEST_THREADS = 4
# A binary whose source drives a `--html` page needs the html runtime rlib (and the
# `wasm32-unknown-unknown` target); one whose source builds for wasi needs that rlib.
HTML_MARKS = ("--html", "html_wasm", ".html")
WASI_MARKS = ("wasip2", "wasm32-wasi", "--native-wasm")
# What the runtime rest leg pays before its tests start and the other rest legs do not —
# the wasm target, the runtime rlib builds and their warm-up: 3.5 min measured on run
# 39411552589 (rest-a 11.0 min before its tests, rest-b 7.5) — in the pack's unit (test
# seconds over `TEST_THREADS` threads): the leg starts that much heavier, so the greedy
# pack hands it that much less.
RUNTIME_LEG_OVERHEAD = 3.5 * 60 * TEST_THREADS


def excluded_binaries() -> set[str]:
    """Excluded from every leg by the filterset's `not binary(x)` (dedicated steps run them)."""
    out = set()
    for clause in ci_test_filter.BASE:
        m = re.fullmatch(r"not binary\((\w+)\)", clause)
        if m:
            out.add(m.group(1))
    return out


def bins_with_tests() -> list[str]:
    """The `[[bin]]` targets of Cargo.toml whose source holds a `#[test]` (nextest runs them)."""
    with CARGO_TOML.open("rb") as fh:
        cargo = tomllib.load(fh)
    out = []
    for b in cargo.get("bin", []):
        src = ROOT / b.get("path", f"src/bin/{b['name']}.rs")
        if src.is_file() and "#[test]" in src.read_text(encoding="utf-8", errors="replace"):
            out.append(f"bin/{b['name']}")
    return out


def all_binaries() -> list[str]:
    """Every integration test binary (`tests/<name>.rs`), the unit tests (`lib`), the bins'."""
    names = sorted(p.stem for p in TESTS.glob("*.rs"))
    return ["lib", *bins_with_tests(), *names]


def serial_binaries() -> set[str]:
    """The binaries of the single-slot groups (`heavy-serial`, `html-wasm-serial`)."""
    out = set()
    for group in ci_test_filter.SERIAL_GROUPS:
        out.update(re.findall(r"binary\((\w+)\)", ci_test_filter.group_filter(group)))
    return out


def source(binary: str) -> str:
    if binary == "lib" or binary.startswith("bin/"):
        return ""
    return (TESTS / f"{binary}.rs").read_text(encoding="utf-8", errors="replace")


def runtimes(binary: str) -> set[str]:
    """The wasm runtime rlibs the binary's tests need: `html`, `wasi`, both or none."""
    text = source(binary)
    out = set()
    if any(mark in text for mark in HTML_MARKS):
        out.add("html")
    if any(mark in text for mark in WASI_MARKS):
        out.add("wasi")
    return out


def exclusive_tests() -> dict[str, list[re.Pattern[str]]]:
    """Per binary, the tests that take the whole runner: the `threads-required` overrides of
    `.config/nextest.toml` at or over `TEST_THREADS`, read as `binary(x) & (test(…) | …)`."""
    with NEXTEST_TOML.open("rb") as fh:
        config = tomllib.load(fh)
    out: dict[str, list[re.Pattern[str]]] = {}
    for profile in config.get("profile", {}).values():
        for override in profile.get("overrides", []):
            if override.get("threads-required", 0) < TEST_THREADS:
                continue
            for binary, tests in re.findall(
                r"binary\((\w+)\) & \(((?:\s*test\([^)]*\)\s*\|?)+)\)", override["filter"]
            ):
                pats = out.setdefault(binary, [])
                for exact in re.findall(r"test\(=([^)]+)\)", tests):
                    pats.append(re.compile(re.escape(exact) + r"\Z"))
                for rx in re.findall(r"test\(/([^/]+)/\)", tests):
                    pats.append(re.compile(rx))
    return out


def weights() -> dict[str, float]:
    out = {}
    if TABLE.is_file():
        for line in TABLE.read_text(encoding="utf-8").splitlines():
            if line.startswith("#") or not line.strip():
                continue
            name, _, secs = line.partition("\t")
            out[name] = float(secs)
    return out


def heavy_binaries() -> list[str]:
    return sorted(b for b in serial_binaries() if b != CORPUS_BINARY and "html" in runtimes(b))


def packs() -> list[list[str]]:
    """The rest legs' binaries, `REST_LEGS` lists in leg order."""
    taken = set(heavy_binaries()) | {CORPUS_BINARY} | excluded_binaries()
    rest = [b for b in all_binaries() if b not in taken]
    w = weights()
    median = statistics.median(w.values()) if w else 1.0
    weight = {b: w.get(b, median) for b in rest}
    legs: list[list] = [[0.0, []] for _ in range(REST_LEGS)]
    legs[0][0] = RUNTIME_LEG_OVERHEAD
    # The binaries that need a wasm runtime rlib first, all into leg 0, heaviest first —
    # the one rest leg that builds a runtime; then the others greedily.
    pinned = sorted((b for b in rest if runtimes(b)), key=lambda b: (-weight[b], b))
    for b in pinned:
        legs[0][0] += weight[b]
        legs[0][1].append(b)
    for b in sorted((b for b in rest if b not in pinned), key=lambda b: (-weight[b], b)):
        leg = min(legs, key=lambda l: l[0])
        leg[0] += weight[b]
        leg[1].append(b)
    return [sorted(names) for _, names in legs]


def leg_binaries(shard: str) -> list[str]:
    if shard == "heavy":
        return heavy_binaries()
    if shard == "corpus":
        return [CORPUS_BINARY]
    if shard in REST_NAMES:
        return packs()[REST_NAMES.index(shard)]
    raise SystemExit(f"unknown shard '{shard}' (heavy, corpus, {', '.join(REST_NAMES)})")


def target_flag(binary: str) -> str:
    if binary == "lib":
        return "--lib"
    if binary.startswith("bin/"):
        return f"--bin {binary[4:]}"
    return f"--test {binary}"


def args(shard: str) -> str:
    return " ".join(target_flag(b) for b in leg_binaries(shard))


def leg_runtimes(shard: str) -> set[str]:
    out: set[str] = set()
    for b in leg_binaries(shard):
        out |= runtimes(b)
    return out


def proof() -> int:
    seen: dict[str, list[str]] = {}
    for shard in ["heavy", "corpus", *REST_NAMES]:
        for b in leg_binaries(shard):
            seen.setdefault(b, []).append(shard)
    skip = excluded_binaries()
    bad = 0
    for b in all_binaries():
        legs = seen.get(b, [])
        if b in skip:
            if legs:
                print(f"ci_shards: {b} is excluded by the filterset yet packed into {legs}")
                bad += 1
        elif len(legs) != 1:
            print(f"ci_shards: {b} is in {len(legs)} legs: {legs}")
            bad += 1
    if bad:
        return 1
    print(f"ci_shards: {len(all_binaries()) - len(skip)} binaries, each in exactly one leg")
    return 0


def measure(paths: list[str]) -> None:
    exclusive = exclusive_tests()
    per: dict[str, float] = {}
    for p in paths:
        for suite in ET.parse(p).getroot().iter("testsuite"):
            # nextest names a suite `loft::<binary>`; the unit tests are `loft`.
            name = suite.get("name", "")
            binary = name.split("::", 1)[1] if "::" in name else "lib"
            for case in suite.iter("testcase"):
                secs = float(case.get("time", 0))
                test = case.get("name", "")
                if any(rx.match(test) for rx in exclusive.get(binary, [])):
                    secs *= TEST_THREADS
                per[binary] = per.get(binary, 0.0) + secs
    head = [
        l for l in TABLE.read_text(encoding="utf-8").splitlines() if l.startswith("#")
    ] if TABLE.is_file() else ["# binary <TAB> seconds"]
    body = "\n".join(head + [f"{b}\t{s:.1f}" for b, s in sorted(per.items())]) + "\n"
    TABLE.write_text(body, encoding="utf-8")
    print(f"wrote {TABLE.relative_to(ROOT)}: {len(per)} binaries")


def main() -> int:
    a = sys.argv[1:]
    if not a or a[0] in ("-h", "--help"):
        print(__doc__.strip())
        return 0
    cmd = a[0]
    if cmd == "args" and len(a) == 2:
        print(args(a[1]))
    elif cmd in ("html", "wasi") and len(a) == 2:
        print("true" if cmd in leg_runtimes(a[1]) else "false")
    elif cmd == "pack":
        w = weights()
        median = statistics.median(w.values()) if w else 1.0
        for name, bins in zip(["heavy", "corpus", *REST_NAMES],
                              [heavy_binaries(), [CORPUS_BINARY], *packs()]):
            total = sum(w.get(b, median) for b in bins)
            print(f"{name}: {len(bins)} binaries, {total:.0f} s, runtime "
                  f"{','.join(sorted(leg_runtimes(name))) or 'none'}; heaviest "
                  + ", ".join(sorted(bins, key=lambda b: -w.get(b, median))[:4]))
    elif cmd == "proof":
        return proof()
    elif cmd == "measure" and len(a) >= 2:
        measure(a[1:])
    else:
        raise SystemExit(__doc__)
    return 0


if __name__ == "__main__":
    sys.exit(main())
