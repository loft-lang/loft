#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Claim an issue before fixing it, on the one channel every machine shares: the issue itself.

    scripts/claim.py take N [--why TEXT] [--take-over]   claim an existing issue before starting
    scripts/claim.py file --title T --body-file F [--label L …] [--why TEXT]
                                                          file a bug you found AND will fix now:
                                                          it is born claimed, so no other machine
                                                          can pick it up in between
    scripts/claim.py release N [--why TEXT]              hand a claim back (work stopped, moved)
    scripts/claim.py check N                             who holds it, if anyone

    --repo loft-lang/loft (default) | loft-lang/plans    LOFT_STREAM=<name> overrides the stream

A claim is a comment (or, for `file`, a body line) whose first line starts `claim: <stream>`, and
a release one that starts `release: <stream>`.  The stream is `<host>/<checkout>@<branch>`, so two
checkouts on one machine, and two machines, are told apart.  The live claim is the EARLIEST claim
not released by its own stream: when two streams claim at once both see it, and the later one
withdraws.  The `in-progress` label mirrors a live claim, and `make work` skips it.

A claim with no comment on the issue for STALE_HOURS can be taken over with `--take-over`; the
take-over is written on the issue, so the old holder finds it.  An issue already labelled
`fixed-pending-merge` is refused: its fix has landed on a branch.  Exit 0 = claimed / released /
free, 1 = held by another stream (its claim is printed), 2 = could not ask.
"""

import argparse
import datetime
import json
import os
import socket
import subprocess
import sys

STALE_HOURS = 48
LABEL = "in-progress"


def gh(*args, input_text=None):
    r = subprocess.run(["gh", *args], capture_output=True, text=True, input=input_text)
    if r.returncode != 0:
        sys.exit(f"claim: gh {' '.join(args[:3])} failed: {r.stderr.strip()}")
    return r.stdout


def stream_name():
    if os.environ.get("LOFT_STREAM"):
        return os.environ["LOFT_STREAM"]
    top = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    branch = subprocess.run(["git", "branch", "--show-current"], capture_output=True, text=True)
    checkout = os.path.basename(top.stdout.strip()) or "?"
    return f"{socket.gethostname()}/{checkout}@{branch.stdout.strip() or 'detached'}"


def when(stamp):
    return datetime.datetime.fromisoformat(stamp.replace("Z", "+00:00"))


def read_issue(repo, n):
    raw = gh("issue", "view", str(n), "--repo", repo, "--json", "state,labels,body,createdAt,comments")
    return json.loads(raw)


def events(issue):
    """Every claim / release on the issue, oldest first: (time, kind, stream, text)."""
    out = []
    posts = [(issue["createdAt"], issue.get("body") or "")]
    posts += [(c["createdAt"], c.get("body") or "") for c in issue.get("comments", [])]
    for stamp, body in posts:
        for line in body.splitlines():
            line = line.strip()
            for kind in ("claim", "release"):
                if line.startswith(f"{kind}: "):
                    stream = line[len(kind) + 2:].split(" — ")[0].split()[0]
                    out.append((when(stamp), kind, stream, line))
    return sorted(out, key=lambda e: e[0])


def live_claims(issue):
    """Claims not released by their own stream, earliest first."""
    held = {}
    for stamp, kind, stream, text in events(issue):
        if kind == "claim" and stream not in held:
            held[stream] = (stamp, text)
        elif kind == "release":
            held.pop(stream, None)
    return sorted(held.items(), key=lambda kv: kv[1][0])


def last_activity(issue):
    stamps = [when(issue["createdAt"])] + [when(c["createdAt"]) for c in issue.get("comments", [])]
    return max(stamps)


def labels(issue):
    return {l["name"] for l in issue.get("labels", [])}


def take(args, me):
    issue = read_issue(args.repo, args.n)
    if issue["state"] != "OPEN":
        print(f"claim: #{args.n} is {issue['state'].lower()}")
        return 1
    if "fixed-pending-merge" in labels(issue):
        print(f"claim: #{args.n} already has a fix on a branch (fixed-pending-merge) — join it, do not refix")
        return 1
    others = [(s, v) for s, v in live_claims(issue) if s != me]
    if others and not args.take_over:
        stream, (stamp, text) = others[0]
        idle = datetime.datetime.now(datetime.timezone.utc) - last_activity(issue)
        stale = idle.total_seconds() > STALE_HOURS * 3600
        print(f"claim: #{args.n} is held — {text} ({stamp:%Y-%m-%d %H:%M} UTC)")
        if stale:
            print(f"  no activity for {idle.days}d {idle.seconds // 3600}h: `--take-over` may take it")
        return 1
    why = f" — {args.why}" if args.why else ""
    note = ""
    if others:
        note = f" (taken over from {', '.join(s for s, _ in others)}: no activity for {STALE_HOURS}h+)"
    gh("issue", "comment", str(args.n), "--repo", args.repo, "--body", f"claim: {me}{why}{note}")
    gh("issue", "edit", str(args.n), "--repo", args.repo, "--add-label", LABEL)
    # Two streams claiming in the same minute both posted; the earliest live claim wins.
    after = live_claims(read_issue(args.repo, args.n))
    if after and after[0][0] != me and not args.take_over:
        winner = after[0][1][1]
        gh("issue", "comment", str(args.n), "--repo", args.repo,
           "--body", f"release: {me} — withdrawn, an earlier claim holds it")
        print(f"claim: lost the race to — {winner}")
        return 1
    print(f"claim: #{args.n} claimed by {me}")
    return 0


def file_claimed(args, me):
    body = open(args.body_file).read().rstrip() + "\n\n"
    why = f" — {args.why}" if args.why else " — found while working nearby; fixing it now"
    body += f"claim: {me}{why}\n"
    cmd = ["issue", "create", "--repo", args.repo, "--title", args.title, "--body", body,
           "--label", LABEL]
    for l in args.label or []:
        cmd += ["--label", l]
    url = gh(*cmd).strip()
    print(f"claim: filed and claimed {url}")
    return 0


def release(args, me):
    why = f" — {args.why}" if args.why else ""
    gh("issue", "comment", str(args.n), "--repo", args.repo, "--body", f"release: {me}{why}")
    if not [s for s, _ in live_claims(read_issue(args.repo, args.n)) if s != me]:
        gh("issue", "edit", str(args.n), "--repo", args.repo, "--remove-label", LABEL)
    print(f"claim: #{args.n} released by {me}")
    return 0


def check(args, me):
    issue = read_issue(args.repo, args.n)
    claims = live_claims(issue)
    if not claims:
        print(f"claim: #{args.n} is free")
        return 0
    for stream, (stamp, text) in claims:
        mine = " (this stream)" if stream == me else ""
        print(f"claim: #{args.n} — {text} ({stamp:%Y-%m-%d %H:%M} UTC){mine}")
    return 0 if claims[0][0] == me else 1


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--repo", default="loft-lang/loft")
    sub = p.add_subparsers(dest="cmd", required=True)
    t = sub.add_parser("take")
    t.add_argument("n", type=int)
    t.add_argument("--why")
    t.add_argument("--take-over", action="store_true")
    f = sub.add_parser("file")
    f.add_argument("--title", required=True)
    f.add_argument("--body-file", required=True)
    f.add_argument("--label", action="append")
    f.add_argument("--why")
    r = sub.add_parser("release")
    r.add_argument("n", type=int)
    r.add_argument("--why")
    c = sub.add_parser("check")
    c.add_argument("n", type=int)
    args = p.parse_args()
    me = stream_name()
    return {"take": take, "file": file_claimed, "release": release, "check": check}[args.cmd](args, me)


if __name__ == "__main__":
    sys.exit(main())
