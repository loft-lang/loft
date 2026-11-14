<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Stability roadmap — history

The record behind [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md): the directive as it read before
the current one, the dated evaluation reads against the meters, and the items closed in earlier
cycles.  Moved here whole; headings are kept, so anchors resolve.

---

## The owner's directive as it read from 2026-09-23 to 2026-11-12

**Attention goes to the engine; loft's job is to stop needing it.** No hurry: the end of
the year is the horizon for a stable loft. What loft must do meanwhile is minimise owner
interrupts — so a question the written rules already answer is answered by the agent that
meets it (the standing ruling in [DEVELOPMENT.md § Where a rule's letter and the code
disagree](DEVELOPMENT.md#where-a-rules-letter-and-the-code-disagree)), and only three kinds
reach the owner: a rule that does not exist yet, a change to a shipped surface, or two rules
in conflict. The months, in order: **October** closes the language walk (`heap.md` is the
least-walked core chapter); **November** is the library pass the freeze doc already owes
([COMPATIBILITY.md § The road to contract 1](COMPATIBILITY.md)); **December** is the read.
The perf stream continues — optimisation is the usability measure game developers judge by —
pointed WIDE, not deep ([PERFORMANCE.md § Wide before deep](PERFORMANCE.md#wide-before-deep)).

**The meters, read weekly — together they are the definition of "stable" the owner uses.**
No figure is written here: each is a query, and a figure in prose is stale the day after.
`make stability-read` computes every row below for each week and `--record` keeps them in
`index/stability_weekly.tsv` ([STABILITY_METHOD.md § The weekly read](STABILITY_METHOD.md)).

| meter | stable when | read it with |
|---|---|---|
| `contract:strained` share of the week's judged fixes | **0 for two consecutive weeks** | `make bug-review` § 5 (weekly window) |
| owner rulings asked for | **0** — a ruling the rules already gave is a process defect | the week's `Contract: strained — letter applied` trailers, read in one sitting |
| the consumer gate | **green with no cure needed, two weeks running** | the `consumer-main-health` nightly |
| `silent-wrong` open **on main** | **0** — `fixed-pending-merge` does not count; the freeze binds what main ships | `gh issue list -R loft-lang/loft --state open --label silent-wrong` minus `fixed-pending-merge` |

**The raw issue count is not a meter.** It is effort-driven: a walk reaching a chapter nobody
had read files dozens in days, and a quiet month can mean nobody ran anything. The measured
case (2026-09-23): consumer-found issues fell from ~120 in August to 5 in September — because
the consumers made no commits after 2026-08-31, not because loft stabilised. Run against
main from scratch copies, their suites went red on three causes, two of them loft moving under
code nobody was running; the gate that sees that is the consumer nightly, not the tracker.

---

## The evaluation reads of 2026-09-28, 2026-09-29 and 2026-10-19

## Evaluation (2026-09-28) — the position read against the meters, and what it changes

A read of the whole stream at one sitting, asked by the owner: *is loft stable, is the path
right, should the priorities move, and is performance part of stability?*  The figures are
the day's measurements — dated so a later reader knows they are a snapshot, not a claim.

**Verdict.** Not stable yet by the meters above, on the right path, and one priority
moves: the `--native` performance bar of
[formal/performance.md](formal/performance.md) (Perf-Weight: 3× a routine's Rust twin,
2× median over the shipped population) joins the meters as the fifth row rather than
running beside them.  It is already Goal G and already a formal rule; what it lacked was
a seat in the definition of "stable", and
[COMPATIBILITY.md](COMPATIBILITY.md) keeps optimiser-rule changes out of
`contract:strained` precisely so the perf stream cannot hold the freeze open.  The
reason it belongs in anyway: a routine a consumer avoids for being slow is an
**unwalked path** in the exact sense COMPATIBILITY.md § What a falling bug rate does and
does not license says the freeze cannot protect, so the perf bar and the freeze bar are
one bar seen from two sides.

| meter | stable when | measured 2026-09-28 |
|---|---|---|
| `contract:strained` share of judged fixes | 0 for two weeks | 122 of 757 trailers over 60 days (16 %) |
| owner rulings asked for | 0 | not counted this read |
| the consumer gate | green two weeks running, consumers active | game consumers dormant since 2026-08-31 (by design — see below) |
| `silent-wrong` open on main | 0 | 1 (loft#1710, filed that morning by the `@FR-Slice-*` walk) |
| rule coverage (`make rule-coverage`) | 70 % annotated / 40 % guarded, floors | 73.4 % / 57.6 % — both met |
| release gate (`release-gate.yml`) | green | never green in three runs; 2026.9.0 shipped on hand-run substitutes |
| shipped routines vs Rust twin (`make perf-portal`) | median 2×, none over 3× | median 2.80×, 73 of 162 over 3× |

**Re-read (2026-09-29), one day on — the same meters, measured again.**  Asked by the
owner as a re-evaluation; the verdict does not move, three rows do, and one row turns out
to have been failing silently.

| meter | stable when | measured 2026-09-29 | moved |
|---|---|---|---|
| `contract:strained` share of judged fixes | 0 for two weeks | 21 of 147 closes in the seven days to this date (14 %) | 16 % over 60 days → 14 % this week |
| owner rulings asked for | 0 | 2 this week (@C129, @C130), both *a rule that does not exist yet* — the kind that may reach the owner; 0 of the process-defect kind | first count |
| the consumer gate | green two weeks running | **red 2026-09-27 and 2026-09-28, `dryopea` only (3 of 4 jobs green)**: the interpreted suite hits the 600 s watchdog in `loot_rate` (`wallet.loft:289`) and again during the parse of `tests/23_k1_composition.loft`, then SIGABRT — twenty minutes both nights, so deterministic; **no issue tracks it** | red, and untracked |
| `silent-wrong` open on main | 0 | 0 (loft#1710 closed in the #1730 join) | 1 → 0 |
| rule coverage (`make rule-coverage`) | 70 % / 40 % floors | 73.9 % annotated / 57.7 % guarded over 395 rules | 73.4 / 57.6 → up |
| release gate (`release-gate.yml`) | green | last run 2026-09-08, red; not dispatched since | unmoved |
| shipped routines vs Rust twin (`make perf-portal`) | median 2×, none over 3× | median 2.80×, 73 of 162 over 3× (portal of 2026-09-27, `7df5a4f4b`) | not re-measured |
| open deviations (`rule_tags.py registers`) | 0 that block a tag | 3: `D-op-1`, `D-op-2` (definitional, never resolvable in a release) and `D-perf-1` (loft#1570) | 6 → 3: D-bind-38/39/44 and D-heap-8/9 closed by #1715–#1729 |
| the tracker | — (not a meter) | 4 open, 0 pick-up work: 3 `fixed-pending-merge` (one parser message, two `ci-advisory` regressions on main's 70079f138 — the browser `make game` and the pinned lib fixtures), 1 `status:planned` | — |

**Trend read (2026-09-29) — the same meters as ten weekly points, not one.**  Asked by
the owner: *are we reaching language stability?*  The tracker by week filed (`created:`
queries), the trailers on `main`'s merge commits by week (the `contract:` meter as
`make bug-review` § 5 reads it), and the tree at each week's end.  Labels begin in the
week of 08-17, `hit-by:` in the week of 08-31; the last row is two days.

| week of | issues filed | `silent-wrong` (share) | not found by loft | `regression` | strained / settled (share) | merges | fixes referenced | guard files | rules defined |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 08-03 | 86 | — | — | — | — | 11 | 80 | 636 | 233 |
| 08-10 | 105 | — | — | — | — | 8 | 103 | 740 | 240 |
| 08-17 | 119 | 26 (22 %) | 50 | — | — | 11 | 119 | 841 | 244 |
| 08-24 | 140 | 52 (37 %) | — | — | 8 / 60 (12 %) | 9 | 108 | 961 | 259 |
| 08-31 | 166 | 80 (48 %) | 4 | 2 | 29 / 158 (16 %) | 9 | 166 | 1206 | 277 |
| 09-07 | 114 | 55 (48 %) | 5 | 0 | 20 / 142 (12 %) | 7 | 109 | 1371 | 321 |
| 09-14 | 28 | 10 (36 %) | 3 | 0 | 13 / 42 (24 %) | 4 | 23 | 1475 | 349 |
| 09-21 | 138 | 73 (53 %) | 9 | 2 | 54 / 236 (19 %) | 14 | 136 | 1789 | 388 |
| 09-28 † | 27 | 11 (41 %) | 0 | 3 | 5 / 20 (20 %) | 2 | 20 | 1835 | 390 |

Beside the table: open deviations 6 → 3 in the last week (two of the three definitional);
`silent-wrong` open on main 0; the consumer nightly red on every night it has existed
(five, since 09-24); the release gate never green (three runs, last 09-08); `main`'s own
push CI green on 7 of its last 60 runs, a figure that counts the advisory jobs and so says
only that the advisory surface has never been steadily green.

**What the series say.**  Two things are moving in opposite directions, and "stable" is
the second one.

- *The contract is converging.*  Rules defined +72 % in ten weeks and +41 in the last two;
  guard files ×3.3; the deviation registers at three with one resolvable; nothing
  `silent-wrong` open on main; regressions two or three a week and mostly CI-advisory
  shape; consumer-found issues in single digits since the consumers went quiet.  That is
  coverage, and it is what the walk is for.
- *Discovery is not converging.*  The week's yield tracks the week's effort — 166 with nine
  merges, 28 with four, 138 with fourteen — and the `silent-wrong` share of what is found
  is RISING (22 % → 53 %), which is the walk reaching the cells the freeze will bind, not a
  language running out of wrong answers.  And the `contract:strained` share, the one meter
  that measures whether a fix still moves a rule or a surface, reads 12 → 16 → 12 → 24 →
  19 → 20 %: flat to rising over six weeks, against a target of 0 for two consecutive
  weeks.  One fix in five still changes what the language promises.

**Verdict.**  Not reaching stability yet, on the evidence; reaching it on the path.  The
meters that will call it stable are the strained share and the consumer gate, and neither
has begun to fall — the first because the walk is still in the chapters that yield (the
owner's October is `heap.md`, the least-walked core chapter), the second because it has
never been green.  The shape to expect if the path is right: `silent-wrong` per walked
week falling chapter by chapter once the core chapters are exhausted, and the strained
share falling with it, since a rule already written is a fix that settles.  The shape that
would say the path is wrong: the strained share still at 20 % after `heap.md` closes, which
would mean the rules are being written to match the code after all.  The December read is
where that is decided; nothing in these ten weeks decides it earlier.

**What the day moved.**  The join to main (#1730) closed five of the six deviations that
blocked the 2026.10.0 tag and the one open `silent-wrong`; the walk's yield is now hygiene
gates on new files rather than wrong answers (this branch's own day: a store in condition
position refused on both backends, a `default/` that does not match its binary refused at
load, and [@C130](DESIGN_DECISIONS_FAILURE.md#c130--a-store-carries-its-own-failure-arm-else-after-the-assignment)
with [@PLN178](plans/178-store-else.md) as the language's answer to a dropped write —
each a refusal or a rule, none a fix to a wrong answer).  Two advisory CI regressions
appeared on main the same morning and were fixed within two hours; that is the cadence
the meters want, and it is the ONLY meter that read well two days running.

**What it changes — one priority, and it is the consumer gate.**  The read of 2026-09-28
recorded the consumers as dormant *by design* and the gate as the instrument that sees loft
moving under them.  The instrument has been red for a week with nothing on the tracker,
which is the meter failing in the one way a meter must not: silently.  So the first item
is not a rule walk — it is to reproduce `dryopea`'s interpreted `loot_rate` on main from a
scratch copy under `LOFT_MAX_OPS` (a count tells a hang from a long run, which a 600 s
watchdog cannot), file it with `hit-by:dryopea` or fix it in the session, and make the
consumer nightly file its own issue the way `advisory-failures.yml` already does for CI —
a red that files nothing is the shape this read caught.  Second, with the deviations at
three and two of them definitional, **dispatch `release-gate.yml` against main once**: it
has not run in three weeks, and its verdict is the only thing that turns "the tag is not
blocked" from a register count into evidence.  Nothing else in the order above moves.

**What the read says is working.** The tracker took 552 issues in the 30 days to this
date and closed 559; 522 of them were found by loft's own rule-led walk, 259 were
`silent-wrong`, 191 `sev:high`.  That is the walk reaching cells no consumer program
has — the only instrument that reaches the paths the freeze will bind, and the yield a
pre-freeze phase should show.  The consumer-hit issues (19 in the same window, against at
least 86 the month before) were closed in a median of under 13 hours from report to pushed
fix.  The edit loop was cut in the same month (@PLN166: front end −48 % instructions, a
warm native run 23× fewer) and is gated by the allocation ratchet.

**What it says is not.**

- *The keystone is not converging on the null class.*  Three keystones in a row left
  null/sentinel at a quarter of new bugs ([BUG_REVIEW.md](BUG_REVIEW.md)), and the review
  itself records the class as mis-cut: five of fourteen residual bugs are one mechanism
  (the nullable spelling lowered differently from its twin, routed to @PLN160), three are
  not null defects.  The keyed-collections keystone of this cycle has no payoff
  measurement yet.  **Re-cut the class through @PLN160 before naming it a keystone again.**
- *A gate the docs claimed and CI did not run.*  `scripts/release-checklist.py` (the
  `M-perf-pass` row) and `releases/2026-10/README.md` said `scripts/native_ratio.sh` fails
  ratios over `bench/ratio_oracle.tsv` in `make ci`, while nothing in `make ci` or
  `.github/workflows/` called it and the one caller (`make native-ratio`) ran without
  `--gate` — the stale-figure failure [STABILITY_METHOD.md](STABILITY_METHOD.md) already
  confessed to once.  **Wired the same day**: `native_ratio.sh --gate` is the last step of
  the local `make ci` (`make native-ratio-gate` runs it alone), kept out of ci.yml because a
  shared runner's timing is noise.  The oracle's bars still sit at 5–8 against the stated
  ceiling of 3: **the ratchet is what is left**, one row at a time as its ratio falls.
- *The release workflow.*  Six open deviations (D-bind-38/39/44, D-heap-8/9, D-perf-2)
  block the 2026.10.0 tag, and `release.yml`'s docs job has failed on every release
  since 2026.7.2.  **Get `release-gate.yml` green once**; a language that cannot pass its
  own release gate is not operationally stable whatever the bug meters say.

**The owner's rulings on this read (2026-09-28), recorded so the next reader does not
re-ask them.**

1. *The games are an oracle, not a stream, on purpose.*  Their pause is the deliberate
   choice to spend the time in loft while the list here is long; they stay in focus as the
   validation targets — the five suites the `consumer-main-health` nightly runs against
   main, and the three consumer lanes in the portal (`bench/16`–`18`, 29 routines).  They
   get development time again **when loft is green on the problematic axis**.  The trigger
   to read for that: the four meters above held for two weeks, and the consumer-shapes
   lane of the portal with no routine over 3× (it read a median of 2.80× with 14 of 29
   over on this date).  What a frozen oracle cannot report — a new shape avoided for being
   slow, a workaround become the idiom — is covered by giving every routine the games use
   a row and a twin through @PLN158's census, and by their agents filing `performance`
   issues with `hit-by:` when they do run (12 such issues exist since July against roughly
   a thousand bugs; that ratio is under-reporting, not health).
2. *The libraries the games produce are the point, and are already in focus.*  They are
   the frozen surface at contract 1 (COMPATIBILITY.md: everything published, no carve-out;
   the games are each library's readiness bar), and the plans to extend them across more
   kinds of game are phase-2 work that rightly lands before the flip.  They are also the
   population that pulls the perf median over the bar: stdlib alone reads 1.90×, the
   libraries 3.12× with 65 of 126 over 3×, and the worst rows are game-library rows in
   their natural spelling (`mesh3d` `mesh_to_floats` 38.7× and `mat4_mul` 36.9×, `cbor`
   `encode_bytes` 26.3×, `stage` `pack_instances` 24.9×, `text2d` `draw_quads` 12.8×)
   — three mechanisms (record-field, vector-build, alloc-temp) behind most of them.  The
   order that follows: **engine classes before library breadth**, the perf census folded
   into November's library pass so the extension plans start measured, and a new library
   routine accepted with a row under 3× or a `D-perf` entry naming the engine class that
   owes it (Perf-Cure: the natural spelling stays in the library, the pressure on the
   engine).
3. *Performance is not the worry; the rate is the evidence.*  Library perf measurement is
   weeks old and moved at class speed: the drawing library from 10–50× behind its twin on
   2026-09-07 to a 1.41× median on 2026-09-21, `text2d` `write_text` from 81× to 3.13×
   inside a week.  One engine change moves every row on its mechanism, and the five rows
   above sit on three.  So the perf axis changes no priority; it only keeps its
   measurement honest (the two items above: the gate wired, the census in the pass).

**Sequenced, not dropped: the interpreter (owner, 2026-09-28).**  The interpreter's
speed — 3.5–16× behind CPython, 50–500× behind native (PERFORMANCE-history.md § Benchmark
results) — waits until compiled speed is at the bar, and then it is looked at.  It will not
measure up to compiled code and is not asked to; what it IS asked is to use **the same
efficient data access as the compiled version** — because a game is DEVELOPED interpreted
(the edit loop) and shipped native, so an interpreter that trails by two orders of
magnitude makes developing the game feel horrible while the shipped product is fine.
The interpreter's speed is the game developer's daily experience, not only the oracle's.  Today every access rewrite
([NATIVE_SWITCHES.md](NATIVE_SWITCHES.md): the hoisted vector header, element bases and
record addresses, loop-local buffers, counted pushes, guarded reads) is generation-time,
so the runtime helpers they call (`get_elem_hoisted` and its kin in `src/vector.rs`) are
reached only from `src/generation/` while the interpreter runs the unrewritten ops through
`src/fill.rs` with a `DbRef` walk per access.  The target is that those rewrites become
BYTECODE-level — a hoisted base or a cached record address the interpreter carries as an
operand — so both backends call one helper and one fix serves both; P1 (superinstructions)
and P2 (the stack pointer cache) in PERFORMANCE.md § Open performance work are the
existing homes, and P2's "low priority" reads as "after the native bar", not "never".

*The light side, and the floor under it (owner, 2026-09-28).*  During development the
parts of a program NOT touched for a while should be compiled and optimised automatically,
so a day's work feels fast even while the part under edit runs interpreted.  That is
[DESIGN_DECISIONS.md § @C71](DESIGN_DECISIONS.md) with its boundary moved inward: today the
line between native and interpreted is "published library vs the user's script", and the
ask is to draw it by EDIT AGE instead — a function or module whose source has been stable
for a set duration is built in the background and dispatched through the same
`OpStaticCall` → `dlsym` primitive the stdlib and native libraries already use, sharing the
one `Stores` heap so nothing crosses a boundary.  Its two real constraints, named so the
plan that takes it starts from them: a struct edit in the interpreted part changes the
layout every compiled part addresses (the artefact key already folds the consumer's
`layout_fp`, so such an edit re-tiers everything on that type), and the debugger forces
interpretation to step and hot-reload, so a tiered function with a breakpoint in it falls
back.  The floor the tier cannot restore: **store access has to be efficient in both
backends regardless**, because no processor cache or interpreter trick makes up for moving
chunks of memory around — and "moving chunks of memory" is precisely the alloc-temp,
record-build and vector-build classes (mint, copy, free) that top the portal.  Those are
shared-runtime paths, which is why the engine classes come first for the interpreter too.  The October → November → December order in the
directive above stands; this read adds the perf meter, the null re-cut, the two gate
repairs and the interpreter's place in the sequence, and takes nothing out.

**Re-read (2026-10-19), three weeks on — the same meters, now a computed series.**  Asked by
the owner as *how far are we from a stable loft, on the metrics*; the rows below are
`make stability-read`'s (`index/stability_weekly.tsv`), which this read is the first to use.

| meter | stable when | 2026-09-29 | 2026-10-19 | moved |
|---|---|---|---|---|
| `contract:strained` share of judged closes | 0 for two weeks | 14 % (week) | 10 / 6 / 14 % the last three full weeks; zero streak 0 | falling, not at 0 |
| `silent-wrong` open on main | 0 | 0 | 0 (one `fixed-pending-merge`) | holds |
| the consumer gate | green two weeks | red every night since 09-24 | red 25 of 25 scheduled nights; its reds reach the tracker only from this read on | no trend to read |
| release gate | green | last run 09-08, red | last run 10-01, red; weekly from this read on | no trend to read |
| rule coverage, annotated / guarded | 70 % / 40 % floors | 73.9 / 57.7 % of 395 | 85.8 / 46.7 % of 527 over tests/ and the rewrite census, the suite checkout absent | floors met; the register grew 33 %, 68 of the 133 new rules rewrites |
| open deviations | 0 that block a tag | 3 | 3, two definitional | holds |
| shipped routines vs Rust twin | median 2×, none over 3× | 2.80×, 73 of 162 over 3× (09-27) | x86 10-16: 1.99×, 49 over 3×, 5 over 5×; arm64 10-09: 2.27×, 71 over 3×, 27 over 5× | falling on every host; the 5× cut met on x86 only |

Filed per week since the table above: 143, 112, 76 — the first fall that is not a quiet
week — with the `silent-wrong` share down from its 53 % peak to 29–40 % and `sev:high`
filings down 42 % between the two 30-day windows (178 → 103), the walk still running.
The register gained 16 rules in the last full week after +79, +38 and +40; the deviation
registers sat at 3 for three weeks; the tracker holds 8 open issues and no pick-up work;
median hours from filing to close stayed between 10 and 24 all autumn.  The rulings
(`@C`, 70 → 93) are read as the rules being consulted, not rewritten — the owner's call on
this read — and are not a concern.

**Verdict.**  Not stable by the meters; closer on four aspects than on 09-29 — the strained
share, the discovery mix, the perf median and the 5× tail — and unchanged on two, both for
the same reason: the consumer gate and the release gate produced no trend because nothing
exercised them, which this read's instruments fix (the nightly files its reds, the gate
runs weekly).  What has not begun to converge: the mechanism classes — three of four monthly
keystones read NO EFFECT and null/sentinel rose again (10.8 → 18.6 %), so discovery is
cooling by chapter exhaustion, not because the classes are closed — and the guard tier
against a register growing a quarter per month, where the rewrite family is now 114 of
527 and its own guard is the census.  On the trends alone, four meters read stable by the
December read; the two gates have no trend to extrapolate until they have run.

---

## Done (this cycle — closing commits in the canonical homes)

- **#462 adopt-and-re-return vector leak — FIXED** 2026-06-26 (commit `cafe98a0`). A vector-returning fn that adopts a call result and re-returns it (`t = base(); t`), and the `t = base(); t += …; t` merge shape (`game_items()`/`game_monsters()`), leaked one store per call: the NRVO collapse redirected the inner call's hidden `__ref_N` buffer onto the retbuf but left its eager allocation orphaned. Fix in `parser/control.rs` (`nrvo_collapse_tail_set` + new `nrvo_collapse_defining_call`); vector-only. Crawler interp 531→0, native 752→216 (the 216 is the 462-leak row above). Regression `tests/leak_cases/clean/p462_adopt_rereturn_vector.loft`, both backends. #462 stays open for the record-leak residual.
- **Cluster A residuals #426 + #429 — both CLOSED** 2026-06-22 (see the A row above).

- **Pass-2 arity-growth CASCADE — COMPLETE.** Reference arm 2026-06-12 (one-buffer design: arity = signature+1 fixed at declaration, every return path delivers in THE `__retbuf`; 10-cell matrix green both backends). Vector/struct-Enum arm then closed via **#355** (multi-return-site vector behind a forward caller silently returned the WRONG element) and **#356** (mid-body `return f(g(x))` returned the null sentinel on native; fn-refs to struct-returning fns couldn't be CALLED — the latter also fixed by **#383**, merged in #393 this session). Both #355/#356 CLOSED. Live guards `tests/issues.rs::pass2_arity_growth_*` + `loft-suite/attic/scripts/387-text-fn-ref.loft`. The @PLN55 growth assert (the H2 residual) fell out — cleared.
- **Armed-corpus residuals ×4** — all resolved 2026-06-14: `132` silent interpreter UAF → `OpFreeRefIfDistinct` (regression `tests/scripts/372`); `collections.loft` two bugs → `parse_object_field` accepts `{}` (regression `373`) + `dedup_keyed` `secondary` flag for `other_indexes` (regression `374`); `166` verified over-strict; `75-native-stub` clean on the rebuilt armed binary. The armed channel's silence is now trustworthy.
- **H1 analysis-dependent arity** — RETIRED 2026-06-11 (@PLN55 phases 0–2; signature-time `__retbuf`, uniform return ABI, retro-patch deleted).
- **H2 typed deps** — steps 1–5 DONE 2026-06-12 (`Deps` newtype, space-asserting accessors, the positional contract retired via `CALLEE_FRAME_BIT`).  Residual (the @PLN55 growth assert on two lib fns) → cleared with the arity cascade above.
- **F11 error-path state** — swept, all four breaks FIXED 2026-06-12.
- **The armed-channel restoration** — four stale duals fixed 2026-06-12; the channel is the standing instrument.
- **`store.rs:1640` armed row (the "keyed armed UAF", 7 files)** — RESOLVED 2026-06-12 (4cba84c5): three mechanisms (header-as-`room` accessor → `Store::record_words`; parallel s_pos array header stomp; OpDatabase bytes-vs-words under-claim = a real release OOB write).  Armed corpus 12 → 5 files.
- **Plan-57 vector store-lifetime watermark** — CLOSED (@PLN2; rc removal complete).
- **Plan-53 cluster 1 + the aligned-V2 allocator half of cluster 2** — fixed/validating; the S4 half is queue #4.
