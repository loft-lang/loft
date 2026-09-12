#!/usr/bin/env python3
"""Compose the nextest filterset for a CI test leg.

Two callers needed the same expression and would otherwise each spell it: the
unsharded push/nightly matrix, and the two-way sharded PR path.  A filter
duplicated across workflow steps drifts silently — a test excluded on one leg
and not the other reads as a flake — so it is built here, once.

Usage:  ci_test_filter.py <event_name> [heavy|corpus|rest|rest-a|rest-b]

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


# The native script corpus — ONE test, `native::native_scripts`, that compiles every
# `tests/scripts/*.loft` through `--native` — gets a shard of its own (@PLN159 phase E1).
#
# It lives in the `native` binary, which is in `heavy-serial`, so before this cut it ran
# inside the heavy shard's single slot: 219.6 s of a 764.8 s nextest phase on the PR run
# that measured it (job 101548309931, 2026-09-06), and the corpus doubled in the month
# before (613 → 1206 files), so it is the one heavy-shard item that grows with every
# guard.  On its own runner it overlaps everything instead of nothing.  The rest of the
# `native` binary stays in `heavy`, where its rustc storms still cannot starve a server
# test — the shard boundary is a JOB question; the group stays as it is in nextest.toml.
#
# `test(=name)` is nextest's EXACT matcher; the substring form would also take any
# later test whose name merely contains `native_scripts`, and a test in two shards is
# the failure mode the partition proof below exists to catch.
CORPUS = "binary(native) & test(=native_scripts)"


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
# ⚠ THIS LIST DRIFTS BY CONSTRUCTION, and the drift is one-directional: the light half is
# the COMPLEMENT, so every test binary added to the repo lands in `rest-b` and never in
# `rest-a`.  Balance therefore decays monotonically between re-packs, and the re-pack is
# the maintenance this scheme trades for its safety property.  Re-measure whenever
# `rest-b`'s wall clock pulls away from `rest-a`'s.
#
# Measured 2026-08-28: A 1317s over 16 binaries, B 1285s over 200 — 1.2 percent apart.
# RE-MEASURED 2026-09-12 from a full local `nextest --profile ci` run's `junit.xml`
# (`target/nextest/ci/junit.xml`; sum each testcase's `time` per binary): A 2973s, B 5417s
# — **82 percent apart**, and the CI wall clock showed it (rest-b 21.4 min against rest-a
# 16.1 on the last PR-path run, the leg that put the run over its 20-minute budget).
# Re-packed by moving the five heaviest unlisted binaries across, which lands at 4199s /
# 4192s — 0.2 percent apart — for the smallest possible change to the list.
#
# To regenerate: run the suite once with `--profile ci`, then sum `time` per testsuite from
# the junit report, drop the `heavy`/`corpus` binaries and the four excluded ones, and move
# the largest unnamed binaries across until the halves meet.
#
# Drift costs only balance, never coverage, so re-measuring is a tuning job and not a gate.
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
    # Added by the 2026-09-12 re-pack (seconds from that run).
    "e1_code_set",      # 515.2s
    "heap_nstore",      # 194.1s
    "coroutine_matrix", # 190.8s
    "wrap",             # 166.9s
    "exit_codes",       # 158.7s
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
        clauses.append(f"not ({CORPUS})")
    elif shard == "corpus":
        clauses.append(f"({CORPUS})")
    elif shard in ("rest", "rest-a", "rest-b", "rest-c"):
        # `rest-a/b/c` are ONE filterset, split by `nextest --partition hash:i/3` in the
        # workflow rather than by a named list here.  The list-based half (REST_HEAVY_HALF,
        # kept below for the record) could not hold: its light side is the COMPLEMENT, so
        # every binary added to the repo landed there and the balance decayed in one
        # direction — measured 1.2 % apart when written and 82 % apart three weeks later.
        # A partition cannot drift, because nothing has to be maintained.
        #
        # ci.yml records that hash partitioning was tried across the WHOLE suite and
        # reverted, for a reason that does not reach here: it scattered the single-slot
        # serial groups across shards, pinning each to a serial floor.  Every such group
        # now lives whole in `heavy`, so `rest` has none left to scatter — the same
        # argument that admitted the duration split, applied one step further.  And the
        # partition is over TESTS, not binaries, so a 765-second binary spreads across all
        # three legs instead of pinning one.
        clauses.append(f"not ({serial_boundary()})")
        clauses.append(f"not ({CORPUS})")
    elif shard is not None:
        raise SystemExit(
            f"unknown shard '{shard}' (expected 'heavy', 'corpus', 'rest', 'rest-a' or 'rest-b')"
        )

    print(" and ".join(clauses))


if __name__ == "__main__":
    main()
