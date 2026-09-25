---
name: doc-quality
description: >-
  The documentation REVIEWER's pass in the loft repo: bring one doc (or one PR's doc
  changes) fully under doc/claude/DOC_CONTRACT.md. Use it when reviewing documentation —
  the release checklist's D-review and D-user rows, a PR's comments and docs, a doc a
  reviewer session was started to split or clean — and when asked to "review the docs",
  "clean up the comments" or "check this doc against the contract". It carries the
  judgment the lint cannot: whether a comment documents the contract or retells an
  incident (the deletion test, and converting the story into the rule), whether a
  reference is a live pointer or a dead stamp, and whether a maintainer doc answers one
  question. A builder writing code or docs does NOT need it: the doc-lint edit hook
  reports what a script can check as the file is written.
user-invocable: false
---

# Documentation review

The rules are [`doc/claude/DOC_CONTRACT.md`](../../../doc/claude/DOC_CONTRACT.md), one line
each, with the doc that owns each one; [`DOC_QUALITY.md`](../../../doc/claude/DOC_QUALITY.md)
has the detail and the evidence.  This skill is the part a reviewer applies by judgment.

## Start from the worklist

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

## The deletion test — is this about the contract or the incident?

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

## Stamp or pointer?

- **A dead stamp goes:** `@PLAN12 phase 3.5a (2026-05-24) — …` — following it tells you only
  when the line was written.
- **A live pointer stays:** a link to a doc, issue or plan that explains the code
  (`see LIFETIME.md § Lock bracket`), even when the plan or issue is closed.  Prefer a stable
  target (an issue URL, or a ref `./scripts/idx` resolves) over a raw plan path.

## The questions only a reader can answer

- Does each function description say **why to use it**, and each body comment **what** the
  non-obvious code achieves (DOC_QUALITY rules 5–6)?
- Is user-facing text plain: common words, one idea per sentence, no idioms, a term
  explained on first use (§ Write for every reader)?
- Does a maintainer doc answer **one** question, and is it navigable under its headings
  (§ Maintainer docs, rules 1–2)?
- Is a contract doc stating the rule, or telling how it came to be (rule 4)?
- Does every reference row name something that exists (rule 8)?
