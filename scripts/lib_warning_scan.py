#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Report the loft WARNINGS a library carries against the loft under test.
#
# The forward-compatibility gate (.github/workflows/revalidate-libs.yml) proves a
# published library still COMPILES and PASSES against the newest loft — deliberately
# tolerating warnings, because a new deprecation must never fail an already-shipped
# artifact.  That leaves a blind spot: a library can accumulate warnings for weeks
# and nobody sees it, until the day someone opens a PR on the library repo and its
# own CI (LOFT_DENY_WARNINGS=1) goes red on code they did not touch.  gridmesh
# 0.1.2 is the worked example — 24 `not null` / `&`-parameter / null-flow warnings
# against loft 2026.7.x, invisible in a green nightly.
#
# This script makes that debt VISIBLE without making it fatal.  Two readings, because
# they answer different questions:
#
#   published tag   → what a user of the library sees today   (fix = republish)
#   default branch  → what the library's own CI does next PR  (fix = clean the source)
#
# Warnings raised inside a DEPENDENCY are reported separately and never counted
# against the package: a consumer is not blocked by lint debt in a dep they do not
# own (the same rule `loft test --deps` applies with --no-warnings).
#
# Usage:
#   scripts/lib_warning_scan.py scan <pkg-dir> [--name N] [--label L] [--json OUT]
#   scripts/lib_warning_scan.py scan --from-log <log> --root <pkg-dir> [--name N] ...
#   scripts/lib_warning_scan.py collect <dir-of-jsons>
#
# `scan` runs `loft --interpret --tests tests` in <pkg-dir> (or re-reads a log a
# previous run already captured, so CI never pays for the same suite twice) and
# writes a JSON report.  `collect` merges those reports into one table.
#
# Exit status is 0 whenever the scan itself worked — warnings are reported, never
# gated.  A non-zero exit means the scan could not run (no loft, no tests dir).

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

# The test runner prints one line per warning:
#     "  Warning: <message> at <file>:<line>:<col>"
# The message itself may contain " at ", so the location is anchored at the end.
WARNING_RE = re.compile(r"^\s*Warning:\s*(?P<msg>.*?)(?:\s+at\s+(?P<loc>\S+:\d+:\d+))?\s*$")

# A warning KIND is its message with the variable parts blanked out, so
# "`&` on parameter `m` only slows it down" and the same for `dst` count as one
# kind.  Deriving the kind mechanically (rather than from a hardcoded list) means a
# warning loft adds tomorrow is grouped correctly without touching this script.
BACKTICKED = re.compile(r"`[^`]*`")
NUMBERS = re.compile(r"\b\d+\b")
KIND_WIDTH = 72


def warning_kind(msg: str) -> str:
    """Collapse one warning message to the kind it belongs to."""
    kind = BACKTICKED.sub("`…`", msg)
    kind = NUMBERS.sub("N", kind)
    kind = " ".join(kind.split())
    if len(kind) > KIND_WIDTH:
        kind = kind[:KIND_WIDTH].rstrip() + "…"
    return kind


def parse_warnings(text: str, root: Path):
    """Split the run output into the package's OWN warnings and its deps'.

    A warning belongs to the package when its source file sits inside `root`;
    everything else came from a dependency (or the stdlib) and is reported apart.
    Each (message, location) pair is counted once however many test files
    re-parsed the same source.
    """
    own, deps = {}, {}
    root_str = str(root.resolve()) if root else None
    for line in text.splitlines():
        if "Warning:" not in line:
            continue
        m = WARNING_RE.match(line)
        if not m:
            continue
        msg = m.group("msg").strip()
        loc = (m.group("loc") or "").strip()
        if not msg:
            continue
        file_part = loc.rsplit(":", 2)[0] if loc else ""
        inside = bool(root_str) and bool(file_part) and (
            os.path.realpath(file_part).startswith(root_str + os.sep)
        )
        # A location-less warning is attributed to the package — it is more useful
        # to over-report our own debt than to lose a warning in the deps bucket.
        bucket = own if (inside or not file_part) else deps
        bucket[(msg, loc)] = {"message": msg, "location": loc, "kind": warning_kind(msg)}
    return list(own.values()), list(deps.values())


def relative(loc: str, root: Path) -> str:
    """Trim an absolute source location down to a package-relative one."""
    if not loc or not root:
        return loc
    try:
        file_part, line, col = loc.rsplit(":", 2)
        rel = os.path.relpath(os.path.realpath(file_part), str(root.resolve()))
        return f"{rel}:{line}:{col}"
    except (ValueError, OSError):
        return loc


def run_suite(pkg_dir: Path, loft: str, timeout: str) -> str:
    """Run the package's interpret suite and return its combined output.

    A failing suite is fine here: warnings print per file as it goes, and a run that
    dies on a missing system dep (ALSA, a display) still reports everything loft
    parsed.  Only an unusable loft binary is an error.
    """
    env = dict(os.environ, LOFT_TIMEOUT=timeout)
    try:
        proc = subprocess.run(
            [loft, "--interpret", "--tests", "tests"],
            cwd=str(pkg_dir),
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            errors="replace",
        )
    except FileNotFoundError:
        sys.exit(f"lib_warning_scan: loft binary '{loft}' not found")
    return proc.stdout


def sort_key(w):
    """Order warnings by file then NUMERIC line/column (so 23 precedes 230)."""
    try:
        file_part, line, col = w["location"].rsplit(":", 2)
        return (file_part, int(line), int(col))
    except (ValueError, KeyError):
        return (w.get("location", ""), 0, 0)


def kind_counts(warnings):
    """Group warnings by kind, most frequent first.

    Grouping is on the blanked template (so every `&`-parameter warning is one
    kind whatever the parameter is called), but each group carries a REAL message
    as its `example` — a template full of `…` is unreadable in a report.
    """
    groups = {}
    for w in warnings:
        g = groups.setdefault(w["kind"], {"kind": w["kind"], "count": 0, "messages": {}})
        g["count"] += 1
        g["messages"][w["message"]] = g["messages"].get(w["message"], 0) + 1
    out = []
    for g in sorted(groups.values(), key=lambda g: (-g["count"], g["kind"])):
        example = max(g["messages"].items(), key=lambda kv: kv[1])[0]
        out.append({"kind": g["kind"], "count": g["count"], "example": short(example)})
    return out


def short(msg: str) -> str:
    """One-line, report-width form of a warning message."""
    msg = cell_safe(msg)
    return msg if len(msg) <= KIND_WIDTH else msg[:KIND_WIDTH].rstrip() + "…"


def cell_safe(value: str) -> str:
    """One-line, pipe-free text — every field here can land in a markdown cell."""
    if not value:
        return ""
    return " ".join(value.split()).replace("|", "/")


# `test result: ok. 20 passed; 4 files  [scope note…]` — the runner's last line.
RESULT_RE = re.compile(r"^\s*test result:.*?(?P<files>\d+)\s+files?", re.MULTILINE)


def suite_evidence(text: str):
    """Did the suite actually run, and over how many files?

    Zero warnings is only good news when something was COMPILED to warn about.  A
    run that died before parsing — no loft on PATH, a missing dependency, an empty
    `tests/` — emits no warnings either, and reporting that as `clean` is the same
    silence-reads-as-coverage failure this report exists to expose.  So the count
    is published together with the evidence behind it, and a reading with no
    evidence is INCONCLUSIVE rather than clean.
    """
    m = RESULT_RE.search(text)
    if not m:
        line = next(
            (ln.strip() for ln in text.splitlines() if ln.strip().startswith("test result:")),
            "",
        )
        return {"ran": bool(line), "files": 0, "result": cell_safe(line)}
    files = int(m.group("files"))
    return {
        "ran": files > 0,
        "files": files,
        "result": cell_safe(m.group(0)),
    }


def cmd_scan(args) -> int:
    root = Path(args.root or args.pkg_dir or ".")
    if args.from_log:
        text = Path(args.from_log).read_text(errors="replace")
    else:
        pkg_dir = Path(args.pkg_dir)
        if not (pkg_dir / "tests").is_dir():
            sys.exit(f"lib_warning_scan: no tests/ dir under {pkg_dir}")
        text = run_suite(pkg_dir, args.loft, args.timeout)

    own, deps = parse_warnings(text, root)
    for w in own:  # the package's own locations read better relative to it
        w["location"] = relative(w["location"], root)

    evidence = suite_evidence(text)
    # Did the suite compile THIS directory's source, or the registry's copy?
    #
    # A package's own tests say `use <pkg>;`, and loft may satisfy that from the
    # registry rather than the checkout beside them — the run then prints
    # `[registry] resolving <pkg> from registry`.  Scanning an OLD version's dir
    # after a newer one is published therefore reports the NEW source's warnings:
    # hex_world 0.1.2 has seven `not null` in its src, yet scanned clean once 0.2.0
    # existed.  Harmless in the nightly (discover always checks out the LATEST tag,
    # so the two coincide) but true by luck, not construction — so the reading says
    # which source it measured instead of leaving the reader to assume.
    self_resolved = f"[registry] resolving {args.name or root.name} from registry" in text
    report = {
        "package": cell_safe(args.name or root.name),
        "label": cell_safe(args.label),
        "ref": cell_safe(args.ref),
        "count": len(own),
        "dep_count": len(deps),
        "kinds": kind_counts(own),
        "warnings": sorted(own, key=sort_key),
        "dep_kinds": kind_counts(deps),
        # The evidence behind `count` — see suite_evidence().
        "ran": evidence["ran"],
        "files": evidence["files"],
        "result": evidence["result"],
        "registry_resolved": self_resolved,
    }
    if args.json:
        out = Path(args.json)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(report, indent=2) + "\n")

    ref = f" {args.ref}" if args.ref else ""
    if not report["ran"]:
        print(
            f"{report['package']}{ref} ({args.label}): INCONCLUSIVE — the suite did not "
            f"run (no compiled file to warn about), so {report['count']} warning(s) "
            f"means nothing here"
        )
        return 0
    head = (
        f"{report['package']}{ref} ({args.label}): {report['count']} warning(s) "
        f"over {report['files']} file(s)"
    )
    if report["dep_count"]:
        head += f"  [+{report['dep_count']} in deps — not counted]"
    if self_resolved:
        head += "  [resolved from the REGISTRY — reflects the published copy, not this dir]"
    print(head)
    for g in report["kinds"]:
        print(f"  {g['count']:>3}×  {g['example']}")
    for w in report["warnings"][: args.examples]:
        print(f"       {w['location']}")
    return 0


# ── the ratchet ───────────────────────────────────────────────────────────────────────
#
# Warnings must NOT gate a shipped library: a new deprecation breaking an already-published
# artifact is exactly what COMPATIBILITY.md forbids, and the hard gate above tolerates them
# for that reason.  But "reported, never gated" turned out to report into a GREEN check that
# nobody opens — measured on this very workflow, the dirty set went **2 → 11 libraries over
# eight days with every run green**, and the first anyone heard of it was a red check in the
# library repos, on code those authors had not touched.  That is the failure mode the step's
# own comment predicted, still happening.
#
# So the gate is a RATCHET over the SET, not the count.  Every library already carrying debt
# stays tolerated — the compat rule is untouched — and a library that goes dirty for the
# FIRST time is red, at the change that did it, while the diff is still on screen.  A count
# would not do: nine can stay nine while the membership turns over completely.
#
# Falling is never a failure: a library cleaned up prints the re-pin command and exits 0, so
# a PR is never blocked by its own improvement.  Re-pin in the same commit, so the baseline
# in the diff is the receipt.
#
# ⚠ The baseline is a DERIVED row and must never be CARRIED across a join or a rebase —
# re-run and re-pin on the joined tree.
RATCHET = Path(__file__).resolve().parent.parent / "index" / "lib_warning_ratchet.json"


def ratchet(dirty, inconclusive, write: bool, partial: bool = False) -> int:
    """Compare the dirty SET against the committed baseline. `dirty` is the packages that
    warn; `inconclusive` is the readings that produced no compiled file, which are never
    counted as clean and never silently pinned.

    `partial` says the run did not read every package — a local sweep with a sibling clone
    missing, say.  A package that IS dirty and is not in the baseline is still a real
    finding then, so the failure stands; a package MISSING from the reading is not evidence
    that it was cleaned, so the shrink advice is suppressed rather than nagging for a re-pin
    the run did not earn."""
    now = sorted(dirty)
    if write:
        RATCHET.parent.mkdir(parents=True, exist_ok=True)
        RATCHET.write_text(json.dumps({"dirty": now}, indent=2) + "\n")
        print(f"lib-warning ratchet: pinned {len(now)} package(s) into {RATCHET.name}")
        return 0
    try:
        base = sorted(json.loads(RATCHET.read_text()).get("dirty", []))
    except (OSError, json.JSONDecodeError):
        print(f"::warning title=lib-warning ratchet::no baseline at {RATCHET.name} — "
              "run `--write-ratchet` to pin the current set", file=sys.stderr)
        return 0
    new = [p for p in now if p not in base]
    gone = [p for p in base if p not in now]
    for p in new:
        print(f"  NEW DIRTY  {p}")
    for p in gone:
        print(f"  cleaned    {p}")
    if new:
        print(
            f"\nERROR: {len(new)} library that was warning-clean now warns against this "
            "loft: " + ", ".join(new) + ".\n"
            "  This does not break the published artifact — that is what the hard gate\n"
            "  above checks, and it is still green.  What it DOES break is that library's\n"
            "  own CI (`LOFT_DENY_WARNINGS=1`) on its next PR, for an author who did not\n"
            "  touch the code.  Fix the warning here, or accept the debt deliberately by\n"
            "  re-pinning:  python3 scripts/lib_warning_scan.py collect <dir> --write-ratchet"
        )
        print(
            "::error title=a library newly warns::" + ", ".join(new)
            + " — clean against this loft before, warning now.",
            file=sys.stderr,
        )
        return 1
    if gone and partial:
        print(
            f"\n{len(gone)} baseline package(s) were not read in this partial run, which is "
            "not evidence they were cleaned — no re-pin is owed."
        )
    elif gone:
        print("\nThe dirty set SHRANK — re-pin it in this commit:")
        print("  python3 scripts/lib_warning_scan.py collect <dir> --write-ratchet")
    else:
        print(f"lib-warning ratchet: at baseline ({len(base)} package(s) carrying debt).")
    if inconclusive:
        print(
            "::warning title=lib-warning ratchet::"
            f"{len(inconclusive)} reading(s) were INCONCLUSIVE, so the set below is a "
            "floor, not a measurement: " + ", ".join(sorted(inconclusive)),
            file=sys.stderr,
        )
    return 0


def cmd_collect(args) -> int:
    reports = []
    for path in sorted(Path(args.dir).rglob("*.json")):
        try:
            reports.append(json.loads(path.read_text()))
        except (json.JSONDecodeError, OSError) as e:
            print(f"lib_warning_scan: skipping {path}: {e}", file=sys.stderr)
    if not reports:
        print("_no warning reports collected._")
        return 0

    # One row per package: the two readings side by side.  "published" is what a
    # user of the library sees; "source" is what the library's own CI will do.
    by_pkg = {}
    for r in reports:
        row = by_pkg.setdefault(r["package"], {})
        row[r.get("label", "?")] = r
    labels = sorted({r.get("label", "?") for r in reports})

    print("## Library warnings against this loft")
    print()
    print("Warnings never fail this gate — a new deprecation must not break a shipped")
    print("library.  But a package with warnings **fails its own CI**")
    print("(`LOFT_DENY_WARNINGS=1`) on its next PR, so this is the advance notice.")
    print()
    print("| package | " + " | ".join(labels) + " | top warning kind |")
    print("|---|" + "---|" * (len(labels) + 1))
    dirty, inconclusive, resolved = [], [], []
    for pkg in sorted(by_pkg):
        cells, kinds = [], {}
        for label in labels:
            r = by_pkg[pkg].get(label)
            if r is None:
                cells.append("–")
                continue
            n = r["count"]
            ref = f" ({r['ref']})" if r.get("ref") else ""
            # A reading whose suite never ran is NOT clean — nothing was compiled,
            # so nothing could warn.  Older reports predate this evidence field;
            # treat a missing `ran` as ran (they were all real runs).
            if not r.get("ran", True):
                cells.append("**n/a**" + ref)
                inconclusive.append(f"{pkg} ({label})")
                continue
            mark = "†" if r.get("registry_resolved") else ""
            cells.append(("clean" if n == 0 else f"**{n}**") + ref + mark)
            if mark:
                resolved.append(f"{pkg} ({label})")
            for g in r["kinds"]:  # sum a kind across the readings, keep one example
                seen = kinds.setdefault(g["kind"], {"count": 0, "example": g["example"]})
                seen["count"] += g["count"]
        top = max(kinds.values(), key=lambda g: g["count"])["example"] if kinds else "—"
        print(f"| `{pkg}` | " + " | ".join(cells) + f" | {top} |")
        if kinds:
            dirty.append(pkg)

    print()
    if resolved:
        print(
            "† the package's own tests resolved it from the REGISTRY, so this reading "
            "describes the published copy rather than the scanned directory: "
            + ", ".join(f"`{x}`" for x in resolved)
            + ". Identical for the latest version (what the nightly checks out), "
            + "different for any older one."
        )
        print()
    if inconclusive:
        print(
            f"**n/a = INCONCLUSIVE, not clean** — the suite did not run for "
            + ", ".join(f"`{x}`" for x in inconclusive)
            + ", so no file was compiled and nothing could warn. Fix the run before "
            + "reading a zero there as good news."
        )
        print()
        print(
            "::warning title=warning scan inconclusive::"
            + f"{len(inconclusive)} reading(s) produced no compiled file: "
            + ", ".join(inconclusive),
            file=sys.stderr,
        )
    if dirty:
        print(f"**{len(dirty)} package(s) carry warnings:** " + ", ".join(f"`{p}`" for p in dirty))
        print()
        for pkg in dirty:
            for label in labels:
                r = by_pkg[pkg].get(label)
                if not r or not r["kinds"]:
                    continue
                print(f"<details><summary><code>{pkg}</code> — {label} ({r['count']})</summary>")
                print()
                for g in r["kinds"]:
                    print(f"- {g['count']}× {g['example']}")
                print()
                print("```")
                for w in r["warnings"][:40]:
                    print(f"{w['location']}: {w['message']}")
                if len(r["warnings"]) > 40:
                    print(f"… and {len(r['warnings']) - 40} more")
                print("```")
                print()
                print("</details>")
                print()
        # A run-level annotation, so the debt is visible without opening the summary.
        print(
            "::warning title=library warnings::"
            + f"{len(dirty)} published librar(y/ies) warn against this loft: "
            + ", ".join(dirty)
            + " — each fails its own LOFT_DENY_WARNINGS CI on its next PR.",
            file=sys.stderr,
        )
    elif inconclusive:
        # The all-clear is a CLAIM, and it is only true of readings that happened.
        # Printing ✅ beside an n/a would re-introduce, in the summary line, exactly
        # the false green the n/a exists to prevent.
        print(
            f"No warnings found — but {len(inconclusive)} reading(s) above are "
            f"INCONCLUSIVE, so this is not an all-clear."
        )
    else:
        print("✅ every scanned library is warning-clean against this loft.")
    print()
    return ratchet(
        dirty,
        inconclusive,
        getattr(args, "write_ratchet", False),
        getattr(args, "partial", False),
    )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("scan", help="scan one package directory")
    s.add_argument("pkg_dir", nargs="?", help="package dir (holding loft.toml + tests/)")
    s.add_argument("--from-log", help="parse this captured run output instead of re-running")
    s.add_argument("--root", help="package dir the warnings are attributed to (with --from-log)")
    s.add_argument("--name", help="package name for the report (default: dir name)")
    s.add_argument(
        "--label",
        default="scan",
        help="which reading this is — keep it STABLE across packages ('published' / 'source'), "
        "it becomes a column in `collect`",
    )
    s.add_argument("--ref", help="the version or branch scanned, e.g. '0.1.2' or 'main'")
    s.add_argument("--json", help="write the JSON report here")
    s.add_argument("--loft", default=os.environ.get("LOFT", "loft"), help="loft binary")
    s.add_argument("--timeout", default=os.environ.get("LOFT_TIMEOUT", "240"))
    s.add_argument("--examples", type=int, default=10, help="locations to echo (default 10)")

    c = sub.add_parser("collect", help="merge scan reports into one markdown table")
    c.add_argument("dir", help="directory holding the JSON reports")
    c.add_argument(
        "--partial",
        action="store_true",
        help="this run did not read every package (a local sweep with clones missing), so a "
        "package absent from the reading is not treated as cleaned",
    )
    c.add_argument(
        "--write-ratchet",
        action="store_true",
        help="pin the current dirty SET as the new baseline (the receipt for a deliberate "
        "acceptance, or for a clean-up that shrank it)",
    )

    args = ap.parse_args()
    if args.cmd == "scan":
        if not args.from_log and not args.pkg_dir:
            ap.error("scan needs a package dir (or --from-log with --root)")
        return cmd_scan(args)
    return cmd_collect(args)


if __name__ == "__main__":
    sys.exit(main())
