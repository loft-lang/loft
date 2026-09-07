#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which agent skills owe a human read — and which have MOVED under their sources.

The skills in `.claude/skills/` are the standing instructions an agent loads before
working on loft, and loft is in heavy development: the docs and scripts a skill
paraphrases move daily, while the skill itself is only edited when someone notices.
A skill that quotes last month's behaviour steers every future session wrong, in the
one channel that is loaded *instead of* re-reading the canonical doc.

Whether a skill is still TRUE, still USABLE, and still CONCISE is a person's judgement
(the three axes are defined in `doc/claude/SKILLS_REVIEW.md`).  What a script can do:

  * derive each skill's SOURCES — the repo docs, scripts and make targets its text
    cites — and put a skill back on the worklist when any of them moves past the
    commit it was last read through (the same watermark idea `reference-review.py`
    and LIBRARY_DOC_REVIEW.md use);
  * check the mechanical half outright: every cited path exists, every cited
    `make <target>` is a real target, every cited `LOFT_*` switch still appears in
    the tree.  A broken reference needs no judgement — it is stale by construction.

Sources are DOCS AND SCRIPTS only (doc/, scripts/, Makefile, default/, .github/):
those are what a skill paraphrases, so their movement is the review trigger.  A cited
`src/` or `tests/` path is existence-checked but does not re-open the review — a
skill documents the method, not the code, and tying it to source churn would put
`loft-codegen` back on the list weekly for commits that change nothing it says.

    scripts/skills-review.py               # the worklist
    scripts/skills-review.py --verbose     # + the commits behind each MOVED skill
    scripts/skills-review.py --done loft-test   # record a skill as validated

The watermark table lives in `doc/claude/SKILLS_REVIEW.md` and is edited by
`--done` (or by hand) when a skill is read.  It is the ONE home for "reviewed
through"; a second machine-readable copy would drift the moment someone updated
only the prose.
"""

from __future__ import annotations

import argparse
import datetime
import os
import pathlib
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DOC = os.path.join(ROOT, "doc", "claude", "SKILLS_REVIEW.md")
SKILLS_DIR = os.path.join(ROOT, ".claude", "skills")

# Reference-derivation: which cited prefixes are (a) checked for existence and
# (b) tracked as review-(re)opening sources.  Keys are path prefixes relative to ROOT.
TRACKED_PREFIXES = ("doc/", "scripts/", "default/", ".github/", ".claude/skills/")
CHECKED_PREFIXES = TRACKED_PREFIXES + ("src/", "tests/", "editors/", "index/")


def sh(*args: str) -> tuple[int, str]:
    try:
        p = subprocess.run(args, cwd=ROOT, capture_output=True, text=True, timeout=60)
        return p.returncode, (p.stdout + p.stderr).strip()
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return 1, ""


def skills() -> list[str]:
    """Every directory under .claude/skills/ that carries a SKILL.md.

    Derived, never listed: a skill added tomorrow appears here without anyone
    maintaining a second list.
    """
    if not os.path.isdir(SKILLS_DIR):
        return []
    return sorted(
        d
        for d in os.listdir(SKILLS_DIR)
        if os.path.isfile(os.path.join(SKILLS_DIR, d, "SKILL.md"))
    )


def skill_text(name: str) -> str:
    text = []
    base = os.path.join(SKILLS_DIR, name)
    for dirpath, _dirs, files in os.walk(base):
        for f in files:
            if f.endswith(".md"):
                p = os.path.join(dirpath, f)
                text.append(pathlib.Path(p).read_text(encoding="utf-8", errors="replace"))
    return "\n".join(text)


# A path-like token: at least one `/`, made of ordinary path characters, ending in a
# word character or a known extension.  Trailing punctuation from prose (`.`, `,`, `)`)
# is stripped before the existence test.
PATH_RE = re.compile(r"(?<![\w./-])((?:doc|scripts|src|tests|default|editors|index|\.github|\.claude)/[\w./+-]*\w)")
MAKE_RE = re.compile(r"\bmake ([a-z][a-z0-9_-]+)")
ENV_RE = re.compile(r"\b(LOFT_[A-Z][A-Z0-9_]+)\b")

# A path segment that is a stand-in, not a file: `src/my_lib.loft`, `tests/scripts/NN`.
PLACEHOLDER_SEG = re.compile(r"^(NN+|x{1,2}|foo|bar|my_[a-z_]*)(\.[a-z]+)?$")

# Files a skill may legitimately cite that are BUILT on demand and git-ignored by
# design, so their absence from a checkout says nothing (@PLN112 for LIBRARIES.md).
KNOWN_GENERATED = {"doc/claude/LIBRARIES.md"}


def code_regions(text: str) -> str:
    """Fenced blocks plus inline backtick spans — the only places a `make <target>`
    citation is a command rather than the English verb ("make sure", "make each…")."""
    fenced = re.findall(r"^```.*?^```", text, re.M | re.S)
    inline = re.findall(r"`[^`\n]+`", text)
    return "\n".join(fenced + inline)


def make_targets() -> set[str]:
    text = pathlib.Path(os.path.join(ROOT, "Makefile")).read_text(encoding="utf-8")
    return set(re.findall(r"^([a-z][a-z0-9_-]+):", text, re.M))


def cited_paths(text: str) -> set[str]:
    out = set()
    for m in PATH_RE.findall(text):
        p = m.rstrip(".,;:)]}'\"")
        # A glob or a placeholder is an idiom, not a reference to one file.
        if any(c in p for c in "*<>{}$"):
            p = p.split("*")[0].rstrip("/")
            if not p or "/" not in p:
                continue
        if any(PLACEHOLDER_SEG.match(seg) for seg in p.split("/")):
            continue
        if p.startswith(CHECKED_PREFIXES):
            out.add(p)
    return out


def env_vars_in_tree() -> set[str]:
    """Every LOFT_* name that appears anywhere in src/, scripts/ or the Makefile."""
    code, out = sh(
        "git", "grep", "-oh", r"LOFT_[A-Z][A-Z0-9_]*", "--", "src", "scripts", "Makefile", "doc/claude"
    )
    return set(out.split()) if code == 0 else set()


def check_references(name: str, targets: set[str], tree_env: set[str]):
    """(broken, tracked_sources) for one skill.

    `broken` is the mechanical verdict: cited things that do not resolve in THIS tree.
    `tracked_sources` are the doc/script paths whose movement re-opens the review.
    """
    text = skill_text(name)
    broken: list[str] = []
    tracked: set[str] = set()
    for p in sorted(cited_paths(text)):
        if p in KNOWN_GENERATED:
            continue
        if not os.path.exists(os.path.join(ROOT, p)):
            broken.append(f"path does not exist: {p}")
            continue
        if p.startswith(TRACKED_PREFIXES):
            tracked.add(p)
    for t in sorted(set(MAKE_RE.findall(code_regions(text)))):
        if t not in targets:
            broken.append(f"make target does not exist: make {t}")
    for v in sorted(set(ENV_RE.findall(text))):
        if v not in tree_env:
            broken.append(f"env switch not found in tree: {v}")
    return broken, tracked


def watermarks() -> dict[str, tuple[str, str]]:
    """The table in SKILLS_REVIEW.md, as data: skill -> (reviewed, commit)."""
    marks: dict[str, tuple[str, str]] = {}
    if not os.path.isfile(DOC):
        return marks
    in_table = False
    with open(DOC, encoding="utf-8") as f:
        for line in f:
            if re.match(r"^\| *skill *\| *reviewed through *\|", line):
                in_table = True
                continue
            if in_table:
                if re.match(r"^\|[ :\-]*-[ :\-]*\|", line):
                    continue
                if not line.startswith("|"):
                    in_table = False
                    continue
                cells = [c.strip().strip("`") for c in line.split("|")[1:-1]]
                if len(cells) >= 3 and cells[0]:
                    marks[cells[0]] = (cells[1], cells[2])
    return marks


def review_paths(name: str, tracked: set[str]) -> list[str]:
    """The skill's own directory plus every tracked source it cites."""
    return [f".claude/skills/{name}"] + sorted(tracked)


def source_commit(paths: list[str]) -> str:
    """The last commit that touched the skill or anything it paraphrases.

    Recorded as the watermark rather than `HEAD`: it names the change the reviewer
    actually read, and re-marking a skill nobody has touched writes the same value,
    so the table only changes when the answer does.
    """
    code, out = sh("git", "log", "-1", "--format=%h", "--", *paths)
    return out.strip() if code == 0 and out.strip() else "HEAD"


def moved_since(commit: str, paths: list[str]) -> list[str]:
    code, out = sh("git", "log", "--oneline", f"{commit}..HEAD", "--", *paths)
    if code != 0:
        return ["(cannot resolve that commit — check the watermark)"]
    return [l for l in out.splitlines() if l.strip()]


def write_watermark(name: str, commit: str | None) -> str:
    """Add, replace or remove one row of the table in SKILLS_REVIEW.md."""
    text = pathlib.Path(DOC).read_text(encoding="utf-8")
    lines = text.splitlines()
    head = next((i for i, l in enumerate(lines) if re.match(r"^\| *skill *\|", l)), None)
    if head is None:
        sys.exit(f"{DOC}: no watermark table to write to")
    start = head + 2  # header + separator
    end = start
    while end < len(lines) and lines[end].startswith("|"):
        end += 1
    rows = {}
    for line in lines[start:end]:
        cells = [c.strip().strip("`") for c in line.split("|")[1:-1]]
        if len(cells) >= 3 and cells[0]:
            rows[cells[0]] = (cells[1], cells[2])
    if commit is None:
        if name not in rows:
            return f"{name} had no row"
        rows.pop(name)
        verdict = f"removed the watermark for {name}"
    else:
        today = datetime.date.today().isoformat()
        rows[name] = (today, commit)
        verdict = f"{name} — validated at {commit} ({today})"
    body = [f"| `{k}` | {v[0]} | `{v[1]}` |" for k, v in sorted(rows.items())]
    pathlib.Path(DOC).write_text(
        "\n".join(lines[:start] + body + lines[end:]) + "\n", encoding="utf-8"
    )
    return verdict


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--verbose", action="store_true", help="list the commits behind each MOVED skill")
    ap.add_argument(
        "--done",
        metavar="SKILL",
        action="append",
        help="record a skill as validated at its sources' current commit (repeatable)",
    )
    ap.add_argument("--undo", metavar="SKILL", help="remove a skill's watermark row")
    args = ap.parse_args()

    pop = skills()
    targets = make_targets()
    tree_env = env_vars_in_tree()
    refs = {name: check_references(name, targets, tree_env) for name in pop}

    for target in args.done or []:
        if target not in pop:
            print(f"not a skill: {target}", file=sys.stderr)
            print("  run with no arguments to see the list", file=sys.stderr)
            return 2
        print(write_watermark(target, source_commit(review_paths(target, refs[target][1]))))
    if args.undo:
        print(write_watermark(args.undo, None))

    marks = watermarks()
    never, moved, current, broken_any = [], [], [], []
    for name in pop:
        broken, tracked = refs[name]
        if broken:
            broken_any.append((name, broken))
        mark = marks.get(name)
        if mark is None:
            never.append(name)
            continue
        commits = moved_since(mark[1], review_paths(name, tracked))
        if commits:
            moved.append((name, mark, commits))
        else:
            current.append(name)

    stale_rows = [k for k in marks if k not in pop]

    print(f"Skills review — {len(pop)} skills\n")
    if broken_any:
        n = sum(len(b) for _, b in broken_any)
        print(f"BROKEN REFERENCES ({n}) — stale by construction, no judgement needed:")
        for name, broken in broken_any:
            for b in broken:
                print(f"  {name:<22} {b}")
        print()
    if never:
        print(f"NEVER REVIEWED ({len(never)}) — no watermark row:")
        for name in never:
            print(f"  {name}")
        print()
    if moved:
        print(f"MOVED since its watermark ({len(moved)}) — owes a re-read:")
        for name, mark, commits in moved:
            print(f"  {name:<22} reviewed through {mark[1]} ({mark[0]}) — {len(commits)} commit(s) since")
            if args.verbose:
                for c in commits:
                    print(f"        {c}")
        print()
    if stale_rows:
        print(f"STALE ROWS ({len(stale_rows)}) — a watermark for something that is not a skill:")
        for k in stale_rows:
            print(f"  {k}")
        print()
    print(f"{len(current)}/{len(pop)} skills reviewed at their current sources.")
    if never or moved or broken_any:
        print(
            f"\nRead each of the above on the three axes in doc/claude/SKILLS_REVIEW.md "
            f"(content / usability / conciseness), fix what the read finds, then record it: "
            f"scripts/skills-review.py --done <skill>."
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
