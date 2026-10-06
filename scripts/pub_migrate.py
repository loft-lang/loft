#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN187 step 4 — add `pub` exactly where `scripts/pub_census.sh` says it is needed.

Reads census rows (`tree  declared-in:line  kind  item  uses`) for one tree and edits the
DECLARING files under `--root`:

  field / literal-field / pattern-field / key   `pub` on that field, found inside the braces of
                                                the struct or variant declared at that line;
  literal                                      `pub` on the struct, and on EVERY field of it —
                                                a literal outside the file needs them all (C139);
  variant                                      `pub` on the enum.

Paths outside `--root` (an installed registry copy, a sibling library) are reported and left
alone: a library is migrated in its own repo (@PLN187 step 5).  Idempotent: an item already
`pub` is left as it is.  `--dry-run` prints the edits instead of writing them.

Usage:  scripts/pub_migrate.py <census-file> --tree loft --root . [--only PATH-PREFIX] [--dry-run]
"""
import argparse
import os
import re
import sys
from collections import defaultdict

FIELD_KINDS = {"field", "literal-field", "pattern-field", "key"}


def body_span(text, start):
    """(open, close) offsets of the first `{ … }` at or after `start`, matching nested braces;
    `#` comments and string literals are skipped."""
    i = text.find("{", start)
    if i < 0:
        return None
    depth, j, in_str = 0, i, False
    while j < len(text):
        c = text[j]
        if in_str:
            if c == "\\":
                j += 2
                continue
            if c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == "/" and text.startswith("//", j):
            j = text.find("\n", j)
            if j < 0:
                return None
            continue
        elif c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i, j
        j += 1
    return None


def field_offsets(text, span):
    """The offset where each top-level field of the body in `span` starts (its `pub`, `const`
    or name), by name."""
    open_, close = span
    out = {}
    depth, j, item_start = 0, open_ + 1, open_ + 1
    in_str = False
    while j < close:
        c = text[j]
        if in_str:
            if c == "\\":
                j += 2
                continue
            if c == '"':
                in_str = False
            j += 1
            continue
        if c == '"':
            in_str = True
        elif c == "/" and text.startswith("//", j):
            nl = text.find("\n", j)
            j = close if nl < 0 else nl
            # a comment line separates nothing: the next item starts after it
            if depth == 0 and item_start >= close:
                pass
            elif depth == 0 and text[item_start:j].strip().startswith("//"):
                item_start = j + 1
            continue
        if c in "{([<":
            depth += 1
        elif c in "})]" or (c == ">" and text[j - 1] != "-"):
            depth -= 1
        elif c == "," and depth == 0:
            item_start = j + 1
        elif c == ":" and depth == 0:
            head = re.sub(r"//[^\n]*", "", text[item_start:j])
            m = re.search(r"(?:pub\s+)?(?:const\s+)?([A-Za-z_]\w*)\s*$", head)
            if m:
                # the name's offset in the ORIGINAL text: the last match of the field head
                # before the `:`, comments excluded
                raw = text[item_start:j]
                hits = [h for h in re.finditer(r"(?:pub\s+)?(?:const\s+)?" + re.escape(m.group(1)) + r"\s*$", raw)]
                start = item_start + (hits[-1].start() if hits else m.start())
                out.setdefault(m.group(1), start)
            # skip to the end of this field's type/default
            item_start = close
        j += 1
    return out


def line_offset(text, line):
    off = 0
    for _ in range(line - 1):
        nl = text.find("\n", off)
        if nl < 0:
            return len(text)
        off = nl + 1
    return off


def make_pub_at(text, off, edits):
    """Record `pub ` at `off` unless the text there already starts with `pub`."""
    if not text.startswith("pub", off) or not re.match(r"pub\s", text[off:off + 4]):
        edits.add(off)


def migrate_file(path, rows, dry_run):
    text = open(path, encoding="utf-8").read()
    edits = set()
    for line, kind, item in rows:
        start = line_offset(text, line)
        if kind in FIELD_KINDS:
            field = item.rsplit(".", 1)[-1]
            span = body_span(text, start)
            off = span and field_offsets(text, span).get(field)
            if off is None:
                print(f"pub_migrate: {path}:{line}: no field `{field}` found", file=sys.stderr)
                continue
            make_pub_at(text, off, edits)
        elif kind == "literal":
            m = re.compile(r"(?:pub\s+)?(?:value\s+)?struct\b").search(text, start)
            if not m:
                print(f"pub_migrate: {path}:{line}: no struct for `{item}`", file=sys.stderr)
                continue
            make_pub_at(text, m.start(), edits)
            span = body_span(text, m.end())
            for off in (field_offsets(text, span).values() if span else []):
                make_pub_at(text, off, edits)
        elif kind == "variant":
            m = re.compile(r"(?:pub\s+)?enum\b").search(text, start)
            if not m:
                print(f"pub_migrate: {path}:{line}: no enum for `{item}`", file=sys.stderr)
                continue
            make_pub_at(text, m.start(), edits)
    if not edits:
        return 0
    new = text
    for off in sorted(edits, reverse=True):
        new = new[:off] + "pub " + new[off:]
    if dry_run:
        print(f"{path}: {len(edits)} `pub` added")
    else:
        open(path, "w", encoding="utf-8").write(new)
    return len(edits)


REGISTRY_RE = re.compile(r"/\.loft/registry/([A-Za-z0-9_]+)-[0-9][^/]*/(.*)$")


def package_dirs(root):
    """Every package (a directory with a `loft.toml`) under `root`, by its directory name."""
    out = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if not d.startswith(".") and d != "target"]
        if "loft.toml" in filenames:
            out.setdefault(os.path.basename(dirpath), dirpath)
    return out


def declaration_line(path, kind, item):
    """The line of the declaration a row's edit starts from, found by NAME: the struct or the
    variant for a field, the struct for a literal, the enum holding the variant for a variant."""
    try:
        lines = open(path, encoding="utf-8").read().split("\n")
    except OSError:
        return None
    name = item.split(".")[0]
    decl = re.compile(r"(?:\bstruct\s+" + re.escape(name) + r"\b|(?:^|[{,])\s*" + re.escape(name) + r"\s*\{)")
    for i, l in enumerate(lines):
        if decl.search(l.split("//")[0]):
            if kind == "variant":
                for j in range(i, -1, -1):
                    if re.search(r"\benum\b", lines[j].split("//")[0]):
                        return j + 1
                return None
            return i + 1
    if kind == "variant":
        word = re.compile(r"\b" + re.escape(name) + r"\b")
        for i, l in enumerate(lines):
            if word.search(l.split("//")[0]):
                for j in range(i, -1, -1):
                    if re.search(r"\benum\b", lines[j].split("//")[0]):
                        return j + 1
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("census")
    ap.add_argument("--tree", default="loft")
    ap.add_argument("--root", default=".")
    ap.add_argument("--only", default="")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--registry", default="",
                    help="map a row declared in an installed registry copy onto this checkout's "
                         "package of the same name (any tree), finding the line by type name")
    a = ap.parse_args()
    root = os.path.realpath(a.root)
    per_file = defaultdict(set)
    skipped = set()
    unmapped = set()
    packages = package_dirs(a.registry) if a.registry else {}
    for raw in open(a.census, encoding="utf-8"):
        if raw.startswith("#") or not raw.strip():
            continue
        tree, decl, kind, item, _uses = raw.rstrip("\n").split("\t")
        path, _, line = decl.rpartition(":")
        reg = REGISTRY_RE.search(path) if a.registry else None
        if reg:
            pkg_dir = packages.get(reg.group(1))
            mapped = os.path.join(pkg_dir, reg.group(2)) if pkg_dir else None
            at = mapped and declaration_line(mapped, kind, item)
            if not at:
                unmapped.add(f"{reg.group(1)}: {item}")
                continue
            per_file[os.path.realpath(mapped)].add((at, kind, item))
            continue
        if tree != a.tree:
            continue
        full = os.path.realpath(path if os.path.isabs(path) else os.path.join(root, path))
        if not full.startswith(root + os.sep) or not os.path.exists(full) or (a.only and not os.path.relpath(full, root).startswith(a.only)):
            skipped.add(full)
            continue
        per_file[full].add((int(line), kind, item))
    total = sum(migrate_file(f, sorted(r), a.dry_run) for f, r in sorted(per_file.items()))
    print(f"pub_migrate: {total} `pub` added in {len(per_file)} files; {len(skipped)} declaring files outside --root left alone")
    for s in sorted(skipped):
        print(f"  outside: {s}")
    for u in sorted(unmapped):
        print(f"  registry row with no declaration found here: {u}")


if __name__ == "__main__":
    main()
