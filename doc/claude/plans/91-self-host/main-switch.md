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

> `rust-free-build.md` and `rust-dependencies.md` are the interpreter stream's pages and live on
> `tuxedo-pln204-int32` until its next join; they are named here, not linked, until then.

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
