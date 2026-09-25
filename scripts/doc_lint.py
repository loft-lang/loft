#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The documentation lint: every rule of doc/claude/DOC_CONTRACT.md a script can check.

One implementation, three callers:

    python3 scripts/doc_lint.py <path>…                  findings in these files
    python3 scripts/doc_lint.py --since HEAD <path>…     only findings HEAD's version lacks
                                                         (the edit hook, .claude/settings.json)
    python3 scripts/doc_lint.py --gate --since <base> --changed
                                                         the PR check: a change may not ADD a
                                                         gated finding
    python3 scripts/doc_lint.py --all [--baseline F]     the report behind `make docs-lint`
    python3 scripts/doc_lint.py --all --write-baseline F re-pin that report's baseline
    python3 scripts/doc_lint.py --hook                   the PostToolUse hook: a tool call on
                                                         stdin, findings the edit added out

Output is one line per finding, `path:line: rule: message`, and nothing when a file is clean.
The exit status is 0 unless `--gate` is given and a gated finding is new.

A finding is keyed by its rule and the text of its line, never by the line number, so an
edit that moves text does not make old findings look new.

Rules (the ids DOC_CONTRACT.md's lines point at):

A contract-doc line carrying `<!-- doc-lint: ok -->` is one its author checked and meant.

    stamp      a plan tag, phase or date in a code comment                     gated
    narration  change narration or an incident as a code comment's subject     report
    history    before/after phrasing or a dated ruling in a contract doc        gated
    timeline   a date, hash or status word in a contract doc                   report
    size       a maintainer doc over the 1000-line ceiling                     gated when crossed
    two-h1     more than one H1 outside code fences                            gated
    orphan     a maintainer doc not reachable from CLAUDE.md in two hops       report
    hedge      a temporal or hedge word in user-facing prose                   report
    link       a broken relative link                                          gated in tests/doc_hygiene.rs

The patterns are not repeated here: code comments use `scripts/lint_comments.sh`'s own
regular expressions, contract docs `scripts/doc_history_report.py`'s, hedge words
`scripts/doc_review.py`'s and links `tools/indexer/fix_broken_links.py`'s.
"""
from __future__ import annotations

import argparse
import collections
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
sys.path.insert(0, os.path.join(ROOT, "tools", "indexer"))

import doc_history_report as history_report  # noqa: E402
import doc_review  # noqa: E402

SIZE_CEILING = 1000
SIZE_EXEMPT = re.compile(r"<!--\s*size-exempt:")
GATED = {"stamp", "history", "two-h1", "size"}
FENCE = re.compile(r"^\s*(```|~~~)")


# ── the patterns, read from their homes ──────────────────────────────────────

def _shell_regex(name: str) -> re.Pattern:
    """A `NAME='…'` regular expression from lint_comments.sh, so both tools match alike."""
    text = open(os.path.join(ROOT, "scripts", "lint_comments.sh"), encoding="utf-8").read()
    m = re.search(rf"^{name}='(.*)'$", text, re.M)
    if not m:
        sys.exit(f"doc_lint: {name} is missing from scripts/lint_comments.sh")
    return re.compile(m.group(1), re.I if name in ("HIST_RE", "INCIDENT_RE") else 0)


TAGS_RE = _shell_regex("TAGS_RE")
HIST_RE = _shell_regex("HIST_RE")
INCIDENT_RE = _shell_regex("INCIDENT_RE")
LICENCE_RE = _shell_regex("LICENCE_RE")

# `history` is the narrow, gated part of the history signal: phrasing that only a sentence
# about a change uses.  A date or an issue id is often a live pointer, so those stay in the
# `timeline` report.
BEFORE_AFTER = re.compile(
    r"\b(previously(?!-)|had been|used to be|the bug was|turned out to be|"
    r"until (?:this|that) (?:fix|landed))\b", re.I)
# A line the author has checked and meant: `<!-- doc-lint: ok -->` anywhere on it.
LINE_OK = "<!-- doc-lint: ok -->"
# Quoted and code-spanned text is an example, not the doc's own voice.
QUOTED = re.compile(r"`[^`]*`|\"[^\"]*\"|“[^”]*”|\*\"[^\"]*\"\*")
DATED_RULING = re.compile(
    r"\b(ruled|retired|decided|superseded|reverted)\b[^.]{0,40}\b20\d\d-\d\d(?:-\d\d)?\b"
    r"|\b20\d\d-\d\d(?:-\d\d)?\b[^.]{0,40}\b(ruled|retired|decided)\b", re.I)


# ── which surface a file is ──────────────────────────────────────────────────

def is_code(path: str) -> bool:
    return (path.endswith(".rs") and (path.startswith("src/") or "/src/" in path)) or \
        (path.startswith("default/") and path.endswith(".loft"))


def is_maintainer_doc(path: str) -> bool:
    return path.endswith(".md") and (path.startswith("doc/claude/") or path == "CLAUDE.md"
                                     or path.startswith(".claude/"))


def is_contract_doc(path: str) -> bool:
    if not path.endswith(".md") or path.endswith("-history.md"):
        return False
    if path in history_report.EXCLUDE_EXACT or path.startswith(history_report.EXCLUDE_DIRS):
        return False
    return path.startswith(("doc/claude/", "doc/")) or path in ("README.md", "CLAUDE.md")


def is_user_prose(path: str) -> bool:
    return path == "README.md" or (path.startswith("doc/") and path.endswith(".md")
                                   and not path.startswith("doc/claude/")) or \
        (path.startswith("tests/docs/") and path.endswith(".loft"))


# ── the checks ───────────────────────────────────────────────────────────────

def comment_lines(lines):
    for n, line in enumerate(lines, 1):
        s = line.lstrip()
        if s.startswith("//"):
            yield n, s


def prose_lines(lines):
    """Lines of a markdown file outside code fences."""
    fenced = False
    for n, line in enumerate(lines, 1):
        if FENCE.match(line):
            fenced = not fenced
            continue
        if not fenced:
            yield n, line


def check_code(path, lines):
    for n, s in comment_lines(lines):
        if LICENCE_RE.search(s):
            continue
        if TAGS_RE.search(s):
            yield n, "stamp", "a plan tag, phase or date in a comment — git blame keeps it (DOC_QUALITY § A)"
        elif INCIDENT_RE.search(s) or HIST_RE.search(s):
            yield n, "narration", "describes a change or an incident, not the code (DOC_QUALITY § B, B2)"


def check_contract(path, lines):
    for n, line in prose_lines(lines):
        if "<!--noindex-->" in line or LINE_OK in line:
            continue
        voice = QUOTED.sub("", line)
        if BEFORE_AFTER.search(voice) or DATED_RULING.search(voice):
            yield n, "history", "a change told in a contract doc — state the rule, move the story (DOC_QUALITY § Maintainer docs 4)"
        elif any(p.search(line) for p, _ in history_report.SIGNALS):
            yield n, "timeline", "a date, hash or status word in a contract doc"


def check_markdown_shape(path, lines):
    h1 = [n for n, line in prose_lines(lines) if line.startswith("# ")]
    for n in h1[1:]:
        yield n, "two-h1", f"a second H1 (the first is line {h1[0]}) — one doc, one title"
    if is_maintainer_doc(path) and len(lines) > SIZE_CEILING \
            and not any(SIZE_EXEMPT.search(l) for l in lines[:20]):
        yield len(lines), "size", f"{len(lines)} lines, over the {SIZE_CEILING}-line ceiling (DOC_QUALITY § Maintainer docs 2)"


def check_hedge(path, lines):
    if path.endswith(".loft"):
        src = ((n, l) for n, l in enumerate(lines, 1) if l.lstrip().startswith("//"))
    else:
        src = prose_lines(lines)
    for n, line in src:
        m = doc_review.HEDGE_RE.search(doc_review.CODE_SPAN.sub("", line))
        if m:
            yield n, "hedge", f"'{m.group(0)}' — say what is true now, or cite the issue (API_SURFACE S7)"


def check_links(path, text):
    import fix_broken_links as fbl
    if not fbl.in_scope(path):
        return
    _, found = fbl.scan(path, text, _all_paths())
    for n, target, new, why in found:
        if new is not None:
            yield n, "link", f"{target} is broken; `make doc-fix` repairs it to {new}"
        else:
            yield n, "link", f"{target} is broken ({why})"


_PATHS = None


def _all_paths():
    global _PATHS
    if _PATHS is None:
        import fix_broken_links as fbl
        from rewrite_links import tracked_files
        _PATHS = fbl.all_paths(tracked_files())
    return _PATHS


# ── reachability ─────────────────────────────────────────────────────────────

_REACH = None
LINK = re.compile(r"\]\(([^)\s#]+\.md)(?:#[^)]*)?\)")


def reachable():
    """The .md files within two link hops of CLAUDE.md."""
    global _REACH
    if _REACH is not None:
        return _REACH

    def links_of(rel):
        try:
            text = open(os.path.join(ROOT, rel), encoding="utf-8").read()
        except OSError:
            return set()
        base = os.path.dirname(rel)
        return {os.path.normpath(os.path.join(base, t)) for t in LINK.findall(text)}

    hop1 = links_of("CLAUDE.md")
    hop2 = set().union(*(links_of(r) for r in hop1)) if hop1 else set()
    _REACH = {"CLAUDE.md"} | hop1 | hop2
    return _REACH


def check_orphan(path):
    # The rule covers the maintainer docs a reader browses: doc/claude/*.md and the formal
    # chapters.  Plans are reached through their tracker issues, not the index.
    top = os.path.dirname(path) in ("doc/claude", "doc/claude/formal")
    # A `-history.md` companion is reached through the doc it belongs to.
    if top and path != "doc/claude/LIBRARIES.md" and not path.endswith("-history.md") \
            and path not in reachable():
        yield 1, "orphan", "not reachable from CLAUDE.md in two hops (DOC_QUALITY § Maintainer docs 3)"


# ── one file ─────────────────────────────────────────────────────────────────

def lint_text(path: str, text: str, with_orphan: bool = True):
    lines = text.split("\n")
    out = []
    if is_code(path):
        out += check_code(path, lines)
    if path.endswith(".md"):
        if is_contract_doc(path):
            out += check_contract(path, lines)
        out += check_markdown_shape(path, lines)
        out += check_links(path, text)
        if with_orphan:
            out += check_orphan(path)
    if is_user_prose(path):
        out += check_hedge(path, lines)
    return [(n, rule, msg, lines[n - 1].strip() if 0 < n <= len(lines) else "")
            for n, rule, msg in out]


def key(rule, text):
    return (rule, " ".join(text.split()))


def old_text(path, rev):
    r = subprocess.run(["git", "show", f"{rev}:{path}"], cwd=ROOT, capture_output=True)
    return r.stdout.decode("utf-8", "replace") if r.returncode == 0 else None


def new_findings(path, rev):
    """Findings in the working file that `rev`'s version of it does not have."""
    text = open(os.path.join(ROOT, path), encoding="utf-8").read()
    now = lint_text(path, text)
    before = old_text(path, rev)
    if before is None:
        return now
    old = lint_text(path, before, False)
    seen = collections.Counter(key(r, t) for _, r, _, t in old if r != "size")
    was_over = any(r == "size" for _, r, _, _ in old)
    out = []
    for f in now:
        if f[1] == "size":
            if not was_over:          # only CROSSING the ceiling is new
                out.append(f)
            continue
        k = key(f[1], f[3])
        if seen[k] > 0:
            seen[k] -= 1
        else:
            out.append(f)
    return out


def lintable(path: str) -> bool:
    return path.endswith((".md", ".rs", ".loft")) and not path.startswith(("target/", "index/"))


def tracked(pattern_paths=None):
    r = subprocess.run(["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True)
    return [p for p in r.stdout.split("\n") if p and lintable(p)
            and (is_code(p) or p.endswith(".md") or is_user_prose(p))]


def changed(rev):
    r = subprocess.run(["git", "diff", "--name-only", "--diff-filter=AMR", rev],
                       cwd=ROOT, capture_output=True, text=True, check=True)
    return [p for p in r.stdout.split("\n") if p and lintable(p)
            and os.path.exists(os.path.join(ROOT, p))]


def hook() -> int:
    """The edit hook: read the tool call Claude Code sends on stdin, lint the file it
    wrote against HEAD, and hand back only what the edit added.  Silent when clean,
    and never blocks: a finding is context for the writer, not a refusal."""
    import json
    try:
        call = json.load(sys.stdin)
    except ValueError:
        return 0
    inp = call.get("tool_input") or {}
    path = inp.get("file_path") or inp.get("notebook_path") or ""
    if not path:
        return 0
    rel = os.path.relpath(os.path.abspath(path), ROOT)
    if rel.startswith("..") or not lintable(rel) or not os.path.exists(os.path.join(ROOT, rel)):
        return 0
    found = new_findings(rel, "HEAD")
    if not found:
        return 0
    lines = [f"{rel}:{n}: {rule}: {msg}" for n, rule, msg, _ in found]
    note = ("doc_lint (doc/claude/DOC_CONTRACT.md) — this edit added:\n" + "\n".join(lines))
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PostToolUse",
                                             "additionalContext": note}}))
    return 0


def main(argv):
    if argv == ["--hook"]:
        return hook()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("paths", nargs="*")
    ap.add_argument("--since", metavar="REV", help="report only findings REV's version lacks")
    ap.add_argument("--changed", action="store_true", help="lint the files changed since --since")
    ap.add_argument("--gate", action="store_true", help="exit 1 when a gated finding is new")
    ap.add_argument("--all", action="store_true", help="every tracked file in scope")
    ap.add_argument("--baseline", metavar="FILE", help="with --all: print the delta against FILE")
    ap.add_argument("--write-baseline", metavar="FILE", help="with --all: write FILE")
    a = ap.parse_args(argv)

    if a.changed and not a.since:
        ap.error("--changed needs --since")
    paths = [os.path.relpath(os.path.abspath(p), ROOT) for p in a.paths]
    if a.changed:
        paths += changed(a.since)
    if a.all:
        paths = tracked()
    paths = [p for p in dict.fromkeys(paths) if lintable(p) and os.path.exists(os.path.join(ROOT, p))]

    findings = []
    for p in paths:
        if a.since:
            fs = new_findings(p, a.since)
        else:
            fs = lint_text(p, open(os.path.join(ROOT, p), encoding="utf-8", errors="replace").read())
        findings += [(p, *f) for f in fs]

    if a.all:
        return report(findings, a.baseline, a.write_baseline)
    for p, n, rule, msg, _ in findings:
        print(f"{p}:{n}: {rule}: {msg}")
    if a.gate and any(r in GATED for _, _, r, _, _ in findings):
        print("\ndoc_lint: this change adds a gated finding (stamp, history, two-h1, or a file "
              "crossing the size ceiling).\nThe rule set is doc/claude/DOC_CONTRACT.md.", file=sys.stderr)
        return 1
    return 0


def counts(findings):
    """Findings per (file, rule) — what the baseline pins.  The report needs the delta and
    the worklist, and both are per file, so the baseline carries no line text."""
    return collections.Counter((p, r) for p, _, r, _, _ in findings)


def read_baseline(path):
    out = collections.Counter()
    for line in open(os.path.join(ROOT, path), encoding="utf-8"):
        parts = line.rstrip("\n").split("\t")
        if len(parts) == 3 and parts[2].isdigit():
            out[(parts[0], parts[1])] = int(parts[2])
    return out


def report(findings, baseline, write):
    now = counts(findings)
    if write:
        with open(os.path.join(ROOT, write), "w", encoding="utf-8") as f:
            f.write("# path\trule\tcount — pinned by `make docs-lint-baseline`; read by `make docs-lint`\n")
            for (p, r), c in sorted(now.items()):
                f.write(f"{p}\t{r}\t{c}\n")
        print(f"doc_lint: {sum(now.values())} findings in {len(now)} (file, rule) rows pinned in {write}")
        return 0
    by_rule = collections.Counter(r for _, _, r, _, _ in findings)
    print(f"== docs-lint: {len(findings)} findings ==")
    for r, c in sorted(by_rule.items()):
        print(f"   {r:<10} {c}")
    if baseline and os.path.exists(os.path.join(ROOT, baseline)):
        old = read_baseline(baseline)
        grew = {k: now[k] - old.get(k, 0) for k in now if now[k] > old.get(k, 0)}
        fell = sum(max(0, c - now.get(k, 0)) for k, c in old.items())
        print(f"\n   since the baseline: {sum(grew.values())} new, {fell} fixed "
              f"({sum(old.values())} → {sum(now.values())})")
        for (p, r), d in sorted(grew.items(), key=lambda kv: -kv[1])[:10]:
            print(f"      +{d:<4} {r:<10} {p}")
    print("\n== worklist: files by rule, largest first ==")
    per = collections.Counter((r, p) for p, _, r, _, _ in findings)
    sizes = {p: n for p, n, r, _, _ in findings if r == "size"}
    for rule in ("size", "history", "two-h1", "orphan", "stamp", "hedge", "timeline", "narration", "link"):
        rows = sorted(((sizes[p] if rule == "size" else c, p) for (r, p), c in per.items()
                       if r == rule), reverse=True)[:8]
        if rows:
            print(f"-- {rule}")
            for c, p in rows:
                print(f"   {c:>5}  {p}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
