#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# rule_tags.py — the formal rules as `@`-tags, and the code sites that cite them.
#
# A rule is only an anchor for code if it can be found EXACTLY.  See
# doc/claude/formal/README.md § Rule tags for the convention and CLAUDE.md § Tracker tags
# for the sigil it reuses.  Two constraints, both measured rather than assumed:
#
#   * BOUNDARY-EXACT.  23 of the defined rules are a prefix of another (`@B-View` vs
#     `@B-View-Base`), because a general rule and its refinements share a stem.  `\b` does
#     not help — `-` is already a word boundary — so a citation matches `@Name` only when
#     the next character cannot continue a tag.
#   * ONLY A DEFINED RULE.  `B-Ref`, `D-op`, `D-own`, `D-cap`, `D-op-null` read like rules
#     and are family prefixes that appear only in prose.  Citing one is an error.
#   * A NAMESPACED PREFIX, `@FR-<Rule>` (Formal Rule).  A bare `@Name` is NOT unambiguous
#     here: `@` already carries the tracker tags (`@P259`, `@PLN3`, `@PLAN22`, `@F7`, `@I81`,
#     `@GH247`), the worked-example family (`@AAA-###`), and the corpus annotations (`@ARGS`,
#     `@NAME`, `@IGNORE`, `@EXPECT_ERROR`).  Measured: a bare-`@` reading of src/ returned
#     4142 "citations", not one of them a rule.  `@FR-` sits in the same family shape as the
#     others and cannot be confused with `@F<digits>`, whose next character is a digit.
#
# Subcommands:
#   list           every defined rule and the doc that defines it
#   check          every citation resolves; no rule defined twice   (exit 1 on failure)
#   sites <tag>    the code sites citing one rule (tag with or without the @FR- prefix)
#   registers      each chapter's stated `OPEN: n` vs the entries it lists;
#                  `--issues` also asks whether an open entry's issue has closed
#   dups           rules cited from 2+ sites — the duplication question, asked by MEANING
#                  rather than by code shape (which is what rule_predicate_audit.py does)

import collections
import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# Portable by design (the formal-rules skill): another project vendors this file
# unchanged and points the two roots at its own layout.  RULES_DIR is where the
# rules docs live; CITE_DIRS (colon-separated) is where citations are scanned.
FORMAL = os.environ.get("RULES_DIR", os.path.join(ROOT, "doc/claude/formal"))
SRC = os.path.join(ROOT, "src")
CITE_DIRS = os.environ.get("CITE_DIRS", "").split(":") if os.environ.get("CITE_DIRS") else [SRC]
CITE_EXTS = os.environ.get("CITE_EXTS", ".rs").split(",")

# A rule is DEFINED by a rules-block line `  (Name)  prose` or a deviation header `### Name —`.
# A RULE is defined by a line `  (Name)  prose` INSIDE A FENCED BLOCK — that is the shape the
# rules blocks use.  Two narrowings, each forced by a false positive rather than foreseen:
# markdown SECTION headers are not a definition form (reading them as one turned `## Rules`,
# `## Notation` and `## Deviations` into "rules defined in 17 docs"), and a parenthesised
# MENTION in prose is not one either (`heap.md` refers to "(D-cap-3)", which read as a second
# definition of a rule `capabilities.md` owns).
DEF_INLINE = re.compile(r"^\s*\(([A-Z][A-Za-z0-9-]{1,40})\)\s", re.M)
# A DEVIATION is a register entry.  Two spellings are in use and BOTH are definitions —
# a `### D-own-7 — …` header (ownership.md) and a `> **D-bind-11 — …` blockquote
# (binding.md).  Reading only the header form left `@FR-D-bind-11` unresolvable, which the
# check reported the moment it was cited; the doc format varying by file is exactly the kind
# of thing a registry has to absorb rather than legislate away.
# The blockquote form must keep the em-dash INSIDE the bold (`> **D-bind-12 — CLOSED …`);
# `> **D-bind-10**:` is a cross-REFERENCE from another doc, not a second definition.
# A register entry names its tag in a HEADING or a `>` quote, and what follows the tag is
# free prose: an em-dash, a `(CLOSED)`, or `, part three —`.  So the tag ends at the first
# character that cannot be part of one — `'` included, or `### D-gen-4's closure summary`
# reads as a second definition of D-gen-4 (loft#1452).
# A deviation tag's final segment is a NUMBER for the counted families (`D-bind-28`,
# `D-heap-1`) and a WORD for the named ones (`D-Opt-NoNull`, `D-col-null`, `D-Null-Guard`).
# Requiring a number made every named entry invisible — `types.md` declared `D-Opt-NoNull`
# open while the tool could not see it at all, and three deviations registered on 2026-09-08
# were unfindable the moment they were written (loft#1452).
DEV_TAG = r"D[A-Za-z]*(?:-[A-Za-z][A-Za-z0-9]*)*-(?:\d+|[A-Za-z][A-Za-z0-9]*)|DN\d+[A-Za-z-]*"

DEV_COUNT = re.compile(r"OPEN:\s*\**\s*\d+")
DEV_DATE = re.compile(r"(20\d\d-\d\d-\d\d)")

DEF_DEV = re.compile(
    rf"(?:^#{{2,5}}\s+`?(?P<h>{DEV_TAG})(?![A-Za-z0-9_'-])"
    rf"|^>\s*\*\*(?P<q>{DEV_TAG})(?![A-Za-z0-9_'-]))",
    re.M,
)
# A CITATION is `@FR-<Rule>`, boundary-exact so `@FR-B-View` does not match `@FR-B-View-Base`.
CITE = re.compile(r"@FR-([A-Z][A-Za-z0-9-]{1,40})(?![-A-Za-z0-9])")


def defined_rules():
    """{tag: [defining files]} — a rule defined twice is a bug the check reports.

    RULES ONLY.  A deviation heading (`D-own-7`, `DN3-Float`) is a defect REPORT —
    a thing that was once true of the code — and a rule is a thing that must always
    be true of it.  They are not the same kind and only one is quotable from a code
    site: citing `@FR-D-own-7` would pin a closed bug's id as if it were law, and
    `dups` would then police it as one.  See `defined_deviations`.
    """
    out = collections.defaultdict(list)
    for path in sorted(glob.glob(FORMAL + "/*.md")):
        fenced = "\n".join(_fenced_lines(open(path, encoding="utf-8").read()))
        for name in set(DEF_INLINE.findall(fenced)):
            out[name].append(os.path.basename(path))
    return out


def defined_deviations():
    """{tag: (files, status)} — the deviation register's entries, OPEN or CLOSED.

    The status is what decides whether a code site may cite one, and the two
    answers are opposite:

    * an **OPEN** deviation is a LIVE fact about the code.  A site that implements
      the shortfall should say so — `ref_tuple_element_ok`'s "the heap half is
      refused under @FR-D-bind-11" is how you find every site that has to change
      when D-bind-11 closes.  That citation is worth more than the rule's would be,
      because the site does NOT enforce the rule.
    * a **CLOSED** deviation is HISTORY.  Citing one pins a defect id as if it were
      law, and the site's real subject is the rule the deviation was measured
      against — which is still true, where the deviation no longer is.
    """
    entries = collections.defaultdict(list)
    for path in sorted(glob.glob(FORMAL + "/*.md")):
        text = open(path, encoding="utf-8").read()
        for m in DEF_DEV.finditer(text):
            tag = m.group("h") or m.group("q")
            # `OPEN: 3` is a chapter's COUNT, never this entry's status — and it sits within
            # reach of a summary heading's tail.
            tail = DEV_COUNT.sub("", text[m.end():m.end() + 120])
            status = "CLOSED" if "CLOSED" in tail else "OPEN" if "OPEN" in tail else "?"
            date = DEV_DATE.search(tail)
            line = text.count("\n", 0, m.start()) + 1
            entries[tag].append(
                (os.path.basename(path), line, status, date.group(1) if date else "0000-00-00"))
        # The BULLET form is a third spelling, and it is how five chapters write every entry
        # they own.  Reading only the two above made `D-layout-1`, `D-match-4` and `D-tup-10`
        # answer *"is not a defined rule"* — in the GATING path, so a site citing an open
        # deviation the way the skill instructs would have failed CI with a message saying the
        # rule does not exist.  Green only because nothing cited one yet.  Bullets are scoped
        # to the `## Deviations` section for the reason `chapter_registers` gives: elsewhere
        # the same shape is prose (`DbRef`, `Destructure`) rather than an entry.
        for tag, status, _issues, date, line in _section_bullets(text):
            entries[tag].append((os.path.basename(path), line, status, date))
    return {tag: ([r[0] for r in rows], _resolve_status(rows)) for tag, rows in entries.items()}


def _section_bullets(text):
    """(tag, status, issues, date, line) for each bullet entry in a `## Deviations` section."""
    m = re.search(r"^## Deviations\s*$", text, re.M)
    if not m:
        return
    end = re.search(r"^## ", text[m.end():], re.M)
    body = text[m.end():m.end() + end.start()] if end else text[m.end():]
    base = m.end()
    found = list(REG_ENTRY_BULLET.finditer(body))
    for i, e in enumerate(found):
        stop = found[i + 1].start() if i + 1 < len(found) else len(body)
        head = REG_OPEN.sub("", body[e.end():stop].split("\n\n")[0][:400])
        date = DEV_DATE.search(head)
        yield (e.group("tag"),
               "CLOSED" if "closed" in head.lower() else "OPEN",
               REG_ISSUE.findall(head),
               date.group(1) if date else "0000-00-00",
               text.count("\n", 0, base + e.start()) + 1)


def _resolve_status(rows):
    """The status of a tag that has SEVERAL entries — the latest dated one wins.

    Two different shapes share a tag, and only the date tells them apart.  A tag can be a
    TIMELINE — `D-bind-11` was opened 2026-08-19 and closed 2026-09-03, and both entries
    stand — or it can be one deviation stated in PARTS, as `D-bind-28` is: part two CLOSED
    and part three REOPENED, both on 2026-09-07, and the tag is OPEN because a part of it
    is.  Taking the FIRST entry's answer got the second shape wrong for as long as it
    existed (loft#1452: D-bind-28 read CLOSED while part three said REOPENED); taking
    "OPEN wins" gets the first shape wrong (it would reopen D-bind-11, D-clo-14, D-gen-4).
    The latest date is right for both, and ties break on file order, which is the order the
    parts are written in.
    """
    dated = [r for r in rows if r[2] != "?"]
    return max(dated, key=lambda r: (r[3], r[1]))[2] if dated else "?"


def _fenced_lines(text):
    """Only the lines inside ``` fences — where the rules blocks live."""
    out, inside = [], False
    for line in text.split("\n"):
        if line.lstrip().startswith("```"):
            inside = not inside
            continue
        if inside:
            out.append(line)
    return out


# A chapter states its own open count as `OPEN: n` at the top of its `## Deviations`
# section, and lists the entries below it.  Those are two claims about one number, and
# `defined_deviations` above reads a THIRD — so the register has three decoders and
# nothing compared them.  Measured 2026-09-12: they disagreed in five chapters at once.
REG_OPEN = re.compile(r"OPEN:\s*\**\s*(\d+)")
# An entry inside that section, in all three spellings the docs use.  The BULLET form is
# the one `defined_deviations` cannot see (it reads headings and blockquotes only), and it
# is how `operational`, `layout`, `matching`, `tuples` and `calls` write every entry they
# own — so those chapters' open entries were invisible to the tool that counts them.
REG_ENTRY_STRICT = re.compile(
    rf"^(?:#{{2,5}}\s+`?|>\s*\*\*`?)(?P<tag>{DEV_TAG})(?![A-Za-z0-9_'-])", re.M)
REG_ENTRY_BULLET = re.compile(
    rf"^-\s+\*\*`?(?P<tag>{DEV_TAG})(?![A-Za-z0-9_'-])", re.M)
REG_ISSUE = re.compile(r"loft#(\d+)")


def chapter_registers():
    """[(file, stated_open, [(tag, status, [issues])])] — each chapter's own register.

    Scoped to the `## Deviations` section on purpose.  A bullet naming a `D-` tag is a
    definition only there; elsewhere the same shape is prose or a cross-reference, and
    reading it as an entry finds `Deep-copied`, `DbRef` and `Destructure` (18 such bullets
    across the chapters).  The section boundary is the discriminator that separates them.

    An entry's status comes from its HEAD, not its body: `D-tup-10` is open and carries ⚠ notes
    about *different*, closed issues further down, so a whole-entry read calls it closed.  `CLOSED` wins over `OPEN` in that head, which is what makes `OPENED AND CLOSED`
    read correctly, and an entry marking neither is open — the form `D-op-1` and
    `D-layout-1` use.
    """
    out = []
    for path in sorted(glob.glob(FORMAL + "/*.md")):
        text = open(path, encoding="utf-8").read()
        m = re.search(r"^## Deviations\s*$", text, re.M)
        if not m:
            continue
        end = re.search(r"^## ", text[m.end():], re.M)
        body = text[m.end():m.end() + end.start()] if end else text[m.end():]
        stated = REG_OPEN.search(body)
        entries = {}
        # Two scans, because the three spellings are not equally safe.  A heading or a
        # blockquote is unambiguous anywhere in the file — `defined_deviations` already
        # trusts them file-wide — and an entry may sit beside the rule it qualifies rather
        # than in the register: `heap.md` states `D-heap-LIFO` next to `(H-FreeLIFO)` and
        # says so in its own section.  A BULLET is only a definition inside the section.
        for scope, pattern in ((text, REG_ENTRY_STRICT), (body, REG_ENTRY_BULLET)):
            entries.update(_register_entries(scope, pattern))
        entries = list(entries.values())
        out.append((os.path.basename(path),
                    int(stated.group(1)) if stated else None, entries))
    return out


def _register_entries(body, pattern):
    """{tag: (tag, status, [issues])} for one scan of one body."""
    out = {}
    found = list(pattern.finditer(body))
    for i, e in enumerate(found):
        # The head ends where the entry does: at the next entry, or at the blank line that
        # ends its first paragraph.  A fixed window instead of this boundary runs into the
        # neighbour and into the closing sentence every chapter carries (*"plus every
        # closed one with its dates"*), which read `D-op-2` and `D-heap-LIFO` — both open —
        # as closed.
        stop = found[i + 1].start() if i + 1 < len(found) else len(body)
        head = REG_OPEN.sub("", body[e.end():stop].split("\n\n")[0][:400])
        out[e.group("tag")] = (e.group("tag"),
                               "CLOSED" if "closed" in head.lower() else "OPEN",
                               REG_ISSUE.findall(head))
    return out


def closed_issues(numbers):
    """({n: title} for the CLOSED ones, [n] for the ones the tracker could not be asked).

    The second half exists because a silent partial answer is the failure this whole check
    is about: an issue `gh` cannot reach looks exactly like an issue that is still open, so
    an unreachable one must be NAMED rather than counted as fine.

    An open deviation names the issue it was filed as, so an issue that has since closed is
    a claim the register has stopped checking.  It is not proof the deviation closed: one
    can outlive its issue deliberately (`D-clo-14` records an over-free that closed and the
    leak it traded for, which did not).  So this REPORTS the pairs to re-measure and never
    decides them.
    """
    import json
    import subprocess
    out, unreachable = {}, []
    for n in sorted(numbers):
        try:
            r = subprocess.run(["gh", "issue", "view", str(n), "--json", "state,title"],
                               capture_output=True, text=True, timeout=30)
            if r.returncode:
                unreachable.append(n)
                continue
            d = json.loads(r.stdout)
            if d.get("state") == "CLOSED":
                out[n] = d.get("title", "")
        except Exception:
            unreachable.append(n)
    return out, unreachable


def citations():
    """{tag: [(file, line)]} for every `@Tag` in the citation dirs (default: src/*.rs)."""
    out = collections.defaultdict(list)
    for d in CITE_DIRS:
        for ext in CITE_EXTS:
            for path in glob.glob(os.path.join(d, "**/*" + ext), recursive=True):
                for n, line in enumerate(open(path, encoding="utf-8", errors="replace"), 1):
                    for tag in CITE.findall(line):
                        out[tag].append((os.path.relpath(path, ROOT), n))
    return out


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "check"
    rules = defined_rules()
    devs = defined_deviations()

    if cmd == "list":
        # A deviation carries a STATUS as well as its files, so it prints differently from a
        # rule — and printing it the rule's way raised `unhashable type: 'list'`, which is why
        # `list --deviations` had never once run (loft#1452).
        if "--deviations" in sys.argv:
            for tag in sorted(devs):
                files, status = devs[tag]
                print(f"@FR-{tag:<28} {status:<7} {', '.join(sorted(set(files)))}")
            print(f"\n{len(devs)} deviation entries (NOT citable), "
                  f"{sum(1 for v in devs.values() if v[1] == 'OPEN')} open")
            return 0
        for tag in sorted(rules):
            print(f"@FR-{tag:<28} {', '.join(sorted(set(rules[tag])))}")
        print(f"\n{len(rules)} defined rules")
        return 0

    cites = citations()

    if cmd == "sites":
        tag = sys.argv[2].removeprefix("@FR-").lstrip("@")
        if tag in devs:
            files, status = devs[tag]
            article = "an" if status == "OPEN" else "a"
            print(f"@FR-{tag} is {article} {status} DEVIATION entry "
                  f"({', '.join(sorted(set(files)))}), not a rule.")
            if status == "CLOSED":
                print("A closed deviation is history — cite the RULE it was measured against.")
            else:
                print("An OPEN deviation is a live limitation; a site that IMPLEMENTS the "
                      "shortfall may cite it, and these are the sites that must change when "
                      "it closes:")
                for f, n in cites.get(tag, []):
                    print(f"  {f}:{n}")
            return 0 if status == "OPEN" else 1
        if tag not in rules:
            print(f"@FR-{tag} is not a defined rule")
            return 1
        for f, n in cites.get(tag, []):
            print(f"{f}:{n}")
        print(f"\n@FR-{tag}: {len(cites.get(tag, []))} citation(s)")
        return 0

    if cmd == "registers":
        # The three decoders side by side: the chapter's stated `OPEN: n`, the entries it
        # actually lists, and (with --issues) whether each open entry's issue still is.
        want_issues = "--issues" in sys.argv
        regs = chapter_registers()
        # Status comes from `defined_deviations`, the ONE home: it resolves a tag with several
        # entries by date, which a fresh scan does not — `D-bind-11` and `D-bind-28` each read
        # OPEN in isolation and are closed once their later rows are taken into account.  It
        # also attributes a chapter that keeps its register in the `-history` companion, which
        # is how `types.md` stated `OPEN: 0` over an open `D-Domain-Guard` next door.
        devs = defined_deviations()
        chapter_open = collections.defaultdict(set)
        for tag, (files, status) in devs.items():
            if status != "OPEN":
                continue
            for f in set(files):
                chapter_open[f.replace("-history.md", ".md")].add(tag)
        drift, live = [], []
        for f, stated, entries in regs:
            found = sorted(chapter_open.get(f, ()))
            if stated is not None and stated != len(found):
                drift.append((f, stated, len(found), found))
            issues = {t: iss for t, st, iss in entries}
            live += [(f, t, issues.get(t, [])) for t in found]
        print(f"{len(regs)} chapters with a Deviations section · "
              f"{len(live)} open entries · {len(drift)} chapter(s) whose count disagrees\n")
        for f, stated, found, tags in drift:
            print(f"  {f}: states OPEN: {stated}, lists {found} "
                  f"({', '.join(tags) if tags else 'none'})")
        if want_issues:
            named = {int(n) for _, _, iss in live for n in iss}
            done, unreachable = closed_issues(named)
            print(f"\n{len(named)} issue(s) named by an open entry, {len(done)} now CLOSED")
            if unreachable:
                print(f"  ⚠ {len(unreachable)} could not be asked "
                      f"({', '.join('loft#%d' % n for n in unreachable)}) — unknown, not clean")
            for f, tag, iss in live:
                hit = [int(n) for n in iss if int(n) in done]
                if hit:
                    print(f"  {f}: {tag} is OPEN but names "
                          f"{', '.join('loft#%d' % n for n in hit)}, CLOSED — re-measure")
        # A REPORT, not a gate, and deliberately so even though the count half is offline and
        # deterministic.  This parser surprised its author twice on the day it was written — a
        # fixed head window read `D-op-2` and `D-heap-LIFO` as closed, and `heap.md` states an
        # entry beside the rule it qualifies rather than in the register — so it has not yet
        # earned the right to stop a build.  Let it run clean for a while first, then graduate it
        # the way `check` gates — a `doc_hygiene` test shelling out to this same command, so the
        # gate and the tool cannot drift (`every_rule_citation_resolves` is the pattern).
        return 0

    if cmd == "dups":
        multi = {t: v for t, v in cites.items() if len({f for f, _ in v}) >= 2}
        print(f"{len(multi)} rule(s) cited from 2+ files\n")
        for tag, where in sorted(multi.items(), key=lambda kv: -len(kv[1])):
            print(f"[{len(where):2d} sites] @FR-{tag}")
            for f, n in where:
                print(f"           {f}:{n}")
        return 0

    # check
    problems = []
    implementing = 0
    for tag, files in rules.items():
        if len(files) > 1:
            problems.append(f"@FR-{tag} defined in {len(files)} docs: {', '.join(files)}")
    for tag, where in sorted(cites.items()):
        if tag in devs:
            if devs[tag][1] == "CLOSED":
                for f, n in where:
                    problems.append(
                        f"{f}:{n}: cites @FR-{tag}, a CLOSED deviation — that is history, not "
                        "law.  Cite the rule it was measured against.")
            else:
                implementing += len(where)
        elif tag not in rules:
            for f, n in where:
                problems.append(f"{f}:{n}: cites @FR-{tag}, which is not a defined rule")
    cited = sum(1 for t in cites if t in rules)
    n_open = sum(1 for v in devs.values() if v[1] == "OPEN")
    # This count and `registers`' are now the SAME set — both read `defined_deviations`, in all
    # three entry spellings.  They differ in the question asked, not the number: this one decides
    # whether a CITATION is legal; `registers` checks each chapter's own stated `OPEN: n` against
    # it.  They disagreed while this one could not see the bullet form, and that is what let four
    # closed deviations sit in the register reading OPEN.
    print(f"{len(rules)} defined rules · {cited} cited · "
          f"{sum(len(v) for v in cites.values())} citation sites · "
          f"{len(devs)} deviation entries ({n_open} open; "
          f"`registers` checks the chapters' stated counts against these), "
          f"{implementing} site(s) implementing one")
    if problems:
        print(f"\n{len(problems)} problem(s):")
        for p in problems:
            print(f"  {p}")
        return 1
    print("ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
