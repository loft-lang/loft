#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which test BINARIES a CI test leg builds and runs — so a leg builds only what it runs.

    scripts/ci_shards.py args <shard>        # the cargo target flags of one leg: `--test a --test b … [--lib]`
    scripts/ci_shards.py needs-wasm <shard>  # `true` when a binary of the leg builds a --html page or a wasm rlib
    scripts/ci_shards.py pack [--legs N]     # the rest legs' packs with their weights (a report)
    scripts/ci_shards.py proof               # every test binary is in exactly one leg, or exit 1
    scripts/ci_shards.py measure <junit>…    # rewrite scripts/ci_shards.tsv from nextest junit reports

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
The `heavy` and `corpus` legs keep their membership from `scripts/ci_test_filter.py` (the
single-slot nextest groups, and the native script corpus inside the `native` binary).  The
REST — every other `tests/*.rs` plus the crate's unit tests (`lib`) — is packed into
`REST_LEGS` legs by MEASURED weight: `scripts/ci_shards.tsv` holds each binary's test seconds
from one run, a binary the table does not know packs at the table's median, and the greedy
pack (heaviest first, into the lightest leg) is deterministic, so every leg of one run agrees.
The binaries that build a `--html` page or need the wasm target are packed together into
`rest-a`, so the wasm toolchain and rlib are built on that one leg and skipped on the others.

The partition is checked, not trusted: `proof` lists every test binary and fails when one is
in no leg or in two.  A binary added to the repo lands in a rest leg by construction (the
table only weights; the file list is read from `tests/`), so nothing can run nowhere.
"""

import re
import statistics
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TABLE = ROOT / "scripts" / "ci_shards.tsv"
TESTS = ROOT / "tests"
sys.path.insert(0, str(ROOT / "scripts"))
import ci_test_filter  # noqa: E402

REST_LEGS = 3
REST_NAMES = ["rest-a", "rest-b", "rest-c"]
# A binary whose source builds a page or a wasm rlib needs `wasm32-unknown-unknown` and the
# `--html` runtime rlib: all of them go to ONE rest leg, the only one that installs them.
WASM_MARKS = ("wasm32", "--html", "html_wasm", ".html")
# Excluded from every leg by the filterset (`ci_test_filter.BASE` and `PR_ONLY`): not built
# either.  Read off the clauses so the two lists cannot drift apart.
def excluded_binaries() -> set[str]:
    out = set()
    for clause in ci_test_filter.BASE + ci_test_filter.PR_ONLY:
        m = re.fullmatch(r"not binary\((\w+)\)", clause)
        if m:
            out.add(m.group(1))
    return out


def all_binaries() -> list[str]:
    """Every integration test binary (`tests/<name>.rs`) and `lib`, the unit tests."""
    names = sorted(p.stem for p in TESTS.glob("*.rs"))
    return ["lib", *names]


def heavy_binaries() -> set[str]:
    """The binaries of the single-slot groups: the `heavy` leg (and `corpus` within `native`)."""
    out = set()
    for group in ci_test_filter.SERIAL_GROUPS:
        out.update(re.findall(r"binary\((\w+)\)", ci_test_filter.group_filter(group)))
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


def needs_wasm(binary: str) -> bool:
    if binary == "lib":
        return False
    text = (TESTS / f"{binary}.rs").read_text(encoding="utf-8", errors="replace")
    return any(mark in text for mark in WASM_MARKS)


def packs() -> list[list[str]]:
    """The rest legs' binaries, `REST_LEGS` lists in leg order."""
    heavy = heavy_binaries()
    skip = excluded_binaries()
    rest = [b for b in all_binaries() if b not in heavy and b not in skip]
    w = weights()
    median = statistics.median(w.values()) if w else 1.0
    weight = {b: w.get(b, median) for b in rest}
    legs = [[0.0, []] for _ in range(REST_LEGS)]
    # The wasm binaries first, all into leg 0, heaviest first; then the others greedily.
    wasm = sorted((b for b in rest if needs_wasm(b)), key=lambda b: (-weight[b], b))
    for b in wasm:
        legs[0][0] += weight[b]
        legs[0][1].append(b)
    for b in sorted((b for b in rest if b not in wasm), key=lambda b: (-weight[b], b)):
        leg = min(legs, key=lambda l: l[0])
        leg[0] += weight[b]
        leg[1].append(b)
    return [sorted(names) for _, names in legs]


def leg_binaries(shard: str) -> list[str]:
    if shard == "heavy":
        return sorted(heavy_binaries())
    if shard == "corpus":
        return ["native"]
    if shard in REST_NAMES:
        return packs()[REST_NAMES.index(shard)]
    raise SystemExit(f"unknown shard '{shard}' (heavy, corpus, {', '.join(REST_NAMES)})")


def args(shard: str) -> str:
    return " ".join("--lib" if b == "lib" else f"--test {b}" for b in leg_binaries(shard))


def proof() -> int:
    seen: dict[str, list[str]] = {}
    for shard in ["heavy", *REST_NAMES]:
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
    per: dict[str, float] = {}
    for p in paths:
        for suite in ET.parse(p).getroot().iter("testsuite"):
            # nextest names a suite `loft::<binary>`; the unit tests are `loft`.
            name = suite.get("name", "")
            binary = name.split("::", 1)[1] if "::" in name else "lib"
            per[binary] = per.get(binary, 0.0) + sum(
                float(c.get("time", 0)) for c in suite.iter("testcase")
            )
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
    elif cmd == "needs-wasm" and len(a) == 2:
        bins = leg_binaries(a[1])
        print("true" if any(needs_wasm(b) for b in bins) else "false")
    elif cmd == "pack":
        w = weights()
        median = statistics.median(w.values()) if w else 1.0
        for name, bins in zip(REST_NAMES, packs()):
            total = sum(w.get(b, median) for b in bins)
            wasm = sum(1 for b in bins if needs_wasm(b))
            print(f"{name}: {len(bins)} binaries, {total:.0f} s, {wasm} need wasm; heaviest "
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
