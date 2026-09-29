#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Where a CI test leg's time goes: per shard, and for a whole PR run.

Every test leg writes nextest's junit (`target/nextest/ci/junit.xml`): each test's
start and duration.  `ci.yml` uploads it and this reads it, so a slow shard is
EXPLAINED rather than guessed at:

  * wall against the summed test time, and how many tests ran at once on average;
  * the TAIL — how long the leg ran with at most two tests left, and which test
    was last: a leg that is long because one test started late is a different
    problem from one that is long because it holds too much work;
  * the slowest tests, and (for a PR) the tests slower than on the default branch's
    last run, whose junit is the baseline.

    ci_timing.py shard JUNIT [LABEL]   # markdown for the leg's job summary
    ci_timing.py report RUN_ID         # the PR comment, updated in place
    ci_timing.py dry-run RUN_ID        # print the PR comment; writes nothing
    ci_timing.py selftest              # against tests/fixtures/ci_timing/
"""

import datetime
import json
import os
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "tests" / "fixtures" / "ci_timing"
MARKER = "<!-- ci-timing -->"
ARTIFACT_PREFIX = "nextest-junit-"


def when(stamp: str) -> float:
    return datetime.datetime.fromisoformat(stamp.replace("Z", "+00:00")).timestamp()


def parse(path: Path) -> dict:
    root = ET.parse(path).getroot()
    start = when(root.get("timestamp")) if root.get("timestamp") else None
    tests = []
    for suite in root.iter("testsuite"):
        for tc in suite.iter("testcase"):
            t0 = when(tc.get("timestamp")) if tc.get("timestamp") else None
            flaky = tc.find("flakyFailure") is not None or tc.find("rerunFailure") is not None
            tests.append({"binary": suite.get("name"), "name": tc.get("name"),
                          "start": t0, "secs": float(tc.get("time") or 0), "flaky": flaky})
    return {"wall": float(root.get("time") or 0), "start": start, "tests": tests}


def shape(run: dict) -> dict:
    """Wall, work, parallelism, and the tail: the stretch at the end with <= 2 tests running."""
    tests = [t for t in run["tests"] if t["start"] is not None]
    work = sum(t["secs"] for t in run["tests"])
    wall = run["wall"] or 1e-9
    out = {"wall": wall, "work": work, "tests": len(run["tests"]), "parallel": work / wall,
           "tail": 0.0, "last": None, "flaky": [t for t in run["tests"] if t["flaky"]]}
    if tests and run["start"]:
        end = run["start"] + wall
        events = sorted([(t["start"], 1) for t in tests] + [(t["start"] + t["secs"], -1) for t in tests])
        running, tail_from = 0, end
        for at, delta in events:
            running += delta
            if running > 2:
                tail_from = None
            elif tail_from is None:
                tail_from = at
        out["tail"] = max(0.0, end - tail_from) if tail_from is not None else 0.0
        last = max(tests, key=lambda t: t["start"] + t["secs"])
        out["last"] = last
    return out


def slowest(run: dict, n: int) -> list[dict]:
    return sorted(run["tests"], key=lambda t: -t["secs"])[:n]


def mins(s: float) -> str:
    return f"{s / 60:.1f} min" if s >= 90 else f"{s:.0f} s"


def short(t: dict) -> str:
    b = (t["binary"] or "").removeprefix("loft::")
    return f"`{b}::{t['name']}`" if b and b != "loft" else f"`{t['name']}`"


def shard_markdown(path: Path, label: str) -> str:
    run = parse(path)
    s = shape(run)
    lines = [f"### Test timing — {label}", "",
             f"{s['tests']} tests · wall **{mins(s['wall'])}** · test time {mins(s['work'])} · "
             f"{s['parallel']:.1f} running at once on average"]
    if s["last"]:
        lines.append(f"Tail: the last **{mins(s['tail'])}** ran with at most two tests; the last to "
                     f"finish was {short(s['last'])} ({mins(s['last']['secs'])}).")
    if s["flaky"]:
        lines.append(f"Passed only on retry: {', '.join(short(t) for t in s['flaky'][:10])}")
    lines += ["", "| secs | test |", "|---:|---|"]
    lines += [f"| {t['secs']:.1f} | {short(t)} |" for t in slowest(run, 12)]
    return "\n".join(lines) + "\n"


# ── the PR report ───────────────────────────────────────────────────────────────

def gh(*args: str, check: bool = True, input_text: str | None = None) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True, input=input_text)
    if check and r.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args[:3])}…: {r.stderr.strip()[:300]}")
    return r.stdout


def api(path: str) -> object:
    return json.loads(gh("api", path) or "null")


def repo() -> str:
    return os.environ.get("GH_REPO") or gh("repo", "view", "--json", "nameWithOwner",
                                           "--jq", ".nameWithOwner").strip()


def download(run_id: int, dest: Path) -> dict[str, Path]:
    """The run's junit artifacts: artifact name (without the prefix) -> junit path."""
    gh("run", "download", str(run_id), "-p", f"{ARTIFACT_PREFIX}*", "-D", str(dest), check=False)
    out = {}
    for d in sorted(dest.glob(f"{ARTIFACT_PREFIX}*")):
        junit = next(d.rglob("junit.xml"), None)
        if junit:
            out[d.name.removeprefix(ARTIFACT_PREFIX)] = junit
    return out


def steps_of(job: dict) -> tuple[float, float, float]:
    """(setup, test, after) seconds of a test leg: the steps before `Test`, it, and after."""
    phase, acc = 0, [0.0, 0.0, 0.0]
    for s in job.get("steps") or []:
        if not s.get("started_at") or not s.get("completed_at"):
            continue
        d = when(s["completed_at"]) - when(s["started_at"])
        if s["name"] == "Test":
            acc[1] += d
            phase = 2
        else:
            acc[phase] += d
    return acc[0], acc[1], acc[2]


def baseline(r: str, run: dict) -> tuple[dict | None, dict]:
    """The default branch's last CI run before this one that kept a junit: test -> secs."""
    default = api(f"repos/{r}")["default_branch"]
    runs = api(f"repos/{r}/actions/workflows/{run['workflow_id']}/runs?branch={default}"
               f"&status=completed&per_page=20")["workflow_runs"]
    for other in runs:
        if other["event"] == "pull_request" or other["created_at"] >= run["created_at"]:
            continue
        with tempfile.TemporaryDirectory() as tmp:
            found = download(other["id"], Path(tmp))
            linux = [p for name, p in found.items() if name.startswith("ubuntu")]
            if not linux:
                continue
            secs = {}
            for p in linux:
                for t in parse(p)["tests"]:
                    secs[(t["binary"], t["name"])] = t["secs"]
            return other, secs
    return None, {}


def report_markdown(r: str, run_id: str) -> tuple[dict, str]:
    run = api(f"repos/{r}/actions/runs/{run_id}")
    jobs = [j for j in api(f"repos/{r}/actions/runs/{run_id}/jobs?per_page=100")["jobs"]
            if j["name"].startswith("Test (") and j.get("completed_at") and j["conclusion"] != "skipped"]
    with tempfile.TemporaryDirectory() as tmp:
        found = download(int(run_id), Path(tmp))
        runs = {name: parse(p) for name, p in found.items()}
    base_run, base = baseline(r, run)
    rows, leg_total = [], {}
    for j in sorted(jobs, key=lambda j: j["name"]):
        setup, test, after = steps_of(j)
        if test == 0:
            continue  # a placeholder leg (macOS / Windows on a PR) that runs no tests
        total = when(j["completed_at"]) - when(j["started_at"])
        leg = j["name"].split("(", 1)[1].rstrip(")").replace(") [", "-").rstrip("]")
        key = next((k for k in runs if k == leg or k == f"{leg}-full"), None)
        s = shape(runs[key]) if key else None
        leg_total[j["name"]] = total
        rows.append((j, total, setup, test, after, s))
    crit = max(leg_total, key=leg_total.get) if leg_total else None
    out = [MARKER, "### CI timing", "",
           f"[Run {run['id']}]({run['html_url']}) @ `{run['head_sha'][:9]}`"
           + (f" · baseline: `{base_run['head_branch']}` [run {base_run['id']}]({base_run['html_url']})"
              if base_run else " · no baseline run on the default branch kept a junit yet"),
           "", "| leg | total | setup | test | after | tests | running at once | tail | last to finish |",
           "|---|---:|---:|---:|---:|---:|---:|---:|---|"]
    for j, total, setup, test, after, s in rows:
        name = f"**{j['name']}**" if j["name"] == crit else j["name"]
        extra = (f"{s['tests']} | {s['parallel']:.1f} | {mins(s['tail'])} | "
                 f"{short(s['last']) if s['last'] else ''}") if s else "— | — | — | —"
        out.append(f"| [{name}]({j['html_url']}) | {mins(total)} | {mins(setup)} | {mins(test)} | "
                   f"{mins(after)} | {extra} |")
    everything = [t for run_ in runs.values() for t in run_["tests"]]
    if everything:
        out += ["", "<details><summary>Slowest tests</summary>", "", "| secs | test |", "|---:|---|"]
        out += [f"| {t['secs']:.1f} | {short(t)} |" for t in sorted(everything, key=lambda t: -t["secs"])[:20]]
        out += ["", "</details>"]
    if base and everything:
        slower = []
        for t in everything:
            was = base.get((t["binary"], t["name"]))
            if was is not None and t["secs"] - was >= 10 and t["secs"] >= 1.5 * was:
                slower.append((t["secs"] - was, t, was))
        slower.sort(key=lambda x: -x[0])
        if slower:
            out += ["", "**Slower than the baseline** (≥ 10 s and ≥ 1.5×):", "",
                    "| now | baseline | test |", "|---:|---:|---|"]
            out += [f"| {t['secs']:.0f} s | {was:.0f} s | {short(t)} |" for _, t, was in slower[:12]]
    flaky = [t for t in everything if t["flaky"]]
    if flaky:
        out += ["", "Passed only on retry: " + ", ".join(short(t) for t in flaky[:12])]
    out += ["", "_Setup is every step before `Test`; after is every step since.  Tail is the stretch "
            "at the end with at most two tests running.  Written by `scripts/ci_timing.py`._"]
    return run, "\n".join(out)


def post(r: str, run: dict, body: str) -> None:
    prs = run.get("pull_requests") or api(f"repos/{r}/commits/{run['head_sha']}/pulls")
    if not prs:
        print("no PR for this run; nothing posted")
        return
    pr = prs[0]["number"]
    comments = api(f"repos/{r}/issues/{pr}/comments?per_page=100")
    mine = next((c for c in comments if MARKER in c["body"]), None)
    if mine:
        gh("api", "-X", "PATCH", f"repos/{r}/issues/comments/{mine['id']}", "-f", f"body={body}")
    else:
        gh("api", f"repos/{r}/issues/{pr}/comments", "-f", f"body={body}")
    print(f"timing report on PR #{pr}")


# ── selftest ────────────────────────────────────────────────────────────────────

def selftest() -> int:
    run = parse(FIXTURES / "junit.xml")
    s = shape(run)
    bad = []
    if s["tests"] != 5:
        bad.append(f"tests {s['tests']} != 5")
    if abs(s["work"] - 170.0) > 0.01:
        bad.append(f"work {s['work']} != 170")
    if abs(s["parallel"] - 1.7) > 0.01:
        bad.append(f"parallel {s['parallel']:.2f} != 1.70")
    # a_one 0-40, b_two 0-30, c_three 0-20, d_four 10-30, e_long 40-100, wall 100: three or more
    # run until 30 s (a_one, b_two and d_four at 20 s), one from then on, so the tail is 70 s.
    if abs(s["tail"] - 70.0) > 0.01:
        bad.append(f"tail {s['tail']} != 70")
    if not s["last"] or s["last"]["name"] != "e_long":
        bad.append(f"last {s['last'] and s['last']['name']} != e_long")
    if [t["name"] for t in s["flaky"]] != ["b_two"]:
        bad.append(f"flaky {[t['name'] for t in s['flaky']]} != ['b_two']")
    md = shard_markdown(FIXTURES / "junit.xml", "fixture")
    if "e_long" not in md or "1.7 running at once" not in md:
        bad.append("shard markdown lacks the slowest test or the parallelism")
    for b in bad:
        print("FAIL", b)
    print("selftest:", "ok" if not bad else f"{len(bad)} failure(s)")
    return 1 if bad else 0


if __name__ == "__main__":
    a = sys.argv[1:]
    if a[:1] == ["selftest"]:
        sys.exit(selftest())
    if a[:1] == ["shard"] and len(a) in (2, 3):
        path = Path(a[1])
        if not path.exists():
            print(f"### Test timing\n\nno junit at `{path}` — the test step did not run to the end.")
            sys.exit(0)
        print(shard_markdown(path, a[2] if len(a) == 3 else path.parent.name))
        sys.exit(0)
    if a[:1] in (["report"], ["dry-run"]) and len(a) == 2:
        r = repo()
        run, body = report_markdown(r, a[1])
        if a[0] == "dry-run":
            print(body)
        else:
            post(r, run, body)
        sys.exit(0)
    print(__doc__)
    sys.exit(2)
