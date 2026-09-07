#!/usr/bin/env python3
"""Compose the nextest filterset for a CI test leg.

Two callers needed the same expression and would otherwise each spell it: the
unsharded push/nightly matrix, and the two-way sharded PR path.  A filter
duplicated across workflow steps drifts silently — a test excluded on one leg
and not the other reads as a flake — so it is built here, once.

Usage:  ci_test_filter.py <event_name> [heavy|rest|rest-a|rest-b]

The optional shard restricts the leg to the `heavy-serial` test group, or to
its complement.  The group's membership is NOT repeated here: it is read out of
`.config/nextest.toml`, which is where the group is defined and where nextest
itself reads it from.  (nextest has no `test_group()` filterset predicate as of
0.9.138 — checked — so a shard boundary has to be spelled as binaries, and this
is what keeps that spelling honest.)
"""

import sys
try:
    import tomllib
except ModuleNotFoundError:  # python < 3.11 (macOS ships 3.9)
    try:
        import tomli as tomllib
    except ModuleNotFoundError:
        import sys
        sys.exit("needs python >= 3.11 (tomllib) or `pip3 install tomli`")
from pathlib import Path

NEXTEST_TOML = Path(__file__).resolve().parent.parent / ".config" / "nextest.toml"

# `index_hygiene` and `viewer_markdown` are extracted to their own advisory ubuntu
# jobs (and the nightly): both are platform-independent — a whole-repo doc-link
# check, and the markdown renderer's HTML output — so there is no value running
# them 3x in the required matrix.  `viewer_markdown` is also an HTTP smoke test
# whose interpreted viewer gets starved by this suite's parallel native-build load
# (empty response under contention), so it runs ISOLATED in the `viewer-smoke` job
# where it's reliable.
BASE = ["not binary(index_hygiene)", "not binary(viewer_markdown)"]

# The two exhaustive stdlib round-trips are excluded from EVERY leg, because they
# have a leg of their own: ci.yml's `Stdlib round-trip` step runs them on
# push-to-main and nightly, on every platform, in parallel with nothing else.
# What they verify is FORMAT STABILITY — the whole parsed stdlib survives
# serialise->deserialise byte-identical — which breaks when the IR schema or the
# serialiser changes, so it is rare and always deliberate.  The cheap canary
# `tests_scripts_round_trip` (83s) stays on the PR path and still fails if
# round-tripping breaks at all.
#
# Excluding them only on the PR path ran them TWICE per push: once here in the
# contended suite, once alone in their own step.  The contended copy is the
# expensive one — 501s vs 269s on Windows, 493s vs 232s on ubuntu, where it also
# SET the suite's critical path, since a parallel suite cannot finish faster than
# its slowest single test.  It is also what turned the Windows leg red: contended,
# the pair rides nextest's 600s `slow-timeout` (545/533/571/589/501s over
# 08-01..08-09) and hit the cap on three consecutive runs once the suite's total
# load grew 16% (1844s -> 2141s) on a 1% test-count rise.  Measured ISOLATED the
# pair did not get slower across those same shas (76.6s -> 79.9s), so the cap was
# never the real problem and raising it would only have hidden the double-run.
DEDICATED_STEP = [
    "not test(stdlib_load_compares_equal_to_fresh)",
    "not test(stdlib_whole_data_round_trip)",
]

# The Chrome + SwiftShader browser-render tests (html_render, and the headless-page
# asyncify resume in html_asyncify) are GPU/headless-browser FLAKY — they gate the
# PR path with noise for a layer (WebGL/shader) that rarely regresses
# independently.  Keep them OFF the per-PR run; they run nightly (like the
# differential oracle).  The DETERMINISTIC, node-based html_wasm instantiate-probe
# (catches the LinkError / import-mismatch class) STAYS on the PR path — it does
# not flake.
PR_ONLY = [
    "not binary(html_render)",
    "not binary(html_asyncify)",
]


# The shard boundary is EVERY single-slot test group, not just `heavy-serial`.
#
# A `max-threads = 1` group scattered across shards pins every shard holding a member to
# a serial floor while the work divides unevenly — the root cause ci.yml records for both
# reverted sharding strategies.  Cutting along `heavy-serial` fixed that for one group and
# left `html-wasm-serial` (139s over 32 tests) in `rest`, which is the critical path: the
# same defect, one group over.
#
# This is a SHARD boundary, not a runtime grouping.  The two groups stay separate in
# `.config/nextest.toml` on purpose — `heavy-serial` exists so a native rustc storm never
# starves a timing-sensitive server test, which is a different question from which JOB a
# binary runs in — so the fix belongs here and not in a group merge.
SERIAL_GROUPS = ["heavy-serial", "html-wasm-serial"]


# The heavier half of `rest`, BY DURATION.  ci.yml records that a duration-balanced split
# was tried and reverted — but for a reason that no longer applies: it scattered the
# single-slot `heavy-serial` group across shards, and every such group now lives whole in
# the `heavy` shard, so there is nothing left in `rest` to scatter.  That is the axis none
# of the three earlier attempts tried.
#
# Named as the HEAVY HALF, with the light half taken as its COMPLEMENT.  That asymmetry is
# the safety property: a binary added later and not listed here lands in `rest-b` and still
# runs.  A pair of explicit lists could omit one silently, and a test that runs on no leg
# is the one failure mode a sharding scheme must not have.
#
# Measured 2026-08-28 from a full `make ci` (per-binary seconds; regenerate the same way
# and re-pack when the halves drift): A 1317s over 16 binaries, B 1285s over 200 — 1.2
# percent apart.  Drift costs only balance, never coverage, so re-measuring is a tuning
# job and not a gate.
REST_HEAVY_HALF = [
    "issues",
    "store_persist_loft",
    "mut_closure_matrix",
    "deliver_wasm",
    "ir_schema_roundtrip",
    "html_gl_imports",
    "tuple_matrix",
    "issue_896_nullable_field",
    "native_loader",
    "binary_io_matrix",
    "template_matrix",
    "engine_host_connector",
    "closure_matrix",
    "use_analysis",
    "engine_host_kernel",
    "parse_errors",
]


def rest_half(heavy_half: bool) -> str:
    """The filterset for one half of `rest`, split by measured duration."""
    named = " + ".join(f"binary({b})" for b in REST_HEAVY_HALF)
    return f"({named})" if heavy_half else f"not ({named})"


def serial_boundary() -> str:
    """The filterset selecting every binary in a single-slot group — the shard cut."""
    return " + ".join(f"({group_filter(g)})" for g in SERIAL_GROUPS)


def group_filter(group: str) -> str:
    """The filterset nextest itself uses to populate `group`."""
    with NEXTEST_TOML.open("rb") as fh:
        config = tomllib.load(fh)
    for profile in ("ci", "default"):
        for override in config["profile"].get(profile, {}).get("overrides", []):
            if override.get("test-group") == group:
                return override["filter"]
    raise SystemExit(f"no override defines test-group '{group}' in {NEXTEST_TOML}")


def main() -> None:
    if not 2 <= len(sys.argv) <= 3:
        raise SystemExit(__doc__)
    event, shard = sys.argv[1], (sys.argv[2] if len(sys.argv) == 3 else None)

    clauses = list(BASE) + list(DEDICATED_STEP)
    if event == "pull_request":
        clauses += PR_ONLY
    if shard == "heavy":
        clauses.append(f"({serial_boundary()})")
    elif shard in ("rest", "rest-a", "rest-b"):
        clauses.append(f"not ({serial_boundary()})")
        if shard != "rest":
            clauses.append(rest_half(shard == "rest-a"))
    elif shard is not None:
        raise SystemExit(
            f"unknown shard '{shard}' (expected 'heavy', 'rest', 'rest-a' or 'rest-b')"
        )

    print(" and ".join(clauses))


if __name__ == "__main__":
    main()
