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
DEF_DEV = re.compile(
    r"(?:^#{2,5}\s+`?(?P<h>D[A-Za-z]*-[a-z]+-\d+|DN\d+[A-Za-z-]*)\b"
    r"|^>\s*\*\*(?P<q>D[A-Za-z]*-[a-z]+-\d+|DN\d+[A-Za-z-]*)\s+—)",
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
    out = {}
    for path in sorted(glob.glob(FORMAL + "/*.md")):
        text = open(path, encoding="utf-8").read()
        for m in DEF_DEV.finditer(text):
            tag = m.group("h") or m.group("q")
            tail = text[m.end():m.end() + 120]
            status = "CLOSED" if "CLOSED" in tail else "OPEN" if "OPEN" in tail else "?"
            files, prev = out.get(tag, ([], "?"))
            out[tag] = (files + [os.path.basename(path)], status if prev == "?" else prev)
    return out


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
        want_devs = "--deviations" in sys.argv
        table = devs if want_devs else rules
        for tag in sorted(table):
            print(f"@FR-{tag:<28} {', '.join(sorted(set(table[tag])))}")
        kind = "deviation entries (NOT citable)" if want_devs else "defined rules"
        print(f"\n{len(table)} {kind}")
        return 0

    cites = citations()

    if cmd == "sites":
        tag = sys.argv[2].removeprefix("@FR-").lstrip("@")
        if tag in devs:
            files, status = devs[tag]
            print(f"@FR-{tag} is an {status} DEVIATION entry ({', '.join(sorted(set(files)))}), "
                  "not a rule.")
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
    print(f"{len(rules)} defined rules · {cited} cited · "
          f"{sum(len(v) for v in cites.values())} citation sites · "
          f"{len(devs)} deviations ({n_open} open), {implementing} site(s) implementing one")
    if problems:
        print(f"\n{len(problems)} problem(s):")
        for p in problems:
            print(f"  {p}")
        return 1
    print("ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
