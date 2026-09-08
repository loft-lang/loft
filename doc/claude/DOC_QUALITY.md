# In-Code Documentation Quality

How to comment Rust code in this repo. This is the longer companion to
[CODE.md § Doc Comments](CODE.md#doc-comments), with the evidence behind each rule.

**The main idea:** a comment describes the **code as it is now**. Write what a
future reader needs but cannot get from the code itself. Anything you could find
with `git blame` or `git log` (when a change happened, which plan it came from,
what the code used to be) is history, not documentation. It belongs in the
commit message.

**Fast path:** in a hurry, read [§ The rules](#the-rules). The
rest is evidence and worked examples.

**Doing a review?** Load the **`doc-quality` skill** at the start
of any documentation review (release doc review, a PR's doc changes, a doc-edit
pass). It is the loadable companion to this doc — the rules condensed with
the stamp-vs-pointer test — so the review applies them consistently instead of
from memory.

**When to apply.** Apply these to the comment you are **writing or editing right
now**. Do not sweep a file to "fix its comments" during unrelated work — that
burns effort and risks churn. The [Check](#check) is a thermometer you run on
purpose, not a gate on every edit.

---

## Write for every reader

Two reader groups must always be able to follow the text:

- **Entry-level programmers**, not just senior engineers.
- **Readers whose first language is not English.**

**Where this bar applies (audience matters):**

- **Always — no exception:** code comments, and any **user-facing** or
  **on-ramp** documentation (install guide, first-program walkthrough, library
  usage). This is where these two reader groups actually land, and it is exactly
  [Goal B — "legible on contact"](GOALS.md#goal-b--release--legibility). A clean
  on-ramp that only a senior native speaker can read fails Goal B.
- **Maintainer-facing design docs** (most of `doc/claude/`, e.g. `GOALS.md`,
  `PERFORMANCE.md`) are written for a reader already deep in the project. They
  may use denser language and project terms. Plain language is still preferred,
  but the strict bar above is not a gate here. Do not let this exception creep
  into the user-facing surface.

How to do that (for any text held to the bar):

- Use common words. Write "history", not "provenance". Write "clearest
  example", not "poster child".
- Keep sentences short. One idea per sentence.
- Avoid idioms and metaphors ("in lockstep", "baked in", "code smell"). Say the
  plain thing instead.
- Explain a technical term the first time you use it.
- Start with a concrete example when you can.

This does **not** mean writing more, or talking down to the reader. Plain and
short is the goal — not long and simple.

---

## Why this doc exists (evidence)

A scan of `src/` (about 129,000 lines) when this doc was written. The last two
rows are what `scripts/lint_comments.sh -c` (the [Check](#check)) reports, so the
doc and its tool agree:

| Measure | Value |
|---|---|
| Comment lines | 27,280 (**21.2%** of the source) — a healthy amount |
| of these, `///` doc comments | 11,133 |
| history-stamp lines (plan tag / phase / date) | **1,198** |
| change-narration lines (past tense about the code) | **97** |

(These numbers are from one point in time — run the Check to get today's. The
baseline file merges both categories, deduped by comment text, into 1,265
entries.)

Two findings shaped the rules below:

- **Length is not the problem.** The longest comment block in the whole tree —
  the 74-line `//!` header in `src/trace.rs:4-77` — is very good. It explains
  why the module exists, how to use it, what it costs, and how to extend it. A
  rule based on length would delete the wrong comments. **Judge a comment by
  what it says, not how long it is.**
- **History narration is the problem.** Over a thousand lines repeat what
  `git blame` already knows, and another group tell the story of a past change
  instead of describing the current code. This is the waste a tool can find
  automatically, and it grows by about one line per plan-tagged commit.

---

## Keep — high-value comments

These describe the code as it is now, and you cannot get them from history. The
examples are paraphrased from real comments (file named at the time of writing).

1. **Rules the code enforces.** `parser/expressions.rs` — "VECTOR `+= elem`
   (a bare element) is rejected; `vec<vec<T>> += vec<T>` looks like both
   push-one and concat." A future editor needs this, or they will bring the
   problem back.
2. **Why the code is written this odd way** (as an *inline* comment inside the
   body, not in the function description). The historic `parser/vectors.rs`
   example — "skip text when the parent function returns text (this avoids the
   locked-store panic)" (that skip has since been retired by #687; the SHAPE of
   the comment is the point). It explains why the obvious simpler version is
   wrong. In a function
   `///` description, this design reason becomes a pointer instead — see
   [§ Function descriptions](#function-descriptions-say-why-to-use-it-not-why-it-was-written).
3. **A link between two files you cannot see locally.** Same place: "matches
   `flip_scalars_to_box_types`" — names the other function you must change at
   the same time.
4. **Costs and dangers.** `trace.rs` — "when tracing is off, each point is one
   bool load plus one branch; the cost is below measurement noise." Also notes
   where the interpreter and the native backend can differ.
5. **A short intro at the top of a module.** A `//!` header that tells a
   newcomer what the file is for before they read any code.

---

## Trim — could have been left out from the start

### A. Plan tags and dates — `git blame` already has them

```rust
// @PLN80 phase 3.5a (2026-05-24) — …
/// Plan-22 phase 02c (2026-05-12): override the alignment …
// @PLAN52 cluster I iteration 2 (2026-05-30): honor skip_free …
```

The tag and date answer "when, and from which plan". `git log` answers that
better, and the tag turns into noise once the plan is closed. The text *after*
the tag is often useful, so the fix is to **remove the tag**, not the whole
line.

**Stamp vs pointer — the one distinction this doc turns on.** The rule is about
the dead *stamp*, not about all references:

- **Dead stamp — remove it:** `@PLN80 phase 3.5a (2026-05-24) — …`, a bare prefix
  that records which plan and when, with nothing to open. Following it tells you
  only when the line was written.
- **Live pointer — keep it:** a link to a doc, issue, or plan that *explains* why
  the code is needed (`see LIFETIME.md § Lock bracket for why`). Following it
  teaches you about the present code. It counts **even if the plan or issue is
  closed** — a finished investigation in `plans/finished/` still explains. Prefer
  a stable target (an issue URL, or a tracker ref `./scripts/idx` resolves) over a
  raw plan path, since plans move between folders.

This does not conflict with the project's tracker tags (CLAUDE.md § "Tracker
tags"): those tags belong in plans, design docs, and commit messages — where
`./scripts/idx` resolves them — not in a `.rs` comment. The comment carries a
*link* to the tagged plan, not the tag.

The rest of this doc refers back to this distinction; it is not re-explained.

### B. Comments that describe a change, not the code

The clearest example (`src/ops.rs`, a 9-line block):

> `// … RNG thread-local + the rand_int / rand_seed / shuffle_ints helpers`
> `// removed. random's drain to lib/random/native/ makes the cdylib …`

The file no longer **has** an RNG thread-local. The whole block describes
something that was *deleted*. A reader of today's code is told about code that
is not there. Other examples: "the parser branch order used to misroute the
latter", "previously inlined the same 5-line loop".

**Note for the lint:** the words "used to" and "previously" also appear in
normal sentences ("used to size the gutter" means *is used to*). The real
warning sign is **past tense about the code's own structure** — *removed*,
*used to misroute*, *previously inlined* — not "used to &lt;verb&gt;".

### B2. Comments whose SUBJECT is the bug, not the code

Section B is about *tense* — a comment describing something deleted. This one is about
*subject*, and it is the harder half: a comment can be present-tense, stamp-free, and
about code that genuinely exists, and still be organised around the incident that
produced it rather than the thing it documents.

**Why it matters.** A feature and a formal rule are meant to be timeless — that is what
they are for. A bug is true at a moment and stops being true once it is fixed. So an
algorithm documented as *"the fix for the double-free"* answers a question nobody has
any more, while the reader's actual question — *what does this compute, and what may I
rely on?* — goes unanswered. The bug may still be worth **linking**; it must not be the
main body.

**How much of it there is — and why the number is soft.** A first sweep of the 7 967
doc-comment blocks in `src/` matched 1 023 on a broad failure vocabulary, only 236 of
which the existing history-stamp / change-narration checks could see. That 1 023 does
**not** survive inspection, and the reason is the useful part: this axis is *semantic*
("is the bug the subject?"), not lexical. `SIGSEGV` is what `crash_report.rs` installs a
handler for. `the hole` is Robin Hood hashing, and the lexer's unclosed brace.
`silently dropped` describes a live spoof-check. `never reported` is a contract. And
`loft#885's hoisted element reads` names a mechanism by its issue — a *pointer*, which
rule 2 explicitly keeps.

So the check is deliberately a strong **under-approximation** (12 lines at the time of
writing) and the honest position is: the prevalence is real and high, but it is not
reliably countable by grep. **The detector finds the loud cases; the deletion test below
is the actual check**, run on the comment in front of you.

**The test.** Delete every sentence about the incident. Does what remains still say what
the code does and what a caller may rely on? If not, the comment was documenting history.

**The move is CONVERSION, not deletion.** Most incident narration is a timeless fact
wearing a story's clothes; deleting it loses real knowledge. Extract the rule, drop the
story.

#### Worked rewrite — `Key::start` (`src/keys.rs`)

The field carries a `min` so that `compare_key` / `hash_ref` / `get_key` can decode a
narrow width without reaching the type table. Its comment spent fifteen lines on how the
absence of that field behaved:

```text
BEFORE — the subject is the failure
    Before this field they passed a literal `0`, so the record side decoded `val - min`
    while the lookup side had the user's `val`: the two differed by exactly `min` and
    never compared Equal.  A key declared `i8`, `i16` or `integer limit(min, max)` with
    a non-zero `min` therefore inserted fine, counted fine, and could never be looked
    up.  Ordering survived, because subtracting a constant is monotonic — only equality
    was wrong, which is why it read as "the record is missing" rather than a decode bug.

AFTER — the subject is the contract
    The field's storage START: the `min` a `Parts::Byte` / `Parts::Short` subtracts when
    it stores a value and adds back when it reads one.

    It travels WITH the key because the comparison happens in `compare_key` / `hash_ref`
    / `get_key`, none of which can see the type table — so a key cannot be decoded from
    the key alone without it.

    `0` for every width that stores raw (`integer`, `long`, `text`, `float`, `single`,
    `Parts::Int`, `Parts::ShortRaw`), and also the correct value for a `u8` / `u16`
    whose range starts at zero.

    ⚠ A wrong `start` breaks EQUALITY only. Ordering survives it, because subtracting a
    constant is monotonic — so the symptom is "the record is missing" from a lookup that
    inserted and counted fine, not a decode error.  (loft#812)
```

Nothing is lost. The monotonicity fact and the "reads as missing" symptom were the two
genuinely useful things in the original, and both survive — as a **standing warning to
whoever changes this**, which applies every time, instead of as a report of one Tuesday.

**When a formal rule exists, cite it.** `doc/claude/formal/` is timeless by construction
and `@FR-<Rule>` is its name, so *"Enforces `@FR-L-Null` for the narrow widths"* is the
ideal form: it states the guarantee and resolves to every other site making it
(`scripts/rule_tags.py sites @FR-L-Null`). An invariant you are about to narrate that has
**no** rule yet is a signal the rule is missing, not that the story should stay.

### C. Comments that just repeat the code

`// increment i` above `i += 1`. It adds nothing. See
[CODE.md § Doc Comments](CODE.md#doc-comments).

### D. Prose in generated output, narration in build recipes, and chatter on the CLI

Three places where a comment reaches the wrong audience and rots:

- **Generated files** (`LIBRARIES.md`, features shadows, …): emit only the terse data the
  *reader* needs — a tag legend, a pointer. A generator that writes multi-paragraph
  rationale into its output produces prose no one maintains; it goes stale on the next
  change. Put the "why" in the hand-maintained source (the plan, `CLAUDE.md`), not the
  artifact.
- **Build recipes** (`make install`): make echoes recipe lines, so a `# because …` comment
  in the recipe *narrates to whoever runs it* — noise they don't want. `make install`
  should do the work correctly and quietly (`@`-silence the plumbing; surface only real
  errors + a final status). The rationale for a step lives in git history, not on the
  installer's terminal.
- **What a command PRINTS.** Same failure, most visible surface. Say nothing when
  nothing needs acting on — a line reporting that all is well is one the reader learns
  to skip, and the day it says otherwise they skip that too. Keep plan numbers, phase
  names and "not implemented yet" apologies out of it: a user asking a question wants
  the answer, not our backlog. Reserve the full explanation for the moment something
  IS wrong, where it is wanted rather than endured. The test:
  *would a user who does not care about loft notice this line?* If yes, and nothing is
  wrong, remove it. ([GOALS.md § The destination is BORING](GOALS.md))

---

## Worked rewrites

*Illustrative — the BEFORE blocks are composed from the patterns above, not
verbatim quotes.*

```rust
// BEFORE — a good rule, hidden under a plan tag and a history note
// @PLAN52 cluster IV-Vec-nested-field-push (2026-05-30): strict rule —
// VECTOR `+= elem` (bare element) is rejected. … the parser branch
// order used to misroute the latter.

// AFTER — the rule and the reason, in the present tense; no tag, no date
// VECTOR `+= elem` (a bare element) is rejected. When elem is itself a
// vector, it looks the same as concat. Use `+= [elem]` to push one item,
// or `+= other_vec` (same type) to join two vectors.
```

```rust
// BEFORE — ops.rs, a 9-line story about a deletion  →  remove it, or keep one line:
// AFTER
// RNG state lives in lib/random/native/, the single source for both
// backends. rand_pcg stays here only as an indirect dependency.
```

---

## Function descriptions: say why to use it, not why it was written

A function's `///` description is for the **caller**. Say why someone should
call it:

- what it is for,
- when to use it (and when not to),
- what must be true before calling it (preconditions),
- the trade-offs.

Do **not** explain why the function was written, or why its internals have their
current shape. That is design reasoning, and a caller does not need it to use
the function. You almost never have to *write that reasoning down again*: it
already lives in the bug, issue, or plan you implemented this from — the document
you were just reading. **Link to that.** Only write a fresh home (for example a
section in `DATABASE.md` or `LIFETIME.md`) when the reason is a lasting design
fact with no such source — never build doc scaffolding in the middle of a fix.

That link is a *live pointer*, not a dead stamp — see
[§ A. Plan tags and dates](#a-plan-tags-and-dates--git-blame-already-has-them)
for the distinction.

Illustrative (the function `box_captured_names_for_outer_scalars` is real; the
comments are written to show the contrast):

```rust
// BEFORE — the description explains why the function was WRITTEN
/// Added during the closure-capture rework to fix the "Write to locked
/// store" panic: when the parent returns text we must not re-type text
/// captures as Reference, because the closure record is built after the
/// store is locked. Mirrors flip_scalars_to_box_types.
fn box_captured_names_for_outer_scalars(…) { … }

// AFTER — the description says why to USE it; the design reason is a pointer
/// Re-types a lambda's captured outer scalars to their cell form, so the
/// closure record can hold them by reference. Call this after capture
/// analysis and before emitting the closure record.
/// Capture storage is decided per binding — see loft#687 for why the
/// blanket text-skip was retired.
fn box_captured_names_for_outer_scalars(…) { … }
```

---

## Inside a function: comment what the non-obvious code does

A comment inside a function body explains the **code**, not the caller's
decision. It has two jobs:

- **Say what a block does when the code does not already make it clear.** If the
  code is plain, add nothing (see [§ C. Comments that just repeat the
  code](#c-comments-that-just-repeat-the-code)). But a dense or clever block — a
  bit twiddle, a hand-written binary search, a careful order of steps — should
  have one line that says what it achieves.
- **Point to the problem or issue this code is needed for, when that helps.** If
  a block exists to handle a specific problem — a workaround, an edge case from
  a bug, a tricky case written up elsewhere — a pointer to that problem or issue
  is allowed and useful. It tells the next reader why the block is here, and
  where to read more before they change it.

The pointer must be a *live pointer*, not a dead stamp — see
[§ A. Plan tags and dates](#a-plan-tags-and-dates--git-blame-already-has-them).

Illustrative (the hash and `#482` are invented, to show the shape):

```rust
// BEFORE — a clever block with no explanation
let g = (raw ^ (raw >> 13)).wrapping_mul(0x9E37_79B1);

// AFTER — one line on WHAT it does, plus a pointer to the case it must handle
// Mix the slot's generation into the probe offset to avoid clustering.
// Must stay stable across reloads — see issue #482 (slot-reuse crash).
let g = (raw ^ (raw >> 13)).wrapping_mul(0x9E37_79B1);
```

---

## The rules

1. **Describe the code as it is now, never the change.** If a comment only makes
   sense to someone who saw the old version, put it in the commit message.
2. **No plan tags, dates, or resolved-bug history in code comments** — `git blame`
   keeps that. A **live pointer** to a doc/issue/plan that explains the code is
   the allowed exception; see
   [§ A. Plan tags and dates](#a-plan-tags-and-dates--git-blame-already-has-them).
3. **Keep** the five kinds above: rules, the odd-but-needed why, cross-file
   links, costs and dangers, and module intros.
4. **Judge by content, not length.** A 70-line module header can be right; a
   one-line "increment i" is wrong. There is no line limit.
5. **A function description says *why to use it*, not *why it was written*** —
   what it is for, when to use it, what must be true first, the trade-offs. For
   the design reason, **link to the issue or plan you implemented it from** — you
   are usually already reading it; do not rewrite it inline or build a new doc
   home for it. See
   [§ Function descriptions](#function-descriptions-say-why-to-use-it-not-why-it-was-written)
   and [CODE.md § Doc Comments](CODE.md#doc-comments).
6. **Inside a function, explain *what* the non-obvious code does** — and you may
   point to the problem or issue the code handles. See
   [§ Inside a function](#inside-a-function-comment-what-the-non-obvious-code-does).
7. **Write for entry-level and non-native-English readers** — see
   [§ Write for every reader](#write-for-every-reader).
8. **A reference row that names something absent is worse than no row.** A missing
   entry sends a reader to grep; a row promising `seek(self: File, pos: integer)` that
   no such method answers sends them away convinced the *capability* is missing.
   Earned (moros H11): `STDLIB.md` listed that method, the consumer tried all three
   call forms, concluded random access into a binary file was impossible, and
   restructured their file format around the limitation — while the operation was
   documented 22 lines lower as `f#next = pos`, under a name nobody looking for
   "seek" would search. So: when a reference table and the implementation disagree,
   the fix is not only to delete the row — check whether the row is what a reader
   would *reach for*, and if it is, make the name real. And when a capability has two
   spellings, name the other one in both entries, so either search lands.
9. **Document the contract, not the incident.** The body says what the code computes
   and what a caller may rely on — the rule, the domain, the invariant, the trade-off.
   A bug may be *cited*; it is never the *subject*. A feature and a formal rule are
   meant to be timeless, and an incident stops being the reader's question once it is
   fixed. This is a different axis from rules 1–2: a comment can be present-tense and
   stamp-free and still be built around the bug. The move is CONVERSION — extract the
   rule the story contains — not deletion. See
   [§ B2. Comments whose SUBJECT is the bug](#b2-comments-whose-subject-is-the-bug-not-the-code),
   and cite `@FR-<Rule>` wherever `doc/claude/formal/` already states the invariant.

---

## An attribution in a comment is a claim, and nothing gates it

A comment that says *which mechanism causes which symptom* — "narrowing this list un-fixes
that issue", "this arm is what makes the interpreter answer null", "the copy comes from here" —
is an assertion about behaviour, and it goes into the permanent record with no test standing
between it and a reader. Code is gated: a wrong `if` fails a guard. **A wrong sentence fails
nothing.** So the attributions that most need measuring are exactly the ones that feel too
small to measure, because they are going into prose rather than into a branch.

Measured on 2026-09-06, when a two-checkout pairing produced six such claims between them in
one day and every one was wrong in the same way — plausible, cheap to check, and asserted
instead:

- a mechanism named from behaviour that `loft introspect` contradicted (the pass being blamed
  did not run in that program at all);
- a control described without the axis it depends on (*"an undisturbed arm must still alias"*
  is right for a struct tail and inverted for a collection one), which argued for reverting a
  correct fix;
- a one-to-one mapping of "narrowing X breaks issue Y, respectively", where one issue was in
  fact lost by BOTH narrowings, for two different reasons;
- a "values unchanged" two sentences before the measurement that contradicted it;
- a fix credited with closing a case that the falsify control showed already passing;
- an argument position asserted to be the same across two ops that do not share it.

The instruments are all cheap and all already in the tree: read the emitted IR
(`loft introspect`), run the PLAIN spelling of the same shape on the same build, edit the
change out and rebuild, and make each narrowing you are about to describe rather than
predicting it. The last one is the general form — **if a comment is about to say that removing
something would break something else, remove it and look.** A table of what actually fired is
worth the twenty minutes, and it is the only version a later reader can trust.

⚠ **And never use a binary you suspect as the oracle for the suspicion.** When the question is
*did the shipped release do this too?*, the shipped release cannot answer it; build the control
from source, which is what `make falsify` does and why. An oracle has to be independent of the
thing under test — that is a property of what an oracle IS, not a precaution about one bug.

**The practical failure mode is not disagreeing with that rule; it is that the non-independent
oracle is one command away and the independent one is a rebuild.** Measured the same day: twenty
minutes were spent believing a correct fix was over-wide, and what cost them was reaching for
the shipped binary FIRST because it was closest, then reaching for the independent instrument
only once the fast answer came back surprising. By then the surprising answer had already been
believed. So ask **"which oracle is independent here?" before the first measurement**, not after
one that startles you — the ordering is the whole of it, because an oracle consulted second
arrives after a conclusion has formed.

---

## Check

Following loft's own rule — *progress is evaluated, not asserted*
([GOALS.md](GOALS.md)) — this standard ships with a runnable detector, not just
advice. It is a thermometer (a count to watch), the same shape as
`LOFT_STORE_GUARD` for Goal E:

```bash
scripts/lint_comments.sh        # full report, all three patterns
scripts/lint_comments.sh -c     # counts only (the thermometer)
scripts/lint_comments.sh tags   # only history stamps
scripts/lint_comments.sh history  # only change-narration
scripts/lint_comments.sh incident # only incident-subject (§ B2, rule 9)
```

It flags comment lines that match:

- **History stamps:** `@PLAN | @P\d | plan-\d | phase \d | cluster \d | arc [A-Z] | \d{4}-\d{2}`
- **Change-narration:** `\b(removed | no longer | used to \w+ | previously \w+ed | formerly | changed from)\b`

**Read the output as hints, not verdicts.** The script never fails CI and never
edits code. The text of a flagged comment is often worth keeping once you remove
the part that triggered the flag. In particular:

- A flagged `#NNN` or doc pointer that explains why the code is needed is a
  **keeper** — the lint does not even scan for issue numbers, because a live
  pointer is allowed (rule 2). Only the bare plan/date stamps and resolved-bug
  stories should go.
- "used to" appears in innocent sentences ("used to size the gutter"). A human
  decides; the lint only narrows where to look.

A rising count over time is the alarm that history narration is creeping back
in.

### Timeline — baseline ratchet (adopt without a big-bang cleanup)

The tree has over a thousand pre-existing flags. You do not have to fix them all
before the Check is useful. Instead, accept today's flags as a **baseline** and
let the ratchet block only *new* ones, then shrink the baseline over time:

```bash
scripts/lint_comments.sh --baseline   # T0: accept today's flagged lines
                                       #     (writes .lint_comments_baseline)
scripts/lint_comments.sh --check       # CI: list only NEW flags (advisory)
scripts/lint_comments.sh --prune        # after a cleanup pass: drop fixed lines
```

- The baseline is keyed by **file + comment text**, not line number, so it
  survives reformatting and code moving around.
- `--check` runs in CI as an **advisory** job (`comment quality vs baseline`):
  new flags appear as GitHub warning annotations and never block the PR.
- For a cleanup pass, start with the worst files: `scripts/lint_comments.sh top`.
  Fix some, run `--prune` to drop the now-fixed lines, and the baseline's
  shrinking size is your progress. New code stays clean because the ratchet
  flags anything not already in the baseline.

---

## See also
- **`doc-quality` skill** (`.claude/skills/doc-quality/SKILL.md`) — the actionable form of this reference; auto-loads when writing or editing comments and docs.
- [CODE.md](CODE.md) — Code quality rules (naming, functions, doc comments, clippy, deps)
- [DEVELOPMENT.md](DEVELOPMENT.md) — Contribution workflow and validation against CODE.md
- [DOC.md](DOC.md) — HTML doc generation from `tests/docs/` (a different "doc": user-facing language docs)
