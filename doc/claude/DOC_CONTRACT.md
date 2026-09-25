<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The documentation contract

Every standing rule for writing documentation in this repo, one line each, with the doc that
owns the detail.  If a line and its owner differ, the owner wins: fix the line.  `Gate` means a
test in the suite fails; `report` means a tool prints and never fails.

## Every surface

1. Describe the thing as it is now; its history goes to the commit, CHANGELOG_TECHNICAL.md or `<doc>-history.md`. — [DOC_QUALITY](DOC_QUALITY.md) rule 1 · report: `lint_comments.sh`, `doc_history_report.py`
2. Document the contract, not the incident: a bug may be cited, never be the subject; convert the story into the rule. — DOC_QUALITY rule 9, § B2
3. Every fact has one home; everywhere else points to it. — [USER_DOCS](USER_DOCS.md) (library text), [DEVELOPMENT](DEVELOPMENT.md) § How items are documented
4. Plain language for code comments and user docs: common words, one idea per sentence, no idioms, a term explained on first use. — DOC_QUALITY § Write for every reader
5. A live pointer to a doc, issue or plan is kept; a dead plan-or-date stamp is not. — DOC_QUALITY § A
6. A durable fact an agent keeps in memory also goes into a repo doc; machine-specific values stay out of shared docs. — [CLAUDE.md](../../CLAUDE.md) § Conventions
7. A generated file is never hand-edited, and carries data, not rationale. — DOC_QUALITY § D · gate: the `*_is_up_to_date` guards in `tests/doc_hygiene.rs`
8. Every relative link resolves; move a doc with `make plan-move`, repair with `make doc-fix`. — [DEVELOPMENT](DEVELOPMENT.md) § Moving a doc · gate: `every_markdown_link_resolves`
9. A reference row naming something absent is worse than none; a capability with two spellings names the other in both. — DOC_QUALITY rule 8
10. A causal claim ("X causes Y") is measured before it is written. — DOC_QUALITY § An attribution in a comment

## Code comments (`src/`, `default/*.loft`, a library's `src/`)

11. A function's description says why to use it; its design reason is a link to the issue or plan. — DOC_QUALITY rule 5, [CODE](CODE.md) § Doc Comments
12. Inside a body, comment only what non-obvious code achieves. — DOC_QUALITY rule 6
13. Judge a comment by what it says, not its length. — DOC_QUALITY rule 4
14. Cite `@FR-<Rule>` where `formal/` states the invariant. — [formal/README](formal/README.md) § Rule tags · gate: `every_rule_citation_resolves`
15. A doc block belongs to the item below it; insert above the neighbour's block, read with `grep -B`. — DOC_QUALITY rule 10, § Maintainer docs rule 12
16. Every `pub` item in `default/*.loft` or a library has a doc comment above it. — [DOC](DOC.md), [API_SURFACE](API_SURFACE.md) S3 · gate: `every_published_stdlib_entry_carries_its_documentation`
17. A workaround comment names the canonical home of its defect, and goes in the commit that ships the fix. — DEVELOPMENT § Inserting Discovered Enhancements

## User-facing (guides, reference, comparison pages, READMEs, CLI output)

18. What a command prints is silent when nothing needs acting on, and carries no plan tags or "not yet". — CLAUDE.md § Conventions, DOC_QUALITY § D
19. Four questions, four tiers: what is there, how do I start, what is the signature, where is the source. — [USER_DOCS](USER_DOCS.md) § The design (Tiers 0–3)
20. A library's user-facing text has one home: the library. — USER_DOCS § The one-home rule
21. A topic page states `@NAME`/`@TITLE`, asserts something, and renders no directive as prose. — DOC § Adding a new topic page · gate: `every_doc_page_asserts_something`
22. Every example runs on both backends; an indented `$ ` line is executed and its output checked. — [RELEASE](RELEASE.md) § 0b · gate: the docs suite, `tests/doc_commands.rs`
23. No temporal or hedge words (currently, planned, for now, not yet, TODO); a fault claim cites a live issue. — API_SURFACE S7 · report: `doc_review.py`
24. Every `make` target or flag named in prose resolves. — RELEASE § 0d · report: `doc_review.py`
25. A comparison claim is a program in `tests/comparisons/`; its rationale is a link, never a restatement. — [SUBJECTS](SUBJECTS.md) · gate: `wrap::comparisons`
26. A library ships a guide, `docs/01-getting-started.loft`, whose numbers are measured. — [LIBRARY_AUTHORING](LIBRARY_AUTHORING.md) § 2c
27. CHANGELOG.md is plain language under `## YYYY-MM`; CHANGELOG_TECHNICAL.md records every change. — DEVELOPMENT § Documentation updates

## Maintainer docs (`doc/claude/`, CLAUDE.md, skills)

28. A doc answers one question. — DOC_QUALITY § Maintainer docs rule 1
29. 1000 lines is a hard ceiling; under it, the structure must let a reader navigate. — rule 2 · report: `make file-sizes`
30. Every doc is reachable from CLAUDE.md in two hops, and an index entry names the start-here doc. — rule 3
31. A contract doc states the current rule; a record doc keeps its dates. — rule 4, RELEASE § 5b
32. Commit the goal and the command that reports the position, never the position. — rule 5
33. Prose beside a gated number carries the reason, not the value. — rule 6
34. Search a doc before adding to it; read the code before stating what it does. — rules 7–8
35. Name the tree a fact is about; a cure ships with its signpost and is read back against its cause. — rules 9–11
36. Docs ship in the branch of the code they describe, in their own commit; a small edit gets no PR of its own. — rule 13, DEVELOPMENT § Documentation commit
37. A formal register's `OPEN: n` is a claim to re-measure, and each open entry names its issue. — formal/README · gate: `register_entries_name_their_tracking_issue`
38. A diagnostic code is frozen once shipped and lands with its DIAGNOSTICS.md row. — [DIAGNOSTICS](DIAGNOSTICS.md) · gate: `every_pinned_code_is_documented`
39. A plan is its GitHub issue; its file follows `plans/_TEMPLATE.md`. — [plans/README](plans/README.md)
40. A new guard records `@falsified-at:` and how to score it again. — [TESTING](TESTING.md) · gate: `every_new_guard_records_its_control`
41. A skill points at its canonical doc and names the command that measures, never the measurement. — [SKILLS_REVIEW](SKILLS_REVIEW.md)

## Reviewer pass

Judgment rules are applied by a reviewer, not by whoever wrote the text, on the release
checklist's `[mid]` and `[pre]` beat (@PLN172).  The reviewer is a fresh session that loads
this contract and the `doc-quality` skill, runs the reports named above, takes the top doc from
their worklist, and brings it fully under the contract in one PR: split it, move its history
out, fix its index entry.  It changes no code.
