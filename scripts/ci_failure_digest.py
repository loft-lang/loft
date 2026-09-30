#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Keep ONE issue that lists every red CI step, with what failed and where to look.

A required check blocks the merge, so it is fixed before anything lands.  An advisory
one does not: it goes red on a PR, the PR merges, and the finding lives on only if
someone read the log.  That reading is what this script does, once per finished run:

  * which jobs FAILED and are advisory — on a PR, every job the PR's own check list
    does not mark required (asked live, never a copied list); on the default branch,
    every job, because nothing there is gated after the fact;
  * what each one says — a panic's location and message, the corpus runner's failing
    `.loft` file and function, nextest's failing tests, a rustc error, the valgrind
    sweep's red rows — cut from the job's log (`digest`, a pure function of the text);
  * where to look — the commit that last changed each named test file or source line;
  * whether it is NEW — the same job's conclusion on the default branch's last run.

All of it goes to one `[ci] Red steps` issue (label `ci-advisory`), grouped by where the
step is red — the default branch, or each open PR — and by workflow.  The issue body
carries its own state as JSON, so each run rewrites exactly its own origin and workflow:
a step that is green again leaves, a merged or closed PR's steps leave with it, and the
issue closes itself when nothing is left and reopens when something is.  One issue per
red step clogged the issue list with findings a merge had already settled.  A failure of
the RUNNER, not of loft (a download timeout, a lost runner, a full disk), is a row too,
marked `runner`, and a run whose failures were all of that kind is retried once.

    ci_failure_digest.py dry-run RUN_ID   # print what `file` would do; writes nothing
    ci_failure_digest.py file RUN_ID      # what advisory-failures.yml runs
    ci_failure_digest.py selftest         # the extractor against tests/fixtures/ci_logs

Log text is written by whatever the run executed, a fork's PR included, so it is DATA:
it only ever reaches an issue inside a code fence longer than any backtick run in it,
with `@` mentions defused, and it is never passed to a shell.
"""

import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "tests" / "fixtures" / "ci_logs"
EXCERPT_LIMIT = 12000
BODY_LIMIT = 60000
LABELS = {
    "ci-advisory": ("5319e7", "Auto-maintained: the one issue that lists every red CI step (advisory-failures.yml)"),
}
# Jobs that REPORT on the others and fail only because they did; filing them repeats
# the failure they report.
META_JOB = re.compile(r"→ tracked issue|Daily status|Nightly health|^notify$", re.I)

# ── the extractor: a pure function of the log text ─────────────────────────────

STAMP = re.compile(r"^\d{4}-\d\d-\d\dT[\d:.]+Z ?")
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
INFRA = [
    re.compile(p, re.I)
    for p in (
        r"client error \(SendRequest\)",
        r"error: .*(could not|failed to) download",
        r"The runner has received a shutdown signal",
        r"lost communication with the server",
        r"The hosted runner encountered an error",
        r"No space left on device",
        r"API rate limit exceeded",
        r"Unable to (resolve|download) action",
        r"(ECONNRESET|ETIMEDOUT|EAI_AGAIN).*(github|crates\.io|rustup|npmjs)",
    )
]
PANIC = re.compile(r"thread '([^']+)' \(\d+\) panicked at ([^\s:]+):(\d+):\d+:")
CUSTOM = re.compile(r'Error: Custom \{ kind: \w+, error: "(.*)" \}')
NEXTEST_FAIL = re.compile(r"^\s*FAIL \[\s*[\d.]+s\] (?:\([\d/ ]+\) )?(\S+) (\S+)")
CARGO_FAIL = re.compile(r"^\s*test (\S+) \.\.\. FAILED")
RUSTC = re.compile(r"^error(\[E\d+\])?: (.+)")
RUSTC_AT = re.compile(r"^\s*--> ([^\s:]+):(\d+):\d+")
VG_RED = re.compile(r"RED — the offending runs")
RUNNER_PATH = re.compile(r"/home/runner/work/[^/]+/[^/]+/")
LOFT_FILE = re.compile(r"\b(tests/(?:scripts|docs|comparisons)/[\w.\-]+\.loft)\b")
RS_AT = re.compile(r"\b((?:src|tests)/[\w/\-]+\.rs):(\d+)")


def clean(text: str) -> list[str]:
    return [ANSI.sub("", STAMP.sub("", ln)).rstrip("\r") for ln in text.splitlines()]


def digest(text: str) -> dict:
    """What a job's log says went wrong: `kind`, one-line `headline`, the failing
    `tests`, the `files` and `locations` it names, and an `excerpt` to read."""
    lines = clean(text)
    out = {"kind": "unknown", "headline": "", "tests": [], "files": [], "locations": [], "excerpt": ""}
    for ln in lines:
        if any(p.search(ln) for p in INFRA):
            out["kind"] = "infra"
            out["headline"] = ln.strip()[:200]
            out["excerpt"] = window(lines, lines.index(ln), 6, 20)
            return out
    tests = []
    for ln in lines:
        m = NEXTEST_FAIL.match(ln)
        if m and m.group(2) not in tests:
            tests.append(m.group(2))
        m = CARGO_FAIL.match(ln)
        if m and m.group(1) not in tests:
            tests.append(m.group(1))
    out["tests"] = tests
    at = None
    for i, ln in enumerate(lines):
        m = CUSTOM.search(ln)
        if m:
            out["kind"], out["headline"], at = "corpus", m.group(1), i
            break
    if at is None:
        for i, ln in enumerate(lines):
            m = PANIC.search(ln)
            if m:
                msg = next((x.strip() for x in lines[i + 1:i + 4] if x.strip()), "")
                out["kind"] = "panic"
                out["headline"] = f"panicked at {m.group(2)}:{m.group(3)}: {msg}"
                at = i
                break
    if at is None:
        for i, ln in enumerate(lines):
            m = RUSTC.match(ln.strip())
            if m and "aborting due to" not in ln:
                where = next((RUSTC_AT.match(x) for x in lines[i + 1:i + 4] if RUSTC_AT.match(x)), None)
                loc = f" at {where.group(1)}:{where.group(2)}" if where else ""
                out["kind"], out["headline"], at = "compile", f"error{m.group(1) or ''}: {m.group(2)}{loc}", i
                break
    if at is None:
        for i, ln in enumerate(lines):
            if VG_RED.search(ln):
                rows = [x.split("\t") for x in lines[i + 1:] if x.count("\t") >= 4]
                names = [RUNNER_PATH.sub("", r[1]) for r in rows][:3]
                summary = next((x.strip() for x in lines[max(0, i - 3):i] if "invalid accesses:" in x), "")
                out["kind"] = "sweep"
                out["headline"] = (summary or f"{len(rows)} red run(s)") + \
                    f"; first: {names[0]}" if names else ""
                out["files"] = [n.split("::")[0] for n in names if n.startswith("tests/")]
                at = i
                break
    if at is None and tests:
        out["kind"], out["headline"] = "test", "failed: " + ", ".join(tests[:3])
    if at is None:
        errs = [i for i, ln in enumerate(lines) if "##[error]" in ln]
        at = errs[0] if errs else max(len(lines) - 1, 0)
        if not out["headline"]:
            out["headline"] = (lines[at].replace("##[error]", "").strip() if lines else "") or "the job failed"
    # A panic under a corpus failure is where the fault is; the runner's line names the file.
    panics = [i for i, ln in enumerate(lines) if PANIC.search(ln)]
    # Named files come from the headline and from lines that REPORT a failure — the corpus
    # runner prints every file it runs, and those are not where to look.
    failing = "\n".join([out["headline"]] + [ln for ln in lines if FAILURE_LINE.search(ln)])
    out["files"] += [f for f in dict.fromkeys(LOFT_FILE.findall(failing)) if f not in out["files"]]
    spots = out["headline"] + "\n" + "\n".join(lines[i] for i in panics)
    out["locations"] = [f"{f}:{n}" for f, n in dict.fromkeys(RS_AT.findall(spots))]
    if not out["excerpt"]:
        if panics:
            parts = [panic_block(lines, i) for i in panics[:2]]
            if out["kind"] == "corpus":
                parts.append(lines[at])
            out["excerpt"] = "\n\n".join(parts)[:EXCERPT_LIMIT]
        else:
            out["excerpt"] = stdout_block(lines) or window(lines, at, 20, 60)
    return out


# A line that REPORTS a failure — not one that mentions one: the corpus runner echoes every
# expected error (`expected @EXPECT_ERROR matched: Error: …`) and every file it runs.
FAILURE_LINE = re.compile(r"^\s*FAIL\b|\.\.\. FAILED|panicked at|^Error: Custom|leaked at|stores? not freed|"
                          r"USE AFTER FREE|assertion failed")
FRAME = re.compile(r"^\s+\d+: (.+)")


def panic_block(lines: list[str], at: int) -> str:
    """A panic, its message, and the backtrace frames that are loft's own."""
    out = [lines[at]]
    for x in lines[at + 1:at + 120]:
        m = FRAME.match(x)
        if m:
            if "loft" in m.group(1) or "wrap::" in m.group(1):
                out.append(x)
        elif x.strip().startswith("at ") or x.strip() == "stack backtrace:":
            continue
        elif PANIC.search(x) or x.startswith("----") or x.startswith("Error:"):
            break
        else:
            out.append(x)
        if len(out) > 30:
            break
    return "\n".join(out)


def window(lines: list[str], at: int, before: int, after: int) -> str:
    return "\n".join(lines[max(0, at - before):at + after])[:EXCERPT_LIMIT]


def stdout_block(lines: list[str]) -> str:
    """The first captured `---- test stdout ----` block, backtrace frames dropped."""
    for i, ln in enumerate(lines):
        if re.match(r"^\s*---- \S+ std(out|err) ----", ln):
            body = [x for x in lines[i:i + 400] if not re.match(r"^\s+\d+: ", x) and not x.strip().startswith("at ")]
            return "\n".join(body)[:EXCERPT_LIMIT]
    return ""


def signature(workflow: str, job: str, headline: str) -> str:
    norm = RUNNER_PATH.sub("", headline)
    norm = re.sub(r"\b[0-9a-f]{7,40}\b", "H", norm)
    norm = re.sub(r"\d+", "N", norm)
    return hashlib.sha1(f"{workflow}\n{job}\n{norm}".encode()).hexdigest()[:12]


# ── the GitHub side ─────────────────────────────────────────────────────────────

def gh(*args: str, input_text: str | None = None, check: bool = True) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True, input=input_text)
    if check and r.returncode != 0:
        raise RuntimeError(f"gh {' '.join(args[:3])}…: {r.stderr.strip()[:300]}")
    return r.stdout


def api(path: str, **kw) -> object:
    return json.loads(gh("api", path, **kw) or "null")


def repo() -> str:
    return os.environ.get("GH_REPO") or gh("repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner").strip()


def jobs_of(r: str, run: dict) -> list[dict]:
    out, page = [], 1
    while True:
        chunk = api(f"repos/{r}/actions/runs/{run['id']}/attempts/{run['run_attempt']}/jobs?per_page=100&page={page}")
        out += chunk["jobs"]
        if len(chunk["jobs"]) < 100:
            return out
        page += 1


def pr_of(r: str, run: dict) -> int | None:
    # Only a `pull_request` run speaks for a PR: a dispatch or push on a branch that HAS
    # an open PR is that branch's own evidence and lists the PR all the same.
    if run["event"] != "pull_request":
        return None
    if run.get("pull_requests"):
        return run["pull_requests"][0]["number"]
    hits = api(f"repos/{r}/commits/{run['head_sha']}/pulls")
    return hits[0]["number"] if hits else None


def required_names(r: str, pr: int, sha: str) -> set[str]:
    owner, name = r.split("/")
    q = ("query($o:String!,$n:String!,$pr:Int!,$sha:GitObjectID!){repository(owner:$o,name:$n){"
         "object(oid:$sha){... on Commit{checkSuites(first:50){nodes{checkRuns(first:100){nodes{"
         "name isRequired(pullRequestNumber:$pr)}}}}}}}}")
    data = json.loads(gh("api", "graphql", "-f", f"query={q}", "-F", f"o={owner}", "-F", f"n={name}",
                         "-F", f"pr={pr}", "-F", f"sha={sha}"))
    suites = data["data"]["repository"]["object"]["checkSuites"]["nodes"]
    return {c["name"] for s in suites for c in s["checkRuns"]["nodes"] if c["isRequired"]}


def main_conclusions(r: str, run: dict, default: str) -> tuple[dict, dict | None]:
    """The same workflow's latest finished run on the default branch: job → conclusion."""
    runs = api(f"repos/{r}/actions/workflows/{run['workflow_id']}/runs?branch={default}&status=completed&per_page=20")
    # The last one STARTED BEFORE this run: a later one says nothing about whether this
    # failure was already there.
    for other in runs["workflow_runs"]:
        if other["id"] != run["id"] and other["event"] != "pull_request" and other["created_at"] < run["created_at"]:
            return {j["name"]: j["conclusion"] for j in jobs_of(r, other)}, other
    return {}, None


def last_touched(sha: str, target: str) -> str:
    """`abc1234 subject` of the commit that last changed a file, or a source line."""
    if ":" in target:
        path, line = target.rsplit(":", 1)
        args = ["git", "-C", str(ROOT), "blame", "--porcelain", "-L", f"{line},{line}", sha, "--", path]
        r = subprocess.run(args, capture_output=True, text=True)
        if r.returncode != 0 or not r.stdout:
            return ""
        commit = r.stdout.split()[0]
        r = subprocess.run(["git", "-C", str(ROOT), "log", "-1", "--format=%h %s", commit, "--", path],
                           capture_output=True, text=True)
        return r.stdout.strip()
    # A test FILE: the commit that added it is usually the one to ask; the last one to touch
    # it is often a receipt re-derivation.
    def one(*extra: str) -> str:
        r = subprocess.run(["git", "-C", str(ROOT), "log", *extra, "--format=%h %s", sha, "--", target],
                           capture_output=True, text=True)
        return r.stdout.strip().splitlines()[0] if r.stdout.strip() else ""
    added, last = one("--diff-filter=A", "--follow"), one("-1")
    return last if not added or added == last else f"{added} (added); last changed in {last}"


def findings(r: str, run_id: str) -> tuple[dict, list[dict], dict]:
    run = api(f"repos/{r}/actions/runs/{run_id}")
    default = api(f"repos/{r}")["default_branch"]
    pr = pr_of(r, run)
    ctx = {"run": run, "pr": pr, "default": default, "skipped": ""}
    if pr is None and run["head_branch"] != default:
        ctx["skipped"] = f"a {run['event']} run on `{run['head_branch']}` — only PRs and `{default}` file"
        return ctx, [], {}
    jobs = jobs_of(r, run)
    required = required_names(r, pr, run["head_sha"]) if pr else set()
    on_main, main_run = main_conclusions(r, run, default)
    ctx["main_run"] = main_run
    out = []
    for j in jobs:
        if META_JOB.search(j["name"]) or j["name"] in required:
            continue
        failed_steps = [s["name"] for s in j.get("steps") or [] if s.get("conclusion") == "failure"]
        if j["conclusion"] not in ("failure", "timed_out") and not failed_steps:
            continue
        log = gh("api", f"repos/{r}/actions/jobs/{j['id']}/logs", check=False)
        d = digest(log)
        if j["conclusion"] == "timed_out" and d["kind"] == "unknown":
            d["headline"] = "the job ran past its time limit"
        d.update(job=j["name"], url=j["html_url"], steps=failed_steps, workflow=run["name"],
                 main=on_main.get(j["name"]))
        d["signature"] = signature(run["name"], j["name"], d["headline"])
        d["where"] = [(t, last_touched(run["head_sha"], t)) for t in d["files"][:4] + d["locations"][:3]]
        out.append(d)
    return ctx, out, on_main


def fence(text: str) -> str:
    ticks = max([len(m) for m in re.findall(r"`+", text)] + [2]) + 1
    return f"{'`' * ticks}text\n{defuse(text)}\n{'`' * ticks}"


def defuse(text: str) -> str:
    return text.replace("@", "@​")


def origin_of(ctx: dict) -> str:
    return f"pr:{ctx['pr']}" if ctx["pr"] else "main"


TITLE = "[ci] Red steps"
STATE_RE = re.compile(r"<!-- ci-state: (.*?) -->", re.S)
ROW_EXCERPT = 2500


def ensure_labels() -> None:
    for name, (color, desc) in LABELS.items():
        gh("label", "create", name, "--color", color, "--description", desc, "--force", check=False)


def row_of(ctx: dict, d: dict) -> dict:
    """One red step, as the combined issue stores it (a JSON object in its body)."""
    run = ctx["run"]
    return {
        "job": d["job"], "workflow": d["workflow"], "kind": d["kind"], "signature": d["signature"],
        "headline": d["headline"], "url": d["url"], "main": d["main"] or "",
        "steps": d["steps"], "tests": d["tests"][:8], "where": d["where"][:5],
        "excerpt": d["excerpt"][-ROW_EXCERPT:], "run": run["id"], "run_url": run["html_url"],
        "sha": run["head_sha"][:9], "since_run": run["id"], "since_url": run["html_url"],
    }


def load_state() -> tuple[dict | None, dict]:
    """The one combined issue (open or closed) and the state its body carries."""
    hits = json.loads(gh("issue", "list", "--label", "ci-advisory", "--state", "all", "--search",
                         "\"ci-state:\" in:body", "--json", "number,state,body", "--limit", "10"))
    hits = [h for h in hits if STATE_RE.search(h["body"])]
    if not hits:
        return None, {"sections": {}}
    issue = next((h for h in hits if h["state"] == "OPEN"), hits[0])
    raw = STATE_RE.search(issue["body"]).group(1).replace("--\\u003e", "-->")
    try:
        return issue, json.loads(raw)
    except ValueError:
        return issue, {"sections": {}}


def update_state(state: dict, ctx: dict, items: list[dict]) -> None:
    """This run's origin and workflow now hold exactly the steps red in it: a green step
    leaves, a new one enters, and one red before keeps the run it was first seen in."""
    run = ctx["run"]
    key = f"{origin_of(ctx)}|{run['name']}"
    before = {(r["job"], r["signature"]): r for r in state["sections"].get(key, {}).get("rows", [])}
    rows = []
    for d in items:
        row = row_of(ctx, d)
        old = before.get((row["job"], row["signature"]))
        if old:
            row["since_run"], row["since_url"] = old["since_run"], old["since_url"]
        rows.append(row)
    if rows:
        state["sections"][key] = {"origin": origin_of(ctx), "workflow": run["name"], "branch": run["head_branch"],
                                  "run": run["id"], "run_url": run["html_url"], "sha": run["head_sha"][:9],
                                  "rows": rows}
    else:
        state["sections"].pop(key, None)


def prune_prs(state: dict, r: str, current: str) -> None:
    """A merged or closed PR's steps are no longer the PR's to report: what it merged, the
    default branch's own runs report from then on."""
    for key in list(state["sections"]):
        origin = key.split("|", 1)[0]
        if origin.startswith("pr:") and origin != current:
            p = api(f"repos/{r}/pulls/{origin[3:]}")
            if p.get("state") != "open":
                del state["sections"][key]


def render(state: dict, default: str) -> tuple[str, str]:
    secs = sorted(state["sections"].values(),
                  key=lambda s: (s["origin"] != "main", int(s["origin"][3:]) if s["origin"] != "main" else 0,
                                 s["workflow"]))
    counts: dict[str, int] = {}
    for s in secs:
        where = f"`{default}`" if s["origin"] == "main" else f"PR #{s['origin'][3:]}"
        counts[where] = counts.get(where, 0) + len(s["rows"])
    title = TITLE + (" — " + ", ".join(f"{n} on {w.strip('`')}" for w, n in counts.items()) if counts else "")
    parts = ["Every CI step that is red right now, and where. `advisory-failures.yml` rewrites this "
             "after each CI and nightly run: a step leaves when its job is green again, a merged or "
             "closed PR's steps leave with it, and the issue closes itself when nothing is left.", ""]
    last_origin = None
    for s in secs:
        if s["origin"] != last_origin:
            head = f"`{default}`" if s["origin"] == "main" else f"PR #{s['origin'][3:]} (`{s['branch']}`)"
            parts += [f"## {head}", ""]
            last_origin = s["origin"]
        parts += [f"**{s['workflow']}** — [run {s['run']}]({s['run_url']}) @ `{s['sha']}`", "",
                  "| step | what failed | vs main | red since |", "|---|---|---|---|"]
        for r in s["rows"]:
            vs = {"success": "**new**", "": "?"}.get(r["main"], "pre-existing")
            if r["kind"] == "infra":
                vs = "runner"
            since = "this run" if r["since_run"] == s["run"] else f"[run {r['since_run']}]({r['since_url']})"
            parts.append(f"| [{defuse(r['job'])}]({r['url']}) | `{defuse(r['headline'][:100])}` | {vs} | {since} |")
        parts.append("")
        for r in s["rows"]:
            detail = []
            if r["steps"]:
                detail.append("Failed steps: " + ", ".join(f"`{defuse(x)}`" for x in r["steps"]))
            if r["tests"]:
                detail.append("Failing tests: " + ", ".join(f"`{defuse(t)}`" for t in r["tests"]))
            for target, commit in r["where"]:
                detail.append(f"- `{target}`" + (f" — {defuse(commit)}" if commit else ""))
            if r.get("excerpt"):
                detail.append(fence(r["excerpt"]))
            if detail:
                parts += [f"<details><summary>{defuse(r['job'])}</summary>", "", *detail, "", "</details>"]
        parts.append("")
    if not secs:
        parts.append("Nothing is red.")
    blob = json.dumps(state, separators=(",", ":")).replace("-->", "--\\u003e")
    body = "\n".join(parts)
    # The state is what the next run reads, so it always fits: excerpts go first.
    while len(body) + len(blob) > BODY_LIMIT and any(r.get("excerpt") for s in secs for r in s["rows"]):
        for s in secs:
            for r in s["rows"]:
                r["excerpt"] = ""
        return render(state, default)
    return title, f"{body}\n<!-- ci-state: {blob} -->"


def publish(issue: dict | None, state: dict, default: str, dry: bool) -> str:
    title, body = render(state, default)
    empty = not state["sections"]
    if dry:
        print(f"\n=== combined issue {('#' + str(issue['number'])) if issue else '(new)'}: {title}\n")
        print(body.split("<!-- ci-state:")[0])
        return "(dry-run)"
    if issue is None:
        if empty:
            return ""
        url = gh("issue", "create", "--title", title, "--label", "ci-advisory", "--body-file", "-",
                 input_text=body).strip()
        return url.rsplit("/", 1)[-1]
    n = str(issue["number"])
    gh("issue", "edit", n, "--title", title, "--body-file", "-", input_text=body)
    if empty and issue["state"] == "OPEN":
        gh("issue", "close", n, "--reason", "completed", "--comment", "Nothing is red — closing.")
    elif not empty and issue["state"] != "OPEN":
        gh("issue", "reopen", n, "--comment", "Red again — see the body.")
    return n


def pr_summary(ctx: dict, items: list[dict], issue_no: str, dry: bool) -> None:
    """One comment per PR, one section per workflow, updated in place."""
    if not ctx["pr"]:
        return
    run, wf = ctx["run"], ctx["run"]["name"]
    start, end = f"<!-- ci-advisory:{wf} -->", f"<!-- /ci-advisory:{wf} -->"
    ref = f"#{issue_no}" if issue_no.isdigit() else issue_no
    if items:
        rows = [f"| {defuse(d['job'])} | `{defuse(d['headline'][:90])}` | "
                f"{'new here' if d['main'] == 'success' else 'pre-existing' if d['main'] else '?'} |" for d in items]
        section = (f"{start}\n**{wf}** — [run {run['id']}]({run['html_url']}) @ `{run['head_sha'][:9]}` "
                   f"(details in {ref})\n\n| advisory step | what failed | vs main |\n|---|---|---|\n"
                   + "\n".join(rows) + f"\n{end}")
    else:
        section = f"{start}\n**{wf}** — every advisory step green in [run {run['id']}]({run['html_url']}).\n{end}"
    marker = "<!-- ci-advisory-summary -->"
    if dry:
        print(f"\n=== would update the PR #{ctx['pr']} summary with:\n{section}")
        return
    r = repo()
    comments = api(f"repos/{r}/issues/{ctx['pr']}/comments?per_page=100")
    mine = next((c for c in comments if marker in c["body"]), None)
    if mine is None:
        if not items:
            return
        body = f"{marker}\n### Advisory checks that failed\n\nNot merge gates, and still to be fixed.\n\n{section}"
        gh("api", f"repos/{r}/issues/{ctx['pr']}/comments", "-f", f"body={body}")
        return
    body = mine["body"]
    body = (re.sub(re.escape(start) + r".*?" + re.escape(end), lambda _: section, body, flags=re.S)
            if start in body else body + "\n\n" + section)
    gh("api", "-X", "PATCH", f"repos/{r}/issues/comments/{mine['id']}", "-f", f"body={body}")


def process(run_id: str, dry: bool) -> int:
    r = repo()
    ctx, items, _ = findings(r, run_id)
    if ctx["skipped"]:
        print(f"nothing to do: {ctx['skipped']}")
        return 0
    if not dry:
        ensure_labels()
    issue, state = load_state()
    update_state(state, ctx, items)
    prune_prs(state, r, origin_of(ctx))
    n = publish(issue, state, ctx["default"], dry)
    pr_summary(ctx, items, n, dry)
    # A run whose every failure was the RUNNER's is retried once: nothing of loft's failed.
    run = ctx["run"]
    if items and all(d["kind"] == "infra" for d in items) and run["run_attempt"] == 1 and not dry:
        gh("run", "rerun", str(run["id"]), "--failed", check=False)
    print(f"\n{len(items)} red step(s) in run {run_id}; combined issue {n or '(none — all green)'}")
    return 0


# ── selftest ────────────────────────────────────────────────────────────────────

EXPECT = {
    "debug-asserts-panic.log": {"kind": "corpus", "headline": "test_spatial: Unknown record 39",
                                "files": ["tests/scripts/a-comprehension-over-a-keyed-collection-walks-its-snapshot.loft"],
                                "tests": ["loft_suite_01"], "locations": ["src/store.rs:4081"],
                                "excerpt": "Unknown record 39", "only_files": 1},
    "valgrind-red.log": {"kind": "sweep", "headline": "invalid accesses: 1945 file(s)", "files": ["tests/scripts/1549-a-pooled-buffer-releases-its-previous-occupant.loft"]},
    "rustup-timeout.log": {"kind": "infra", "headline": "could not download"},
    "nextest-fail.log": {"kind": "panic", "headline": "panicked at tests/frontend_counts.rs:201: the front end allocates",
                         "tests": ["front_end_allocations_do_not_grow"], "locations": ["tests/frontend_counts.rs:201"]},
    "rustc-error.log": {"kind": "compile", "headline": "error[E0594]: cannot assign", "locations": ["src/generation/coroutine.rs:3520"]},
}


def selftest() -> int:
    bad = 0
    for name, want in EXPECT.items():
        got = digest((FIXTURES / name).read_text())
        errs = []
        if got["kind"] != want["kind"]:
            errs.append(f"kind {got['kind']!r} != {want['kind']!r}")
        if want["headline"] not in got["headline"]:
            errs.append(f"headline {got['headline']!r} lacks {want['headline']!r}")
        for key in ("files", "tests", "locations"):
            for item in want.get(key, []):
                if item not in got[key]:
                    errs.append(f"{key} {got[key]} lacks {item!r}")
        if not got["excerpt"].strip():
            errs.append("empty excerpt")
        if want.get("excerpt", "") not in got["excerpt"]:
            errs.append(f"excerpt lacks {want['excerpt']!r}")
        if "only_files" in want and len(got["files"]) != want["only_files"]:
            errs.append(f"files {got['files']} — only the failing one belongs there")
        print(f"{'ok  ' if not errs else 'FAIL'} {name}" + "".join(f"\n       {e}" for e in errs))
        bad += bool(errs)
    # The signature must not move with the numbers a rerun changes, and must with the job.
    a = signature("W", "J", "panicked at src/store.rs:4081: Unknown record 39")
    b = signature("W", "J", "panicked at src/store.rs:4082: Unknown record 41")
    c = signature("W", "K", "panicked at src/store.rs:4081: Unknown record 39")
    if a != b or a == c:
        print("FAIL signature: numbers must not split it, the job must")
        bad += 1
    # The combined issue's state: a step red again keeps its first run, a green re-run of
    # the same origin and workflow drops it, and the body round-trips the state.
    def fake(run_id, jobs):
        ctx = {"run": {"id": run_id, "html_url": f"u/{run_id}", "head_sha": "abcdef012345", "name": "CI",
                       "head_branch": "main"}, "pr": None, "default": "main"}
        items = [{"job": j, "workflow": "CI", "kind": "panic", "signature": signature("CI", j, "h"),
                  "headline": "h", "url": "j", "main": "failure", "steps": [], "tests": [], "where": [],
                  "excerpt": "x"} for j in jobs]
        return ctx, items
    st = {"sections": {}}
    update_state(st, *fake(1, ["a", "b"]))
    update_state(st, *fake(2, ["b", "c"]))
    rows = {r["job"]: r for r in st["sections"]["main|CI"]["rows"]}
    if set(rows) != {"b", "c"} or rows["b"]["since_run"] != 1 or rows["c"]["since_run"] != 2:
        print(f"FAIL combined state: {sorted(rows)} {[(r['job'], r['since_run']) for r in rows.values()]}")
        bad += 1
    _, body = render(st, "main")
    back = json.loads(STATE_RE.search(body).group(1).replace("--\\u003e", "-->"))
    if back != st:
        print("FAIL combined state: the body does not round-trip the state")
        bad += 1
    update_state(st, *fake(3, []))
    if st["sections"] or "Nothing is red." not in render(st, "main")[1]:
        print("FAIL combined state: a green run must empty its section")
        bad += 1
    fenced = fence("x ``` y @someone")
    if not fenced.startswith("````") or "@someone" in fenced:
        print("FAIL fence: a fence must outrun the text's backticks and defuse mentions")
        bad += 1
    print("selftest:", "ok" if not bad else f"{bad} failure(s)")
    return 1 if bad else 0


if __name__ == "__main__":
    if len(sys.argv) >= 2 and sys.argv[1] == "selftest":
        sys.exit(selftest())
    if len(sys.argv) == 3 and sys.argv[1] in ("dry-run", "file"):
        sys.exit(process(sys.argv[2], dry=sys.argv[1] == "dry-run"))
    print(__doc__)
    sys.exit(2)
