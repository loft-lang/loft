# Skills review — validating the standing instructions

The skills in `.claude/skills/` are loaded *instead of* re-reading the canonical docs:
an agent that trusts `loft-test`'s command table never opens TESTING.md to check it.
That is their value — and their risk. loft and its procedures are in heavy development,
so the docs and scripts a skill paraphrases move daily while the skill is only edited
when someone notices. A skill quoting last month's behaviour steers every future
session wrong, in the one channel nobody cross-checks.

**This pass is by hand, and that is not a gap to be closed.** What a script can decide,
`make skills-review` already does: every cited path exists, every cited `make <target>`
is a real target, every cited `LOFT_*` switch still appears in the tree. All of that can
be green on a skill that describes a procedure the project stopped using — reading it
against the tree as it behaves NOW needs a person.

## The three axes of one review

A skill is *validated* when it has been read on all three, in this order — each axis is
a different failure mode, and a skill can fail one while passing the others:

1. **Content — is every claim true of this tree?** Commands run as written, flags mean
   what the skill says, numbers (timings, counts, line references) are current, and the
   behaviour it describes is what the language and tooling DO now, not what they did
   when the skill was written. Run the load-bearing commands; do not eyeball them.
2. **Usability — does following it actually work?** The trigger description fires when
   it should (an agent deciding "does this skill apply?" gets the right answer); the
   steps are actionable in the order given, with no step that assumes knowledge the
   skill neither states nor links; and it ROUTES to the canonical doc for detail rather
   than being a dead end.
3. **Conciseness — is it sized for its purpose?** A skill is working memory, not a
   manual: every fact it restates from a canonical doc is a drift surface that this
   review will pay for forever, so a fact that lives in TESTING.md/DEBUG.md/CLAUDE.md
   is *pointed at*, not copied — unless it is load-bearing at the moment of use (a
   command to type, a trap to avoid), which is exactly what a skill is for. Flag
   sections nobody acting on the skill would read, duplicated tables, and history.

The cure for a conciseness finding is usually to move the fact to its one home and
leave a pointer — shrinking what CAN go stale beats re-verifying it every cycle.

## Do it early — the watermark

Left to tag day this becomes an afternoon of reading under time pressure, which is how
a review turns into a skim. So it is **continuous**, with the same watermark idea
REFERENCE_REVIEW.md and LIBRARY_DOC_REVIEW.md use: each skill records the commit it was
last read through, and it comes back on the list when the skill itself OR any doc or
script it cites moves past that. Review a skill the week its sources change and the
tag-day list is short because the work already happened.

```bash
make skills-review                          # what owes a read
make skills-review ARGS=--verbose           # + the commits behind each MOVED skill
make skills-review ARGS="--done loft-test"  # record one as validated
```

`A-skills-review` reports the count on the release checklist. Sources are derived from
each skill's own text (docs, scripts, make targets it cites), so a skill added or
rewritten tomorrow is tracked without anyone maintaining a second list. Cited `src/`
and `tests/` paths are existence-checked but do not re-open the review: a skill
documents the method, not the code, and source churn says nothing about whether the
method's description still holds.

## Watermarks

The one home for "reviewed through". `--done` edits it; edit by hand only to remove a
row whose skill was deleted.

| skill | reviewed through | commit |
|---|---|---|

## Findings that outlive a single review

Per-skill defects are fixed in the skill and disappear; what belongs here is the
recurring shape a pass observes across skills, recorded so the next pass knows what
to look for first.
