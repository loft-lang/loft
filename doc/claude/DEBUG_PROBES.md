// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Debugging Strategy: probe matrices

The probe-matrix runner and the rule that the axis your cells hold fixed is where the defect hides.  Part of the debugging guide: [DEBUG.md](DEBUG.md).

---

## Boundary-matrix runner (`scripts/probe-matrix`)

The mechanics for CLAUDE.md § "Before fixing a non-trivial bug" — runs a
directory of probe cells uniformly and enforces the matrix-validity rules
as hard errors, so a matrix cannot silently measure nothing.

```bash
scripts/probe-matrix init /tmp/p_followups/mybug   # scaffold (template + control cell)
# … copy the template per cell, vary ONE dimension each, hand-write @EXPECT …
scripts/probe-matrix /tmp/p_followups/mybug                          # interp, fast iteration
scripts/probe-matrix /tmp/p_followups/mybug --backend both           # final verify (native pays rustc per cell)
scripts/probe-matrix /tmp/p_followups/mybug \
    --baseline .claude/worktrees/prechange/target/release/loft       # A/B classification
```

Each cell is a plain `.loft` program with header annotations:

| Annotation | Meaning |
|---|---|
| `// @EXPECT: <line>` | expected stdout line (repeat per line, exact match) — **mandatory**; hand-compute it BEFORE the first run, "two binaries agree" is not a pass |
| `// @EXPECT_LEAK` | a `stores not freed` warning is the expected outcome (red-documenting cells) |
| `// @CONTROL` | deliberately wrong expectation; the run errors unless this cell FAILS (proves the harness detects failure) |

The runner FAILS on: any non-control cell mismatch / crash / unexpected
leak, any cell with no stdout (**vacuous** — a parse error reads as silence,
not as green), any cell missing `@EXPECT`, and a missing or *passing*
control cell.  With `--baseline`, failing cells are labelled
`=> REGRESSION` (baseline passes) or `=> PRE-EXISTING` (baseline fails too)
— keep a main-tip worktree built for this (`git worktree add` +
`cargo build --release --bin loft` inside it).  Leak detection: interp
reads the exit warning; native runs under `LOFT_NATIVE_LEAK_CHECK=1`.

**The REPORT's axes are the reporter's, and the ones it holds fixed are where the rest of the
defect lives.**  A matrix grown outward from the filed reproducer inherits its choices, and
those choices are not a sample — they are whatever the reporter happened to run into first.
Measured on loft#1443 (a closure written into a `&fn(…)` parameter, an ICE on both backends):
the reproducer CAPTURES, and a capturing lambda already emits the fn-ref pair, so the first of
three fixes made the filed program pass while a NON-capturing lambda, a bare fn name and a
second write through the same link stayed broken on `--native` — one of them a silent prune
that panicked `invalid fn-ref` at the call.  The issue even said which axes it was holding
fixed (*"a capturing closure is not required either"*), and that sentence names the cells that
had to be built.  So read the report for what it VARIES, then build the cells it does not: the
fix is finished when those pass, not when the reproducer does.  The corollary for a large leg
is that sampling should cross the held-fixed axes rather than the moving ones.

**A missing WARNING is not a passing cell.**  The vacuous-cell rule above is about
empty stdout, but the same trap has a second door: reading a cell through a
*diagnostic channel* — no leak warning, no error, exit 0 — while never checking the
value.  Chasing loft#835 (an abandoned generator leaks its vector) the matrix was
scored on `grep 'stores not freed'`, and `g = steps(); for x in g { … break … }` came
back clean.  It read as a clean workaround and went into the issue as one.  It is not:
that form never runs the generator at all, yields `65535` forever, and only stops
because the probe happened to `break` — one run without the break wrote **1.5 GB** to
stdout.  A program that does no work leaks nothing, so on a leak-only channel the most
broken cell in the matrix scores best.

The rule that catches it is the one already written for `@EXPECT`, applied to every
channel: **the cell asserts its VALUE, and the diagnostic is an extra assertion, never
the only one.**  `@EXPECT` plus `@EXPECT_LEAK` together, not `@EXPECT_LEAK` alone.

**And a third door: a channel CAPTURED but never compared.**  The two above are about a
check that looked at the wrong thing.  This one is about a check that collected the right
thing and threw it away.  `tests/differential_oracle.rs` has recorded each backend's
stderr in its `ModeRun` since the day it was built, and used it only to grep for the leak
substring — the two backends' diagnostics were never compared to each other.  That is how
the same failed `assert` printed a loft diagnostic on `--interpret` and a Rust panic
naming `/tmp/loft_native_*.rs` on `--native` for as long as both existed (loft#1056),
while a green oracle sat over it.  A field in the harness's own result struct reads like
coverage and is not.

**A fourth door: a cell that does not USE its value does not compile the code that
would have faulted.**  The three above are about which channel a cell reads.  This one is
about whether the faulting code is ever emitted at all — and it can turn one defect into
an apparent backend difference, a runtime panic, a compile-time ICE, or silence,
depending only on how the cell reads.

Probing the nested-tuple fn-ref hole (`t = ((dbl, 1), "z")`, nothing else) reported it as
native-only: rustc E0308 on one side, a clean `built` on the other.  It is not
native-only.  The reason was in the probe's own output — `warning[never-read]: Variable t
is never read` — and a cell that reads the tuple back faults on both.  Measured on a
pristine tree, the *interpreter* gives two different failures depending on what the read
is: reading by CALLING the nested fn panics at runtime (`fn_call_ref: fn_var=16 < 20`),
while reading only PLAIN members dies before running at all, an ICE in
`state/codegen.rs` (`attempt to subtract with overflow`).

So "runtime backend vs compile-time backend" is the wrong axis, and reaching for it is
the trap: **`--interpret` compiles too** (parser → IR → bytecode), so it has its own
compile-time failures.  What actually varies is whether a tuple read is emitted, and a
construct-only cell emits none — which is why `--interpret` says `built` for the same
reason `--native` says nothing: nobody asked.

A cross-backend cell must therefore **use** what it builds, `warning[never-read]` on a
probe means the cell is inert rather than that the lint is noisy, and when one backend
disagrees with the other about whether a bug EXISTS, suspect the probe's shape before
believing the split.  ⚠ Do not read an ICE on one side and a panic on the other as two
different bugs — on this one they are the same defect, and the ICE cell is the sharper
statement of it: the damage lands on whoever reads next, and that reader need have
nothing to do with functions.

**A fifth door: a cell whose SPELLING is lowered to something else.**  A cell that binds `c =
&place` and only reads `c` gives the right answer whether `c` became a link, a copy or a view.
So it can pass without running the mechanism its assert message names.  Measured in one guard
for `(B-Ref-Repoint)`: `e = &es[1]` on an enum element lowers to `e: Col = OpGetEnum(…)`, a
copy with the `&` dropped, and `s = &ps[0]` on a struct element to `s: ref(P)`, a view.  Both
cells were green and claimed "a link re-points".  The dropped `&` was itself a silent defect
(a write through it is lost, `binding.md` D-bind-39).  It showed only because a cell that
WROTE through the link to a MIDDLE element failed.  Check the bind's type in the cell's `loft
introspect` output, and give each cell a write through the link plus a neighbour after the
element: a copy then fails on the write, and an op of the wrong width fails on the neighbour.

The doors together give one question to ask of any instrument: **for each channel
it captures, name the assertion that compares it, and name a case where that assertion
FIRES.**  A channel with no comparison is the third door; a cell whose value is
never used is the fourth; a cell lowered to a construct other than the one it names is the
fifth; a comparison with no case that
can disagree is the "exercised by nothing" trap (thirty corpus programs all exited 0, so
an exit-code comparator that had run since the oracle was built had never once compared a
NON-ZERO code); and a comparison scored on the wrong channel is the first two.

**A filed blocker is a hypothesis, exactly like a filed root cause.**  CLAUDE.md already
says an `OPEN: 0` line is a claim to re-measure; the same holds for the sentence in an
issue that says why it was NOT fixed.  loft#1056 was filed with "converging the two
renderings would lose the loft call frames, so it needs a decision about `panic`'s output
too" — written from reading `report_and_exit` and the browser panic hook.  Measured, the
frames did not exist to lose: `RuntimeError::call_chain` is hardcoded `Vec::new()` at both
the `user_panic` and `assertion_failed` constructors, so neither backend printed any.  One
`--interpret` run of a three-deep call chain says so in ten seconds, and the "design call"
evaporates.  Before honouring a blocker written by anyone — including yourself — run the
probe that would show it is not there.

**The INSTALLED `loft` is a free before/after oracle.**  `$(which loft)` is
whatever `make install` last put there, so during a fix it is a ready-built
binary from before your edits — no worktree, no second build, and none of the
tree-destroying moves (`git stash`, `git checkout HEAD -- <file>`) the
[debugging policy](../../CLAUDE.md) forbids.  Three uses, all of which paid off
in the nested-narrow-width fix:

```bash
loft --interpret probe.loft            # baseline: is this failure mine or pre-existing?
./target/debug/loft --interpret probe.loft   # ... vs the working tree
loft --interpret tests/scripts/new-guard.loft   # must FAIL — proves a new guard isn't vacuous
```

Check its date first (`ls -l $(which loft)`) so you know which commit you are
comparing against; re-running `make install` mid-investigation destroys the
baseline.

Graduate the cells that earn guarantees into `tests/scripts/` when the fix
lands (protocol step 7) — e.g. `302-vector-buffer-delivery.loft` /
`303-ref-reassign-free.loft` are graduated matrices.

---

## The axis your cells hold FIXED is where the defect is

A matrix that reads green on a build which HAS the defect is the expensive failure, because it
retires the question. Both instances measured in one session moved the same axis by accident and
then held it fixed on purpose.

**Reproduce the filed spelling before generalising it.** loft#1471 reports that an out-of-range
read of a `vector<value struct>` fabricates a zero record. A thirteen-cell matrix built from that
sentence read **11/13 green on the merge-base**, and the two reds were unrelated — so the issue
looked already fixed. It was not: every cell asked `v[9] == null` INLINE, and the filed
reproducer BINDS first (`a = v[9]; a == null`). Inline, the read is never materialised; bound, the
materialised copy of an absent value was allocated and zeroed and the absence was gone. The bind
was in the report and out of the matrix, which is the specific way a paraphrase loses a defect.

**A guard's own header is the place to name the axis, because the next reader will paraphrase
too.** The cure is one line in the file — keep the inline read as a CONTROL beside the bound one,
so the pair says *these differ* rather than leaving the reader to rediscover it.

**Then ask the tool, because counting axes by hand is what keeps failing.**
`python3 scripts/matrix_axes.py file <guard.loft>` reads the axes a finished guard actually
reaches. On that same file it named three it did not — a PARAMETER-provenance read, evaluation in
a LOOP, and `(Col-Lookup)`'s keyed miss — and all three were on the ISSUE'S OWN list of rules that
construct a `τ?`. None was broken, which is the ordinary outcome and still worth the minute: an
unreached axis with a bug history is a probe to build, and the ones that pass become cells rather
than neighbours.

**A deviation's measured cells are a claim to re-measure, not a record to cite.** `D-tup-10`
listed four cells as REFUSED; three of them answer correctly today, carried along by unrelated
work on the null model. An entry that overstates what is broken sends the next reader to fix
something that already works.

---

## Count it before you time it

**A symptom that appears only on one target usually has a target-INDEPENDENT count behind
it.** What code does — ranges fetched, bytes, pages resident, allocator calls — is a
property of the algorithm and is the same everywhere. Only the *price* of each operation is
target-specific. So when a defect is reported somewhere you cannot easily run, ask first
*what does this change the count of?* and assert that count here.

loft#787 was a browser-only 1.14x on a paged load. Three native probes saw nothing, so a
CDP harness was built — three served roots, rotating arms, a cold browser per sample, four
kernels. Every rung of that ladder landed inside a +/-100 ms noise floor. A counting
`GlobalAlloc` over a one-page read range settled the same question in 20 ms:

| | 200 reads | 300 000 reads |
|---|---|---|
| before | 4 | 300 011 |
| after | 0 | 0 |

One allocation per read (`resolve` returned an owning `Vec`, so a 4-byte index word cost a
malloc and a free). Invisible natively only because glibc's tcache is ~15 ns — the *count*
was always readable. `tests/paged_read_alloc.rs` is the pattern:

- **A counting `#[global_allocator]`** in the test binary, armed around a tight window.
- **Assert a SCALING property, never a bare zero.** Same work at 200 and 300 000 iterations,
  counts must be EQUAL. A pinned zero breaks the day a read range widens by a page and says
  nothing about the defect; the defect was that the count tracked reads.
- **Run the control.** Restore the pre-fix file, watch it fail, restore. A harness not shown
  to fail has asserted nothing (`CLAUDE.md` matrix rule 3).
- **Say what stays non-zero and why.** A span read still owns one buffer per *record* — the
  right unit — and a test states that, so a later "drive it to zero" knows what it is
  breaking.

Reach for the target's own harness only for the part that is genuinely a price.

**How it ended, because the sequence is the lesson.** The reported ratio went
`1.5x` (un-interleaved, withdrawn by the reporter) → `1.14x` (interleaved, real) →
`1.03x` and, on the headline workload, **694 ms against a 763 ms pre-arc baseline** — faster
than before the work started, with 42 fewer reads. Two per-read costs did it: an allocation
per read, and hashing the page key twice per read. Both were found by counting, both were
invisible to three native wall-clock probes, and the browser A/B built to see them could not
resolve either — every rung of that ladder landed inside its own noise floor. **The counts
were exact on the first try.**
