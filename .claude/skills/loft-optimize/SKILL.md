---
name: loft-optimize
description: >-
  How to make loft programs faster on either backend — the interpreter or the compiled (native)
  build — without making the other backend slower or any answer wrong. Use this whenever the
  task is to speed something up: a slow benchmark row, a routine far behind its Rust twin, an
  interpreted script much slower than Python, a profile that points at a hot line, or "close the
  gap". Use it especially when you already feel sure what to build (a new specialised
  instruction, a hand-written Rust helper, a special case in the code generator) — that
  certainty is exactly when to measure the efficient form first. For the compiler edit itself
  pair it with loft-codegen; for diagnosing a wrong answer use engineering-rigor.
user-invocable: true
---

# loft-optimize — write the fast form by hand, then build the rewrite

## The idea in one paragraph

A program gets faster when it does less work: fewer copies, fewer objects created, fewer
conversions, fewer calls, less data pulled through the cache. An optimisation therefore
*removes* work that the natural way of writing the program causes. The reliable way to find out
which work matters is to write the efficient version of one concrete case by hand, look at what
it compiles to, and time it. Only when that hand-written version is clearly faster is it worth
building the general rewrite that turns the natural spelling into it. Building first and
measuring afterwards is how days disappear into changes that buy nothing.

A slow routine is rarely the problem itself: it is evidence of a MECHANISM in loft that costs
more than it should. Any one routine can be slow, and a user can work around it. The aim is to
find the mechanism and remove its cost, so that every program using it gets faster, including
the ones nobody measured. A fix that makes only the routine in front of you fast has treated a
symptom.

## Two backends, one direction

loft runs a program two ways. The compiled build (through Rust and LLVM) is what ships, so it
is the optimisation target. The interpreter is what people use while they develop and debug; its
job is to never be so slow that it becomes unusable — roughly, not more than a hundred times
slower than the compiled program on the same code. So:

- Prefer changes to the shared intermediate representation (the IR both backends read). A fact
  proven once there — "this copy is not needed", "this record never escapes" — speeds up both.
- A change that makes the interpreter faster and the compiled build slower is a regression,
  even if the interpreter gain is large. Check both before and after.
- Interpreter-only tricks come last, and some are ruled out entirely (below).

## The loop

1. **Pick the case by measurement.** Start from a benchmark or profile row that is clearly off,
   and among those the one where you can also *name* a reason. A large profile share with no
   visible cause is a poor first target; a moderate share with an obvious wasted copy is a good
   one. Prefer a mechanism CLASS over a single routine: the portal groups routines by what their
   cost is made of (`bench/portal/classes.tsv`). When a class's median is over the bar the
   slowness is systemic, and one fix to its mechanism moves many routines at once; the worst
   single routine is often a special case of it. Inside a class already under the bar, a slow
   routine is an outlier and can be handled as an ordinary bug.

2. **Measure real time, on the build that ships.** Debug and "semantics" builds are
   unoptimised; their numbers say nothing about the product. Pin the process to one CPU core
   and count instructions and cycles (`perf stat`), which gives a stable before/after even on a
   noisy machine. Be suspicious of any profiler that samples by *number of operations* rather
   than time: it makes a line full of trivially cheap operations look hot. Run nothing else heavy
   beside a measurement — or beside a test gate, where it can also starve the build of memory.

3. **Name the work.** What does the slow version do that a fast version would not? A copy of
   each element, a record created and freed per call, a value converted back and forth, a
   function call where the body is one operation, a collection grown step by step. Read the
   compiled output (the interpreter's bytecode dump, the generated Rust) to see it happen. For
   "why does this get slower as the input grows", count calls at two input sizes before
   reaching for a profiler. Then name the mechanism behind the waste: not "a store is minted per
   call" but "a temporary value passes through store machinery as if it were permanent". The
   mechanism is what the fix should remove; the wasted work is how you found it.

4. **Write the efficient form by hand and measure it.** Take the one concrete case and write the
   version that avoids the named work — in loft if the language can express it, or as the IR or
   generated Rust the rewrite would produce. Check it produces byte-identical output, read what
   it compiles to on both backends, and time it exactly as in step 2. This number is the prize.
   If it is not clearly faster, the named work was not the cost: go back to step 3 rather than
   building anything. Expect the built rewrite to land somewhat under this price — two thirds
   is common — because the remainder is often the call into the new code itself. Accept that
   gap rather than chasing it with inlining or special cases unless a measurement shows real
   work left in it.

5. **Build the rewrite that removes the unneeded work.** Now make the compiler turn the natural
   spelling into that form, under conditions it can *prove* hold (never "usually true"). The
   natural spelling is the one users write, so it is the one to make fast; never ask users to
   rewrite their code around a slow compiler. Put the rewrite where both backends benefit when
   you can. Prefer a runtime improvement (a better allocator, a faster lookup) over a new
   compiler rule when either would do — it changes no emitted program and is easier to verify.
   Prefer the fix that removes the mechanism even when it is larger than a rewrite for this
   shape. A rewrite that covers only one shape is a STOPGAP, and it is never built for the
   bench alone: only when the shape occurs in real programs (grep the consumers and the
   `loft-libs-*` libraries) and the root fix is far off.  It is then recorded as such, with the
   root fix that will retire it, the way `KERNELS.md` keeps every Rust stand-in with its
   removal trigger. Before building, check that the rule's own
   conditions hold on the case you priced: a rule whose gate refuses its target costs a build
   and measures nothing. Two traps when stating the condition:
   - **Sound is not the same as profitable.** State where the gain is paid back, not only where
     the rewrite is correct. Handing a store on to the next call pays only when that call runs
     again; outside a loop the same rewrite was correct and cost an extra allocation.
   - **A whole-function fact cannot be decided while one statement is emitted.** Code emitted
     earlier — an early return inside the loop — is already written by the time the rewrite's
     own statement is reached. Collect such facts in a pass over the body before emission.

6. **Verify it cannot be wrong.** Speed is worthless if an answer changes. Write a test whose
   expected values you computed by hand (two backends agreeing proves nothing — they may share
   the bug), run it on both backends, then deliberately break the rewrite and confirm the test
   fails. Give the rewrite an off-switch and check that on and off produce the same output.
   Confirm the other backend and every other benchmark did not get slower, and that other
   rewrites still fire where they did. Regenerate any file derived from what you changed.
   - **Check what the program holds, not only what it answers.** Many rewrites change
     resources without changing a single value: stores created, records left alive inside a
     store. Compare those counts with the switch on and off (`LOFT_TRACE_DB=1` prints every
     record allocation, `OpDatabase`; `store_memory()` reports live records). A wrong "nothing to release" answer
     leaves every value right and leaks records that no store-level check can see.
   - **Use a measurement that works where you run it.** A resident-memory check reads nothing
     on macOS; a live-record count is exact on every platform. A guard whose channel is blind
     on the machine at hand passes for the wrong reason.
   - **Measure what the fix moved that you did not target.** Compare the class medians before
     and after (`make perf-portal`). A fix that moves only its own routine points at a symptom;
     a root fix moves routines nobody looked at.
   - **When the new rule takes over a shape an older rule handled, run the older rule's pins
     with the new rule switched off.** Otherwise they count the new rule's work as missing
     evidence for the old one, or pass on evidence the old rule never produced.

7. **Record it.** `make perf-check ARGS="--only N --record"` makes the run the new committed
   row (`bench/portal/results/<host>.tsv` is the trend history); commit it with the change and
   put the before and after in the commit message, so the next person starts from numbers.

## Tempting shortcuts, and why they fail

- **A new rule for the shape at hand.** It makes this routine fast while the mechanism stays
  slow everywhere else, and every such rule is one more place a wrong answer can hide and one
  more condition the next change must respect. Never build one for the bench alone: the goal
  is a language that is quick overall, and the bench against its Rust twin is the instrument
  that measures it, not the target.  A one-shape rewrite is admitted only as step 5's recorded
  stopgap — the shape occurs in real programs and the root fix is far off. Rules fitted to the bench routines store fast
  copies of those routines and leave the language as slow as it was. Before building, name the
  programs beyond the bench the rewrite reaches: grep the consumers and the `loft-libs-*`
  libraries for its shape. A `par`-only row mapper was built, measured −49 % on its one bench
  row, then removed when that grep found no `par` anywhere outside the bench. A general
  mechanism it contained (a loop unroll, a range proof) is built on its own, for every loop.
- **A new combined instruction for the hot line.** Fusing a few operations into one special
  opcode makes exactly that operand pattern faster and nothing next to it; it grows the
  instruction set and helps only the interpreter. It is a hand-written kernel in disguise. Change
  what the IR says instead, so the work disappears for every spelling.
- **A hand-written Rust routine standing in for a loop.** It fixes one function name, not the
  pattern, and users' own loops of the same shape stay slow. Make the loft code fast; loft
  compiles through Rust, so it can be.
- **A fix for code layout.** A routine that moved because its code moved (alignment, function
  order, a branch predictor's luck) is noted, not fixed, unless the remedy is easy and gains
  consistently across routines and runs (owner): processors are fickle, and a layout tweak that
  helps one routine on one core costs another elsewhere.  Name the cause as layout with
  evidence (identical emitted Rust, the same instruction count) before setting a row aside.
- **Trusting the first profile.** A profile says what a symbol costs, not what removing it saves,
  and an operation-count profile overstates cheap work. Confirm in real time, then confirm the
  prize by hand (step 4).
- **"Obviously faster" changes.** Many are not. A fold of a redundant conversion that looked
  like a sixth of the work measured 3 %; reading an element in place instead of copying it
  measured nothing. The measurement is cheap; the unneeded change is not.
- **Optimising on a condition that is only usually true.** A rewrite may never change a result.
  If the compiler cannot prove its condition, it does not apply.
- **Complicating the memory model to make temporary objects cheaper.** Remove the temporary
  object instead. A question that only exists because the object is kept is the wrong question.
- **Calling a cost fundamental.** "The memory model costs that" ends the search without
  looking. A copy or allocation still in the compiled code is usually a rule that declined it
  for a nameable reason; ask the rules (their traces) before accepting any cost as the price of
  the language. `make gap-report` lists those declines per routine.
- **Small steps on a routine still a factor off.** Five or ten percent at a time on a routine
  several times slower than its twin means the work is going into the wrong part. First see
  where the gap actually lives (`make gap-report`: the code's own work, the runtime inlined
  into it, the runtime it calls), then work that part as a mechanism.
- **A hand form that changes more than the rule would.** It prices a rewrite nobody can build:
  a hand form that also dropped a safety check measured twice what the lookups alone were
  worth. Change one thing per form, and split a gain by pricing its pieces apart.

## A short example

An interpreted script ran five times slower than its Python original. The operation-count
profile blamed a comparison of a character against a literal, which took seven interpreter
operations. The first idea — a special fused compare instruction — was the shortcut above and
was dropped. The IR turned out to convert the literal back and forth on every comparison;
folding that in the IR (both backends) cut instructions by 6 % but time by only 3 %, because
those operations were cheap. Measuring real time instead found an environment-variable lookup
on every call across the bridge into compiled library code: 5 % of the run, removed by reading
it once. The next candidate, a per-element copy in a loop, was first written by hand without
the copy — it was no faster for this workload, so no rewrite was built for it.

## Where the tools are in this repository

The same loop with the repository's commands, benchmarks and rules is written down in
`doc/claude/PERFORMANCE.md` § How to optimise — the checklist. Useful starting points:
`make perf-check [ARGS="--only N"]` (the targeted A/B against this machine's committed row; a
slowdown exits 1), `make perf-trend` (how a routine moved over the committed history),
`make interp-gap` (interpreter against compiled, per routine), `make perf-portal` (compiled
against Rust), `python3 bench/stats.py` (statistically sound timings), `loft --interpret --dump`
and `loft --native-emit` (what a program compiles to), `bench/portal/hand_price.sh` (time a
hand-edited generated Rust file), and `make rewrite-census` (which rewrites still fire where
they did).
