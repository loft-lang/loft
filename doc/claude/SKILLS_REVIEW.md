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
| `design-protocol` | 2026-09-07 | `2808e183` |
| `doc-quality` | 2026-09-07 | `ae756431` |
| `draw` | 2026-09-07 | `5156c175` |
| `engineering-rigor` | 2026-09-07 | `2808e183` |
| `formal-rules` | 2026-09-07 | `2808e183` |
| `loft-codegen` | 2026-09-07 | `2808e183` |
| `loft-debug` | 2026-09-07 | `2808e183` |
| `loft-plan-workflow` | 2026-09-07 | `ae756431` |
| `loft-ship` | 2026-09-07 | `2e6a04ba` |
| `loft-test` | 2026-09-07 | `e8ada9b6` |
| `loft-write` | 2026-09-07 | `2808e183` |

## Findings that outlive a single review

Per-skill defects are fixed in the skill and disappear; what belongs here is the
recurring shape a pass observes across skills, recorded so the next pass knows what
to look for first.

**First full pass (2026-09-07, ten skills, one agent-read per skill, every finding
verified against the tree before being acted on):**

- **The worst class is an inverted default, not a dead link.** `loft-test` documented
  the cross-mode matrix as ignore-by-default with a `-- --ignored` run command — after
  @PLN114 flipped the cells to run by default, that command ran ZERO tests and read as
  green. A stale skill can be worse than no skill; check documented DEFAULTS first.
- **Restated volatile facts decay fastest.** Suite counts, timings, ignore-states and
  binary tables (loft-test's table carried two deleted binaries; its numbers were a
  dated snapshot). Cure: state the command that measures, never the measurement.
- **Fixed-bug residue reads as over-caution and occasionally as harm.** Skills kept
  teaching workarounds for bugs since fixed (loft-write's hash/param traps, #1172;
  doc-quality's retired skip-text example) — and one "required idiom" had become a
  compile error. A closed issue cited by a skill is a re-read trigger.
- **Section renames and file splits break pointers the scanner cannot see.** It
  checks FILES exist, not SECTIONS (`CLAUDE.md § Debug logging`, `DESIGN_PROTOCOL.md
  § The other half`, `ownership.md` → `ownership-history.md` all pointed at real files
  and wrong places). A future scanner improvement: resolve `§`-style anchors too.
- **Machine-specific values leak into skills** (a sibling checkout's absolute path, a
  toolchain version pair) — CLAUDE.md's "keep machine-specific values out of shared
  docs" applies to skills with full force.
- **The small routing skills aged best.** `loft-ship`'s 117-line SKILL.md needed one
  wording fix; the two largest restaters produced most of the findings. When adding
  to a skill, prefer the pointer.
