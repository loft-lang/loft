#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# api_lint — verify a loft API surface (the stdlib, or a library's pub items)
# against the goals in doc/claude/API_SURFACE.md.
#
# It implements the cheap [auto] checks that gate a library's public API:
#   S3  missing-doc   — a `pub` item with no preceding `//` doc comment
#   S3q doc-quality    — a doc comment that VIOLATES DOC_QUALITY.md: a history
#                       stamp (plan tag / date) or change-narration ("removed",
#                       "used to ..."). Same two patterns scripts/lint_comments.sh
#                       enforces on Rust, brought to the .loft doc-comment layer —
#                       the rules live once, in DOC_QUALITY.md.
#   S1  exact-dup     — two `pub fn`s with the SAME name AND the same param types
#                       (legit overloads — same name, DIFFERENT types — are not dups)
#
# It also lists overload sets as info, so a human can eyeball them for the
# asymmetric-overload check (Phase 2). This is the general instrument — it runs
# over ANY target (the stdlib, or a library package dir); the production CI gate
# (Phase 3) reuses gendoc's Rust API walk.
#
# The tool IS the checklist: it regenerates the live worklist on every run, so a
# finding is "checked off" when it disappears (you fixed it). The count is the
# thermometer. Legit keepers (e.g. an epoch date in a doc) go in a content-keyed
# baseline so they never reappear — same ratchet as scripts/lint_comments.sh.
#
# Usage:
#   scripts/api_lint.py [targets ...]            full report     (default: default/*.loft)
#   scripts/api_lint.py -c [targets ...]         counts only — the thermometer
#   scripts/api_lint.py --check [targets ...]    report only NON-baselined findings (exits non-zero on
#                                                 any; no CI job runs it — RELEASE.md § 0a reads it)
#   scripts/api_lint.py --baseline [targets ...] accept today's findings into .api_lint_baseline
#   scripts/api_lint.py --prune [targets ...]    drop now-fixed entries from the baseline
# Exit: non-zero if any non-baselined [auto] finding remains (so --check can gate).

import sys, os, re, glob
from collections import defaultdict

PUB_RE = re.compile(r'^\s*pub\s+(fn|struct|enum)\s+([A-Za-z_]\w*)')

# DOC_QUALITY.md rules, shared verbatim with scripts/lint_comments.sh so the two
# layers (Rust comments / .loft doc comments) enforce one standard:
#   TAGS — history stamps (plan tags, phase/cluster/arc refs, bare dates); git
#          blame owns these, a doc comment should not.
#   HIST — change-narration: describes a past edit instead of the code as it is.
# A live `#NNN` / doc pointer is a keeper, so TAGS deliberately omits bare `#NNN`.
DOC_TAGS_RE = re.compile(
    r'(@PLAN|@P[0-9]|plan-[0-9]|phase [0-9]|cluster [0-9]|arc [A-Z]'
    r'|[0-9]{4}-[0-9]{2}-[0-9]{2}|[0-9]{4}-[0-9]{2})')
DOC_HIST_RE = re.compile(
    r'\b(removed|no longer|used to [a-z]+|previously [a-z]+ed|formerly|changed from)\b',
    re.IGNORECASE)


def collect_sources(args):
    """Expand args (files / dirs) to a list of .loft files; default to default/*.loft."""
    if not args:
        return sorted(glob.glob("default/*.loft"))
    files = []
    for a in args:
        if os.path.isdir(a):
            files += sorted(glob.glob(os.path.join(a, "**", "*.loft"), recursive=True))
        else:
            files.append(a)
    return files


def param_types(sig):
    """Extract the ordered list of parameter TYPES from a signature string.

    `(self: text, pos: integer, prefix: text)` -> ['text', 'integer', 'text'].
    Names are dropped so two fns differ only when their *types* differ — that is
    what separates a legit overload from an exact dup."""
    m = re.search(r'\((.*?)\)', sig, re.DOTALL)
    if not m or not m.group(1).strip():
        return []
    types = []
    depth = 0
    cur = ""
    # Split on top-level commas (vector<...> may contain commas in principle).
    for ch in m.group(1):
        if ch in "<([":
            depth += 1
        elif ch in ">)]":
            depth -= 1
        if ch == "," and depth == 0:
            types.append(cur)
            cur = ""
        else:
            cur += ch
    types.append(cur)
    out = []
    for p in types:
        p = p.strip()
        if not p:
            continue
        # `name: type` -> type ; collapse whitespace so `vector < File >` == `vector<File>`
        t = p.split(":", 1)[1] if ":" in p else p
        out.append(re.sub(r'\s+', '', t))
    return out


def is_section(t):
    """A `// --- Name ---` divider — structure, not doc prose."""
    return bool(re.match(r'^//\s*-+.*-+\s*$', t))


def doc_block(lines, idx):
    """The caller-facing doc block of the pub item at lines[idx].

    Walk backward over the contiguous preceding block, skipping blank lines and
    `#` annotations (which sit between the doc and the item in this stdlib's
    layout), collecting `//` lines; stop at code, a closing brace, or a section
    header. This attributes exactly the item's own doc — and naturally excludes
    internal comments on `fn Op…` declarations, which are bounded off by code."""
    out = []
    j = idx - 1
    while j >= 0:
        t = lines[j].strip()
        if t == "" or t.startswith("#"):
            j -= 1
            continue
        if t.startswith("//"):
            if is_section(t):
                break
            out.append(t.lstrip("/").strip())
            j -= 1
            continue
        break
    out.reverse()
    return out


def enumerate_api(path):
    """List public API items: dict(kind, name, sig, types, documented, doc, file, line, section)."""
    lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
    items = []
    section = ""
    n = len(lines)
    i = 0
    while i < n:
        t = lines[i].strip()
        sec = re.match(r'^//\s*-+\s*(.*?)\s*-+\s*$', t)
        if sec:
            section = sec.group(1)
            i += 1
            continue
        m = PUB_RE.match(lines[i])
        if m:
            # Accumulate a multi-line signature until the body `{`, native `;`,
            # or a dep-annotation close — whichever ends the declaration.
            buf = lines[i]
            j = i
            while not re.search(r'[{;]', buf) and j + 1 < n:
                j += 1
                buf += " " + lines[j].strip()
            sig = buf.split("{")[0].split(";")[0].strip()
            kind, name = m.group(1), m.group(2)
            doc = doc_block(lines, i)
            items.append({
                "kind": kind, "name": name, "sig": re.sub(r'\s+', ' ', sig),
                "types": tuple(param_types(sig)) if kind == "fn" else (),
                "documented": bool(doc), "doc": list(doc),
                "file": path, "line": i + 1, "section": section,
            })
            i = j + 1
            continue
        i += 1
    return items


BASELINE = ".api_lint_baseline"

CHECK_LABEL = {"S3": "missing-doc", "S3q": "doc-quality", "S1": "exact-dup"}


def compute_findings(items):
    """All [auto] findings as keyed records: dict(check, key, where, msg).

    `key` is content-based (no line number) so the baseline survives edits and
    code moving around — the same property scripts/lint_comments.sh relies on."""
    findings = []
    fns = [it for it in items if it["kind"] == "fn"]

    for it in items:                                   # S3 missing-doc
        if not it["documented"]:
            sig = f"{it['name']}({','.join(it['types'])})"
            findings.append({"check": "S3", "key": f"S3\t{it['file']}\t{sig}",
                             "where": f"{it['file']}:{it['line']}",
                             "msg": f"{it['kind']} {it['name']}"})

    for it in items:                                   # S3q doc-quality
        for ln in it["doc"]:
            for rx, kind in ((DOC_TAGS_RE, "history-stamp"), (DOC_HIST_RE, "change-narration")):
                if rx.search(ln):
                    findings.append({"check": "S3q", "key": f"S3q\t{it['file']}\t{ln}",
                                     "where": f"{it['file']}:{it['line']}",
                                     "msg": f"{it['name']} [{kind}] // {ln}"})

    by_sig = defaultdict(list)                         # S1 exact-dup
    for it in fns:
        by_sig[(it["name"], it["types"])].append(it)
    for (name, types), v in by_sig.items():
        if len(v) > 1:
            findings.append({"check": "S1", "key": f"S1\t{name}({','.join(types)})",
                             "where": "; ".join(f"{x['file']}:{x['line']}" for x in v),
                             "msg": f"{name}({', '.join(types)}) defined {len(v)}×"})
    return findings


def overload_sets(items):
    by_name = defaultdict(set)
    for it in items:
        if it["kind"] == "fn":
            by_name[it["name"]].add(it["types"])
    return {n: s for n, s in by_name.items() if len(s) > 1}


def load_baseline():
    if not os.path.exists(BASELINE):
        return set()
    return set(l.rstrip("\n") for l in open(BASELINE, encoding="utf-8") if l.strip())


def main():
    argv = sys.argv[1:]
    mode = "report"
    if argv and argv[0] in ("-c", "--count", "--check", "--baseline", "--prune"):
        mode = {"-c": "count", "--count": "count"}.get(argv[0], argv[0].lstrip("-"))
        argv = argv[1:]

    files = collect_sources(argv)
    if not files:
        print("api_lint: no .loft files found", file=sys.stderr)
        return 2

    items = []
    for f in files:
        items.extend(enumerate_api(f))
    findings = compute_findings(items)
    baseline = load_baseline()
    active = [f for f in findings if f["key"] not in baseline]

    if mode == "baseline":
        with open(BASELINE, "w", encoding="utf-8") as fh:
            fh.write("\n".join(sorted(f["key"] for f in findings)) + "\n")
        print(f"wrote {len(findings)} entries to {BASELINE} (all current findings accepted)")
        return 0
    if mode == "prune":
        keys = {f["key"] for f in findings}
        kept = sorted(k for k in baseline if k in keys)
        with open(BASELINE, "w", encoding="utf-8") as fh:
            fh.write(("\n".join(kept) + "\n") if kept else "")
        print(f"pruned {BASELINE}: {len(baseline)} -> {len(kept)} entries "
              f"({len(baseline) - len(kept)} now-fixed dropped)")
        return 0

    def tally(group):
        return {c: sum(1 for f in group if f["check"] == c) for c in ("S3", "S3q", "S1")}

    if mode == "count":
        t, tb = tally(active), tally(findings)
        for c in ("S3", "S3q", "S1"):
            base = tb[c] - t[c]
            extra = f"  ({base} baselined)" if base else ""
            print(f"  {c} {CHECK_LABEL[c]:<12} {t[c]}{extra}")
        print(f"== {len(active)} active [auto] finding(s); {len(baseline)} baselined ==")
        return 1 if active else 0

    # report / check
    fns = sum(1 for it in items if it["kind"] == "fn")
    print(f"== api_lint: {len(items)} public items ({fns} fn, {len(items)-fns} struct/enum) "
          f"across {len(files)} file(s) ==")
    shown = active if mode == "check" else findings
    for c in ("S3", "S3q", "S1"):
        group = sorted((f for f in shown if f["check"] == c), key=lambda x: x["where"])
        print(f"\n[{c}] {CHECK_LABEL[c]}: {len(group)}")
        for f in group:
            base = "  (baselined)" if mode == "report" and f["key"] in baseline else ""
            print(f"     {f['where']}  {f['msg']}{base}")

    if mode == "report":
        ov = overload_sets(items)
        print(f"\n[info] overload sets (same name, different param types — not a dup): {len(ov)}")
        for name in sorted(ov):
            sigs = sorted(ov[name], key=lambda x: (len(x), x))
            print(f"     {name}: {len(sigs)} -> " + "; ".join("(" + ", ".join(s) + ")" for s in sigs))

    print(f"\n== {len(active)} active [auto] finding(s)"
          + (f", {len(findings)-len(active)} baselined" if baseline else "") + " ==")
    return 1 if active else 0


if __name__ == "__main__":
    sys.exit(main())
