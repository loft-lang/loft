---
name: doc-quality
description: >-
  The rules for WRITING or EDITING any documentation in the loft repo — a doc under
  doc/claude/, a formal rule chapter, CLAUDE.md, a skill, a release record — and the
  REVIEWER's pass that brings one doc (or one PR's doc changes) fully under
  doc/claude/DOC_CONTRACT.md. Use it BEFORE adding a paragraph, a measurement, a closed
  item or a dated ruling to any doc, whenever a doc is near or over 1000 lines, and when
  reviewing docs (the release checklist's D-review and D-user rows, a PR's comments and
  docs, "review the docs", "clean up the comments", "split this doc"). It carries the two
  hard rules writers keep breaking — history goes to the doc's `<doc>-history.md`
  companion, and no working doc passes 1000 lines — plus the judgment the lint cannot:
  contract versus incident (the deletion test), live pointer versus dead stamp, one
  question per doc.
user-invocable: false
---

# Documentation

The rules are [`doc/claude/DOC_CONTRACT.md`](../../../doc/claude/DOC_CONTRACT.md), one line
each, with the doc that owns each one; [`DOC_QUALITY.md`](../../../doc/claude/DOC_QUALITY.md)
has the detail and the evidence.

## Two hard rules for every doc edit — writing or reviewing

These two are broken more than any other doc rule, by writers who never loaded this skill.
Apply them to every doc you touch, before you save.

**1. History goes to `<doc>-history.md`, never into the working doc** (owner, DOC_QUALITY
§ Maintainer docs rule 4; RELEASE.md § 5b).  A working doc — a reference, a rule chapter, a
guide, CLAUDE.md, a skill — says what holds NOW: the rule, the current state, the open
items, the command that measures them.  Everything else is history and goes to the
companion beside it:

| goes to `<doc>-history.md` | stays in the working doc |
|---|---|
| a date (`2026-09-28`, "measured on …", "until …", "since …") | the rule, stated in the present tense |
| how a defect was found, what it cost, which commit fixed it | the current behaviour and its guard |
| a CLOSED deviation entry, a done or shipped item | the `OPEN: n` line and one line per open item |
| "used to", "was", "before this", the story of a flip | the command that measures the state |
| a dated owner ruling (keep the ruling's CONTENT as the rule) | a link to the companion, once |

- The companion's first line is `<!-- size-exempt: a record companion, read by anchor and
  grep (DOC_QUALITY § Maintainer docs 2) -->`; create it if it does not exist.
- MOVE, never copy.  A deviation register moved to a companion goes under a
  `## Deviations carried by <doc>.md until <date>` heading, so `scripts/rule_tags.py` still
  reads its entries.
- The deletion test below decides a borderline sentence.  A RECORD doc (CHANGELOG, a
  `releases/` cycle, a plan, a `-history.md`) keeps its dates: it IS the history.
- The story itself also belongs in the commit message and the issue.

**2. No working doc passes 1000 lines, at any time** (owner: a hard rule, all the time).
Before you add to a doc, `wc -l` it.  If the edit would cross 1000 lines — or the doc is
already over — move its history out first (rule 1); if it is still over, split it by
SUBJECT: whole sections into new docs, one subject each, each linked from the parent
where the section was, and every inbound `DOC.md#anchor` link rewritten.  Never put a
`size-exempt` marker on a working doc: only formal rule chapters, generated reports and
`-history.md` companions may carry one.

Check before you commit:

```bash
git add -N <every new doc>           # the lint, the link check and the baseline pin read
                                     #   TRACKED files only: an untracked split doc is unchecked
wc -l <doc>                          # ≤ 1000 for a working doc
python3 scripts/doc_lint.py <doc>    # no size / history / timeline finding you added
```

## The reviewer's pass

### Start from the worklist

```bash
make docs-lint            # every finding, the delta since the baseline, the worklist
make file-sizes           # the docs over 1000 lines, and whether each holds one subject
python3 scripts/doc_lint.py <path>…     # one doc's findings
```

Take the top doc from the worklist (the owner may pick another).  Bring it fully under the
contract in one PR: split a doc that answers two questions, move a contract doc's history
into its `-history.md` companion or CHANGELOG_TECHNICAL, fix its index entry.  Change no
code.  After a pass that lowered the count, re-pin with `make docs-lint-baseline` in the
same commit.

### The deletion test — is this about the contract or the incident?

Delete every sentence about the incident.  Does what remains still say what the code does
and what a caller may rely on?  If not, the text documents history, and a reader with a
question about the present code leaves without an answer.

**Convert, do not delete.**  Most incident narration is a rule told as a story; extract the
rule and drop the story.

```text
BEFORE: "The seventh gate outlived that sweep because it asks about the returned VALUE
         rather than the return TYPE — so the tail intercept never fired."
AFTER:  "Every gate here asks the shape question about the return TYPE.  Asking it about
         the returned VALUE is wrong: `v = src(i); return v;` types `v` as
         `Optional(Vector)`, a shape the return type never had."
```

A regression test's doc names the PROPERTY it guards and cites the issue, never the episode.
Where `doc/claude/formal/` states the invariant, cite `@FR-<Rule>` — the ideal comment says
what the code guarantees and resolves (`scripts/rule_tags.py sites @FR-…`) to every other
site guaranteeing it.  The story itself goes to the commit message and the issue.

### Stamp or pointer?

- **A dead stamp goes:** `@PLAN12 phase 3.5a (2026-05-24) — …` — following it tells you only
  when the line was written.
- **A live pointer stays:** a link to a doc, issue or plan that explains the code
  (`see LIFETIME.md § Lock bracket`), even when the plan or issue is closed.  Prefer a stable
  target (an issue URL, or a ref `./scripts/idx` resolves) over a raw plan path.

### The questions only a reader can answer

- Does each function description say **why to use it**, and each body comment **what** the
  non-obvious code achieves (DOC_QUALITY rules 5–6)?
- Is user-facing text plain: common words, one idea per sentence, no idioms, a term
  explained on first use (§ Write for every reader)?
- Does a maintainer doc answer **one** question, and is it navigable under its headings
  (§ Maintainer docs, rules 1–2)?
- Is a contract doc stating the rule, or telling how it came to be (rule 4)?
- Does every reference row name something that exists (rule 8)?
