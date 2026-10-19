#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The weekly stability read — every meter of STABILITY_ROADMAP.md § The owner's directive,
computed for each week and kept as a committed series.

    scripts/stability_read.py                     # the last four weeks, printed
    scripts/stability_read.py --since 2026-08-03  # every week from there
    scripts/stability_read.py --record            # …and write the rows into
                                                  #   index/stability_weekly.tsv (a week already
                                                  #   there is rewritten: the current week is
                                                  #   partial until it ends)
    scripts/stability_read.py --cache issues.json # reuse a tracker download (offline)

Why
---
The directive names the meters and says each is a query, and the read of 2026-09-28 was
assembled by hand from the tracker, git, the portal results and the nightlies — ten weekly
points in a prose table.  Hand assembly is where the misreads come from: on 2026-10-19 the
guard tier was undercounted because the suite checkout was absent and nothing said so, and
the rewrite census was invisible to the count.  This script asks every source the same way
every week and writes one row, so the December read is a file to open, not a table to build.

One row per ISO week (Monday).  The columns, by source:

  the tracker (loft-lang/loft issues, `gh api` when it is signed in, the public REST
  endpoint when it is not — the same answer, unauthenticated reads are rate-limited to
  sixty an hour, a cache file lifts that):
    filed              issues created that week
    silent_wrong       of those, `silent-wrong`
    sev_high           of those, `sev:high`
    consumer_hit       of those, `hit-by:` naming a project other than loft
    regression         of those, `regression`
    ci_shaped          of those, `ci-advisory` / `ci-infra` / `nightly-failure`
    closed             issues closed that week
    strained, settled  of those, `contract:strained` / `contract:settled` — the meter
    strained_pct       strained over judged (strained + settled)
    zero_streak        consecutive weeks, this one included, with judged > 0 and
                       strained = 0 — the directive's "0 for two consecutive weeks" is
                       zero_streak >= 2
    close_h_median     median hours from filing to close, issues filed that week
    open_sw_main       `silent-wrong` open at the end of the week without
                       `fixed-pending-merge` — the freeze meter; approximated for a past
                       week from created/closed stamps (labels are read as they are now)
  the consumers (commits per week in the public consumer repositories — dormant consumers
  read as a stable language, which the roadmap measured as the September mistake):
    consumer_commits
  this repository at the week's last commit on main:
    rules, rules_R     `@FR-` rules defined, and the rewrite family — your own criterion:
                       stable is when R stops growing
    rules_R_new        rewrite rules added that week
    annotated          rules a src/ site cites
    guarded            rules a test under tests/ names OR the rewrite census admits
                       (the suite checkout is outside git, so this is the in-repo tier;
                       `make rule-coverage` with the suite beside the tree reads higher)
    census_only        guarded by the census alone
    deviations_open    the formal chapters' stated OPEN counts, summed
    decisions          `@C` entries in the design-decision registers
  the nightlies (workflow runs, public):
    consumer_green     scheduled consumer-main-health runs green that week / run
    release_gate_age   days since the last release-gate run at the week's end, and its
                       conclusion
  the portal (bench/portal/results at the week's last commit, one entry per host; a
  ratio compares within one machine only):
    portal             host=date:median/over3/over5 …

A REPORT, never a gate.  `--record` is the only write.
"""
import argparse
import collections
import datetime as dt
import json
import os
import re
import statistics
import subprocess
import sys
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SERIES = os.path.join(ROOT, "index", "stability_weekly.tsv")
REPO = "loft-lang/loft"
CONSUMERS = ["jjstwerff/moros", "jjstwerff/dryopea", "jjstwerff/crawler"]
DEF_INLINE = re.compile(r"^\s*\(([A-Z][A-Za-z0-9-]{1,40})\)\s", re.M)
CITE = re.compile(r"@FR-([A-Z][A-Za-z0-9-]{1,40})(?![-A-Za-z0-9])")
COLUMNS = ["week_of", "partial", "filed", "silent_wrong", "sev_high", "consumer_hit", "regression",
           "ci_shaped", "closed", "strained", "settled", "strained_pct", "zero_streak",
           "close_h_median", "open_sw_main", "consumer_commits", "rules", "rules_R", "rules_R_new",
           "annotated", "guarded", "census_only", "deviations_open", "decisions",
           "consumer_green", "release_gate_age", "portal"]


# ── sources ───────────────────────────────────────────────────────────────────────────────

def gh_signed_in():
    try:
        return subprocess.run(["gh", "auth", "status"], capture_output=True, text=True).returncode == 0
    except FileNotFoundError:
        return False


def api(path, params=None, signed_in=None):
    """One GitHub API page as parsed JSON: `gh api` when signed in, the public endpoint
    otherwise.  `path` is below https://api.github.com/."""
    if signed_in is None:
        signed_in = gh_signed_in()
    query = "&".join(f"{k}={v}" for k, v in (params or {}).items())
    url = f"{path}?{query}" if query else path
    if signed_in:
        out = subprocess.run(["gh", "api", url], capture_output=True, text=True)
        if out.returncode != 0:
            sys.exit(f"gh api {url}: {out.stderr.strip()}")
        return json.loads(out.stdout)
    req = urllib.request.Request("https://api.github.com/" + url,
                                 headers={"Accept": "application/vnd.github+json",
                                          "User-Agent": "loft-stability-read"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read().decode("utf-8"))


def issues_since(since, cache=None):
    """Every issue (no pull request) created on or after `since`, newest first."""
    if cache and os.path.exists(cache):
        return json.load(open(cache, encoding="utf-8"))
    signed = gh_signed_in()
    out, page = [], 1
    while True:
        batch = api(f"repos/{REPO}/issues",
                    dict(state="all", sort="created", direction="desc", per_page=100, page=page), signed)
        if not batch:
            break
        for i in batch:
            if "pull_request" in i:
                continue
            out.append(dict(number=i["number"], created_at=i["created_at"], closed_at=i.get("closed_at"),
                            state=i["state"], labels=[l["name"] for l in i["labels"]]))
        if batch[-1]["created_at"][:10] < since.isoformat():
            break
        page += 1
    if cache:
        json.dump(out, open(cache, "w", encoding="utf-8"))
    return out


def consumer_commits(since, until):
    """Commits in the consumer repositories between the dates: {repo: [date, …]}; a repository
    that cannot be read (private to this token, renamed) is left OUT, and said once on stderr,
    so that an unreadable consumer reads as unknown rather than as idle."""
    out = {}
    signed = gh_signed_in()
    for repo in CONSUMERS:
        dates, page = [], 1
        while True:
            try:
                batch = api(f"repos/{repo}/commits",
                            dict(since=f"{since}T00:00:00Z", until=f"{until}T23:59:59Z", per_page=100, page=page),
                            signed)
            except Exception as e:
                print(f"  (consumer {repo}: {e} — its commits read as unknown)", file=sys.stderr)
                dates = None
                break
            if not isinstance(batch, list) or not batch:
                break
            dates.extend(c["commit"]["committer"]["date"][:10] for c in batch)
            if len(batch) < 100:
                break
            page += 1
        if dates is not None:
            out[repo] = dates
    return out


def workflow_runs(workflow, pages=3):
    """The newest runs of one workflow file: [(created date, event, conclusion)]."""
    signed = gh_signed_in()
    out = []
    for page in range(1, pages + 1):
        try:
            batch = api(f"repos/{REPO}/actions/workflows/{workflow}/runs",
                        dict(per_page=100, page=page), signed)
        except Exception as e:
            print(f"  ({workflow}: {e})", file=sys.stderr)
            break
        runs = batch.get("workflow_runs", [])
        out.extend((r["created_at"][:10], r["event"], r["conclusion"] or r["status"]) for r in runs)
        if len(runs) < 100:
            break
    return out


def git(*args):
    return subprocess.run(["git", "-C", ROOT, *args], capture_output=True, text=True).stdout


def main_ref():
    return "origin/main" if git("rev-parse", "--verify", "-q", "origin/main").strip() else "HEAD"


def tree_at(ref, day):
    """The last commit of `ref` on or before the end of `day`."""
    return git("rev-list", "-1", f"--before={day.isoformat()}T23:59:59", ref).strip()


def show(sha, path):
    return git("show", f"{sha}:{path}")


def files_at(sha):
    return git("ls-tree", "-r", "--name-only", sha).split("\n")


def repo_row(sha):
    """The in-repository columns at one commit."""
    files = files_at(sha)
    rules = set()
    deviations = 0
    for f in files:
        if f.startswith("doc/claude/formal/") and f.endswith(".md"):
            text = show(sha, f)
            rules.update(DEF_INLINE.findall(text))
            deviations += sum(int(x) for x in re.findall(r"OPEN:\s*\**\s*(\d+)", text))
    cited = {d: set() for d in ("src", "tests")}
    for d in cited:
        for tag in git("grep", "-ho", r"@FR-[A-Za-z0-9-]*", sha, "--", d).split("\n"):
            m = CITE.match(tag.strip())
            if m:
                cited[d].add(m.group(1))
    census = set()
    for line in show(sha, "bench/portal/rewrite_census.tsv").split("\n"):
        parts = line.split("\t")
        if len(parts) >= 4 and not line.startswith("#"):
            try:
                if int(parts[3]) > 0:
                    census.add(parts[2].strip())
            except ValueError:
                pass
    named = cited["tests"] & rules
    census &= rules
    decisions = set()
    for f in files:
        if f.startswith("doc/claude/DESIGN_DECISIONS") and f.endswith(".md"):
            decisions.update(re.findall(r"^#{2,3}\s+`?@?(C\d+)\b", show(sha, f), re.M))
    portal = []
    for f in files:
        if f.startswith("bench/portal/results/") and f.endswith(".tsv") and not f.endswith("-builds.tsv"):
            meta, ratios = {}, []
            for line in show(sha, f).split("\n"):
                if line.startswith("#"):
                    if "=" in line:
                        k, v = line[1:].split("=", 1)
                        meta[k.strip()] = v.strip()
                    continue
                parts = line.split("\t")
                if len(parts) > 11:
                    try:
                        ratios.append(float(parts[11]))
                    except ValueError:
                        pass
            if ratios:
                portal.append(f"{meta.get('host', os.path.basename(f)[:-4])}={meta.get('date', '?')}:"
                              f"{statistics.median(ratios):.2f}/{sum(x > 3 for x in ratios)}/{sum(x > 5 for x in ratios)}")
    return dict(rules=len(rules), rules_R=sum(r.startswith("R-") for r in rules),
                annotated=len(cited["src"] & rules), guarded=len(named | census),
                census_only=len(census - named), deviations_open=deviations, decisions=len(decisions),
                portal=" ".join(portal) or "-")


# ── the rows ──────────────────────────────────────────────────────────────────────────────

def week_of(day):
    return day - dt.timedelta(days=day.weekday())


def parse_day(stamp):
    return dt.date.fromisoformat(stamp[:10])


def hours_between(a, b):
    fa = dt.datetime.fromisoformat(a.replace("Z", "+00:00"))
    fb = dt.datetime.fromisoformat(b.replace("Z", "+00:00"))
    return (fb - fa).total_seconds() / 3600


def rows(since, until, cache=None):
    today = dt.date.today()
    weeks = []
    w = week_of(since)
    while w <= until:
        weeks.append(w)
        w += dt.timedelta(days=7)
    issues = issues_since(weeks[0] - dt.timedelta(days=60), cache)
    commits = consumer_commits(weeks[0], weeks[-1] + dt.timedelta(days=6))
    consumer_runs = workflow_runs("consumer-main-health.yml")
    gate_runs = workflow_runs("release-gate.yml", pages=1)
    ref = main_ref()
    out, streak, prev_r = [], 0, None
    for w in weeks:
        end = w + dt.timedelta(days=6)
        filed = [i for i in issues if w <= parse_day(i["created_at"]) <= end]
        closed = [i for i in issues if i["closed_at"] and w <= parse_day(i["closed_at"]) <= end]
        has = lambda i, l: l in i["labels"]
        strained = sum(has(i, "contract:strained") for i in closed)
        settled = sum(has(i, "contract:settled") for i in closed)
        judged = strained + settled
        streak = streak + 1 if judged and strained == 0 else 0
        turn = [hours_between(i["created_at"], i["closed_at"]) for i in filed if i["closed_at"]]
        open_sw = sum(1 for i in issues
                      if has(i, "silent-wrong") and not has(i, "fixed-pending-merge")
                      and parse_day(i["created_at"]) <= end
                      and (not i["closed_at"] or parse_day(i["closed_at"]) > end))
        sha = tree_at(ref, end)
        r = repo_row(sha) if sha else {}
        sched = [c for d, ev, c in consumer_runs if ev == "schedule" and w <= parse_day(d) <= end]
        gate = [(d, c) for d, ev, c in gate_runs if parse_day(d) <= end]
        gate_age = f"{(end - parse_day(gate[0][0])).days}d:{gate[0][1]}" if gate else "never"
        out.append(dict(
            week_of=w.isoformat(), partial="yes" if end >= today else "no",
            filed=len(filed), silent_wrong=sum(has(i, "silent-wrong") for i in filed),
            sev_high=sum(has(i, "sev:high") for i in filed),
            consumer_hit=sum(any(l.startswith("hit-by:") and l != "hit-by:loft" for l in i["labels"]) for i in filed),
            regression=sum(has(i, "regression") for i in filed),
            ci_shaped=sum(any(l in ("ci-advisory", "ci-infra", "nightly-failure") for l in i["labels"]) for i in filed),
            closed=len(closed), strained=strained, settled=settled,
            strained_pct=f"{100 * strained / judged:.0f}" if judged else "-",
            zero_streak=streak,
            close_h_median=f"{statistics.median(turn):.0f}" if turn else "-",
            open_sw_main=open_sw,
            consumer_commits=(sum(1 for ds in commits.values() for d in ds if w <= parse_day(d) <= end)
                              if commits else "-"),
            rules=r.get("rules", "-"), rules_R=r.get("rules_R", "-"),
            rules_R_new=(r["rules_R"] - prev_r) if prev_r is not None and r else "-",
            annotated=r.get("annotated", "-"), guarded=r.get("guarded", "-"),
            census_only=r.get("census_only", "-"), deviations_open=r.get("deviations_open", "-"),
            decisions=r.get("decisions", "-"),
            consumer_green=f"{sum(c == 'success' for c in sched)}/{len(sched)}",
            release_gate_age=gate_age, portal=r.get("portal", "-")))
        prev_r = r.get("rules_R") if r else prev_r
    return out


def read_series():
    if not os.path.exists(SERIES):
        return []
    out = []
    for line in open(SERIES, encoding="utf-8"):
        if line.startswith("#") or not line.strip():
            continue
        out.append(dict(zip(COLUMNS, line.rstrip("\n").split("\t"))))
    return out


def write_series(table):
    head = ("# The weekly stability read — one row per ISO week, every meter of STABILITY_ROADMAP.md\n"
            "# § The owner's directive.  Written by `scripts/stability_read.py --record`; the columns are\n"
            "# explained in that script's docstring.  A week marked partial is rewritten when it ends.\n"
            + "\t".join(COLUMNS) + "\n")
    with open(SERIES, "w", encoding="utf-8") as f:
        f.write(head)
        for r in sorted(table.values(), key=lambda r: r["week_of"]):
            f.write("\t".join(str(r[c]) for c in COLUMNS) + "\n")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--since", help="first week to compute (a date; default: four weeks back)")
    ap.add_argument("--until", help="last week (a date; default: today)")
    ap.add_argument("--record", action="store_true", help="write the rows into " + os.path.relpath(SERIES, ROOT))
    ap.add_argument("--cache", help="a JSON file to keep the tracker download in, and read it from")
    a = ap.parse_args()
    today = dt.date.today()
    since = dt.date.fromisoformat(a.since) if a.since else today - dt.timedelta(days=28)
    until = dt.date.fromisoformat(a.until) if a.until else today
    table = rows(since, until, a.cache)
    print("\t".join(COLUMNS))
    for r in table:
        print("\t".join(str(r[c]) for c in COLUMNS))
    last = [r for r in table if r["partial"] == "no"]
    if last:
        r = last[-1]
        print(f"\nweek of {r['week_of']}: strained {r['strained']} of {r['strained'] + r['settled']} judged "
              f"({r['strained_pct']} %), zero streak {r['zero_streak']} week(s) — stable reads at 2; "
              f"silent-wrong open on main {r['open_sw_main']}; consumer gate green {r['consumer_green']}; "
              f"release gate {r['release_gate_age']}; rewrite rules +{r['rules_R_new']}")
    if a.record:
        kept = {r["week_of"]: r for r in read_series()}
        for r in table:
            kept[r["week_of"]] = r
        write_series(kept)
        print(f"\nrecorded {len(table)} week(s) into {os.path.relpath(SERIES, ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
