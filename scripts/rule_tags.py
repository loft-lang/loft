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
#   registers      each chapter's stated `OPEN: n` vs the entries it lists, and every open
#                  entry that names no tracking issue (`loft#N`) and is not marked
#                  `not resolvable in a release`; `--issues` also asks whether an open
#                  entry's issue has closed
#   dups           rules cited from 2+ sites — the duplication question, asked by MEANING
#                  rather than by code shape (which is what rule_predicate_audit.py does)
#   claims         the LIMITATION sentences of the hand-written reference docs (LOFT*.md,
#                  STDLIB*.md, CAVEATS.md, the loft-write skill) that cite a `loft#N`, each with
#                  the guard `tests/scripts/N-*.loft` that would say which way N went; with
#                  `--issues` asks the tracker which of those issues have CLOSED — a limitation
#                  still on the page after its issue closed is what routes every agent around a
#                  feature that works (@PLN176); `--gate` fails on any such sentence.  The
#                  nightly runs it (`stale-claims`); `make ci` prints the offline half.  A
#                  lifted limitation LEAVES the page (a new reader is not helped by what loft
#                  could not do last month) and its record goes to `<doc>-history.md`
#   fences         every `loft` code fence in the hand-written reference docs, and the executed
#                  program it is cut from: the line above a fence reads `<!-- from <path> -->`
#                  and the fence must be a VERBATIM window of that file (indent-normalised), so
#                  a sample on the page is code that runs in `make ci`.  The two comparison
#                  pages are read the same way, each loft block against
#                  `tests/comparisons/<section id>.loft`.  A fence that is a shape, a grammar or
#                  a signature listing rather than a program is a ```grammar fence and is not
#                  asked.  `--gate` fails a fence with no source or one that has drifted
#                  (@PLN176 phase 2; the walk that gives every fence a home is the plan's)
#   sections       every `##`/`###` section of LOFT*.md and STDLIB*.md, and what KEEPS it: a
#                  sourced sample, an `@FR-`/`(Rule)` or `@F` citation, a signature table the
#                  stdlib source resolves, a guard path or a `loft#N`; a section with none is
#                  prose nothing in the repo would contradict.  Also names a signature row no
#                  `default/*.loft` declaration matches.  `--gate` fails on either (@PLN176
#                  phase 3)
#   coverage       what share of the rules carry a code ANNOTATION and what share carry an
#                  active GUARD, against the contract-1 FLOORS — the command a doc links to
#                  INSTEAD of writing a position down.  A measured position is stale the
#                  moment it is committed; a floor is not, so the floor is the only number
#                  worth putting in prose.  Floors are MINIMUMS for the freeze, not targets
#                  to stop at.  A REPORT: always exit 0.

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
# `default/*.loft` is code too: the `#rust"…"` operator templates there are the SHARED half of
# both backends (`src/fill.rs` is generated from them, and the native emitter pastes them), so
# a rule a template enforces is cited THERE, once, and nowhere in `src/`.  Scanning `src/`
# alone read `@FR-H-WriteNull` as unannotated while its one home carried the tag (2026-09-25).
DEFAULT = os.path.join(ROOT, "default")
CITE_DIRS = (os.environ["CITE_DIRS"].split(":")
             if os.environ.get("CITE_DIRS") else [SRC, DEFAULT])
CITE_EXTS = os.environ.get("CITE_EXTS", ".rs,.loft").split(",")

# `coverage`'s second tier and the contract-1 MINIMUM THRESHOLDS it reports against.  These
# are floors, not targets: the owner's informed estimate of the least coverage that could earn
# the `CONTRACT_VERSION` 0 -> 1 freeze, and the work goes on past them.  They are the only
# figures worth committing to prose — a position measured today is stale tomorrow, so a doc
# links to this command rather than restating a number that then rots in several homes at
# once.  Both are env-overridable, and GUARD_DIRS mirrors CITE_DIRS so a vendoring project
# points them at its own layout.  Guards cite from `.loft` as well as `.rs`.
MIN_ANNOTATED = int(os.environ.get("RULE_MIN_ANNOTATED", "70"))
MIN_GUARDED = int(os.environ.get("RULE_MIN_GUARDED", "40"))
TESTS = os.path.join(ROOT, "tests")
GUARD_DIRS = (os.environ["GUARD_DIRS"].split(":")
              if os.environ.get("GUARD_DIRS") else [TESTS])
GUARD_EXTS = os.environ.get("GUARD_EXTS", ".rs,.loft").split(",")

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
    m = REG_SECTION.search(text)
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
# A chapter's register heading.  Most write `## Deviations`; a chapter with numbered sections
# writes `## 3. Deviations / decided edges` (collections.md), and read as the bare spelling alone
# its stated `OPEN:` was never checked.  `doc_history_report.py` reads both spellings the same way.
REG_SECTION = re.compile(r"^## (?:\d+\.\s*)?Deviations\b[^\n]*$", re.M)
REG_OPEN = re.compile(r"OPEN:\s*\**\s*(\d+)")
# An entry inside that section, in every spelling the docs use.  The BULLET form is the one
# `defined_deviations` cannot see (it reads headings and blockquotes only), and it is how
# `operational`, `layout`, `matching`, `tuples` and `calls` write every entry they own — so
# those chapters' open entries were invisible to the tool that counts them.  A bullet is written
# with `-` or with `*` (`binding.md` uses `*`), and both are read.
REG_ENTRY_STRICT = re.compile(
    rf"^(?:#{{2,5}}\s+`?|>\s*\*\*`?)(?P<tag>{DEV_TAG})(?![A-Za-z0-9_'-])", re.M)
REG_ENTRY_BULLET = re.compile(
    rf"^[-*]\s+\*\*`?(?P<tag>{DEV_TAG})(?![A-Za-z0-9_'-])", re.M)
REG_ISSUE = re.compile(r"loft#(\d+)")
# A deviation a PLAN is building names the plan instead of an issue: the plan is the tracker
# for planned work, and a second issue beside it would only be a stale copy of its steps.
REG_PLAN = re.compile(r"@(PLN\d+)(?!\d)")
# The ATTRIBUTION — the first parenthetical of an entry's head, which is where the dates and
# the issue it was filed as are written.  Bounded so it cannot run past the head into the
# explanation; see `_tracking_issues`.
REG_ATTRIBUTION = re.compile(r"^[^(]{0,80}\(([^)]*)\)")
# An open entry that no release can close says so in its head, with its reason in the entry.
UNRESOLVABLE = "not resolvable in a release"


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
        m = REG_SECTION.search(text)
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


def _tracking_issues(head):
    """The issues an entry was FILED AS — not every issue its head mentions.

    An entry's head opens with an ATTRIBUTION — `*(opened 2026-09-24, CLOSED 2026-09-24;
    loft#1664)*`, `(closed 2026-09-07, loft#1446)`, `(2026-09-17, NARROWED 2026-09-21, …,
    loft#1563, loft#1645)` — and then explains itself.  The explanation cross-references OTHER
    issues, because that is what an explanation does: the record former of the same question,
    the stopgap that came before, the split this one survives.  Reading the whole head makes
    every such cross-reference a tracking issue, and a CLOSED cross-reference then reports as a
    pair to re-measure that nobody owes.

    Measured 2026-09-25: of the four open entries the register carried, the single re-measure
    line `registers --issues` printed was exactly that — `D-col-6`, tracked by the OPEN
    loft#1664, naming the CLOSED loft#1662 in its own sentence.  Four of the eight heads that
    name more than one issue are this shape.  A report whose only line is a false positive
    teaches its reader to skip the line, which is the failure that matters here.

    So the attribution decides, WHEN IT NAMES AN ISSUE AT ALL.  `heap.md` writes some entries
    with the date alone in the parenthetical and the issue at the end of the sentence
    (`— CLOSED (2026-09-20): a collection … (loft#1551)`), and a head may open with a rule name
    in parentheses instead, so an attribution carrying no issue falls back to the whole head.
    That fallback is what makes the change safe in the direction that matters: it can DISCARD a
    cross-reference, never lose the tracker.
    """
    m = REG_ATTRIBUTION.search(head)
    named = REG_ISSUE.findall(m.group(1)) if m else []
    if named or REG_ISSUE.findall(head):
        return named or REG_ISSUE.findall(head)
    return REG_PLAN.findall(m.group(1)) if m else []


def _register_entries(body, pattern):
    """{tag: (tag, status, [issues], unresolvable)} for one scan of one body.

    `unresolvable` is the head's own word that no release can close the entry — the marker
    `not resolvable in a release`, which the entry must justify.  Every other open entry owes a
    tracking issue: an open deviation the tracker does not know about is deferred work nobody
    was told about.
    """
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
                               _tracking_issues(head),
                               UNRESOLVABLE in " ".join(head.lower().split()))
    return out


def open_register():
    """`(live, drift, regs)` — every OPEN deviation as `(chapter, tag, [issues], unresolvable)`,
    the chapters whose stated `OPEN: n` disagrees with what they list, and the raw registers.

    The one home for "which deviations are open, and is each tracked?", read by `registers` and
    by `scripts/release-checklist.py`'s deviation item, so the report and the release gate
    cannot disagree about what is open.

    Status comes from `defined_deviations`, the ONE home for it: it resolves a tag with several
    entries by date, which a fresh scan does not — `D-bind-11` and `D-bind-28` each read OPEN in
    isolation and are closed once their later rows are taken into account.  It also attributes a
    chapter that keeps its register in the `-history` companion, which is how `types.md` stated
    `OPEN: 0` over an open `D-Domain-Guard` next door.
    """
    regs = chapter_registers()
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
        issues = {t: iss for t, st, iss, _ in entries}
        marked = {t for t, st, iss, unres in entries if unres}
        live += [(f, t, issues.get(t, []), t in marked) for t in found]
    return live, drift, regs


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
    import shutil
    import subprocess
    out, unreachable = {}, []
    # Without `gh` the REST endpoint answers the same two fields; a public tracker needs no
    # token, and `GH_TOKEN` is sent when set.  The fallback exists so a box that has no `gh`
    # (an agent container) is not silently "unreachable" on every number.
    have_gh = shutil.which("gh") is not None
    repo = os.environ.get("LOFT_REPO", "loft-lang/loft")
    for n in sorted(numbers):
        try:
            if have_gh:
                r = subprocess.run(["gh", "issue", "view", str(n), "--json", "state,title"],
                                   capture_output=True, text=True, timeout=30)
                if r.returncode:
                    unreachable.append(n)
                    continue
                d = json.loads(r.stdout)
                state = d.get("state")
            else:
                import urllib.request
                req = urllib.request.Request(
                    f"https://api.github.com/repos/{repo}/issues/{n}",
                    headers={"Accept": "application/vnd.github+json",
                             "User-Agent": "loft-rule-tags"})
                if os.environ.get("GH_TOKEN"):
                    req.add_header("Authorization", f"Bearer {os.environ['GH_TOKEN']}")
                with urllib.request.urlopen(req, timeout=30) as resp:
                    d = json.loads(resp.read().decode("utf-8"))
                state = (d.get("state") or "").upper()
            if state == "CLOSED":
                out[n] = d.get("title", "")
        except Exception:
            unreachable.append(n)
    return out, unreachable


def citations_in(dirs, exts):
    """{tag: [(file, line)]} for every `@FR-` citation under `dirs` with those `exts`.

    The ONE collector.  `coverage` asks it twice with different roots rather than growing a
    second scanner beside it — two decoders of one question disagree eventually, and here
    they would disagree about what counts as enforcing a rule.
    """
    out = collections.defaultdict(list)
    for d in dirs:
        for ext in exts:
            for path in glob.glob(os.path.join(d, "**/*" + ext), recursive=True):
                for n, line in enumerate(open(path, encoding="utf-8", errors="replace"), 1):
                    for tag in CITE.findall(line):
                        out[tag].append((_rel(path), n))
    return out


def citations():
    """{tag: [(file, line)]} for every `@Tag` in the citation dirs (default: src/*.rs)."""
    return citations_in(CITE_DIRS, CITE_EXTS)


# ---- claims: the limitation sentences of the hand-written reference docs (@PLN176) ----
#
# A limitation written on LOFT.md is what an agent reads first and believes, so a sentence
# that still says "does not work (loft#N)" after N closed routes every consumer around a
# feature that works.  The sentence goes stale on the day the ISSUE closes — an event outside
# any commit — which is why the check that asks the tracker runs in the nightly and not only
# at push time.  Measured 2026-09-28: of five limitation sentences in LOFT.md citing a closed
# issue, two described a bug fixed three weeks earlier, each with a regression guard in
# tests/scripts asserting the opposite of the page.
#
# The docs are the hand-written ones a reader takes as the definition; the generated pages
# (tests/docs/*.loft) run and need no such check.  Env-overridable for a vendoring project.
# The hand-written reference is a FAMILY of files — LOFT.md and STDLIB.md split by subject into
# LOFT_*.md / STDLIB_*.md, the loft-write skill into several pages — so each list below is
# globbed rather than named: a split that adds a file keeps it checked.
def _family(*patterns):
    import glob
    return sorted({p for pat in patterns for p in glob.glob(pat)
                   if not p.endswith("-history.md") and p not in NOT_REFERENCE})


# Named like the family, about something else: the `loft test` runner.
NOT_REFERENCE = {"doc/claude/LOFT_TEST.md"}


REFERENCE_DOCS = ("doc/claude/LOFT.md", "doc/claude/LOFT_*.md",
                  "doc/claude/STDLIB.md", "doc/claude/STDLIB_*.md")
CLAIM_DOCS = (os.environ["CLAIM_DOCS"].split(":") if os.environ.get("CLAIM_DOCS")
              else _family(*REFERENCE_DOCS, "doc/claude/CAVEATS.md",
                           ".claude/skills/loft-write/*.md"))
ISSUE_CITE = re.compile(r"loft#(\d+)")
# A sentence is a LIMITATION when it says something does not work, is refused, needs a
# workaround, or holds "until" something — the forms the two stale callouts took.
CLAIM_LIMIT = re.compile(
    r"\b(does not|doesn't|do not|cannot|can't|not supported|unsupported|refus\w*|until"
    r"|workaround|work around|known|silently|fails?|crash\w*|wrong|missing|dropped"
    r"|limit\w*|not (?:yet )?(?:parse|work|accept|reach)\w*)\b", re.I)
# ... and it is HISTORY, not a limitation, when the sentence (or the words just before the
# citation) puts the fault in the past: that is how a closed issue is cited on purpose, and
# the doc contract asks for exactly one of these words beside such a citation (rule 23).
CLAIM_PAST = re.compile(
    r"\b(since|fixed|closed|before|used to|was|were|had|previously|no longer|(?<!for )now"
    r"|cured|landed|made|corrected|resolved)\b", re.I)   # "for now" is a HEDGE, not history


def _claim_paragraphs(lines):
    """(first line number, [lines]) per blank-separated block; blockquote / bullet / table
    prefixes are stripped so the sentence reads as one string."""
    buf, start = [], 1
    for i, line in enumerate(lines, 1):
        if line.strip() == "":
            if buf:
                yield start, buf
            buf = []
        else:
            if not buf:
                start = i
            buf.append(re.sub(r"^\s*(?:>\s*|[-*]\s+|\|\s*)*", "", line).strip())
    if buf:
        yield start, buf


def _claim_sentence(text, at, end):
    """The sentence around a citation: from the previous `. ` to the next one."""
    a = text.rfind(". ", 0, at)
    b = text.find(". ", end)
    return text[(a + 2 if a >= 0 else 0):(b + 1 if b >= 0 else len(text))].strip()


def classify_claim(sentence, before=""):
    """('limit' | 'past' | 'neutral', the word that decided it).

    `before` is the text just ahead of the citation in the same paragraph — "Since loft#N"
    puts the marker outside the sentence's own words when the citation opens it."""
    past = CLAIM_PAST.search(before[-60:]) or CLAIM_PAST.search(sentence)
    if past:
        return "past", past.group(0)
    lim = CLAIM_LIMIT.search(sentence)
    if lim:
        return "limit", lim.group(0)
    return "neutral", ""


def _rel(path):
    """`path` relative to the repository root, or as given when it cannot be: on Windows a
    path on another drive (a self-test's temp directory on C: under a checkout on D:) has no
    relative form, and `os.path.relpath` raises for it."""
    try:
        return os.path.relpath(path, ROOT)
    except ValueError:
        return path


def guard_for(n, tests_dir=None):
    """The guard that speaks for issue N: `tests/scripts/N-*.loft` (or `N<letter>-*`), else
    the first test file that cites `loft#N` — else None.  The guard is the evidence of which
    way the issue went, so a closed issue WITH a guard is a re-read with its answer beside
    it, and one without is unkept."""
    tests_dir = tests_dir or TESTS
    hits = sorted(glob.glob(os.path.join(tests_dir, "scripts", f"{n}-*.loft")) +
                  glob.glob(os.path.join(tests_dir, "scripts", f"{n}[a-z]-*.loft")))
    if hits:
        return _rel(hits[0])
    needle = re.compile(rf"loft#{n}(?!\d)")
    for path in sorted(glob.glob(os.path.join(tests_dir, "**", "*"), recursive=True)):
        # `.expect` is the error-message baseline: a refusal's guard lives there.
        if not path.endswith((".loft", ".rs", ".expect")):
            continue
        try:
            with open(path, encoding="utf-8", errors="replace") as fh:
                if needle.search(fh.read()):
                    return _rel(path)
        except OSError:
            continue
    return None


def claim_sites(docs=None):
    """Every `loft#N` citation in the reference docs, classified:
    [(file, line, n, kind, word, sentence)]."""
    out = []
    for rel in (docs or CLAIM_DOCS):
        path = os.path.join(ROOT, rel)
        if not os.path.exists(path):
            continue
        with open(path, encoding="utf-8") as fh:
            lines = fh.read().split("\n")
        for start, buf in _claim_paragraphs(lines):
            text = " ".join(buf)
            for m in ISSUE_CITE.finditer(text):
                sentence = _claim_sentence(text, m.start(), m.end())
                kind, word = classify_claim(sentence, text[:m.start()])
                out.append((rel, start, int(m.group(1)), kind, word, sentence))
    return out


# ---- fences: every code sample in a hand-written reference doc is a slice of a program that
# runs (@PLN176 phase 2) ----
#
# A sample that nothing runs is the other way a page goes stale: the language moves, the
# fence keeps the old spelling, and the reader learns it.  LOFT.md carried 65 `loft` fences
# and STDLIB.md 29 with no program behind any of them (measured 2026-09-28: one had a
# `fn main`, 25 of the 94 ran as written, the rest were excerpts assuming context).  So a
# fence NAMES its program and must match a window of it verbatim; the program asserts.
FENCE_DOCS = (os.environ["FENCE_DOCS"].split(":") if os.environ.get("FENCE_DOCS")
              else _family(*REFERENCE_DOCS, "doc/claude/CAVEATS.md",
                           ".claude/skills/loft-write/*.md"))
FENCE_PAGES = (os.environ["FENCE_PAGES"].split(":") if os.environ.get("FENCE_PAGES")
               else ["doc/00-vs-rust.html", "doc/00-vs-python.html"])
FENCE_FROM = re.compile(r"<!--\s*from\s+(\S+)\s*-->")


def _dedent(lines):
    """Trailing whitespace off, blank edges off, the common leading indent off."""
    out = [l.rstrip() for l in lines]
    while out and not out[0]:
        out.pop(0)
    while out and not out[-1]:
        out.pop()
    ind = min((len(l) - len(l.lstrip()) for l in out if l.strip()), default=0)
    return [l[ind:] for l in out]


def fence_matches(fence_lines, source_lines):
    """The 1-based line in `source_lines` where the fence starts, else 0.

    A window of the source is compared after ITS common indent is removed, so a fence cut
    from inside a `fn main` body matches although the file indents it — and blank lines
    inside the fence must be blank in the source too, so a fence cannot be assembled from
    two places."""
    f = _dedent(fence_lines)
    if not f:
        return 0
    src = [l.rstrip() for l in source_lines]
    n = len(f)
    for i in range(len(src) - n + 1):
        if _dedent(src[i:i + n]) == f:
            return i + 1
    return 0


def markdown_fences(text):
    """[(line of the ``` opener, info string, [body lines], source path or None)]."""
    lines = text.split("\n")
    out, i = [], 0
    while i < len(lines):
        m = re.match(r"^\s*```([A-Za-z-]*)\s*$", lines[i])
        if m and m.group(1):
            j = i + 1
            while j < len(lines) and not re.match(r"^\s*```\s*$", lines[j]):
                j += 1
            src = None
            k = i - 1
            while k >= 0 and not lines[k].strip():
                k -= 1
            if k >= 0:
                fm = FENCE_FROM.search(lines[k])
                if fm:
                    src = fm.group(1)
            out.append((i + 1, m.group(1), lines[i + 1:j], src))
            i = j
        i += 1
    return out


def html_loft_blocks(text):
    """[(line, section id, [code lines])] for the loft side of a comparison page: the
    `<pre><code>` blocks with no class (the other language's carry one), text unescaped."""
    import html as _html
    out = []
    section = ""
    pos = 0
    for m in re.finditer(r'<h2 id="([^"]+)"|<pre(?P<attrs>[^>]*)><code>(?P<body>.*?)</code></pre>',
                         text, re.S):
        if m.group(1):
            section = m.group(1)
            continue
        if "class=" in (m.group("attrs") or ""):
            continue
        body = re.sub(r"<[^>]+>", "", m.group("body"))
        body = _html.unescape(body)
        line = text.count("\n", 0, m.start()) + 1
        out.append((line, section, body.split("\n")))
    return out


def fence_report(docs=None, pages=None):
    """[(file, line, status, source, detail)] — status is ok / drift / unsourced / missing."""
    rows = []
    for rel in (docs or FENCE_DOCS):
        path = os.path.join(ROOT, rel)
        if not os.path.exists(path):
            continue
        for line, info, body, src in markdown_fences(open(path, encoding="utf-8").read()):
            if info != "loft":
                continue
            if not src:
                rows.append((rel, line, "unsourced", "", body[0].strip()[:60] if body else ""))
                continue
            if src.startswith("library:"):
                # kept by that library's own testbed (its CI); listed, never gated here
                rows.append((rel, line, "library", src, "kept by the library's testbed"))
                continue
            sp = os.path.join(ROOT, src)
            if not os.path.exists(sp):
                rows.append((rel, line, "missing", src, "names a file that does not exist"))
                continue
            at = fence_matches(body, open(sp, encoding="utf-8").read().split("\n"))
            rows.append((rel, line, "ok" if at else "drift", src,
                         f"line {at}" if at else "not a verbatim window of the file"))
    for rel in (pages or FENCE_PAGES):
        path = os.path.join(ROOT, rel)
        if not os.path.exists(path):
            continue
        for line, section, body in html_loft_blocks(open(path, encoding="utf-8").read()):
            src = f"tests/comparisons/{section}.loft"
            sp = os.path.join(ROOT, src)
            if not os.path.exists(sp):
                rows.append((rel, line, "missing", src, f"#{section}: no comparison program"))
                continue
            at = fence_matches(body, open(sp, encoding="utf-8").read().split("\n"))
            rows.append((rel, line, "ok" if at else "drift", src,
                         f"line {at}" if at else f"#{section}: not a verbatim window"))
    return rows


# ---- sections: does every section of a reference page have something that keeps it? ----
#
# A fence is kept by the program it is cut from (`fences`) and a limitation sentence by its
# issue (`claims`); a SECTION is the unit a reader trusts, and one can be all prose — a rule
# restated from memory, a table of signatures typed by hand — with nothing in the repo that
# would move when the language does.  A section is KEPT when its body carries at least one of:
#   - a sourced ```loft fence (`<!-- from … -->`), a program `make ci` runs;
#   - a formal-rule citation, `@FR-X` or the parenthesised `(X)` form, resolving to a DEFINED
#     rule — the formal chapter and its guards then own the claim;
#   - a feature citation `@F<n>` / `@I<n>` — the catalogue issue and its generated example;
#   - a signature table: a `| \`name(...)\` |` row whose name a `default/*.loft` declaration
#     resolves — the stdlib source is the one home, so a renamed or removed routine is a row
#     that no longer resolves (reported by name);
#   - a path under tests/ that exists (a guard named outright), or a `loft#N` citation (the
#     `claims` check owns its truth).
# A pure index (Contents, See also) is exempt by name.  Everything else with none of these is
# UNKEPT: prose nothing in the repo would contradict when it goes stale.  The body of a
# section runs to the next heading of the same or a higher level, so a `##` with `###`
# children is kept by any child; each child is measured on its own.
SECTION_DOCS = (os.environ["SECTION_DOCS"].split(":") if os.environ.get("SECTION_DOCS")
                else _family(*REFERENCE_DOCS))
# A pure index, and the two sections whose subject is not the language: STDLIB's note on how
# the stdlib is implemented (a maintainer pointer) and its ledger of routines proposed and not
# yet written, whose unresolved names are the content.
SECTION_INDEX = {"Contents", "See also", "Implementation notes", "Open work",
                 # an INFORMAL summary of the grammar: every production it lists is exercised
                 # by the parser tests as a whole, and no one guard holds one line of it
                 "Summary of grammar"}
FEATURE_CITE = re.compile(r"@[FI]\d+\b")
PAREN_RULE = re.compile(r"\(([A-Z][A-Za-z]*(?:-[A-Za-z0-9]+)+)\)")
# A signature is a backticked `name(` — or `x.name(` — whose name is at least three letters:
# the one-letter `f(x)`, the `fn(…)` type spelling and the field modifiers (`limit(0, 255)`,
# `size(1)`) read like calls and are not routines.  `f#read(` is an operator form, skipped.
SIG_NAME = re.compile(r"`(?:[a-z_][a-z0-9_]*\.)?([a-z_][a-z0-9_]{2,})\(")
SIG_PSEUDO = {"func", "method", "takes", "limit", "size", "default", "virtual", "computed",
              "init", "check", "assert", "panic", "http"}
# Names the PARSER lowers rather than the stdlib declaring them — each verified against its
# site: `remove`/`clear` (src/parser/fields.rs), `type_of`/`field_value`
# (src/parser/control.rs), `sizeof`/`type_name` (src/parser/objects.rs).
PARSER_BUILTINS = {"remove", "clear", "type_of", "field_value", "sizeof", "type_name"}
TYPE_NAME = re.compile(r"`([A-Z][A-Za-z0-9]*)(?:\.[A-Z][A-Za-z0-9]*)?`")
CONST_NAME = re.compile(r"`([A-Z][A-Z0-9_]+)`")
TEST_PATH = re.compile(r"\b((?:tests|default)/[A-Za-z0-9_./-]+\.(?:loft|rs|expect))\b")
LIB_SECTION = re.compile(r"`use\s+([a-z_][a-z0-9_]*)\s*;`")


def stdlib_names():
    """Every function, struct, enum, interface and type name `default/*.loft` declares."""
    names = set()
    decl = re.compile(r"^\s*(?:pub\s+)?(?:fn|struct|enum|interface|type|value struct)\s+"
                      r"([A-Za-z_][A-Za-z0-9_]*)", re.M)
    const = re.compile(r"^\s*(?:pub\s+)?(?:const\s+)?([A-Z][A-Z0-9_]+)\s*=", re.M)
    for path in glob.glob(os.path.join(ROOT, "default", "*.loft")):
        text = open(path, encoding="utf-8").read()
        names.update(decl.findall(text))
        names.update(const.findall(text))
    return names | PARSER_BUILTINS


def doc_sections(text):
    """[(line, level, title, [body lines])] — the body runs to the next heading of the same
    or a higher level, fenced blocks included as text (a heading inside a fence is not one)."""
    lines = text.split("\n")
    heads = []
    fence = False
    for i, l in enumerate(lines):
        if re.match(r"^\s*```", l):
            fence = not fence
            continue
        if fence:
            continue
        m = re.match(r"^(#{2,4})\s+(.*?)\s*$", l)
        if m:
            heads.append((i, len(m.group(1)), m.group(2)))
    out = []
    for k, (i, lvl, title) in enumerate(heads):
        end = len(lines)
        for j, l2, _ in heads[k + 1:]:
            if l2 <= lvl:
                end = j
                break
        out.append((i + 1, lvl, title, lines[i + 1:end]))
    return out


def section_report(docs=None, rules=None):
    """[(file, line, level, title, keepers, unresolved)] — `keepers` is the list of what keeps
    the section (empty = UNKEPT); `unresolved` the signature names no stdlib declaration has."""
    rules = rules if rules is not None else defined_rules()
    names = stdlib_names()
    rows = []
    for rel in (docs or SECTION_DOCS):
        path = os.path.join(ROOT, rel)
        if not os.path.exists(path):
            continue
        rows.extend(_section_rows(open(path, encoding="utf-8").read(), rules, names, rel))
    return rows


def _is_pointer(body):
    """A section a split left behind: a sentence or two naming what moved and a link to the
    family file that now holds it.  Its content is kept where the link points."""
    lines = [l for l in body if l.strip()]
    if not lines or len(lines) > 3 or any(l.lstrip().startswith("```") for l in lines):
        return False
    family = {os.path.basename(d) for d in SECTION_DOCS}
    targets = re.findall(r"\]\(([A-Za-z0-9_\-]+\.md)(?:#[^)]*)?\)", " ".join(lines))
    return any(t in family for t in targets)


def _section_rows(text, rules, names, rel):
    """`section_report` over one doc's text — separate so a selftest can hand it a string."""
    rows = []
    if True:
        for line, lvl, title, body in doc_sections(text):
            plain = re.sub(r"\s*\(.*\)\s*$", "", re.sub(r"[`~*]", "", title)).strip()
            if plain in SECTION_INDEX or _is_pointer(body):
                rows.append((rel, line, lvl, title, ["index"], []))
                continue
            joined = title + "\n" + "\n".join(body)
            keepers, unresolved = [], []
            lib = LIB_SECTION.search(title)
            if lib:
                # a library's section: its testbed keeps the signatures, not default/*.loft
                rows.append((rel, line, lvl, title, [f"library {lib.group(1)}"], []))
                continue
            n_from = sum(1 for _, info, _, src in markdown_fences(joined)
                         if info == "loft" and src)
            if n_from:
                keepers.append(f"{n_from} sourced sample(s)")
            fr = {t for t in CITE.findall(joined) if t in rules}
            fr |= {t for t in PAREN_RULE.findall(joined) if t in rules}
            if fr:
                keepers.append("rule " + ", ".join(sorted(fr)[:3]) + (" …" if len(fr) > 3 else ""))
            feats = sorted(set(FEATURE_CITE.findall(joined)))
            if feats:
                keepers.append("feature " + ", ".join(feats[:3]))
            sig_ok, sig_bad = set(), set()
            own = True
            for l in body:
                if re.match(r"^#{2,4} ", l):
                    own = False  # a child's rows are the child's to answer for
                row = l.lstrip().startswith("|") and own
                for n in SIG_NAME.findall(l.replace("#", "#!")):
                    if n in SIG_PSEUDO:
                        continue
                    if n in names:
                        sig_ok.add(n)
                    elif row:
                        sig_bad.add(n)
                if not (l.lstrip().startswith("|")):
                    continue
                if row:
                    # a table row naming a stdlib TYPE (`Format.TextFile`, `File`) or CONSTANT
                    sig_ok.update(t for t in TYPE_NAME.findall(l) if t in names)
                    sig_ok.update(c for c in CONST_NAME.findall(l) if c in names)
            if sig_ok:
                keepers.append(f"{len(sig_ok)} stdlib name(s)")
            unresolved = sorted(sig_bad)
            paths = sorted({p for p in TEST_PATH.findall(joined)
                            if os.path.exists(os.path.join(ROOT, p))})
            if paths:
                keepers.append("guard " + paths[0] + (" …" if len(paths) > 1 else ""))
            issues = sorted(set(re.findall(r"loft#(\d+)", joined)))
            if issues:
                keepers.append("issue loft#" + ", loft#".join(issues[:3]))
            rows.append((rel, line, lvl, title, keepers, unresolved))
    return rows


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

    if cmd == "coverage":
        # Two tiers, because they answer different questions.  ANNOTATION says a code site
        # claims to enforce the rule, which makes "where is this enforced?" a grep.  GUARD is
        # the stronger one and is still a PROXY: a test citing a rule means a guard NAMES it,
        # never that the guard would fail without it.  Say that in the output rather than let
        # the column heading promise a property nobody measured.
        ann = {t for t in citations_in(CITE_DIRS, CITE_EXTS) if t in rules}
        grd = {t for t in citations_in(GUARD_DIRS, GUARD_EXTS) if t in rules}
        total = len(rules)
        pct = (lambda k: 100.0 * k / total if total else 0.0)
        print(f"{total} defined rules\n")
        for label, got, floor in (("code annotation", ann, MIN_ANNOTATED),
                                  ("active guard", grd, MIN_GUARDED)):
            have = pct(len(got))
            short = max(0, int(floor * total / 100 + 0.999) - len(got))
            mark = "floor met" if have >= floor else f"{short} rule(s) short of {floor} %"
            print(f"  {label:<16} {len(got):>4} / {total}   {have:5.1f} %   "
                  f"contract-1 floor {floor} %  ({mark})")
        print(f"\n  both tiers       {len(ann & grd):>4} / {total}   {pct(len(ann & grd)):5.1f} %")
        print(f"  neither          {len(rules.keys() - ann - grd):>4} / {total}   "
              f"{pct(len(rules.keys() - ann - grd)):5.1f} %")
        print("\nA guard tier counts a rule NAMED by a test, which is not the same as a test "
              "that\nwould fail without it — read it as the weaker claim.  The floors are "
              "contract-1 MINIMUMS,\nnot targets to stop at, and are informed estimates the "
              "owner may move (RULE_MIN_ANNOTATED,\nRULE_MIN_GUARDED).  Crossing them is "
              "necessary for the freeze, never sufficient —\nCOMPATIBILITY.md § The road to "
              "contract 1.  A report, never a gate.")
        # The denominator is EVERY defined rule, which quietly asserts that every rule could
        # carry a guard — and not all warrant one (`performance.md`, `capabilities.md` and
        # `concurrency.md` state how to MEASURE or what a capability IS, not what a program
        # does).  So the shortfall above is an upper bound on the real distance.  Said here
        # rather than left silent, because a metric that cannot reach its own floor reads
        # identically to one that simply has not yet.
        print("\n⚠ The share is over ALL defined rules, some of which warrant no guard at all, "
              "so the\n  distance to the guard floor is an UPPER bound — and it overstates the "
              "WORK besides:\n  a tail rule the existing tests already validate closes with an "
              "ANNOTATION, not with\n  verification.  No exclusion set exists, and one is not "
              "being sized up front: the owner\n  judges it against the progress made and the "
              "bugs actually met (2026-09-17).")
        return 0

    cites = citations()

    if cmd == "sites":
        if len(sys.argv) < 3:
            print("usage: rule_tags.py sites <@FR-Tag>", file=sys.stderr)
            return 2
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

    if cmd == "selftest":
        # The cells for `_tracking_issues`, which decides which issue an open entry is asking
        # the tracker about.  They are here rather than in a test file because the question is
        # about TEXT the docs already write in four shapes, and a cell is one string; the
        # `check` subcommand is gated by `doc_hygiene::every_rule_citation_resolves`, and this
        # rides the same route.
        #
        # Each was proven able to fail before it was kept, against a DIFFERENT break: c1 and c6
        # against reading the whole head (the defect this closes), c3 and c5 against reading
        # the attribution ALONE with no fallback, c2 against taking only the first issue, and
        # c4 against a fallback that answers something for a head naming nothing.
        cells = [
            ("c1 attribution, then a cross-reference in the prose (D-col-6)",
             "** - OPEN, opened 2026-09-24 (loft#1664): a write through a payload BINDING "
             "stops reaching the other members, which is loft#1662 split surviving.",
             ["1664"]),
            ("c2 an attribution naming several (D-bind-55)",
             "** *(opened 2026-09-23, CLOSED 2026-09-24; loft#1566, loft#1602, @PLN167 C3)* "
             "- `(B-Ref-Lvalue)` with `(F-ParamRef)`: a text field handed to a `&text` "
             "parameter was refused.",
             ["1566", "1602"]),
            ("c3 the date alone in the attribution, the issue at the end (heap.md D-heap-13)",
             " - CLOSED (2026-09-20): a collection returned from a call and bound to a local "
             "never releases its elements (loft#1551)",
             ["1551"]),
            ("c4 a head naming no issue at all (D-op-1 shape)",
             "** - OPEN, not resolvable in a release: the operator set is what the parser "
             "computes, and no release can make it finite.",
             []),
            ("c5 a rule name in the first parenthetical, the issue after it",
             "** - OPEN: `(Col-Group)` says a record entering through one member is in every "
             "member, and it does not (loft#1700).",
             ["1700"]),
            ("c6 the same issue in the attribution and in the prose (D-clo-28)",
             "** *(closed 2026-09-08, loft#1443)* - `(L-CapOwn)` recognised exactly one way "
             "of leaving, and loft#1443 opened the second.",
             ["1443"]),
        ]
        bad = []
        for name, head, want in cells:
            got = _tracking_issues(head)
            if got != want:
                bad.append(f"  {name}\n    want {want}, got {got}")
        # The `claims` classifier (@PLN176): one cell per verdict, and the guard finder.  Each
        # proven able to fail: s1 against a classifier with no LIMIT vocabulary, s2 against one
        # that ignores the words BEFORE the citation, s3 against one that calls every citation
        # a limitation, s4/s5 against a finder that matches by substring (`14330-…`) or none.
        claim_cells = [
            ("s1 a present-tense fault with a workaround",
             ("Appending through a `&` to a hash does not work: the append is silently "
              "dropped (loft#1433).", ""), "limit"),
            ("s2 the past marker sits before the citation, outside the sentence",
             ("the compiler says exactly that at the call site.", "Since loft#1043 "), "past"),
            ("s3 a citation that states no fault",
             ("The walk is ordered by distance (loft#1002).", ""), "neutral"),
        ]
        for name, (sentence, before), want in claim_cells:
            got, _ = classify_claim(sentence, before)
            if got != want:
                bad.append(f"  {name}\n    want {want}, got {got}")
        import tempfile
        with tempfile.TemporaryDirectory() as td:
            os.makedirs(os.path.join(td, "scripts"))
            open(os.path.join(td, "scripts", "14330-not-this-one.loft"), "w").close()
            if guard_for("1433", tests_dir=td) is not None:
                bad.append("  s4 guard_for matches the number exactly — `14330-…` does not "
                           "speak for loft#1433")
            open(os.path.join(td, "scripts", "1433-a-keyed-alias.loft"), "w").close()
            g = guard_for("1433", tests_dir=td)
            if not g or not g.endswith("1433-a-keyed-alias.loft"):
                bad.append(f"  s5 guard_for finds the issue-numbered guard\n    got {g}")
            if guard_for("999999", tests_dir=td) is not None:
                bad.append("  s6 guard_for answers None when nothing speaks for the issue")
        # The `fences` matcher (@PLN176 phase 2).  f1 against a matcher that ignores indent
        # (a fence cut from a `fn main` body would never match), f2 against one that matches
        # line by line without adjacency (a fence assembled from two places would pass), f3
        # against one that drops blank lines (same), f4 against an HTML reader that keeps the
        # other language's block or the markup.
        src = ["fn main() {", "  a = 1;", "  b = a + 1;", "", "  c = b;", "}", "  a = 1;", "  c = b;"]
        if fence_matches(["a = 1;", "b = a + 1;"], src) != 2:
            bad.append("  f1 fence_matches finds an indented window of the file")
        if fence_matches(["a = 1;", "c = b;"], src) != 7:
            bad.append("  f2 fence_matches needs the lines ADJACENT (lines 2 and 5 are not)")
        if fence_matches(["b = a + 1;", "c = b;"], src) != 0:
            bad.append("  f3 fence_matches keeps a blank line as a line of the window")
        page = ('<h2 id="null">2. Null</h2><pre><code>p = <span class="en">P</span> { x: '
                '<span class="nm">1</span> };\n<span class="kw">if</span> a &lt; b {}</code></pre>'
                '<pre class="rust-pre"><code>let p = P;</code></pre>')
        got = html_loft_blocks(page)
        if got != [(1, "null", ["p = P { x: 1 };", "if a < b {}"])]:
            bad.append(f"  f4 html_loft_blocks reads the loft side, unescaped, untagged\n    got {got}")
        # And one cell for the WIRING, because a helper can be right while the caller still
        # asks the old question: the same c1 head, read the way a chapter is read.
        wired = _register_entries(
            "* **D-col-9** - OPEN, opened 2026-09-25 (loft#1664): a write, which is "
            "loft#1662 split surviving.\n", REG_ENTRY_BULLET)
        if list(wired) != ["D-col-9"] or wired["D-col-9"][2] != ["1664"]:
            bad.append(f"  c7 `_register_entries` reads the attribution\n    got {wired}")
        # The section keepers (g1–g3), each proven able to fail against the break it names:
        # g1 against counting a child's signature rows for the parent (the File System parent
        # was charged with Binary Files' rows), g2 against reading `fn(` / `f(` / `limit(` as
        # routines, g3 against taking a library section's rows to the stdlib.
        g_rules = {"F-Call": ["calls.md"]}
        g_names = stdlib_names()
        g_doc = ("## Parent\n\n| `fn(integer) -> text` | `limit(0, 255)` | `f(x)` |\n\n"
                 "### Child\n\n| `no_such_routine(x)` | a row |\n\n"
                 "### Kept\n\nRules: `(F-Call)`.\n\n"
                 "### Lib — `use imaging;`\n\n| `png(self: File)` | a library row |\n")
        g_rows = {}
        for _, ln, lvl, title, keep, unres in _section_rows(g_doc, g_rules, g_names, "g.md"):
            g_rows[title] = (keep, unres)
        if g_rows["Parent"][1] != [] or g_rows["Child"][1] != ["no_such_routine"]:
            bad.append(f"  g1 a child's unresolved rows are the child's alone\n    got {g_rows}")
        if g_rows["Parent"][1] != [] or g_rows["Parent"][0] != ["rule F-Call"]:
            bad.append(f"  g2 `fn(`, `f(` and a field modifier are not routines; a child's rule keeps "
                       f"the parent\n    got {g_rows['Parent']}")
        if g_rows["Lib — `use imaging;`"] != (["library imaging"], []) or not g_rows["Kept"][0]:
            bad.append(f"  g3 a library section is its testbed's; a rule keeps a section\n    got {g_rows}")
        for line in bad:
            print(line)
        print(f"{len(cells) + 4} cell(s), {len(bad)} failed")
        return 1 if bad else 0

    if cmd == "registers":
        # The three decoders side by side: the chapter's stated `OPEN: n`, the entries it
        # actually lists, and (with --issues) whether each open entry's issue still is.
        want_issues = "--issues" in sys.argv
        live, drift, regs = open_register()
        print(f"{len(regs)} chapters with a Deviations section · "
              f"{len(live)} open entries · {len(drift)} chapter(s) whose count disagrees\n")
        for f, stated, found, tags in drift:
            print(f"  {f}: states OPEN: {stated}, lists {found} "
                  f"({', '.join(tags) if tags else 'none'})")
        untracked = [(f, t) for f, t, iss, unres in live if not iss and not unres]
        unresolvable = [(f, t) for f, t, iss, unres in live if unres]
        print(f"{len(live) - len(untracked) - len(unresolvable)} open entr(y/ies) tracked by an "
              f"issue · {len(unresolvable)} marked not resolvable in a release · "
              f"{len(untracked)} with NO tracking issue")
        for f, t in untracked:
            print(f"  {f}: {t} is OPEN and names no loft#N or @PLN<n> — file its issue, name the "
                  f"plan that builds it, or mark it "
                  f"`{UNRESOLVABLE}` with the reason")
        if want_issues:
            named = {int(n) for _, _, iss, _ in live for n in iss if n.isdigit()}
            done, unreachable = closed_issues(named)
            print(f"\n{len(named)} issue(s) named by an open entry, {len(done)} now CLOSED")
            if unreachable:
                print(f"  ⚠ {len(unreachable)} could not be asked "
                      f"({', '.join('loft#%d' % n for n in unreachable)}) — unknown, not clean")
            for f, tag, iss, _ in live:
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

    if cmd == "claims":
        want_issues = "--issues" in sys.argv
        gate = "--gate" in sys.argv
        sites = claim_sites()
        limits = [c for c in sites if c[3] == "limit"]
        past = sum(1 for c in sites if c[3] == "past")
        print(f"{len(CLAIM_DOCS)} docs · {len(sites)} issue citations · "
              f"{len(limits)} in a LIMITATION sentence · {past} in a past-tense one · "
              f"{len(sites) - len(limits) - past} neutral")
        if not want_issues:
            print("limitation sentences (the nightly asks the tracker whether each issue is "
                  "still open; `--issues` asks now):")
            for f, line, n, _, word, sentence in limits:
                g = guard_for(n)
                print(f"  {f}:{line} loft#{n} [{word}] "
                      f"{'guard ' + g if g else 'NO GUARD'}\n      \"{sentence[:110]}\"")
            return 0
        done, unreachable = closed_issues({n for _, _, n, _, _, _ in limits})
        stale = [c for c in limits if c[2] in done]
        print(f"{len({n for _, _, n, _, _, _ in limits})} issue(s) cited by a limitation, "
              f"{len(done)} now CLOSED")
        if unreachable:
            print(f"  ⚠ {len(unreachable)} could not be asked "
                  f"({', '.join('loft#%d' % n for n in unreachable)}) — unknown, not clean")
        for f, line, n, _, word, sentence in stale:
            g = guard_for(n)
            print(f"  STALE? {f}:{line} states a limitation citing loft#{n}, CLOSED "
                  f"({done[n][:60]})\n      {'guard ' + g + ' says which way it went' if g else 'NO GUARD — unkept'}"
                  f"\n      \"{sentence[:110]}\"")
        if stale:
            print(f"\n{len(stale)} sentence(s) to re-read.  A LIFTED limitation leaves the page: "
                  "state the current behaviour with its guard, and move the record (what it "
                  "was, the issue, the guard) to the doc's `-history.md` companion.  One that "
                  "still holds cites the rule that makes it a decision (rule 23).")
        return 1 if (gate and (stale or unreachable)) else 0

    if cmd == "sections":
        gate = "--gate" in sys.argv
        rows = section_report(rules=rules)
        files = sorted({r[0] for r in rows})
        unkept = [r for r in rows if not r[4]]
        bad_sig = [r for r in rows if r[5]]
        print(f"{len(files)} docs · {len(rows)} sections · {len(rows) - len(unkept)} kept · "
              f"{len(unkept)} with nothing in the repo that would move · "
              f"{len(bad_sig)} with a signature the stdlib does not declare")
        for f in files:
            mine = [r for r in rows if r[0] == f]
            print(f"  {f}: {len(mine)} sections — {sum(1 for r in mine if r[4])} kept, "
                  f"{sum(1 for r in mine if not r[4])} unkept")
        if unkept:
            print("\nunkept (no sourced sample, rule or feature citation, stdlib signature, "
                  "guard path or issue):")
            for f, line, lvl, title, _, _ in unkept:
                print(f"  {f}:{line} {'#' * lvl} {title}")
        if bad_sig:
            print("\nsignature rows naming a routine no default/*.loft declares:")
            for f, line, lvl, title, _, unres in bad_sig:
                print(f"  {f}:{line} {'#' * lvl} {title} — {', '.join(unres)}")
        if unkept or bad_sig:
            print("\nA section is kept by what would MOVE when the language does: a sample cut "
                  "from a program (rule 22), the `@FR-` rule or `@F` feature it restates, or a "
                  "signature the stdlib source declares.  DOC_CONTRACT rule 28.")
        return 1 if (gate and (unkept or bad_sig)) else 0

    if cmd == "fences":
        gate = "--gate" in sys.argv
        rows = fence_report()
        by = collections.Counter(st for _, _, st, _, _ in rows)
        files = sorted({f for f, _, _, _, _ in rows})
        print(f"{len(files)} docs · {len(rows)} loft samples · {by['ok']} verbatim from a "
              f"program that runs · {by['library']} kept by a library's testbed · "
              f"{by['drift']} drifted · {by['missing']} naming a missing file · "
              f"{by['unsourced']} with no source")
        for f in files:
            mine = [r for r in rows if r[0] == f]
            c = collections.Counter(st for _, _, st, _, _ in mine)
            print(f"  {f}: {len(mine)} samples — {c['ok']} ok, {c['library']} library, "
                  f"{c['drift']} drift, {c['missing']} missing, {c['unsourced']} unsourced")
        bad = [r for r in rows if r[2] not in ("ok", "library")]
        if bad:
            print("\nnot kept by a program:")
            for f, line, st, src, detail in bad:
                print(f"  {f}:{line} {st.upper()}{' ' + src if src else ''} — {detail}")
            print("\nA sample is a verbatim window of the program named on the line above it "
                  "(`<!-- from tests/reference/<file>.loft -->`); a shape or signature that is "
                  "not a program is a ```grammar fence.  DOC_CONTRACT rule 22.")
        return 1 if (gate and bad) else 0

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
    # Rules that are a PREFIX of another — the reason a citation matches only when the next
    # character is not `[-A-Za-z0-9]` (§ Rule tags).  Reported here rather than written into
    # prose: it moves with every sub-rule added, and the paragraph that carried it managed to
    # contradict ITSELF (21 in one sentence, 23 two sentences later) before it also went stale.
    prefixes = sum(1 for a in rules if any(b != a and b.startswith(a) for b in rules))
    n_open = sum(1 for v in devs.values() if v[1] == "OPEN")
    # This count and `registers`' are now the SAME set — both read `defined_deviations`, in all
    # three entry spellings.  They differ in the question asked, not the number: this one decides
    # whether a CITATION is legal; `registers` checks each chapter's own stated `OPEN: n` against
    # it.  They disagreed while this one could not see the bullet form, and that is what let four
    # closed deviations sit in the register reading OPEN.
    print(f"{len(rules)} defined rules · {cited} cited · "
          f"{sum(len(v) for v in cites.values())} citation sites · "
          f"{prefixes} a prefix of another · "
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
