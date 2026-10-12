#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# decisions.py — the design-decision register, entry by entry: where each decision is
# cited, and the worklist line that says what keeps it and what it still owes.
#
# A decision is held to what it says (@PLN175): the refusal site, the code that implements
# it, the doc that states it and a guard under tests/ each cite `@C<n>`
# (DESIGN_DECISIONS_RULES.md § Using the register).  The index counts the citations; the
# worklist (doc/claude/DESIGN_DECISIONS_WORKLIST.tsv) is the JUDGMENT the counts cannot
# make — whether code implements the decision at all, where, whether its guard exercises
# it — and `check` holds that judgment to the tree, both ways: a line that claims work done
# that is not, and a line that still lists work the tree has since done, are both failures.
#
# Subcommands:
#   counts         every entry with its code / doc / guard citation counts, as JSON (what
#                  `./scripts/idx decisions` prints)
#   list           one row per entry: kind, citation counts, work left, note.  `--open` only
#                  the lines with work left; `--work <token>` / `--kind <kind>` filter
#   show C<n>      one entry: its line, every site citing it, and the bare `C<n>` mentions in
#                  code that do not count as a citation (the `cite` work, usually)
#   sync           add a `?` line for every entry the worklist lacks
#   check          the worklist against the register and the tree, and every entry stated by
#                  at least one working doc (not the register, a history, a changelog, a plan
#                  or a release record); exit 1 on any finding
#
# Reads index/tags.json (`make index` first — a stale index answers for the tree it was
# built from) and index/library_guards.json (`make guards-fetch`).

import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DOCS = os.path.join(ROOT, "doc", "claude")
WORKLIST = os.path.join(DOCS, "DESIGN_DECISIONS_WORKLIST.tsv")
INDEX = os.path.join(ROOT, "index", "tags.json")
LIBGUARDS = os.path.join(ROOT, "index", "library_guards.json")

COLUMNS = ["id", "kind", "site", "guard", "work", "note"]
KINDS = {"code", "decline", "library", "pending", "?"}
GUARDS = {"ok", "weak"}
WORK = {"cite", "locate", "guard", "entry", "miscite", "build", "classify"}
ENTRY = re.compile(r"^## (C[0-9]+(?:\.[A-Z])?) — (.*)$")


def id_key(cid):
    """`C54.D` sorts after C54 and before C55, as `sort -V` orders them."""
    m = re.match(r"C([0-9]+)(?:\.([A-Z]))?$", cid)
    return (int(m.group(1)), m.group(2) or "") if m else (1 << 30, cid)


def register():
    """Every entry of the register: id → (title, subject file)."""
    out = {}
    for name in sorted(os.listdir(DOCS)):
        if not (name.startswith("DESIGN_DECISIONS_") and name.endswith(".md")):
            continue
        with open(os.path.join(DOCS, name), encoding="utf-8") as f:
            for line in f:
                m = ENTRY.match(line.rstrip("\n"))
                if m:
                    out[m.group(1)] = (m.group(2), name)
    return out


def load_index():
    if not os.path.exists(INDEX):
        sys.exit(f"decisions: {os.path.relpath(INDEX, ROOT)} is missing — run `make index`")
    with open(INDEX, encoding="utf-8") as f:
        return json.load(f)


def library_tags():
    if not os.path.exists(LIBGUARDS):
        return []
    with open(LIBGUARDS, encoding="utf-8") as f:
        lib = json.load(f)
    return [c["tag"] for r in lib.get("repos", {}).values()
            for fl in r.get("files", []) for c in fl.get("cites", [])]


def kind_of(path):
    """A citation under tests/ is a guard, one in a .md file a doc, anything else code."""
    if path.startswith("tests/"):
        return "guards"
    if path.endswith(".md"):
        return "docs"
    return "code"


def states_it(path):
    """A WORKING doc — one that states a rule as it holds now.  The register itself, a
    `-history.md` companion, a changelog, a plan and a release record are not: a citation
    there records the decision, it does not state it to the reader of the subject."""
    return (path.endswith(".md") and not os.path.basename(path).startswith("DESIGN_DECISIONS")
            and not path.endswith("-history.md") and "CHANGELOG" not in path
            and "/plans/" not in path and "/releases/" not in path)


def counts(reg, index, lib):
    rows = []
    for cid in sorted(reg, key=id_key):
        refs = index.get("@" + cid, [])
        c = {"code": 0, "docs": 0, "guards": 0}
        for r in refs:
            c[kind_of(r["file"])] += 1
        library = sum(1 for t in lib if t == "@" + cid)
        rows.append({"id": cid, "title": reg[cid][0], "code": c["code"], "docs": c["docs"],
                     "guards": c["guards"] + library, "library": library})
    return rows


def read_worklist():
    """(header comment lines, column line, {id: row dict}, ids in file order, duplicates)."""
    head, cols, rows, order, dups = [], None, {}, [], []
    with open(WORKLIST, encoding="utf-8") as f:
        for raw in f:
            line = raw.rstrip("\n")
            if line.startswith("#") or not line.strip():
                if cols is None:
                    head.append(line)
                continue
            fields = line.split("\t")
            if cols is None:
                cols = line
                continue
            fields += [""] * (len(COLUMNS) - len(fields))
            row = dict(zip(COLUMNS, fields[:len(COLUMNS)]))
            row["_fields"] = len(line.split("\t"))
            if row["id"] in rows:
                dups.append(row["id"])
            rows[row["id"]] = row
            order.append(row["id"])
    return head, cols, rows, order, dups


def write_worklist(head, cols, rows):
    with open(WORKLIST, "w", encoding="utf-8") as f:
        for h in head:
            f.write(h + "\n")
        f.write(cols + "\n")
        for cid in sorted(rows, key=id_key):
            f.write("\t".join(rows[cid][c] for c in COLUMNS) + "\n")


def sites(row):
    """[(path, text-or-None)] of a line's `site` column; [] for `-` and `?`."""
    if row["site"] in ("-", "?", ""):
        return []
    out = []
    for item in row["site"].split("; "):
        path, _, text = item.partition("::")
        out.append((path.strip(), text or None))
    return out


def work(row):
    return set() if row["work"] in ("-", "") else {w.strip() for w in row["work"].split(",")}


def check(reg, index, lib):
    _, cols, rows, _, dups = read_worklist()
    found = []
    if cols != "\t".join(COLUMNS):
        found.append(f"the column line reads {cols!r}, not {chr(9).join(COLUMNS)!r}")
    for cid in dups:
        found.append(f"{cid}: two lines")
    for cid in sorted(set(reg) - set(rows), key=id_key):
        found.append(f"{cid}: no line — `python3 scripts/decisions.py sync` adds one; classify it")
    for cid in sorted(set(rows) - set(reg), key=id_key):
        found.append(f"{cid}: a line for an id the register does not define")
    by_id = {r["id"]: r for r in counts(reg, index, lib)}
    for cid in sorted(set(rows) & set(reg), key=id_key):
        row, n, w = rows[cid], by_id[cid], work(rows[cid])
        say = lambda msg: found.append(f"{cid}: {msg}")  # noqa: E731
        if row["_fields"] != len(COLUMNS):
            say(f"{row['_fields']} tab-separated fields, not {len(COLUMNS)}")
        if row["kind"] not in KINDS:
            say(f"kind `{row['kind']}` is not one of {sorted(KINDS)}")
        if row["guard"] not in GUARDS:
            say(f"guard `{row['guard']}` is not one of {sorted(GUARDS)}")
        if w - WORK:
            say(f"work {sorted(w - WORK)} not in the vocabulary {sorted(WORK)}")
        if row["kind"] == "?" or "classify" in w:
            say("not classified: set kind, site, guard and work")
            continue
        if n["guards"] == 0:
            say("no guard cites it (tests/ here or the library it is about)")
        if not any(states_it(r["file"]) for r in index.get("@" + cid, [])):
            say("no working doc states it: cite @" + cid + " where the subject's doc states the rule")
        if row["guard"] == "weak" and not w & {"guard", "build"}:
            say("its guard is `weak` but the work lists no `guard`")
        if row["kind"] == "pending" and "build" not in w:
            say("`pending` without `build` in the work")
        if row["kind"] != "pending" and "build" in w:
            say("`build` work on a decision that is not `pending`")
        if row["kind"] in ("decline", "library", "pending") and n["code"]:
            say(f"{n['code']} code citation(s) on a `{row['kind']}`: is code keeping it after all?")
        if row["kind"] != "code":
            continue
        if row["site"] in ("-", ""):
            say("a `code` decision names no site (`?` with `locate` when it is unknown)")
            continue
        if row["site"] == "?":
            if "locate" not in w:
                say("site `?` without `locate` in the work")
            continue
        if "locate" in w:
            say("`locate` work but the site is named")
        citing = {r["file"] for r in index.get("@" + cid, [])}
        uncited, moved = [], False
        for path, text in sites(row):
            full = os.path.join(ROOT, path)
            if not os.path.isfile(full):
                say(f"site `{path}` does not exist — the code moved; re-locate it")
                moved = True
                continue
            if text is not None:
                with open(full, encoding="utf-8", errors="replace") as f:
                    if text not in f.read():
                        say(f"site `{path}` no longer contains `{text}` — re-locate it")
            if path not in citing:
                uncited.append(path)
        if uncited and "cite" not in w:
            say(f"{', '.join(sorted(set(uncited)))} do(es) not cite @{cid}: cite it there or list `cite`")
        if not uncited and not moved and "cite" in w:
            say("every named site cites it now: drop `cite` from the work")
    return found


def bare_mentions(cid):
    """`C<n>` written in code without the `@`, which no index counts."""
    pat = r"(^|[^@A-Za-z0-9_.])" + re.escape(cid) + r"([^0-9.]|$)"
    try:
        out = subprocess.run(["git", "grep", "-n", "-E", pat, "--", "src", "default", "lib",
                              "scripts", "tools", ":!*.md", ":!*.tsv"],
                             cwd=ROOT, capture_output=True, text=True).stdout
    except OSError:
        return []
    return [line for line in out.splitlines() if line]


def short(text, width):
    return text if len(text) <= width else text[:width - 1] + "…"


def main(argv):
    if not argv or argv[0] in ("-h", "--help", "help"):
        with open(__file__, encoding="utf-8") as f:
            lines = f.read().splitlines()[4:]
        for line in lines:
            if not line.startswith("#"):
                break
            print(line[2:])
        return 0
    cmd, args = argv[0], argv[1:]
    reg = register()
    if cmd == "sync":
        head, cols, rows, _, _ = read_worklist()
        added = sorted(set(reg) - set(rows), key=id_key)
        for cid in added:
            rows[cid] = {"id": cid, "kind": "?", "site": "?", "guard": "ok", "work": "classify",
                         "note": reg[cid][0]}
        write_worklist(head, cols, rows)
        print("\n".join(f"added {cid}" for cid in added) or "every entry has a line")
        return 0
    index, lib = load_index(), library_tags()
    if cmd == "counts":
        print(json.dumps(counts(reg, index, lib), indent=2, ensure_ascii=False))
        return 0
    if cmd == "check":
        found = check(reg, index, lib)
        for f in found:
            print(f)
        return 1 if found else 0
    _, _, rows, _, _ = read_worklist()
    if cmd == "list":
        want_open = "--open" in args
        only_work = args[args.index("--work") + 1] if "--work" in args else None
        only_kind = args[args.index("--kind") + 1] if "--kind" in args else None
        print(f"{'id':<7}{'kind':<9}{'code':>5}{'guard':>6}  {'work':<20}note")
        for n in counts(reg, index, lib):
            row = rows.get(n["id"], {"kind": "-", "work": "(no line)", "note": "", "guard": ""})
            w = row["work"]
            if want_open and w in ("-", ""):
                continue
            if only_work and only_work not in work(row) | ({w} if w == "(no line)" else set()):
                continue
            if only_kind and row["kind"] != only_kind:
                continue
            gd = f"{n['guards']}{'w' if row['guard'] == 'weak' else ''}"
            print(f"{n['id']:<7}{row['kind']:<9}{n['code']:>5}{gd:>6}  {w:<20}"
                  f"{short(row['note'] or n['title'], 90)}")
        return 0
    if cmd == "show":
        if not args:
            sys.exit("decisions: show C<n>")
        cid = args[0].lstrip("@")
        if cid not in reg:
            sys.exit(f"decisions: {cid} is not an entry of the register")
        title, subject = reg[cid]
        print(f"{cid} — {title}   ({subject})")
        row = rows.get(cid)
        if row:
            for c in COLUMNS[1:]:
                print(f"  {c:<6} {row[c]}")
        else:
            print("  (no worklist line — `sync`)")
        print("  cited at:")
        for r in index.get("@" + cid, []):
            k = kind_of(r["file"])
            if k == "docs" and not states_it(r["file"]):
                k = "record"
            print(f"    {k:<6} {r['file']}:{r['line']}")
        for t in lib:
            if t == "@" + cid:
                print("    guards (a library's tests/, index/library_guards.json)")
                break
        bare = bare_mentions(cid)
        if bare:
            print("  bare mentions (not citations):")
            for b in bare:
                print(f"    {short(b, 140)}")
        return 0
    sys.exit(f"decisions: unknown subcommand `{cmd}` — counts, list, show, sync, check")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
