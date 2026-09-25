#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# tools/indexer/rewrite_links.py — move a file or directory and rewrite
# every markdown link the move would break.
#
#   rewrite_links.py <from> <to>            dry run: print every rewrite
#   rewrite_links.py <from> <to> --apply    `git mv`, then write the rewrites
#
# The rewrites follow from the rename, so nothing is guessed:
#   - an INCOMING link, anywhere in the tree, that resolves into <from>
#     is re-pointed at the same place under <to>;
#   - an OUTGOING link inside a moved file keeps its target and gets the
#     relative path that target has from the file's new directory;
#   - a repo-rooted path that spells <from> in any tracked text file
#     (`doc/claude/plans/42-x/README.md` in a code comment, a Makefile, a
#     script) is re-spelled with <to>.
# Links inside fenced code blocks are examples, not links, and are left as
# written — the same rule the index scanner applies.  A URL, an absolute
# path and a bare `#anchor` are never touched.
#
# `make plan-move FROM=… TO=…` is the wrapper: it runs this with --apply,
# rebuilds the index and runs the drift checker's link check.

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent

# `[text](target)` and `![alt](target "title")`; the target stops at
# whitespace or the closing paren.
INLINE_LINK = re.compile(r"(\]\()([^)\s]+)")
# `[label]: target` reference definitions.
REF_DEF = re.compile(r"^(\s{0,3}\[[^\]]+\]:\s*)(\S+)")
FENCE = re.compile(r"^\s*(```|~~~)")
TEXT_SUFFIXES = {
    ".md", ".rs", ".loft", ".py", ".sh", ".toml", ".yml", ".yaml",
    ".json", ".txt", ".html", ".css", ".js",
}
TEXT_NAMES = {"Makefile", "CLAUDE.md"}


def tracked_files() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z"], cwd=REPO_ROOT, check=True,
        capture_output=True,
    ).stdout.decode()
    return [f for f in out.split("\0") if f]


def is_text(path: str) -> bool:
    if path.startswith("index/"):
        return False                       # generated; `make index` rebuilds it
    p = Path(path)
    return p.suffix in TEXT_SUFFIXES or p.name in TEXT_NAMES


def under(path: str, root: str) -> bool:
    return path == root or path.startswith(root + "/")


def moved(path: str, frm: str, to: str) -> str:
    """Where `path` lives after `frm` → `to` (unchanged when outside)."""
    if under(path, frm):
        return to + path[len(frm):]
    return path


def split_target(target: str) -> tuple[str, str]:
    """(`path`, `#anchor` or `?query` suffix)."""
    for sep in ("#", "?"):
        if sep in target:
            i = target.index(sep)
            return target[:i], target[i:]
    return target, ""


def is_rewritable(path_part: str) -> bool:
    if not path_part:
        return False                       # a bare `#anchor`
    if path_part.startswith("/"):
        return False                       # a site-absolute route
    if re.match(r"^[a-zA-Z][a-zA-Z0-9+.-]*:", path_part):
        return False                       # http:, mailto:, …
    return True


def rel(from_dir: str, target: str, trailing_slash: bool) -> str:
    r = os.path.relpath(target or ".", from_dir or ".")
    if trailing_slash and not r.endswith("/"):
        r += "/"
    return r


def rewrite_target(raw: str, src_old: str, src_new: str,
                   frm: str, to: str) -> str | None:
    """The new spelling of link `raw` written in a file that moves from
    `src_old` to `src_new`, or None when it stays as written."""
    path_part, suffix = split_target(raw)
    if not is_rewritable(path_part):
        return None
    old_dir = os.path.dirname(src_old)
    resolved = os.path.normpath(os.path.join(old_dir, path_part))
    if resolved.startswith(".."):
        return None                        # points outside the repo
    new_target = moved(resolved, frm, to)
    new_dir = os.path.dirname(src_new)
    if new_target == resolved and new_dir == old_dir:
        return None
    # A link that already resolves correctly from the new place needs
    # no edit (keeps `./x` and other author spellings intact).
    if os.path.normpath(os.path.join(new_dir, path_part)) == new_target:
        return None
    return rel(new_dir, new_target, path_part.endswith("/")) + suffix


def rewrite_markdown(text: str, src_old: str, src_new: str,
                     frm: str, to: str) -> tuple[str, list[str]]:
    out, notes, in_fence = [], [], False
    for n, line in enumerate(text.split("\n"), 1):
        if FENCE.match(line):
            in_fence = not in_fence
            out.append(line)
            continue
        if in_fence:
            out.append(line)
            continue

        def sub(m: re.Match) -> str:
            new = rewrite_target(m.group(2), src_old, src_new, frm, to)
            if new is None:
                return m.group(0)
            notes.append(f"{src_new}:{n}: {m.group(2)} -> {new}")
            return m.group(1) + new

        line = INLINE_LINK.sub(sub, line)
        line = REF_DEF.sub(sub, line)
        out.append(line)
    return "\n".join(out), notes


def rewrite_rooted(text: str, path: str, frm: str, to: str
                   ) -> tuple[str, list[str]]:
    """Re-spell repo-rooted mentions of `frm` (a path followed by `/`,
    a quote, whitespace, `)` or the end of the token)."""
    pat = re.compile(r"(?<![\w.-])" + re.escape(frm) + r"(?=[/\s\"'`)\]:,;#]|$)")
    notes = []
    lines = text.split("\n")
    for i, line in enumerate(lines):
        if pat.search(line):
            new = pat.sub(to, line)
            if new != line:
                notes.append(f"{path}:{i + 1}: {frm} -> {to}")
                lines[i] = new
    return "\n".join(lines), notes


def plan(frm: str, to: str) -> dict[str, tuple[str, list[str]]]:
    """{new path of each changed file: (new content, notes)}."""
    changes: dict[str, tuple[str, list[str]]] = {}
    for f in tracked_files():
        if not is_text(f):
            continue
        p = REPO_ROOT / f
        if not p.is_file():
            continue
        try:
            text = p.read_text()
        except UnicodeDecodeError:
            continue
        new_path = moved(f, frm, to)
        notes: list[str] = []
        new = text
        if f.endswith(".md"):
            new, n1 = rewrite_markdown(new, f, new_path, frm, to)
            notes += n1
        new, n2 = rewrite_rooted(new, new_path, frm, to)
        notes += n2
        if new != text or new_path != f:
            changes[new_path] = (new, notes)
    return changes


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("frm", metavar="from")
    ap.add_argument("to")
    ap.add_argument("--apply", action="store_true")
    a = ap.parse_args()
    frm = os.path.normpath(a.frm).rstrip("/")
    to = os.path.normpath(a.to).rstrip("/")
    if not (REPO_ROOT / frm).exists():
        print(f"rewrite_links: {frm} does not exist", file=sys.stderr)
        return 2
    if (REPO_ROOT / to).exists():
        print(f"rewrite_links: {to} already exists", file=sys.stderr)
        return 2
    if under(to, frm):
        print(f"rewrite_links: {to} is inside {frm}", file=sys.stderr)
        return 2

    changes = plan(frm, to)
    count = 0
    for path in sorted(changes):
        for note in changes[path][1]:
            print(note)
            count += 1
    if not a.apply:
        print(f"{count} rewrites (dry run; --apply to move and write)")
        return 0

    (REPO_ROOT / to).parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "mv", frm, to], cwd=REPO_ROOT, check=True)
    for path, (content, notes) in changes.items():
        if notes:
            (REPO_ROOT / path).write_text(content)
    print(f"moved {frm} -> {to}; {count} rewrites")
    return 0


if __name__ == "__main__":
    sys.exit(main())
