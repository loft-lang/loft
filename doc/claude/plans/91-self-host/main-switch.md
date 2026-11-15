<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Switching main to loft-in-loft — the plan

A child of [README.md](README.md) § The path.  **"Main switched" means one thing:** the
`loft` binary CI gates and a release ships is built from the loft sources through the C seed
(rust-free-build.md (`rust-free-build.md`)), with no Rust in its path, while the Rust
implementation stays live beside it as the comparison under
[README.md § The double implementation](README.md#the-double-implementation--why-the-rust-side-stays-live).
The phases below are read off the two streams' own position tables; each ends at a gate the
tree can measure, and the measured numbers are the streams' at the date of this page's last
edit — the live ones are in rust-free-build.md § Where it stands (`rust-free-build.md`),
[1a-parser.md § Where it stands](1a-parser.md#where-it-stands) and
[12-c-generator.md § Measured](12-c-generator.md#measured).

> The interpreter stream's own pages for this path: [rust-free-build.md](rust-free-build.md)
> (the three levels of a build without Rust, and the end state) and
> [rust-dependencies.md](rust-dependencies.md) (what the runner still takes from Rust).

## Where each part stands (2026-11-15)

| part | measured | remaining |
|---|---|---|
| the runner (the interpreter in loft) | 3,234 of 3,303 corpus programs and test functions agree; 0.86 of the Rust interpreter's time | 12 runner faults, natives called through a function value, lambda generators, the process and lazy-store natives, reflection |
| the bytecode writer in loft | 8,506 of 8,611 functions byte-identical | 104 functions, the text-view rewrite |
| the run image's tables in loft | about half | constants, workers, sidecars, the schema, operator numbers, debug data |
| the lexer and parser in loft | tokens on 1,845 of 1,850 files, syntax on 1,836 of 1,843 | five `"\0"` literals, seven refusal cells |
| the elaborator in loft | 718 of 1,825 programs; 60 of 379 formal rules carried for every cell | dispatch, tuples, closures, calls, interfaces |
| the scope pass, the slots, the IR passes | a first slice; partial; not started | `src/scopes/` (39 k lines), the variable facts, every pass `close_program` runs |
| the C generator in loft | every analysis layer equal over the corpus; about 11,300 of 13,300 bodies under the plain switches | 18 rules, unit assembly, the other forms of the unit, the driver |
| the seed build | built: one `cc` call, 27 MB, checked under an empty environment | where the seed lives, a stable image format, Windows |

## Why the front end is the long pole, and not the C backend

It reads as a surprise: the C backend was the hard part to build, and the interpreter is the
part that was there from the start.  The surprise resolves once the parts are sorted by what
they DO rather than by what they are called.

- **The runner and the C writer consume decisions; the front end makes them.**  Which
  function a call reaches, what type an expression has, which bind copies and which moves,
  where every store is freed, which slot a variable takes: all of it is decided in the parser's
  semantic half (130 k lines), the scope pass (39 k), the variable facts and the IR passes,
  and written into the image.  The runner reads the image and executes it — 71 k lines of loft
  that reached parity with the Rust interpreter in under three weeks.  The C writer reads the
  same image plus `src/opt/`'s decisions and writes text — 25 k lines of loft that reached
  every analysis layer equal in two days.  A decision is the expensive thing to port, because
  each one is a formal rule with its exceptions, and the front end is where the 379 rules live:
  that is why gate E is counted by rule and not only by program.
- **The C backend was built to be ported; the front end was not.**  `src/c/` is three weeks
  old and was written on @PLN200's structure, with its decisions as data in `opt` and `lower`
  and every analysis dumpable as a trace — the port method's rule 6 cost it six commits.  The
  front end is the oldest code in the tree, grown by accretion since the first commit, with
  its decisions spread over the parser, the scopes and the variables and its state in Rust
  types the image never carried (`Definition.width32` was the latest such find).  Porting it
  is not transcription; it is the discovery of what the Rust front end actually decides.
- **The front end port is the dogfood at its hardest, by design.**  Most of the elaborator's
  own failures were language defects (loft#2337–#2346, and the lifetime fixes of this
  fortnight), each fixed in the language with a cell before the port moved on.  That slows the
  port and is its point: the interpreter stream is where loft is being tested as a compiler's
  language, and the C stream inherits a cleaner language because of it.

So the hard part was always the front end.  The C backend only looked hard while it was being
written in Rust against rustc; once it existed as a writer over decided data it became the
easiest part to write again.

**What the interpreter stream builds beside the long pole: the spine.**  The C generator is a
leaf — it reads an image and writes text.  The interpreter stream builds the spine the leaves
hang on: lexer, parser, elaborator, scope pass, bytecode writer, runner, each a loft program
with a gate of its own, and `selfhost/front/loft.loft` is where they are joined into one path
from source to answer.  Once that path runs the corpus, the whole is visible in a way the Rust
tree never made it: one ranked list of what still refuses, by family, across every stage at
once (`elab_gate.py --plan` is its first form), instead of a count per part.  It is also what
lets the C generator drop in at the end as one more consumer of the same image, so the spine
is what both ports eventually stand on.  That is why phase 1's gate is the whole path running
the corpus, never a part reaching a count.

## The phases

Phases 1 and 2 run in parallel, as the two streams run now, with phase 0 ahead of both.

**Phase 0 — the positions become gates (shared, first).**  Every survey in the table above
writes its counts to a committed baseline and fails on any drop or any difference, run on the
nightly ([README.md § The port method](README.md#the-port-method--the-diff-is-the-work-the-reference-is-the-truth)).
At a thousand commits a week between the two streams nobody can otherwise say which push
moved a number.  One session; the item both streams lack today.

**Phase 1 — source in, run (the interpreter stream;
[rust-dependencies.md § The order to build in](rust-dependencies.md#the-order-to-build-in)).**
1. The bytecode writer's 104 functions and the text-view rewrite; the image's tables, so the
   runner's input is written in loft from the Rust image.
2. The front end, family by family as `elab_gate.py --plan` ranks them; then the scope pass
   and the variable facts; then the IR passes — each gated by the image byte-identical before
   and after it.
3. The natives the runner stops on; a library's `[c]` shim built and opened by the runner
   itself; the stdlib's elaborated IR built into the binary as constant data.

Gate: `selfhost/front/loft.loft` runs the whole corpus and the suite from source with every
answer equal to `loft --interpret`'s.

**Phase 2 — loft rebuilds itself (the C stream; [12-c-generator.md](12-c-generator.md)).**
1. The 18 rules still off in plain mode; then unit assembly BEFORE any further rule, so
   `scripts/c_sweep.py` can compile and run the loft-written C against the interpreter — a
   second oracle at no cost.
2. The other forms of the unit, the compiled-bodies fragment, the driver with its shim builds
   and link.

Gate: every unit byte-identical over the corpus and the suite, and the bootstrap closed — the
seed-built binary regenerates a seed equal to the one it was built from.

**Phase 3 — the tool, the subset the switch needs (the interpreter stream, after its gate).**
The driver's options, `loft test`, and `loft install` with the registry and the lock file:
what CI and a release call.  The debugger, the REPL, the language server, `gendoc` and the
exports follow the switch (strand 8).  Gate: each diffed against the Rust tool's output on the
same inputs.

**Phase 4 — the switch (CI; the local stream).**  The PR gate and the nightlies build from the
seed; the release workflow ships the seed-built binary and keeps the Rust-built one as the
reproducible reference artefact; the library CI and the consumer gate run on the loft binary;
a new job builds the Rust side and runs the surveys as the comparison.  Gate: one full release
cycle in which every check ran on the loft-built binary and the Rust side only compared.

## The rustc writer is not kept — the Rust implementation is

The Rust implementation stays live under the double-implementation ledger; the rustc writer
(`src/generation/`, the backend that emits Rust) does not, and is retired ahead of the switch
rather than carried through it.

**Why the ledger reads differently for the two.**  The interpreter, the front end and the Rust
C writer are a second implementation of loft's DECISIONS, gated byte for byte against the loft
ones, so each disagreement is a reading of the language.  The rustc writer is a third writer
over the same decisions: it shares the front end and `src/opt/` with the C writer, so it
differentiates only the text-writing step, which the C sweep already checks differentially
against the interpreter.  Its yield column is near zero.  Its burden column is not: the
interpreter stream's handoffs carry main's nightly rustc-native suite reds (two `B-Copy`, one
`H-Free`) that nobody fixes; the rewrite census on every PR still builds and emits both
writers, so each IR change must keep the rustc writer compiling for the gate to run; and the
rules it alone enforces are still only there (`@FR-B-Ref-Read` has no C site and no exercised
cell).

**What still depends on it:** `--native-android` cross-compiles through rustc.  The browser
page and the wasm target already come from the C writer through clang; an Android build from
the C unit through the NDK's clang is the same shape as the wasm build, and is the one piece
of @PLN203 C6 still owed.

**The order.**
1. **Stop gating on it.**  The rewrite census and the nightly suite run on the C writer only;
   a rustc-native run stays as a report until Android moves, never a gate.
2. **Freeze it.**  A red in it is fixed only if Android needs it.
3. **Harvest before deleting.**  Each rewrite only it holds gets a decision — ported to the C
   writer or recorded as declined (the handoff names one: the accessor view that declines
   every natural spelling); each rule only it enforces gets a C site or a suite cell.  Days of
   reading, and the part a deletion would otherwise lose.
4. **Delete at the Android flip**, by tag, the last release binary kept as the reference
   artefact.

## Decisions the plan needs before phase 1 gets far

1. **Freeze the image format for the switch window.**  The seed design lists a stable image
   format as undecided, and the port's rule that the target moves means every IR change during
   phases 1 and 2 is a change both streams owe at once.  Frozen except under the
   one-commit-both-sides rule, it becomes a schedule instead of a drag.
2. **One owner for the IR passes.**  They sit between the front end and both writers: the
   dependency note puts them in phase 1, the C plan says they come earlier in 1a, and both
   streams read their decisions.  Naming the owner avoids a pass ported twice.

Everything else in the epic — the bundles, the small-board profile, the rest of the tooling —
stays off this path.
