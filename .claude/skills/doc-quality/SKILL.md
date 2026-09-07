---
name: doc-quality
description: >-
  Write and edit documentation in the loft repo the way it should read: code
  comments and doc-comments in `.rs` files, AND prose docs (`.md` — README,
  onboarding, reference). Use this whenever you add or revise a `///` doc comment,
  an inline comment, a module `//!` header, or any Markdown doc, and whenever you
  review the comments in a diff. It keeps comments present-tense (describe the code
  as it is NOW, not its history), makes a function's description about *why to use
  it* (not why it was written), strips dead plan-tag/date stamps while keeping live
  pointers to issues/plans, and keeps prose readable for entry-level and
  non-native-English readers. It also decides what a comment is ABOUT: documentation
  states the timeless contract — the rule, the invariant, the domain — and never retells
  the bug that prompted the code. Reach for it even when the user only says "document
  this", "add comments", "write the docstring", "explain this function", "write up
  the README", or "clean up the comments"; and reach for it whenever you are about to
  write "this used to", "the bug was", or a fix narrative into a comment, or you are
  documenting an algorithm right after fixing one.
user-invocable: false
---

# Documentation quality

These rules are three loft goals applied to documentation. Keeping the goals in
view is the point — the rules are how they show up when you write a comment:

- **Goal B — legible on contact:** comments and the on-ramp are where a reader
  first meets the value. If only a senior native speaker can read them, B fails.
- **Goal F — serve the reader, not the author:** a comment exists for whoever
  reads the code next, not as the author's audit trail. Provenance stamps bill
  the reader for the author's bookkeeping.
- **Goal E — the stated thing must match reality:** a comment describing a
  deleted past is a stated model that no longer matches the code — E's exact
  failure mode. A stale comment hides the present code instead of revealing it.

Full reference (evidence, worked rewrites, the measurement): **`doc/claude/DOC_QUALITY.md`**.
The goals themselves: **`doc/claude/GOALS.md` §B/E/F**.

## When to apply

Apply these to the comment or doc you are **writing or editing right now**. Do
not sweep a file to "fix its comments" during unrelated work — that burns effort
and risks churn. The Check (below) is a thermometer you run on purpose, not a
gate on every edit.

## The rules

1. **Describe the code as it is now, never the change.** If a comment only makes
   sense to someone who saw the old version, it belongs in the commit message.
   *(Goal E: the comment must match the present code.)*
2. **No plan tags, dates, or resolved-bug history in code comments** — `git blame`
   keeps that. A **live pointer** to a doc, issue or plan that explains the code is the
   allowed exception (see "Stamp vs pointer"). *(Goal F: serve the reader, not the
   author's bookkeeping.)*
3. **Keep the high-value kinds:** rules the code enforces, the odd-but-needed
   *why*, cross-file coupling ("change X too"), costs/dangers, and a short module
   intro. These are what a reader cannot recover from the code itself.
4. **Judge by content, not length.** A 70-line module header can be right; a
   one-line "increment i" is wrong. There is no line limit.
5. **A function `///` description says *why to use it*, not *why it was written*** —
   what it is for, when to use it, preconditions, trade-offs. For the design
   reason, **link to the issue or plan you implemented it from** — you are usually
   already reading it; do not rewrite it inline or build a new doc home for it.
   *(Goal F.)*
6. **Inside a function, comment *what* the non-obvious code does** — a dense or
   clever block (a bit twiddle, a hand-rolled search, a careful ordering) gets one
   line on what it achieves. You may point to the problem/issue the block handles.
7. **Write for entry-level and non-native-English readers** (code comments + any
   user-facing or on-ramp doc): common words over fancy ones, short one-idea
   sentences, no idioms or metaphors, explain a term on first use, lead with a
   concrete example. Plain and short — not long and simple. *(Goal B.)*
8. **Document the CONTRACT, not the INCIDENT.** The body of a comment says what the
   code computes and what a caller may rely on — the rule it enforces, the domain it
   is defined over, the invariant it holds, the trade-off it takes. A bug may be
   *cited*; it is never the *subject*. One clause or a link, at the edge — never the
   organising idea. *(Goal E: a feature and a rule are meant to be timeless; an
   incident is true at a moment and stops being the reader's question once it is
   fixed.)*

   This is a different axis from rules 1 and 2, and a comment can pass both and still
   fail this one: present-tense, no date stamp, and still built around "the bug we
   hit". Rules 1–2 are about tense and bookkeeping; rule 8 is about **what the
   documentation is about**. Evidence, a worked rewrite, and why the detector for it
   deliberately under-reports: `DOC_QUALITY.md` § B2 (rule **9** in that document's own
   list). `scripts/lint_comments.sh incident` finds the loud cases only — the deletion
   test below is the real check.

## Rule 8 in practice: the deletion test, and the conversion

**The test.** Delete every sentence about the incident. Does what remains still say
what the code does and what a caller may rely on? If not, the comment was documenting
history, not code — and a reader who arrives with a question about the *present* code
leaves without an answer.

**Do not delete — CONVERT.** Most incident narration is a timeless fact wearing a
story's clothes, and throwing it away loses real knowledge. The story almost always
contains a rule; extract the rule and drop the story around it.

```text
BEFORE (incident): "The seventh gate outlived that sweep because it asks about the
                    returned VALUE rather than the return TYPE — so the tail intercept
                    never fired and the arm handed back its own store."

AFTER  (contract):  "Every gate here asks the shape question about the return TYPE.
                     Asking it about the returned VALUE is wrong: `v = src(i); return v;`
                     types `v` as `Optional(Vector)`, so a gate reading the local's own
                     type sees a shape the return type never had."
```

Same knowledge. The first tells you what happened once; the second tells you what to do
every time. Only the second survives the next reader.

**A regression test is the near-exception, and still not an exception.** Its doc SHOULD
name what it guards — that is its whole purpose — but the thing it guards is a *property*,
not an episode. Write the property, then cite the issue:

```text
BEFORE: "Before the fix the generic-instantiation block was gated on `!self.default`,
         so a stdlib-internal generic call resolved to Unknown function."
AFTER:  "Generic instantiation is caller-source-agnostic: a stdlib fn can call a generic
         stdlib fn exactly as a user program can. (loft#653)"
```

The second tells a reader what breaking the test would MEAN. The first only tells them
what someone once typed.

**Where the story goes instead.** The commit message and the issue — they exist for it,
and `git blame` reaches both. A mechanism that genuinely teaches beyond its own fix
belongs in `doc/claude/` (the plan, `PROBLEMS.md`, a `STABILITY_*` doc), and the comment
carries a *pointer* to it. That is rule 2's stamp-vs-pointer distinction again.

**When a formal rule exists, cite it.** `doc/claude/formal/` is the timeless statement by
construction, and `@FR-<Rule>` is its name — so *"Enforces `@FR-L-Null` for the narrow
widths"* is the ideal rule-8 comment: it says what the code guarantees, and it resolves
(`scripts/rule_tags.py sites @FR-L-Null`) to every other site guaranteeing the same
thing. If the invariant you are about to narrate has no rule yet, that is a signal the
rule is missing — not that the story should stay.

## Two layers, and the prose exception

- **Function `///` description** → rule 5: *why to use it*.
- **Inline body comment** → rule 6: *what* the non-obvious code does.
- **Prose docs (`.md`)** → rule 7 (plain language) always applies to user-facing
  and on-ramp docs. But rules 1–2 are softer here: a changelog, a plan, or
  `GOALS.md` legitimately carries dates and plan refs. The stamp ban is a *code*
  rule. Maintainer-facing design docs may use denser, project-specific language;
  do not let that creep into the user-facing surface.

## Stamp vs pointer (the one distinction to get right)

- **Dead stamp — remove it:** `@PLAN12 phase 3.5a (2026-05-24) — …`, a bare prefix
  recording which plan and when, with nothing to open. Following it tells you only
  when the line was written.
- **Live pointer — keep it:** a link to a doc, issue, or plan that *explains* why
  the code is needed (`see LIFETIME.md § Lock bracket for why`). Following it
  teaches you about the present code. It counts **even if the plan/issue is
  closed** — a finished investigation still explains. Prefer a stable target (an
  issue URL, or a ref `./scripts/idx` resolves) over a raw plan path.

Editing a REFERENCE table (a doc row per function/method): a row naming a method
that does not exist is worse than no row — verify each named symbol resolves
(DOC_QUALITY.md rule 8 has the measured case).

The loft tracker tags (CLAUDE.md § "Tracker tags") belong in plans, design docs,
and commit messages — where `./scripts/idx` resolves them — not in a `.rs`
comment. The comment carries a *link* to the tagged plan, not the tag.

## Worked example

Illustrative — the function is real, the comments are written to show the contrast
(DOC_QUALITY.md carries the same example):

```rust
// BEFORE — the description explains why the function was WRITTEN, under a stamp
/// @PLAN52 (2026-05-30): added during the closure-capture rework to fix a
/// locked-store panic; re-types text captures. Mirrors
/// flip_scalars_to_box_types.
fn box_captured_names_for_outer_scalars(…) { … }

// AFTER — why to USE it (present tense), design reason as a live pointer
/// Re-types a lambda's captured outer scalars to their cell form so the closure
/// record can hold them by reference. Call after capture analysis, before
/// emitting the closure record. Capture storage is decided per binding — see
/// loft#687 for why the blanket text-skip was retired.
fn box_captured_names_for_outer_scalars(…) { … }
```

## Check (advisory)

A runnable detector backs this standard — *progress is evaluated, not asserted*
(`GOALS.md`). It is a thermometer; it never fails CI and never edits.

```bash
scripts/lint_comments.sh            # full report + biggest offenders
scripts/lint_comments.sh -c         # counts only (the thermometer)
scripts/lint_comments.sh top        # files ranked by flagged count (cleanup)
```

For adopting on the existing tree without a big-bang cleanup, use the baseline
ratchet (accept today's flags, block only new ones, shrink over time):

```bash
scripts/lint_comments.sh --baseline   # accept today's flagged lines
scripts/lint_comments.sh --check       # list only NEW flags (CI advisory)
scripts/lint_comments.sh --prune        # after a cleanup pass: drop fixed lines
```

A flagged `#NNN` or doc pointer that *explains* the code is a keeper — only bare
plan/date stamps and change-narration should go. The full workflow is in
`DOC_QUALITY.md` § Check.
