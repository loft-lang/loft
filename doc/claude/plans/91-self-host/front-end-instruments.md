# The front end's instruments — what makes the rest of the port fall into place (@PLN91)

The elaborator is ported one refusal at a time against gate E.  That works, and three
properties of the setup make it slower and blinder than it has to be.  This is the design of
what closes them, in the order of what the END RESULT needs: an interpreter in loft that runs
programs, not a report that matches.  Measured on the tree of 2026-11-17 (suite pin
`0797123`); where a premise was assumed and not measured, it says so.

## What was measured before designing

| Premise | Measurement | Verdict |
|---|---|---|
| "Rust's abandoned attempts are a class worth a facility in the port" | 5 sites mirror one (`abandon_work_refs`), 5 mentions of § Rust accidents in 42,000 lines | FALSE — not a class; no facility |
| "A refused program hides work that already agrees" | 44 refused programs, by function: 331 functions identical, 75 differ and hold a refusal, 125 differ and hold none | TRUE — three quarters of the functions of a refused program already agree, and nothing guards them |
| "Differences behind a refusal are visible before it is lifted" | of those 125: 4 differ in numbers only, 16 in one or two lines, 105 more — a mix of real differences (an element type's borrow, a missing hidden buffer) and of functions the refusal itself emptied | PARTLY — visible, but not attributable today: a refusal records its text once and one line, never the function |
| "The elaborator's result can be run" | `selfhost/front/elab.loft` loads Rust's image for the library, elaborates, prints a report; nothing hands its definitions to the writer | FALSE — there is no path from the loft front end to a run, for any program, agreeing or not |
| "The runner's per-function survey is repeatable" | the tester reproduced a race: every test wrapper is written into one shared directory, where a fixed relative trace file is shared by up to 8 processes | FALSE under load — a harness defect |

## 1. Run what agrees — the front layer

**Invariant.** The report is an instrument; the definitions are the product.  A program gate E
calls `agree` must RUN from the loft front end's own definitions and print what
`loft --interpret` prints.

**Why first.** It is the only item that turns "the interpreter works" from zero programs into a
count, today, for the thousand programs that already agree — and it tests the one assumption
the whole method rests on: that an equal report means equal definitions.  A field the report
does not print is invisible to gate E for ever.

**Shape.** The `written` layer of `run_image_survey.py` already runs the loft writer and the
loft runner on a program's stored IR (`LOFT_RUN_PROGRAM`, an `ir::Bundle` read by
`load::load`).  The elaborator holds an `ir::Bundle` when it finishes.  So the front layer is
`written` with one input changed: a bench program that elaborates the source and hands its
bundle to the writer in the same process — no serialisation — over the run image the Rust front
end still writes for the tables.  One new layer block in the baseline: `front`, the agree count
and every program gate E agrees on that does not run alike.

**Failure paths.** A program that agrees on the report and faults or differs here names a
field the report omits: fix the REPORT first (both sides print it), then the elaborator.  A
program gate E refuses is not in this layer.  The library is still Rust's image; the elaborator
has a `stdlib` mode, and whether its result is compared anywhere is NOT measured here — it is
the next link after this one, since a front end that cannot elaborate the library is not
self-hosted.

**Prediction.** One bench program of about a hundred lines and a layer in the survey; the
first run agrees on most of the agreeing programs and names a handful of report omissions.
If it instead fails broadly, the report is far from the product and that finding outranks
every refusal on the list.

## 2. The function is the unit — verdicts and baseline by function

**Invariant.** A refusal fences the FUNCTION it is met in, not the program.  Every function the
elaborator carries is compared and, once it agrees, pinned.

**Why.** Today 1,183 refused programs contribute nothing: their agreeing functions (about
three in four, by the sample) can regress unseen, and their differing ones surface one lifted
refusal at a time — which is why about one lift in three exposed a difference this week, and
why two fixes to shared rules cost nine agreeing programs before anyone saw it.

**Shape.**
- The port records every refusal with the function it was met in and its line (`el::refuse`
  drops a repeated text and keeps one line; the function is `e.context`), and the report ends
  with one row per refusal: function, line, text.
- The gate splits both reports by definition and gives each function a verdict: `agree`,
  `refused` (it holds a refusal), `beside` (it differs and holds none, in a program that holds
  one).  A program's verdict is derived as now.
- The baseline gains a function layer: the agree count, and nothing else pinned.  `--check`
  fails when a function that agreed stops agreeing, in any program.
- `beside` is a WORKLIST, not a failure: ranked by shape, it is the list of differences that
  are waiting behind refusals, available before they are lifted.

**Failure paths.** A refusal in one function changes another through shared state — schema
rows, definition numbers, a field's united closure record, a generic's instances.  Measured:
4 of 125 differ in numbers only.  So `beside` is never pinned and never fails; only `agree` is,
and agreement is unambiguous.  A function whose own refusal leaves it empty is `refused`, which
the recorded function settles (the line alone does not: a refusal met a second time is not
recorded at all today).

**Prediction.** About thirty lines in the port, about a hundred and fifty in the gate.  The
corpus gains on the order of nine thousand pinned functions (331 per 44 refused programs,
scaled) and a `beside` list of a few hundred, of which an unknown share is noise from emptied
functions — the first run measures that share, and if it is most of the list the worklist is
dropped and the pinning kept.

## 3. The survey runs each test in a directory of its own

The tester's finding, to apply as it stands: `run_image_survey.py` writes each test function's
wrapper into its per-job directory, not the shared one, so a fixed relative file is that
test's own.  Until then the `image-functions` and `written-functions` counts depend on load,
and a baseline written from them pins luck.  Small; before the function layers are re-pinned.

## What is deliberately not built

- **An abandoned-attempt facility.**  Five sites; each is a line or two.  Mirroring them where
  they occur stays the rule (1a-parser.md § Rust accidents).
- **A comparison that forgives numbering.**  Renaming temporaries before comparing made none of
  the 125 sampled differences equal, and an exact comparison is what found the three defects in
  the Rust front end this week.  The comparison stays exact.
- **A local copy of the gate.**  The tester's machine runs it per tip; item 2 makes a local
  sample sharper without adding a second gate.

## Order

3 (an hour, unblocks trustworthy function baselines) → 1 (the product is measured) → 2 (the
net and the worklist).  Each lands with its own layer in a committed baseline; none changes
what the elaborator emits.
