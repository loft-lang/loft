#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# tools/indexer/fix_broken_links.py — repair broken relative links in the
# tracked markdown.
#
#   fix_broken_links.py            dry run: print each repair and each flag
#   fix_broken_links.py --apply    write the repairs
#
# A link is broken when its target, resolved against the file's directory,
# does not exist.  It is REPAIRED only when the answer is unique and
# complete: exactly one tracked path (a file, or a directory that holds
# one) ends in EVERY segment the link names, so the link names the thing
# exactly and only its position relative to the file is wrong — a moved
# plan (`../45-x/` now `../finished/45-x/`), an off-by-one `../`, a doc
# that moved between trees.  A link whose named segments match only in
# part (`lib/server/src/server.loft` after the library left the tree)
# points at something that is gone, and a path several files end in is
# ambiguous; both are FLAGGED for a human, with the reason.  The anchor,
# and a trailing `/`, are kept as written.
#
# Not links, the same as for `scripts/linkcheck.sh` and the index scanner:
# text in a fenced block or an inline code span, a `<placeholder>` or
# `{placeholder}` target, a line marked `<!--noindex-->`, anything under
# `tests/fixtures/` (fixtures hold dead links on purpose), and a link to the
# generated, git-ignored `doc/claude/LIBRARIES.md`.
#
# Exit status: 0 when nothing is left flagged, 1 otherwise.  `make doc-fix`
# is the wrapper: it runs this with --apply and then the drift checker.
# A directory move is `make plan-move`, which computes its rewrites from
# the rename instead of repairing them afterwards.

from __future__ import annotations

import argparse
import os
import re
import sys

from rewrite_links import (
    FENCE, INLINE_LINK, REF_DEF, REPO_ROOT, is_rewritable, rel,
    split_target, tracked_files,
)

# Link targets that are placeholders by design (the plan template).
SKIP_TARGETS = {"FOO.md"}
REF_DEF_M = re.compile(REF_DEF.pattern, re.MULTILINE)


def code_spans(text: str) -> list[tuple[int, int]]:
    """[start, end) of each inline code span, CommonMark's way: a run of
    n backticks opens one and the next run of EXACTLY n closes it."""
    runs = [(m.start(), m.end()) for m in re.finditer(r"`+", text)]
    spans, k = [], 0
    while k < len(runs):
        a, b = runs[k]
        for m in range(k + 1, len(runs)):
            if runs[m][1] - runs[m][0] == b - a:
                spans.append((a, runs[m][1]))
                k = m + 1
                break
        else:
            k += 1
    return spans


def all_paths(files: list[str]) -> set[str]:
    out = set(files)
    for f in files:
        d = os.path.dirname(f)
        while d:
            out.add(d)
            d = os.path.dirname(d)
    return out


def candidates(resolved: str, paths: set[str]) -> tuple[list[str], int]:
    """The paths sharing the longest trailing-segment tail with
    `resolved`, and that tail's length."""
    want = [s for s in resolved.split("/") if s not in ("", ".", "..")]
    best: list[str] = []
    best_len = 0
    for p in paths:
        segs = p.split("/")
        n = 0
        while n < len(want) and n < len(segs) and want[-1 - n] == segs[-1 - n]:
            n += 1
        if n == 0:
            continue
        if n > best_len:
            best, best_len = [p], n
        elif n == best_len:
            best.append(p)
    return best, best_len


def repair(raw: str, src: str, paths: set[str]) -> tuple[str | None, str]:
    """(new spelling or None, reason when None)."""
    path_part, suffix = split_target(raw)
    if not is_rewritable(path_part) or path_part in SKIP_TARGETS:
        return None, ""
    if path_part[0] in "<{" or path_part.endswith("doc/claude/LIBRARIES.md") \
            or path_part.endswith("claude/LIBRARIES.md") or path_part == "LIBRARIES.md":
        return None, ""
    src_dir = os.path.dirname(src)
    resolved = os.path.normpath(os.path.join(src_dir, path_part))
    if (REPO_ROOT / resolved).exists():
        return None, ""
    named = [x for x in path_part.split("/") if x not in ("", ".", "..")]
    found, n = candidates(resolved, paths)
    if not found:
        return None, "no tracked path ends in its last segment"
    if n < len(named):
        return None, f"only {n} of its {len(named)} segments match a tracked path"
    if len(found) > 1:
        return None, f"{len(found)} paths end in all its segments"
    return rel(src_dir, found[0], path_part.endswith("/")) + suffix, ""


def scan(path: str, text: str, paths: set[str]) -> tuple[str, list]:
    """The repaired text of one markdown file, and a finding per broken link:
    `(line, target, repaired target or None, reason)`.  `doc_lint.py` asks this
    too, so the two tools cannot disagree about what a link is."""
    lines = text.split("\n")
    found: list = []
    # Code spans can wrap, so they are found per paragraph: a run of
    # non-blank lines outside a fence.
    para: list[int] = []
    in_fence = False

    def flush() -> None:
        if not para:
            return
        block = "\n".join(lines[k] for k in para)
        spans = code_spans(block)
        starts = []
        pos = 0
        for k in para:
            starts.append(pos)
            pos += len(lines[k]) + 1

        def line_of(off: int) -> int:
            idx = 0
            while idx + 1 < len(starts) and starts[idx + 1] <= off:
                idx += 1
            return para[idx] + 1

        def sub(m):
            if any(a <= m.start() < b for a, b in spans):
                return m.group(0)
            n = line_of(m.start())
            if "<!--noindex-->" in lines[n - 1]:
                return m.group(0)
            new, why = repair(m.group(2), path, paths)
            if new is not None:
                found.append((n, m.group(2), new, ""))
                return m.group(1) + new
            if why:
                found.append((n, m.group(2), None, why))
            return m.group(0)

        block = INLINE_LINK.sub(sub, block)
        block = REF_DEF_M.sub(sub, block)
        for k, text_line in zip(para, block.split("\n")):
            lines[k] = text_line
        para.clear()

    for k, line in enumerate(lines):
        if FENCE.match(line):
            flush()
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        if line.strip() == "":
            flush()
            continue
        para.append(k)
    flush()
    return "\n".join(lines), found


def in_scope(path: str) -> bool:
    """Markdown this tool checks: every tracked `.md` but the generated index and the
    fixtures, which hold dead links on purpose."""
    return path.endswith(".md") and not path.startswith(("index/", "tests/fixtures/"))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--apply", action="store_true")
    a = ap.parse_args()

    files = tracked_files()
    paths = all_paths(files)
    fixed = flagged = 0
    for f in sorted(files):
        if not in_scope(f):
            continue
        p = REPO_ROOT / f
        if not p.is_file():
            continue
        text = p.read_text(encoding="utf-8")
        new_text, found = scan(f, text, paths)
        for n, target, new, why in found:
            if new is not None:
                print(f"  fix   {f}:{n}: {target} -> {new}")
                fixed += 1
            else:
                print(f"  flag  {f}:{n}: {target} ({why})")
                flagged += 1
        if a.apply and new_text != text:
            p.write_text(new_text, encoding="utf-8")

    verb = "fixed" if a.apply else "fixable (dry run; --apply to write)"
    print(f"{verb}: {fixed} / flagged: {flagged}")
    return 1 if flagged else 0


if __name__ == "__main__":
    sys.exit(main())
