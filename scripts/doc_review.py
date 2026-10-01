#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# doc_review — the per-SECTION staleness checklist for loft's user-visual docs.
# Implements RELEASE.md § 0 (the user-visual documentation gate) and
# API_SURFACE.md § S7 (docs don't go stale).
#
# The review UNIT is the section, not the file — reusing the internal structure
# the docs already have (the `// --- Name ---` dividers that gendoc's
# build_sections and the library-side parse_pkg_api both parse, and that
# api_lint already tracks; `<h1..3>` in HTML, `#` headings in Markdown). A
# section maps to the HTML anchor a reader navigates to, so a stale section is
# addressable, and editing one section re-flags only that section.
#
# Two axes per section:
#   auto   — api_lint findings in the section (for .loft API surfaces) + hedge /
#            temporal words ("currently", "planned", "for now", "TODO", …) + an
#            oversized-section red flag (≥ MAX_SECTION pub items → split for review).
#   review — a human sign-off, recorded in a content-hash ledger keyed per
#            section, so it persists and re-surfaces only when that section's
#            text changed (or, for claim pages, when the loft version moved).
# A section is cleared (✓) only when both axes pass.
#
# Targets — reuse for ANY library, not just the stdlib:
#   scripts/doc_review.py                 the built-in loft corpora (stdlib + guides + comparison + prose)
#   scripts/doc_review.py <dir|file>…     a library: enumerates its .loft / .md / .html, same section model
#
# Other modes:
#   -c | --count        counts only (the thermometer)
#   --review SPEC…       sign off a section (`file#Section`) or a whole file (all its sections)
#   --prune             drop ledger entries for sections that no longer exist
# Exit: non-zero while any section is uncleared (so it can gate a release).

import sys, os, re, glob, hashlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import api_lint  # reuse the one API enumerator + DOC_QUALITY patterns

LEDGER = ".doc_review_ledger"

# A section with this many public items is a red flag: too big to review as a
# unit and too big for a user to scan — split it into clearer sub-sections.
# Override per project: DOC_REVIEW_MAX_SECTION=30 scripts/doc_review.py …
MAX_SECTION = int(os.environ.get("DOC_REVIEW_MAX_SECTION", "20"))

HEDGE_RE = re.compile(
    r'\b(currently|for now|not yet|planned|coming soon|will be|to be added'
    r'|temporarily|at the moment|as of now|in the future|soon|TODO|FIXME'
    r'|Q[1-4] follow-up)\b', re.IGNORECASE)

# Built-in loft corpora (used when no target is given). (title, pattern, auto_mode).
# auto_mode: "api" runs api_lint on the .loft surface; "recheck" also requires the
# review to be at the current loft version (claim pages with no code anchor).
CORPORA = [
    ("A. Stdlib API reference", "default/*.loft", {"api"}),
    ("B. Guide pages  (code runs via `make test` docs suite)", "tests/docs/*.loft", set()),
    ("C. Comparison / performance  (hand-maintained; scheduled recheck)",
     ["doc/00-vs-rust.html", "doc/00-vs-python.html", "doc/00-performance.html"], {"recheck"}),
    ("D. Other user prose", ["doc/learn-loft.md", "doc/DEVELOPERS.md"], set()),
    ("E. Flags & routines  (Makefile help / CLI — references resolved)", ["Makefile"], set()),
    ("F. Language reference — fault/limitation claims (anchored + recheck)",
     ["doc/claude/LOFT.md"], {"claims", "recheck"}),
    # The hand-written pages a new user meets first: the repository front page and the site's
    # entry points.  gendoc READS install.html and roadmap.html into the printed reference, so
    # they are sources, not outputs; the numbered guide pages are rendered from tests/docs/.
    ("G. Entry pages  (README + hand-written site pages)",
     ["README.md", "doc/install.html", "doc/roadmap.html", "doc/docs.html",
      "doc/examples.html", "doc/report.html"], {"recheck"}),
]

CODE_SPAN = re.compile(r"`([^`]+)`")

# Fault / limitation claims — "loft can't do X". These rot the worst: a fix lands
# elsewhere and the claim never dies, because nothing local triggers its update.
# The cure is an ANCHOR — a live tracker pointer (#issue / INCONSISTENCIES / @P…) —
# so the tracker's closure becomes the signal to revisit. An UNANCHORED limitation
# claim has no such trigger, so it is flagged. (Comparison pages compare to Rust/
# Python, not an issue — they have no anchor and rely on cadence recheck instead.)
CLAIM_RE = re.compile(
    r'\b(not (yet )?(supported|possible|implemented|allowed)|unsupported'
    r"|no way to|does( not|n't) work yet|\blimitation\b|unlike (rust|python)"
    r'|\blacks\b|currently no)\b', re.IGNORECASE)
ANCHOR_RE = re.compile(r'#\d+|@P\d|INCONSISTENCIES|DESIGN_DECISIONS')


def claim_hits(path, block):
    """Unanchored limitation/comparison claims — a fault claim with no live tracker
    pointer on its line, so nothing will ever flag it when the fault is fixed."""
    out = []
    for l in prose_lines(path, block):
        if CLAIM_RE.search(l) and not ANCHOR_RE.search(l):
            out.append(l.strip()[:56])
    return out


def repo_make_targets():
    """The Makefile's real targets (None if no Makefile) — for ref resolution."""
    if not os.path.exists("Makefile"):
        return None
    out = set()
    for l in open("Makefile", encoding="utf-8", errors="replace"):
        m = re.match(r"^([A-Za-z0-9_.-]+):", l)
        if m:
            out.add(m.group(1))
    return out or None


def repo_flags():
    """Real CLI flags declared in src/*.rs (None if absent) — for ref resolution."""
    out = set()
    for f in glob.glob("src/*.rs"):
        out |= set(re.findall(r'"(--[a-z][a-z0-9-]+)"', open(f, encoding="utf-8", errors="replace").read()))
    return out or None


def code_refs(lines):
    """`make <target>` and `--flag` referenced in CODE context (inline `..` or ```
    fences) — prose like "make sense" is ignored, so resolution has no false hits."""
    targets, flags = set(), set()
    fenced = False
    for l in lines:
        if l.strip().startswith("```"):
            fenced = not fenced
            continue
        for span in ([l] if fenced else CODE_SPAN.findall(l)):
            targets |= set(re.findall(r"\bmake ([a-z][\w-]+)", span))
            flags |= set(re.findall(r"(--[a-z][\w-]+)", span))
    return targets, flags


def loft_version():
    try:
        for line in open("Cargo.toml", encoding="utf-8"):
            m = re.match(r'\s*version\s*=\s*"([^"]+)"', line)
            if m:
                return m.group(1)
    except OSError:
        pass
    return "?"


def section_name(path, line):
    """A section divider in this file's syntax → its name, else None.
    Reuses the conventions the doc generators already parse."""
    if os.path.basename(path) == "Makefile" or path.endswith(".mk"):
        # The `# ==== Title ====` rule and `# Group:` header lines that structure
        # the Makefile's `make help` block into routine groups.
        m = re.match(r'^#\s*={2,}\s*(.+?)\s*={2,}\s*$', line) or re.match(r'^#\s+(\S.*\S):\s*$', line)
        name = m.group(1).strip() if m else None
        return name if name and name.strip("=- ") else None
    if path.endswith(".loft"):
        # Column 0 only: a top-level doc section, like the ones gendoc renders —
        # NOT an indented `// --- … ---` used as in-function sub-structure.
        m = re.match(r'^//\s*-+\s*(.+?)\s*-+\s*$', line)
        return m.group(1) if m and m.group(1).strip("- ") else None
    if path.endswith(".md"):
        m = re.match(r'^#{1,4}\s+(.*\S)', line)
        return m.group(1).strip() if m else None
    if path.endswith((".html", ".htm")):
        m = re.match(r'(?i)\s*<h[1-3][^>]*>(.*?)</h[1-3]>', line)
        return re.sub(r"<[^>]+>", "", m.group(1)).strip() if m else None
    return None


def sections(path):
    """Split a file into its internal sections: [{name, ranges:[(start,end)], lines}].

    Same-named dividers are MERGED into one logical section, because gendoc renders
    them as one (and that is the section a reader inspects) — so the size lint sees
    the true rendered section, not fragmented blocks that each slip under the limit."""
    lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
    if os.path.basename(path) == "Makefile":
        # Review only the leading `make help` comment block — the routine doc —
        # not the build recipes below it (those are code, full of foreign flags).
        cut = next((i for i, l in enumerate(lines) if l.strip() and not l.startswith("#")), len(lines))
        lines = lines[:cut]
    heads = [(section_name(path, ln), i) for i, ln in enumerate(lines)]
    heads = [(nm, i) for nm, i in heads if nm]
    if not heads or heads[0][1] > 0:
        heads.insert(0, ("(intro)", 0))
    merged, order = {}, []
    for k, (nm, start) in enumerate(heads):
        end = heads[k + 1][1] if k + 1 < len(heads) else len(lines)
        if nm not in merged:
            merged[nm] = {"name": nm, "ranges": [], "lines": []}
            order.append(nm)
        merged[nm]["ranges"].append((start + 1, end))  # 1-based inclusive span
        merged[nm]["lines"] += lines[start:end]
    return [merged[nm] for nm in order]


def in_section(line_no, sec):
    return any(s <= line_no <= e for s, e in sec["ranges"])


def prose_lines(path, block):
    """Human-readable lines of a section block (drop code / tags)."""
    if path.endswith((".html", ".htm", ".md")):
        return [re.sub(r"<[^>]+>", " ", l) for l in block]
    # The stdlib publishes `///` only; its `//` lines are maintainer notes (loft#1808).
    mark = "///" if os.path.basename(os.path.dirname(os.path.abspath(path))) == "default" else "//"
    return [l for l in block if l.strip().startswith(mark)]


def hedge_hits(path, block):
    hits = []
    for l in prose_lines(path, block):
        m = HEDGE_RE.search(l)
        if m:
            hits.append(m.group(0).lower())
    return hits


def pub_items(path):
    """(1-based line, name) of every public item — counted by distinct name per
    section, since a reader scans CONCEPTS, not overloads (sin(float)/sin(single)
    are one idea; counting raw items would false-flag overload-heavy sections)."""
    return [(it["line"], it["name"]) for it in api_lint.enumerate_api(path)]


def api_findings_by_line(path):
    """{line_no: count} of active api_lint findings, for attributing to sections."""
    base = api_lint.load_baseline()
    items = api_lint.enumerate_api(path)
    by_line = {}
    for f in api_lint.compute_findings(items):
        if f["key"] in base:
            continue
        for n in re.findall(r":(\d+)", f["where"]):
            by_line[int(n)] = by_line.get(int(n), 0) + 1
    return by_line


def block_hash(block):
    return hashlib.sha256("\n".join(block).encode("utf-8")).hexdigest()


def load_ledger():
    d = {}
    if os.path.exists(LEDGER):
        for line in open(LEDGER, encoding="utf-8"):
            p = line.rstrip("\n").split("\t")
            if len(p) == 3:
                d[p[0]] = (p[1], p[2])
    return d


def save_ledger(d):
    with open(LEDGER, "w", encoding="utf-8") as fh:
        for k in sorted(d):
            fh.write(f"{k}\t{d[k][0]}\t{d[k][1]}\n")


def collect_targets(args):
    """No args → built-in loft corpora. Else enumerate .loft/.md/.html under each
    target (a library dir or file) as one corpus — the reuse-for-libraries path."""
    if not args:
        return [(title, [p for p in (pat if isinstance(pat, list) else sorted(glob.glob(pat)))
                         if os.path.exists(p)], mode)
                for title, pat, mode in CORPORA]
    files = []
    for a in args:
        if os.path.isdir(a):
            for ext in ("loft", "md", "html"):
                files += sorted(glob.glob(os.path.join(a, "**", f"*.{ext}"), recursive=True))
        elif os.path.exists(a):
            files.append(a)
    # .loft API files get the api check; everything else hedge-only.
    return [("Library / target docs", files, {"api"})]


def assess(path, sec, mode, api_lines, items, targets, flags, ledger, version):
    key = f"{path}#{sec['name']}"
    h = block_hash(sec["lines"])
    auto_ok = True
    reasons = []
    if "api" in mode and path.endswith(".loft"):
        n = sum(c for ln, c in api_lines.items() if in_section(ln, sec))
        if n:
            auto_ok = False
            reasons.append(f"{n} api_lint")
    concepts = {nm for ln, nm in items if in_section(ln, sec)}
    if len(concepts) >= MAX_SECTION:
        auto_ok = False
        reasons.append(f"⚠ oversized: {len(concepts)} concepts (≥{MAX_SECTION}) — split for review")
    hh = hedge_hits(path, sec["lines"])
    if hh:
        auto_ok = False
        reasons.append(f"{len(hh)} hedge ({', '.join(sorted(set(hh)))})")
    if "claims" in mode:
        ch = claim_hits(path, sec["lines"])
        if ch:
            auto_ok = False
            reasons.append(f"{len(ch)} unanchored limitation claim(s) — cite #issue/INCONSISTENCIES or re-verify")
    if os.path.basename(path) != "Makefile":  # recipes use foreign (cargo/shell) flags
        rt, rf = code_refs(sec["lines"])
        bad = [f"make {t}" for t in sorted(rt) if targets and t not in targets] \
            + [f for f in sorted(rf) if flags and f not in flags]
        if bad:
            auto_ok = False
            reasons.append(f"unresolved refs: {', '.join(bad)}")
    entry = ledger.get(key)
    review_ok = bool(entry) and entry[0] == h
    if "recheck" in mode and review_ok and entry[1] != version:
        review_ok = False
    if not entry:
        reasons.append("no review on record")
    elif entry[0] != h:
        reasons.append("changed since review")
    elif "recheck" in mode and entry[1] != version:
        reasons.append(f"recheck due ({entry[1]}→{version})")
    return key, h, auto_ok, review_ok, reasons


def main():
    argv = sys.argv[1:]
    version = loft_version()
    ledger = load_ledger()

    if argv and argv[0] == "--review":
        # SPEC = "file#Section" (one section) or "file" (all its sections).
        for spec in argv[1:]:
            path, _, want = spec.partition("#")
            if not os.path.exists(path):
                print(f"  no such file: {path}", file=sys.stderr)
                continue
            for sec in sections(path):
                if want and sec["name"] != want:
                    continue
                key = f"{path}#{sec['name']}"
                ledger[key] = (block_hash(sec["lines"]), version)
                print(f"  reviewed {key} @ {ledger[key][0][:8]} (v{version})")
        save_ledger(ledger)
        return 0

    targets = collect_targets(argv if not (argv and argv[0] in ("-c", "--count")) else argv[1:])

    if argv and argv[0] == "--prune":
        live = {f"{p}#{s['name']}" for _, files, _ in targets for p in files for s in sections(p)}
        dropped = [k for k in ledger if k not in live]
        for k in dropped:
            del ledger[k]
        save_ledger(ledger)
        print(f"pruned {LEDGER}: dropped {len(dropped)} vanished section(s)")
        return 0

    counts_only = argv and argv[0] in ("-c", "--count")
    targets_set, flags_set = repo_make_targets(), repo_flags()
    total = cleared = 0
    print(f"== doc_review: user-visual documentation checklist (loft v{version}) ==")
    for title, files, mode in targets:
        if not counts_only:
            print(f"\n{title}")
        for path in files:
            is_loft = path.endswith(".loft")
            api_lines = api_findings_by_line(path) if ("api" in mode and is_loft) else {}
            items = pub_items(path) if is_loft else []
            secs = sections(path)
            if not counts_only:
                print(f"  {path}")
            for sec in secs:
                total += 1
                _, _, auto_ok, review_ok, reasons = assess(
                    path, sec, mode, api_lines, items, targets_set, flags_set, ledger, version)
                cleared += auto_ok and review_ok
                if not counts_only:
                    tail = ("  — " + "; ".join(reasons)) if reasons else ""
                    print(f"    review {'✓' if review_ok else '☐'}  auto {'✓' if auto_ok else '☐'}  "
                          f"§ {sec['name']}{tail}")

    print(f"\n== {cleared}/{total} sections cleared; {total - cleared} ☐ "
          f"(sign off with `--review <file>` or `--review '<file>#<Section>'`) ==")
    return 0 if cleared == total else 1


if __name__ == "__main__":
    sys.exit(main())
