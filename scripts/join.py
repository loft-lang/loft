#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Join sibling branches into this one: what each has that this tree lacks, picked source by
source, and every derived artefact re-measured on the union.

    scripts/join.py survey [SRC ...]   # the table: per commit HERE / NEW / PARTIAL / DUP / DERIVED
    scripts/join.py apply              # pick the plan's commits, source by source; resumable
    scripts/join.py rederive           # re-measure every artefact in scripts/derived_artefacts.json
    scripts/join.py verify             # the cheap checks a gate would stop on
    scripts/join.py guards             # the guards the join brought in, on both backends
    scripts/join.py run [SRC ...]      # all five in order, stopping at the first thing to decide

SRC is a ref (`origin/157-native-4x`) or `name=<sha>`.  With none, `survey` takes every
`origin/*` branch that has commits this tree lacks and moved in the last `--days` (default 2).
The plan lives in `.git/loft-join/plan.json`; `apply` resumes from it after a conflict you
resolved (`git cherry-pick --continue`, then `scripts/join.py apply` again).

Why
---
A join asks about CHANGES, and ancestry answers about COMMITS: after one cherry-pick or a squash
of `main`, `git cherry` and patch-ids report work as missing that the tree already carries
(doc/claude/JOINING.md § Does this tree already have that change?).  The survey asks the tree:
a commit is HERE when its own diff applies IN REVERSE to this tree, file by file, in a
throw-away index.  The rest of the method — build after each source, a number both sides moved
is a measurement and never a merge — is JOINING.md's, and this script is its mechanical half.

What it will not do: resolve a conflict in source code (it stops and says which file), accept a
pinned count that grew beyond what the sources themselves pinned (it names the source, or the
new sites), or start a gate (`scripts/ci-run.sh start` is yours to run after `verify`).
"""

import argparse
import datetime
import json
import os
import re
import collections
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = ROOT / "scripts" / "derived_artefacts.json"


def git(*args, check=True, env=None, inp=None):
    r = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, env=env,
                       input=inp)
    if check and r.returncode != 0:
        sys.exit(f"join: git {' '.join(args)} failed:\n{r.stderr.strip()}")
    return r.stdout


def git_dir():
    return Path(git("rev-parse", "--absolute-git-dir").strip())


def plan_path():
    return git_dir() / "loft-join" / "plan.json"


def load_registry():
    return json.loads(REGISTRY.read_text(encoding="utf-8"))


def load_plan():
    p = plan_path()
    if not p.exists():
        sys.exit("join: no plan — run `scripts/join.py survey` first")
    return json.loads(p.read_text(encoding="utf-8"))


def save_plan(plan):
    p = plan_path()
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(plan, indent=1), encoding="utf-8")


def sh(cmd, quiet=False):
    """Run a shell command from the repo root; returns (ok, combined output)."""
    r = subprocess.run(cmd, shell=True, cwd=ROOT, capture_output=True, text=True)
    out = (r.stdout + r.stderr).strip()
    if not quiet and r.returncode != 0:
        print("\n".join("      " + l for l in out.splitlines()[-12:]))
    return r.returncode == 0, out


def is_derived(path, registry):
    for a in registry["artefacts"]:
        for p in a["paths"]:
            if path == p or (p.endswith("/") and path.startswith(p)):
                return True
    return False


def clean_tree_or_exit(registry=None):
    """Refuse a dirty tree.  With `registry`, a derived artefact a stopped `rederive` left
    re-measured does not count: re-running starts from it."""
    dirty = [l[3:] for l in git("status", "--porcelain", "--untracked-files=no").splitlines()]
    if registry is not None:
        dirty = [f for f in dirty if not is_derived(f, registry) and f != "doc/claude/QUALITY.md"]
    if dirty:
        sys.exit("join: the working tree has uncommitted changes — commit them first: "
                 + ", ".join(dirty[:5]))


# ── survey ──────────────────────────────────────────────────────────────────────────────


def default_sources(days, head):
    """Every origin/* branch with commits HEAD lacks that moved in the last `days` days."""
    cutoff = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(days=days)
    own = git("rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}", check=False).strip()
    out = []
    for line in git("for-each-ref", "--format=%(refname:short) %(committerdate:iso-strict)",
                    "refs/remotes/origin").splitlines():
        ref, date = line.split(" ", 1)
        if ref in ("origin/HEAD", "origin/main", "origin/gh-pages", own) or ref == "origin":
            continue
        if datetime.datetime.fromisoformat(date) < cutoff:
            continue
        if git("rev-list", "--count", f"{head}..{ref}").strip() != "0":
            out.append(ref)
    return out


def parse_source(spec):
    if "=" in spec:
        name, ref = spec.split("=", 1)
    else:
        name, ref = spec.removeprefix("origin/"), spec
    return name, git("rev-parse", "--verify", f"{ref}^{{commit}}").strip()


def reverse_applies(commit, files, index_env):
    """Does `commit`'s diff of `files` apply in reverse to the tree in the scratch index?"""
    if not files:
        return True
    # Bytes, not text: a `--binary` patch (the reference PDF, the browser bundle) is not UTF-8.
    patch = subprocess.run(["git", "diff", "--binary", f"{commit}^", commit, "--", *files],
                           cwd=ROOT, capture_output=True, check=True).stdout
    r = subprocess.run(["git", "apply", "--cached", "--check", "-R", "-"], cwd=ROOT,
                       input=patch, capture_output=True, env=index_env)
    return r.returncode == 0


def patch_id(diff_args):
    """`git patch-id --stable` of the diff `git diff <diff_args>` — the identity of a change,
    independent of where it sits."""
    diff = subprocess.run(["git", "diff", "--binary", *diff_args], cwd=ROOT,
                          capture_output=True, check=True).stdout
    out = subprocess.run(["git", "patch-id", "--stable"], cwd=ROOT, input=diff,
                         capture_output=True, check=True).stdout.decode()
    return out.split()[0] if out.strip() else None


def mark_reverted(rows):
    """Skip a self-reverting run whole (JOINING.md § Joining a sibling checkout): a pending
    commit whose diff is exactly the INVERSE of an earlier pending one in the same source.
    Replayed, the revert was written against the source's tree and can delete what this tree
    holds; together the pair changes nothing."""
    pending = [r for r in rows if r["class"] in ("NEW", "PARTIAL")]
    undo = {}
    for r in pending:
        inv = patch_id([r["sha"], f"{r['sha']}^"])
        if inv:
            undo[inv] = r
    for r in pending:
        fwd = patch_id([f"{r['sha']}^", r["sha"]])
        a = undo.get(fwd)
        if a is not None and a is not r and pending.index(a) < pending.index(r):
            a["class"] = r["class"] = "UNDONE-PAIR"
            a["reverted_by"], r["reverts"] = r["sha"], a["sha"]


def only_rows(commit, path, registry):
    """Does `commit` change `path` only in measured audit ROWS — lines a re-derive rewrites?

    QUALITY.md is a hand-written doc holding `| **N** |` rows that `rederive` re-measures; a
    commit whose only change there is such a row carries nothing a join must take, and once the
    row moved on this tree its hunk no longer applies in reverse, which read it as missing."""
    if path not in registry.get("row_files", []):
        return False
    diff = subprocess.run(["git", "diff", "-U0", f"{commit}^", commit, "--", path], cwd=ROOT,
                          capture_output=True, text=True).stdout
    changed = [l[1:] for l in diff.splitlines()
               if l[:1] in "+-" and not l.startswith(("+++", "---"))]
    return bool(changed) and all(re.fullmatch(r"\| \*\*\d+\*\* \|", l.strip()) for l in changed)


def classify(commit, registry, index_env, scheduled):
    subject = git("log", "-1", "--format=%s", commit).strip()
    files = [f for f in git("diff-tree", "--no-commit-id", "--name-only", "-r", "--root",
                            commit).splitlines() if f]
    parents = git("rev-list", "--parents", "-n1", commit).split()
    row = {"sha": commit, "subject": subject, "files": len(files)}
    if len(parents) > 2:
        row["class"] = "MERGE"
        return row
    code = [f for f in files if not is_derived(f, registry)]
    if not code:
        row["class"] = "DERIVED"
        return row
    if subject in scheduled:
        row["class"] = "DUP"
        row["dup_of"] = scheduled[subject]
        return row
    present = [f for f in code
               if reverse_applies(commit, [f], index_env) or only_rows(commit, f, registry)]
    if len(present) == len(code):
        row["class"] = "HERE"
    elif present:
        row["class"] = "PARTIAL"
        row["missing"] = [f for f in code if f not in present]
    else:
        row["class"] = "NEW"
    return row


def cmd_survey(args):
    registry = load_registry()
    head = git("rev-parse", "HEAD").strip()
    base = git("rev-parse", args.base).strip()
    specs = args.sources or default_sources(args.days, head)
    if not specs:
        print("join: no branch has commits this tree lacks — nothing to join")
        return 0
    with tempfile.TemporaryDirectory() as tmp:
        env = dict(os.environ, GIT_INDEX_FILE=os.path.join(tmp, "index"))
        subprocess.run(["git", "read-tree", "HEAD"], cwd=ROOT, env=env, check=True)
        plan = {"head": head, "base": base, "created": datetime.datetime.now().isoformat(
            timespec="seconds"), "sources": []}
        scheduled = {}
        # This branch's own subjects since the base: a sibling that picked one of OUR commits
        # and resolved it against its own tree carries a different diff under the same subject.
        ours = set(git("log", "--format=%s", f"{base}..{head}").splitlines())
        # A squash-merge on the base lists what it carried as `* <subject>` lines in its body
        # (JOINING.md § Rebasing onto a squash): a pre-squash branch's commits are there, and
        # the content test cannot see them once the base edited the same lines again.
        squashed = {l[2:].strip() for l in git("log", "--format=%b", "-n", "40", base).splitlines()
                    if l.startswith("* ")}
        for spec in specs:
            name, sha = parse_source(spec)
            on_base = subprocess.run(["git", "merge-base", "--is-ancestor", base, sha],
                                     cwd=ROOT).returncode == 0
            commits = git("rev-list", "--reverse", f"{head}..{sha}").split()
            rows = []
            wip = None
            for c in commits:
                if wip:
                    rows.append({"sha": c, "subject": git("log", "-1", "--format=%s", c).strip(),
                                 "class": "AFTER-WIP", "files": 0})
                    continue
                r = classify(c, registry, env, scheduled)
                # A checkpoint its author has not verified ends what this join takes from the
                # source: the commits after it may build on it.
                if r["class"] in ("NEW", "PARTIAL") and not args.take_wip and re.match(
                        r"(?i)(wip\b|fixup!|squash!)", r["subject"]):
                    r["class"] = wip = "WIP"
                if r["class"] in ("NEW", "PARTIAL") and r["subject"] in ours:
                    r["class"] = "SUBJECT"
                elif r["class"] in ("NEW", "PARTIAL") and r["subject"] in squashed:
                    r["class"] = "SQUASHED"
                if r["class"] in ("NEW", "PARTIAL"):
                    scheduled[r["subject"]] = name
                rows.append(r)
            mark_reverted(rows)
            plan["sources"].append({"name": name, "sha": sha, "on_base": on_base,
                                    "commits": rows})
    save_plan(plan)
    print_plan(plan, verbose=args.verbose)
    return 0


def print_plan(plan, verbose=False):
    marks = {"NEW": "pick", "PARTIAL": "pick", "HERE": "skip", "DUP": "skip", "DERIVED": "skip",
             "MERGE": "skip", "SUBJECT": "skip", "SQUASHED": "skip", "WIP": "stop",
             "AFTER-WIP": "skip", "UNDONE-PAIR": "skip"}
    for s in plan["sources"]:
        counts = {}
        for r in s["commits"]:
            counts[r["class"]] = counts.get(r["class"], 0) + 1
        summary = ", ".join(f"{n} {c}" for c, n in sorted(counts.items()))
        base_note = "" if s["on_base"] else "  (not on the base — a pre-squash branch?)"
        print(f"== {s['name']} @ {s['sha'][:9]}: {summary or 'nothing'}{base_note}")
        for r in s["commits"]:
            if not verbose and r["class"] not in ("NEW", "PARTIAL", "MERGE", "SUBJECT", "WIP",
                                                  "AFTER-WIP", "UNDONE-PAIR"):
                continue
            done = "  done" if r.get("done") else ""
            extra = ""
            if r["class"] == "PARTIAL":
                extra = f"  [missing: {', '.join(r['missing'][:3])}]"
            elif r["class"] == "DUP":
                extra = f"  [= {r['dup_of']}]"
            elif r["class"] == "UNDONE-PAIR":
                other = r.get("reverted_by") or r.get("reverts")
                extra = f"  [a self-reverting pair with {other[:9]} — neither is taken]"
            elif r["class"] == "SUBJECT":
                extra = "  [this branch has a commit of that subject — check it is the same change]"
            print(f"  {marks[r['class']]:4} {r['class']:7} {r['sha'][:9]} {r['subject'][:92]}"
                  f"{extra}{done}")
    todo = sum(1 for s in plan["sources"] for r in s["commits"]
               if r["class"] in ("NEW", "PARTIAL") and not r.get("done"))
    print(f"\n{todo} commit(s) to pick; `scripts/join.py apply` picks them "
          f"(plan: {plan_path().relative_to(ROOT) if plan_path().is_relative_to(ROOT) else plan_path()})")


# ── apply ───────────────────────────────────────────────────────────────────────────────


def resolve_union(path, keep="both"):
    """Resolve every conflict hunk of `path` without a human.

    `both` keeps both sides — an append-only file such as a changelog.  `common` keeps the
    lines BOTH sides still carry — a shrink-only ratchet, where a line either side removed was
    a fix and a line only one side has would be a new entry the ratchet forbids."""
    text = (ROOT / path).read_text(encoding="utf-8")
    out, side, hunk = [], None, {"ours": [], "theirs": []}
    for line in text.splitlines(keepends=True):
        if line.startswith("<<<<<<< "):
            side, hunk = "ours", {"ours": [], "theirs": []}
            continue
        if side and line.startswith("||||||| "):
            side = "base"
            continue
        if side and line.startswith("=======") and line.strip() == "=======":
            side = "theirs"
            continue
        if side and line.startswith(">>>>>>> "):
            side = None
            if keep == "both":
                out += hunk["ours"] + hunk["theirs"]
            elif keep == "ours":
                out += hunk["ours"]
            else:
                out += [l for l in hunk["ours"] if l in hunk["theirs"]]
            continue
        if side in ("ours", "theirs"):
            hunk[side].append(line)
        elif side is None:
            out.append(line)
    (ROOT / path).write_text("".join(out), encoding="utf-8")


def conflicts_only_rows(path):
    """Is every line on either side of every conflict in `path` a measured `| **N** |` row?"""
    side, lines = None, []
    for line in (ROOT / path).read_text(encoding="utf-8").splitlines():
        if line.startswith("<<<<<<< "):
            side = "in"
        elif side and (line.startswith("=======") or line.startswith("||||||| ")):
            continue
        elif side and line.startswith(">>>>>>> "):
            side = None
        elif side:
            lines.append(line)
    return bool(lines) and all(re.fullmatch(r"\| \*\*\d+\*\* \|", l.strip()) for l in lines)


def cherry_pick(commit, registry):
    """Pick one commit.  Returns 'done', 'empty', or the list of files a human must resolve."""
    r = subprocess.run(["git", "cherry-pick", commit], cwd=ROOT, capture_output=True, text=True)
    if r.returncode == 0:
        return "done"
    blob = r.stdout + r.stderr
    if "is now empty" in blob or "nothing to commit" in blob:
        git("cherry-pick", "--skip", check=False)
        return "empty"
    conflicted = [f for f in git("diff", "--name-only", "--diff-filter=U").splitlines() if f]
    if not conflicted:
        return [f"(cherry-pick failed without a conflict: {blob.strip()[:200]})"]
    left = []
    for f in conflicted:
        if is_derived(f, registry):
            git("checkout", "--theirs", "--", f)
            git("add", "--", f)
            print(f"      {f}: a derived artefact — took the source's side, re-derived later")
        elif f in registry.get("union_files", []):
            resolve_union(f)
            git("add", "--", f)
            print(f"      {f}: append-only — kept both sides")
        elif f in registry.get("row_files", []) and conflicts_only_rows(f):
            resolve_union(f, keep="ours")
            git("add", "--", f)
            print(f"      {f}: only measured rows conflict — kept ours, re-derived later")
        elif f in registry.get("shrink_files", []):
            resolve_union(f, keep="common")
            git("add", "--", f)
            print(f"      {f}: a shrink-only ratchet — kept the lines both sides carry")
        else:
            left.append(f)
    if left:
        return left
    env = dict(os.environ, GIT_EDITOR="true")
    r = subprocess.run(["git", "cherry-pick", "--continue"], cwd=ROOT, capture_output=True,
                       text=True, env=env)
    if r.returncode != 0:
        if "is now empty" in r.stdout + r.stderr:
            git("cherry-pick", "--skip", check=False)
            return "empty"
        return [f"(cherry-pick --continue failed: {(r.stdout + r.stderr).strip()[:200]})"]
    return "done"


def cmd_apply(args):
    registry = load_registry()
    plan = load_plan()
    if (git_dir() / "CHERRY_PICK_HEAD").exists():
        sys.exit("join: a cherry-pick is in progress — resolve it, `git cherry-pick --continue`, "
                 "then run `scripts/join.py apply` again")
    clean_tree_or_exit()
    if args.skip:
        for s in plan["sources"]:
            for r in s["commits"]:
                if r["sha"].startswith(args.skip):
                    r["done"] = "skipped"
                    print(f"  {r['sha'][:9]} marked skipped")
        save_plan(plan)
    # A commit a human finished by hand after a stop is the one the plan still lists as pending
    # while HEAD moved past the plan's last recorded head.
    last = plan.get("applied_head", plan["head"])
    if git("rev-parse", "HEAD").strip() != last:
        pending = next((r for s in plan["sources"] for r in s["commits"]
                        if r["class"] in ("NEW", "PARTIAL") and not r.get("done")), None)
        if pending and plan.get("stopped_at") == pending["sha"]:
            pending["done"] = "by hand"
            print(f"  {pending['sha'][:9]} resolved by hand — continuing")
    for s in plan["sources"]:
        todo = [r for r in s["commits"] if r["class"] in ("NEW", "PARTIAL") and not r.get("done")]
        picked_any = any(r["class"] in ("NEW", "PARTIAL") for r in s["commits"])
        if not todo and (s.get("checked") or not picked_any):
            # Nothing of this source's was taken: the tree is what the previous check saw.
            s["checked"] = True
            continue
        if todo:
            print(f"== {s['name']}: picking {len(todo)}")
        for r in todo:
            res = cherry_pick(r["sha"], registry)
            if isinstance(res, list):
                plan["stopped_at"] = r["sha"]
                save_plan(plan)
                print(f"\nSTOP  {r['sha'][:9]} {r['subject'][:80]}\n      conflict in source "
                      f"file(s): {', '.join(res)}\n      resolve by EDITING the markers (keep both "
                      "halves — never `--theirs` on a source file), `git add`, `git cherry-pick "
                      "--continue`,\n      then `scripts/join.py apply` again (to drop it instead: `git "
                      f"cherry-pick --skip`, then `scripts/join.py apply --skip {r['sha'][:9]}`).\n"
                      "      JOINING.md § Resolving a conflict.")
                return 1
            r["done"] = res
            plan["applied_head"] = git("rev-parse", "HEAD").strip()
            save_plan(plan)
            print(f"  {res:5} {r['sha'][:9]} {r['subject'][:92]}")
        if not args.no_check and not s.get("checked"):
            print(f"  cargo check --all-targets after {s['name']} …")
            ok, _ = sh("cargo check --all-targets -q")
            if not ok:
                print(f"\nSTOP  the tree does not compile after {s['name']} — a defect only the "
                      "union has, or a resolution that dropped a half.  Fix it in a commit of its "
                      "own, then run `scripts/join.py apply` again.")
                return 1
        s["checked"] = True
        save_plan(plan)
    plan["applied_head"] = git("rev-parse", "HEAD").strip()
    save_plan(plan)
    print("\nall sources applied — next: `scripts/join.py rederive`")
    return 0


# ── rederive ────────────────────────────────────────────────────────────────────────────


def read_numbers(text, fmt):
    """A pin file's numbers, keyed: flat JSON leaves, or a TSV row's leading columns."""
    out = {}
    if text is None:
        return out
    if fmt == "json":
        try:
            data = json.loads(text)
        except json.JSONDecodeError:
            return out

        def walk(prefix, v):
            if isinstance(v, dict):
                for k, x in v.items():
                    walk(f"{prefix}{k}.", x)
            elif isinstance(v, (int, float)) and not isinstance(v, bool):
                out[prefix.rstrip(".")] = v
        walk("", data)
    else:
        for line in text.splitlines():
            if not line.strip() or line.startswith("#") or line.startswith("<<<<<<<"):
                continue
            cols = line.split("\t")
            if len(cols) >= 2 and re.fullmatch(r"-?\d+", cols[-1].strip()):
                out["\t".join(cols[:-1])] = int(cols[-1])
    return out


def numbers_at(ref, pin):
    r = subprocess.run(["git", "show", f"{ref}:{pin['file']}"], cwd=ROOT, capture_output=True,
                       text=True)
    return read_numbers(r.stdout if r.returncode == 0 else None, pin["format"])


def growth_budget(plan, pin):
    """Per key: the joining branch's own pin plus every source's own growth over its fork point.

    A number the union reads above this came from the UNION, not from a source that measured
    and pinned its own growth — that is the rise a join must not re-pin in silence."""
    pre = numbers_at(plan["head"], pin)
    budget = dict(pre)
    rows = {"(this branch)": pre}
    for s in plan["sources"]:
        fork = git("merge-base", plan["head"], s["sha"]).strip()
        at_fork, at_tip = numbers_at(fork, pin), numbers_at(s["sha"], pin)
        rows[s["name"]] = at_tip
        for k, v in at_tip.items():
            grew = v - at_fork.get(k, v)
            if grew > 0:
                budget[k] = budget.get(k, 0) + grew
    return budget, rows


def inputs_changed(plan, art):
    if not art.get("inputs"):
        return True
    return bool(git("diff", "--name-only", plan["head"], "HEAD", "--", *art["inputs"]).strip())


def optional_sites_diff(plan):
    """The shape tests opaque to `τ?` on the union that the pre-join tree did not have."""
    here = opaque_tests(ROOT)
    wt = git_dir() / "loft-join" / f"pre-{plan['head'][:12]}"
    if not wt.exists():
        git("worktree", "add", "--detach", str(wt), plan["head"])
    try:
        before = opaque_tests(wt)
    finally:
        git("worktree", "remove", "--force", str(wt), check=False)
    return sorted(here - before)


def opaque_tests(tree):
    code = r"""
import os, sys
sys.path.insert(0, os.path.join(os.getcwd(), "scripts"))
import ir_walker_audit as a
f = a.OPTIONAL
for path in a.rust_files():
    for name, start, body in a.functions(path):
        code = a.code_only(body)
        for off, kind, scrut, pats in a.shape_tests(code):
            if not a.test_sees(scrut, pats, a.peel_bound(code, off, f), f):
                vs = " ".join(sorted(a.pattern_variants(pats)))
                print(f"{a.rel(path)}::{name}  {kind} {scrut.strip()[:50]}  [{vs}]")
"""
    r = subprocess.run([sys.executable, "-c", code], cwd=tree, capture_output=True, text=True)
    # A MULTISET: a second identical shape test in a function that already had one is growth
    # too, and a set difference reported it as "none" while the ratchet counted it.
    return collections.Counter(r.stdout.splitlines())


def cmd_rederive(args):
    registry = load_registry()
    plan = load_plan()
    clean_tree_or_exit(registry)
    only = set(args.only.split(",")) if args.only else None
    accept = set(args.accept.split(",")) if args.accept else set()
    moved, stopped = [], []

    def status():
        return git("status", "--porcelain", "--untracked-files=no")

    for art in registry["artefacts"]:
        name = art["name"]
        if only and name not in only:
            continue
        if not art["regen"]:
            print(f"  --    {name:22} not re-derived on a join ({art.get('note', 'no writer')[:60]}…)")
            continue
        if not inputs_changed(plan, art):
            print(f"  --    {name:22} its inputs did not move in the join")
            continue
        if art.get("check_first") and art.get("check"):
            ok, out = sh(art["check"], quiet=True)
            if not ok:
                stopped.append(name)
                print(f"  STOP  {name:22} fails on the union BEFORE re-deriving:")
                print("\n".join("        " + l for l in out.splitlines()[-10:]))
                print(f"        cure: {art.get('cure', 'read the check')}")
                continue
        pin = art.get("pin")
        if pin:
            current = measure_pin(art, pin)
            budget, rows = growth_budget(plan, pin)
            over = {k: (v, budget.get(k)) for k, v in current.items()
                    if budget.get(k) is not None and v > budget[k]}
            if over and name not in accept:
                stopped.append(name)
                print(f"  STOP  {name:22} grew past what the sources pinned themselves:")
                for k, (v, b) in over.items():
                    print(f"          {k}: union {v} > allowed {b}")
                    for src, nums in rows.items():
                        print(f"            {src:32} {nums.get(k, '—')}")
                if art.get("on_growth") == "optional-sites":
                    new = optional_sites_diff(plan)
                    print("        the shape tests the join made opaque:")
                    print("\n".join("          " + l for l in new) or "          (none against "
                          "the pre-join tree — the growth PREDATES this join: this branch's own "
                          "commits since the pin added it; diff against the commit that wrote "
                          "the pin, `git log -1 -- index/optional_ratchet.json`)")
                print(f"        cure: {art.get('cure', '')}")
                continue
        before = status()
        for cmd in art["regen"]:
            ok, _ = sh(cmd)
            if not ok:
                stopped.append(name)
                print(f"  STOP  {name:22} `{cmd}` failed")
                break
        else:
            if art.get("check"):
                ok, _ = sh(art["check"])
                if not ok:
                    stopped.append(name)
                    print(f"  STOP  {name:22} still fails its check after re-deriving")
                    continue
            changed = status() != before
            print(f"  ok    {name:22} {'re-derived' if changed else 'unchanged'}")
            if changed:
                moved.append(name)
    changes = git("status", "--porcelain", "--untracked-files=no").strip()
    if stopped:
        print(f"\n{len(stopped)} artefact(s) need a decision: {', '.join(stopped)} — "
              "nothing is committed")
        return 1
    if not changes:
        print("\nevery derived artefact already matches the union")
        return 0
    msg = rederive_message(plan, moved)
    if args.commit:
        git("add", "-u")
        subprocess.run(["git", "commit", "-q", "-F", "-"], cwd=ROOT, input=msg, text=True,
                       check=True)
        print(f"\ncommitted: {git('log', '-1', '--format=%h %s').strip()}")
    else:
        print("\nre-derived and left staged-for-review; commit with `--commit`, or by hand:\n")
        print("\n".join("  " + l for l in msg.splitlines()))
    return 0


def measure_pin(art, pin):
    """The union's own numbers: re-pin into a scratch copy, read it, and put the file back."""
    path = ROOT / pin["file"]
    saved = path.read_bytes()
    try:
        for cmd in art["regen"]:
            sh(cmd, quiet=True)
        return read_numbers(path.read_text(encoding="utf-8"), pin["format"])
    finally:
        path.write_bytes(saved)


def rederive_message(plan, moved):
    lines = ["The joined tree's derived artefacts, re-measured on the union", ""]
    lines.append("Sources: " + ", ".join(f"{s['name']} @ {s['sha'][:9]}" for s in plan["sources"]))
    lines.append("")
    for name in moved:
        lines.append(f"- {name}")
    lines += ["", "By `scripts/join.py rederive` (doc/claude/JOINING.md § The script)."]
    return "\n".join(lines) + "\n"


# ── verify / guards ─────────────────────────────────────────────────────────────────────


def cmd_verify(args):
    registry = load_registry()
    failed = []
    for name, cmd in registry["verify"]:
        t0 = datetime.datetime.now()
        ok, _ = sh(cmd)
        secs = (datetime.datetime.now() - t0).seconds
        print(f"  {'ok  ' if ok else 'FAIL'}  {name:20} {secs:4}s")
        if not ok:
            failed.append(name)
    if failed:
        print(f"\nverify FAILED: {', '.join(failed)}")
        return 1
    print("\nverify ok — the gate is next: `scripts/ci-run.sh start`")
    return 0


def cmd_guards(args):
    plan = load_plan()
    since = plan["base"] if args.since_base else plan["head"]
    files = [f for f in git("diff", "--name-only", "--diff-filter=AM", since, "HEAD", "--",
                            "tests/scripts/*.loft").splitlines() if f]
    if not files:
        print("the join brought in no tests/scripts guard")
        return 0
    loft = ROOT / "target" / "release" / "loft"
    ok, _ = sh("cargo build --release -q --bin loft")
    if not ok:
        return 1
    modes = [""] if args.interp_only else ["", "--native"]
    jobs = [(f, m) for f in files for m in modes]

    def run(job):
        f, m = job
        env = dict(os.environ, LOFT_TIMEOUT="300")
        r = subprocess.run([str(loft), "--tests", *([m] if m else []), f], cwd=ROOT, env=env,
                           capture_output=True, text=True)
        last = (r.stdout + r.stderr).strip().splitlines()
        return f, m or "--interpret", r.returncode == 0, last[-1] if last else ""

    bad = []
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for f, m, ok, last in pool.map(run, jobs):
            if not ok:
                bad.append((f, m, last))
    print(f"{len(files)} guard(s) × {len(modes)} backend(s): {len(jobs) - len(bad)} ok, "
          f"{len(bad)} failed")
    for f, m, last in bad:
        print(f"  FAIL {m:11} {f}\n       {last[:160]}")
    return 1 if bad else 0


# ── quality rows ────────────────────────────────────────────────────────────────────────

# QUALITY.md's audit rows: (the row's header, the audit mode, the audit's label, whether the
# count may FALL).  The same rows `scripts/gate_preflight.sh` and doc_hygiene's
# `quality_*_table_matches_the_audit` compare; this is the writer they lacked.  A row whose
# count must not shrink (functions handling BOTH projection spellings) is never written
# lower: a fall there is a lost `TupleGet` arm, a code change to find, not a row to update.
QUALITY_ROWS = (
    ("| sites a `Span` hides the shape from", "unspan",
     "neither — a `Span` hides the shape from them", True),
    ("| opaque to a wrapped shape", "optional", "opaque to a wrapped shape", True),
    ("| functions ALSO handling the `TupleGet` spelling", "spellings",
     "ALSO handling the `TupleGet` spelling", False),
)


def cmd_quality_rows(args):
    path = ROOT / "doc" / "claude" / "QUALITY.md"
    lines = path.read_text(encoding="utf-8").split("\n")
    bad, refused = [], []
    for header, mode, label, may_fall in QUALITY_ROWS:
        out = subprocess.run([sys.executable, "scripts/ir_walker_audit.py", mode], cwd=ROOT,
                             capture_output=True, text=True).stdout
        m = re.search(re.escape(label) + r"\s*:\s*(\d+)", out)
        if not m:
            sys.exit(f"join: `ir_walker_audit.py {mode}` printed no '{label}'")
        now = int(m.group(1))
        i = next(k for k, l in enumerate(lines) if l.startswith(header))
        have = [int(c.strip().strip("*")) for c in lines[i + 2].split("|")
                if c.strip().strip("*").isdigit()]
        if have != [now]:
            if not may_fall and have and now < have[0]:
                refused.append(f"QUALITY.md's {mode} row says {have} and the audit reports {now}: "
                               "it must not shrink — find the lost arm, never lower the row")
                continue
            bad.append(f"QUALITY.md's {mode} row says {have}, the audit reports {now}")
            lines[i + 2] = f"| **{now}** |"
    if refused:
        print("\n".join(refused))
    if args.write:
        path.write_text("\n".join(lines), encoding="utf-8")
        for b in bad:
            print(f"  wrote: {b}")
        return 1 if refused else 0
    print("\n".join(bad))
    return 1 if bad or refused else 0


# ── run ─────────────────────────────────────────────────────────────────────────────────


def cmd_run(args):
    for step in (cmd_survey, cmd_apply, cmd_rederive, cmd_verify, cmd_guards):
        print(f"\n#### {step.__name__[4:]}")
        if step(args) != 0:
            return 1
    print("\njoined, re-derived and verified — `scripts/ci-run.sh start` runs the gate")
    return 0


def main():
    # Line-buffered, so a redirected run (`… > join.log`) shows each step as it finishes.
    sys.stdout.reconfigure(line_buffering=True)
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)

    def common(p):
        p.add_argument("sources", nargs="*", help="refs, or name=<sha>")
        p.add_argument("--base", default="origin/main")
        p.add_argument("--days", type=int, default=2)
        p.add_argument("--verbose", action="store_true", help="list skipped commits too")
        p.add_argument("--take-wip", action="store_true",
                       help="take a source's `wip` checkpoints too (the owner's call, per join)")

    common(sub.add_parser("survey"))
    p = sub.add_parser("apply")
    p.add_argument("--no-check", action="store_true", help="skip cargo check between sources")
    p.add_argument("--skip", help="mark this commit (sha prefix) as deliberately not picked")
    p = sub.add_parser("rederive")
    p.add_argument("--only", help="comma-separated artefact names")
    p.add_argument("--accept", help="artefacts whose growth is meant (say why in the commit)")
    p.add_argument("--commit", action="store_true")
    sub.add_parser("verify")
    p = sub.add_parser("guards")
    p.add_argument("--jobs", type=int, default=4)
    p.add_argument("--interp-only", action="store_true")
    p.add_argument("--since-base", action="store_true",
                   help="every guard changed since the base, not only since the join started")
    p = sub.add_parser("quality-rows")
    g = p.add_mutually_exclusive_group(required=True)
    g.add_argument("--check", action="store_true")
    g.add_argument("--write", action="store_true")
    p = sub.add_parser("run")
    common(p)
    p.add_argument("--no-check", action="store_true")
    p.add_argument("--skip")
    p.add_argument("--only")
    p.add_argument("--accept")
    p.add_argument("--commit", action="store_true")
    p.add_argument("--jobs", type=int, default=4)
    p.add_argument("--interp-only", action="store_true")
    p.add_argument("--since-base", action="store_true")
    args = ap.parse_args()
    fn = {"survey": cmd_survey, "apply": cmd_apply, "rederive": cmd_rederive,
          "verify": cmd_verify, "guards": cmd_guards, "quality-rows": cmd_quality_rows,
          "run": cmd_run}[args.cmd]
    sys.exit(fn(args))


if __name__ == "__main__":
    main()
