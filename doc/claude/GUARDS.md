<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Guards — does this test catch the defect it names?

A test that cannot fail is worse than none: it is a standing claim that a behaviour is checked.
This doc is how to make a guard able to fail, how to prove it does (`make falsify` and its
receipts), and the shapes in which a guard reads green while the defect stands.  Writing an
ordinary test: [TESTING.md](TESTING.md).  The incidents each rule came from:
[TESTING-history.md](TESTING-history.md).

## Testing race-prone and backend-divergent mechanics

Two rules govern the hard cases — concurrency and interpret-vs-native divergence.  Both say
the same thing: **the answer lives at small scale; large scale only verifies that the
small-scale answer was sound.**

### Race conditions: reason small, scale only verifies

A race is a property of the *mechanic* — which memory is shared, accessed how, synchronised
how — and that is established by **reasoning about the mechanic at n=1, deterministically**.
A stress run does not *find* a race; it verifies that the small-scale soundness claim held.
A clean 10 000-iteration run proves nothing about absence (the window may be narrow), and a
dirty one is only a louder hint.  (The same distrust of statistics as the store refcount in
[GOALS.md § Goal E](GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth).)

The ladder: **reason the mechanic small → state the soundness claim → only then**, if the
claim is "race-free *because* lock X holds under contention", reach for stress or the
[plan-53 sanitizer](plans/finished/53-sanitizer-ci-lever/README.md) to confirm lock X holds.
A mechanic whose small-scale reasoning concludes "no shared mutable state" leaves nothing for
stress to verify.  Worked example: `parallel {}` gives each arm a **read-only heap clone**
(`clone_locked_for_worker`) and a **private stack** — deterministic isolation, so every
failure is deterministic too, and n=1 probes were the whole answer.

### A fixed delay is not synchronisation — use a barrier

A fixed pause before a partner is assumed ready is a **bet that its startup is faster than the
pause**; the variance lives in process startup (spawn + parse + connect), which no wall-clock
number bounds.  A bigger pause only lengthens the bet.  **Make the concurrency a fact**: the
driver waits until every participant has reported ready, then creates a rendezvous file all
of them spin on (`multiplayer_v2`'s `LOFT_TICTACTOE_GO_FILE`).  Three properties to copy:

- **The barrier releases unconditionally**, so a missing participant fails on a real assertion
  instead of hanging.
- **The spin is bounded** (30 s), so a driver that never writes the file degrades rather than
  wedging the run.
- **Remove a stale barrier file before spawning**, or a killed run's leftover lets the next
  run sail through and silently restores the flake.

With overlap guaranteed, assert that **both** sides observed each other — an OR over two
directions is satisfied by a half-working router.

A flake is diagnosed by **what the failing environment RAN**, not how busy it was: synthetic
load (busy loops, many startups) is preemptible and reproduces nothing, while being scheduled
beside `rustc` subprocesses is what blows a timing budget.  Try one re-run first — two runs of
one binary disagreeing is the cheapest proof of a flake.  Then **delete the race rather than
widen the margin**: `thread::sleep` guarantees a minimum, so a longer sleep only moves the
failure rate.  Sequence the steps deterministically (drive the second event from inside the
first's callback), exit on a **condition** under a deadline, and **assert the sequencing**
(`attempts == 2`) — or the test passes for a reader that never re-read at all.

### `assert`'s message runs too

`assert(cond, msg)` is an ordinary call, so **both** arguments are evaluated
([formal/calls.md](formal/calls.md) `F-Args`).  A side-effecting call written in the condition
and again in the message runs **twice**:

```loft
c: vector<integer> = [0];
assert(bump(c) == 5, "got {bump(c)}");   // c[0] is now 2, not 1
assert(c[0] == 1, "…ran once");          // fails, and the fix is not here
```

Bind first (`got = bump(c); assert(got == 5, "got {got}")`).

### Backend divergence: the differential check is the instrument

Interpret-vs-native disagreement is also an n=1, deterministic property, and the `cross_mode!`
validation matrix is the instrument for it: every cell runs on both backends.  **A per-backend
assertion that passes on both backends does not prove parity** — `--native` once no-opped
`parallel {}` arms, and asserts that hold *when the arms do nothing* passed there.  A real
parity check asserts **identical observable output across backends**, not "each backend
satisfied its own assertion".  Parity (GOALS.md Goal D) has no standing detector the way Goal A
has the sanitizer and Goal E has `LOFT_STORE_GUARD`; `cross_mode!` with a cross-backend oracle
is the closest.

### A round-trip test is closed under its own encoder

A **write, read back, compare** test only exercises inputs its own writer produces.  Every
value the writer cannot emit is untested *and unmentioned*, and coverage reads 100 % because
the decoder really was entered.  (`imaging` read only the 8-bit RGB its own encoder writes;
RGBA, grey, palette and 16-bit files came back corrupt — @PLN141.)  The instrument is an input
the code under test **did not produce**: a fixture authored outside the system, with
hand-computed expected values.  Asking what a *worked example* for the function would have to
assert names those outside values by construction.

## A guard that never failed is not a guard — `make falsify`

**`make falsify GUARD=tests/scripts/<file>.loft REF=<commit-before-the-fix>`**
(`scripts/falsify.sh`) builds `REF` in a cached worktree, runs the guard THERE and HERE, and
compares four channels apart — **exit code, assertion failures, leaked stores, panic** — plus
the `refusals` and `expect` columns for annotation-scored files, so the verdict names *which
one moved*:

```
backend      tree       exit|asserts|leak|panic       verdict
interpret    control    0|0|kt=78 Sk×156|none
interpret    here       0|0|none|none                 falsified
…
// @falsified-at: 3ca5ec79 — interpret leaked kt=78 Sk×156 -> clean, native leaked … -> clean
```

Paste that line into the guard.  `doc_hygiene::every_new_guard_records_its_control` requires
one on every file added under `tests/scripts/`, against the ratchet in
`tests/falsified.baseline`; `// @falsified-at: none — <reason>` is the honest opt-out for a
file that cannot fail on any earlier build.

**Reading the result:**

- **Arm the instrument the defect needs.**  A leaked store is not an exit code, so a leak guard
  scored plainly reads `leak none -> none`.  `LOFT_STRICT_STORES=1 make falsify …` (plus
  `LOFT_POISON=1` for a use-after-free) passes both through to control and HERE; record the
  invocation in the `@falsified-at:` line, and name the CI leg that runs with that instrument
  (the nightly poison sweep), because on the plain suites such a guard passes on every build.
- **A refused tree is the useful half.**  `THIS TREE IS NOT CLEAN` means the guard fails here,
  so it says nothing about what it can catch — narrow the cells to one mechanism.
- **Read the `asserts` column and the per-backend rows, not the verdict.**  A control that
  fails to compile "falsifies" on the exit channel alone.  The verdict is an AND across
  backends, so a native-only defect (the interpreter correctly INERT) reads NOT FALSIFIED while
  its row moved; record which channel is the real one in the guard's header.
- **Annotation-scored files are scored through the suite.**  A passing `@EXPECT_ERROR` guard
  exits 1 on both trees, so the exit channel cannot move; the `expect` column (`FAIL/6 -> 6/6`)
  can.  `falsify.sh` runs such a file through `--tests`, because a pass-1 refusal silences every
  pass-2 diagnostic in a direct run and `tests/wrap.rs` already peels that (loft#1242, loft#1253).
- **A control that ABORTS scores only the abort.**  An ICE or parse refusal on the control stops
  it at the exit channel with `asserts 0|0` — true of the abort, silent about every cell behind
  it (loft#1310 masked two silent-wrong defects that way).  After fixing a compile-time abort,
  re-run the matrix on VALUES and say in `@falsified-at:` which channel the tool scored.
- **Zero assertion failures on the CONTROL is the tell** for both an abort (read its exit
  column) and a guard that ran nothing.

**Shapes it reads as INERT** — which means *this tool cannot see this kind of fix*, not *your
guard measures the wrong thing*:

- *A guard with no `main`.*  `falsify` runs the file as a PROGRAM, so a file whose cells are
  each their own entry point runs nothing.  Give it a `main()` that calls its cells.
- *A guard whose subject is a WARNING.*  `expect_channel` counts only `@EXPECT_ERROR` and
  `@EXPECT_FAIL`, and `entry_modes` routes only those to `--tests`; an `@EXPECT_WARNING` guard
  with a `main` gets a direct run where nothing matches warnings.  Score it by hand (run both
  builds, count the reports) and say so in `@falsified-at:` (loft#1397's guard is the example).
- *A defect the harness cannot see* — a leak freed at frame exit, a script-classifier fault; see
  [§ The harness is not the program](#the-harness-is-not-the-program).

**Concurrent runs are serialised**: every build takes its target directory's `flock`, so a run
arriving mid-build waits.  `this tree does not build`, an INERT backend with an unclean `here`,
or `exit 127` in a bulk row on a checkout older than that lock means a concurrent run, not the
tree.

**The control builds are cached and pruned.**  Each ref costs about 2 GB under
`~/.cache/loft-falsify/<ref>` and `<ref>-target` — on disk, never in `TMPDIR` (often a RAM
tmpfs; the native leg links `libloft.rlib` and its dependency rlibs) — beside a `head-target`
and a `shared-target`.  `LOFT_FALSIFY_KEEP` (default 4) most recently used controls are kept.
A `--patch` control is cached under `patch-<patch content>-<HEAD>`, so a rebased HEAD never
reuses a stale control.  A gate that fails on an unrelated suite right after `low space`
lines is the disk: `df -h /`, then [RUN_BOUNDS.md § Scratch hygiene](RUN_BOUNDS.md).

**The sha in `@falsified-at:` is a RECEIPT, not a pointer.**

- **A cherry-picked guard keeps the peer's sha.**  Re-pointing it at your own "equivalent"
  commit replaces a measured statement with an assumed one; re-point only with a fresh
  `make falsify` run whose output you paste.
- **`origin/main` is not a free substitute** where guards CHAIN: a guard whose control is the
  previous fix in a series moves a channel against main for the EARLIER defect's reason
  (loft#1472 → loft#1471, loft#1479 → loft#1478).  Say which shas depend on which branch.
- **Resolvability is a property of the CHECKOUT.**  Many controls live only under
  `refs/pull/<n>/head` (a GitHub convention with no retention contract) or in one clone.
  `falsify.sh` fetches that namespace once on the first unresolvable control and says when a
  control was recovered that way; the bulk sweep reports `no-such-ref` apart from
  `no-worktree`.  A census of reachable controls therefore measures the fetch state too:
  state the tree and the ref state a total was taken on, or do not subtract two of them.
- **The gate is presence-only** (`src.contains("@falsified-at:")`), so it cannot see a receipt
  that stopped resolving — and gating on resolvability would redden a fresh clone more than a
  warm one.  It also accepts a placeholder (`@falsified-at: PENDING`) that was never derived,
  which is worse; rejecting placeholders has no fresh-clone asymmetry and is an open contract
  decision.
- Checking that a sha exists: `git rev-parse --verify --quiet` or `git cat-file -e`; a plain
  `git rev-parse` prints its argument even for a missing object.

### The patch receipt — `@falsified-by:`

The durable form carries the defect instead of pointing at it.
**`// @falsified-by: tests/falsified/<guard>.patch`** names a patch that reintroduces the fault
on top of HEAD, and `scripts/falsify.sh <guard> --patch <file>` scores it exactly as a ref
control is scored.  Nothing outside the repository has to survive for it to be re-run.

- **Record the patch when you falsify.**  `falsify.sh <guard> <ref>` prints the durable receipt
  beside the ref one and writes the patch, because the derivation is free exactly once: the
  control still resolves and the diff is the fix you just made.
- **It WRITES `tests/falsified/<guard>.patch` on every run.**  Run on an older guard to check a
  new cell, it replaces the receipt with an unrelated diff.  Restore the committed one with
  `git show HEAD:tests/falsified/<guard>.patch > <that file>` and keep only the verdict lines.
- **A patch isolates ONE defect only against the commit right before the fix.**  Against a
  distant control it is the whole source difference since; over 800 source lines `falsify.sh`
  writes no patch and says to re-run against the commit before the fix.
- **Deriving one after the fact**: a control is the PARENT of the commit that added the guard,
  so the patch is that commit's source diff reversed.  It stays runnable only while the tree
  still resembles the fix, and `falsify.sh` exits 3 when it no longer applies — detected, where
  a dangling sha reports nothing.  A patch that no longer applies still RECORDS the defect.
- **Its channels must agree with the ref control's.**  A reverted commit that fixed two things
  reintroduces two, and a receipt moving the WRONG channel looks like proof.  So each patch
  receipt is re-scored, and recorded only when its channel signature matches; otherwise the
  guard keeps a marker saying its control is gone.  Score leak-channel receipts with
  `LOFT_STRICT_STORES=1` — unarmed gives a DIFFERENT channel, not a weaker one — and identify a
  leak by its store SHAPE (`St1295×42`); the `kt=` id shifts with the type table.
- **A guard covering TWO defects holds only ONE patch.**  Keep it for the receipt that moves on
  more channels and backends, record the other as its own `@falsified-at:` block with CHANNEL /
  WITNESS / HOLDS, and say in the file which receipt the patch belongs to
  (`1647-a-moved-call-result-is-released-once.loft` is the worked example).

### What a receipt owes its next reader

Whether a recorded patch reintroduces THE defect cannot be gated — it costs an apply, a build
and a run per guard, and the verdict is a judgement about which channel moved.  So it is a
**read, once per release** (`make falsify-review`, checklist item `M-falsify-receipts`), and
how long that read takes depends on how well each guard documents itself.  **A guard whose
receipt does not say how to score it again is defective on its own terms.**

| field | what it says | what its absence costs |
|---|---|---|
| **CHANNEL** | which channel carries the defect | a good patch and a bad one look identical |
| **ARMED** | the instrument the measurement needs — asked only of a leak-class guard | an unarmed run gives a DIFFERENT channel, not a weaker one |
| **WITNESS** | the concrete observation on the control — a value, a count, a leaked shape (`answers 99 where the file says 88`, `St1295×42`) | the re-read becomes a whole run instead of one comparison |
| **HOLDS** | what must NOT move (*"every assertion passes on both trees"*) | the field that rejects a patch moving the wrong channel |

Write them as LABELLED lines — the canonical form:

```text
// @falsified-by: tests/falsified/1033-a-par-worker-gets-the-right-nested-vector.patch
//   HOLDS: leak, panic, free-refusal and expectations are equal on BOTH trees, and
//   `falsify` requires HERE to be clean, so all four are clean on both.  The VALUE
//   channel is the whole measurement here; a run scored on leaks would learn nothing.
```

Prose an older receipt already carries still counts, but the label is what makes the standard
writable.  A label owns its wrapped continuation lines, and the leak-class test that decides
whether ARMED is owed reads everything EXCEPT the HOLDS section (HOLDS saying "leak and panic
are equal" means the guard is *not* leak-class).

The rule lives in `scripts/falsify-review.py --check` and nowhere else, so the review and the
gate cannot disagree.  `doc_hygiene::every_guard_says_how_to_score_it_again` holds it against
`tests/falsified_docs.baseline`, which is **empty**: a new guard whose receipt lacks the fields
fails outright, and the cure is to write the fields, never to re-add a line to the baseline.

## The defect no guard can catch — the corpus EMISSION diff

`make falsify` scores a guard, and a guard scores a program someone wrote.  Neither sees *"a
program nobody changed is compiled differently now"*.
`scripts/introspect_diff.sh <before-loft> <after-loft>` runs `loft introspect` (IR + bytecode +
generated Rust + stderr) over every corpus file with both binaries and names each file that
moved; a behaviour-preserving change wants `IDENTICAL`, and a fix wants a list it can explain
file by file.

**Run it after a fix, not only after a refactor, and explain every file it names.**  A moved
file that is one of your own new guards is expected; a moved file you never opened is a second
defect or a second fix.  (It found loft#1435: two enum files had silently lost a synthesised
variant dispatcher, and no test could fail because the missing thing was never called.)

## When two trees disagree about the wreckage, guard the REFUSAL

A defect that should be refused and is not leaves damage behind, and the damage varies between
trees for reasons unrelated to the defect (the same missing refusal left a record reachable by
no key on one branch and by its new key on another).  A cell asserting the wreckage passes on
one tree and fails on the other; a cell asserting the refusal — a `tests/parse_errors.rs` entry
whose expectation is the compile error — is true on both.  Such a cell pins no runtime values,
correctly: a refusal fires before any of them can happen.

## The set a suite RUNS is not the set it CONTAINS (`LOFT_TRACE_ASSERTS`)

An `assert` that is written, compiled and **never executed** is reported by nothing: a skipped
file prints `skip` and passes, an uncalled function costs a compile, a branch never taken looks
like one that held.

`LOFT_TRACE_ASSERTS=<path>` appends `file:line` for every `assert` that EXECUTES.  The hook is
in `n_assert` (`src/native.rs`), which the interpreter and every `--native` binary share, so one
setting covers both backends and every spawned process (it appends, never truncates):

```sh
LOFT_TRACE_ASSERTS=/tmp/ran.txt cargo test --release --test wrap loft_suite
# then: every `assert(` line in tests/scripts that has no `file:line` in /tmp/ran.txt
```

It records the position the COMPILER injected, so a file tracing at a constant offset from its
source means the injected line is wrong (how loft#625 was found at a second site).

Two causes are gated in `tests/wrap.rs`, both proven able to fire:

* **`a_refusal_file_carries_no_runtime_assertions`** — a firing `@EXPECT_ERROR:` stops the
  whole file (`run_test` returns *"ok (errors consumed)"*, `native_scripts` skips it).  A file
  asserts a refusal **or** it runs; the convention for both is a companion file
  (`102`/`102b`).  An `assert` inside an `@EXPECT_ERROR` function is fine.
* **`every_assertion_is_reachable_from_the_entry_point`** — when a file has a `main`, both
  runners execute ONLY `main`.  Call every other zero-parameter function from it or delete it.

Both under-approximate on purpose: neither sees a branch never taken, and both allow the
deliberate dual guard (`432b`, `751`) whose `@EXPECT_ERROR` `main` carries assertions that run
only if the refusal regresses.  `LOFT_TRACE_ASSERTS` re-measures the rest — a report, not a
gate, since a gate over ~10 000 sites would need a line-numbered allow-list.  When two harnesses
ask the same question (is this a refusal file?), they share one implementation
(`common::expect_tag`).

## How a guard reads green while the defect stands

`make falsify` catches the commonest case — a guard that never failed on the build it was
written to catch — but only for the commit you name.  These are the shapes that survive it.
Each is one rule; the measured incident behind it is in
[TESTING-history.md § How a guard reads green](TESTING-history.md#how-a-guard-reads-green-while-the-defect-stands--the-incidents).

### The cells do not span what the change ranges over

- **A guard over a CLASS whose cells reach only its easiest member licenses a change to the
  whole class.**  Before citing a guard for a change, name the class in the PREDICATE's terms —
  for a MAY/ANY predicate, *every input for which it can answer true* — list the members, and
  check a cell reaches each.  For a MAY predicate that is always-true, always-false and
  **mixed-per-path**; mixed is the one usually missing.  A member that cannot be a cell is named
  in the guard's header with its blocking issue (loft#1647, loft#1651).
- **Every cell reaches the same SEAM.**  Varying values, types and directions inside one entry
  point asks one question.  The seams a store can enter are countable — local, field, element,
  struct literal, argument, return, compound — so ask which seam each cell enters (loft#1246:
  a guard of four RETURN cells missed the annotated local).
- **The defect's emission path decides which syntactic position exercises it.**  A lookup
  written as an argument may never reach the `Set` path the defect lives in, while the same
  lookup bound to a local does (loft#1217).  Answer *"which position does this emitter see?"* by
  falsifying, not by reading.
- **A guard over a newly-ACCEPTED shape must reach the sites that CONSUME it** — a local bound
  from it, a write through it, a call taking it — not just the spelling the fix stopped refusing.
  A clean refusal traded for an ICE is the one direction a fix must not move.  When a refusal
  goes, rewrite its guard rather than deleting it: its cells are a survey of the shapes that
  reach the site.
- **A flag-driven feature needs a cell with something PARSED AFTER it.**  A parser-global set
  and read around `parse()` is erased when `parse()` re-enters for a sub-expression; the minimal
  spelling has nothing after the construct and hides it (loft#1214).
- **Two guards covering one invariant can share a blind WINDOW.**  When a `debug_assert!` family
  (stripped from ordinary builds by `[profile.dev.package.loft]`) has a plain-`assert!` twin,
  check the twin is CALLED in every state the debug form observes — especially between
  construction and first use, since every test starts by using the thing (`@FR-H-Wilderness`,
  `a_fresh_store_never_claims_its_wilderness`).
- **A predicate asked at dozens of sites answers more than one question.**  Widening it is not
  a local change and its blast radius cannot be bounded by inspection; give each question its
  own NAME (`keyed_kind` vs `owns_keyed_store`, `rebind_must_mint` beside `owns_store` —
  loft#1445, loft#1447).
- **A green boundary matrix says nothing about a fix's BLAST RADIUS.**  The matrix varies the
  defect's axes; a fix's risk is everything else that reaches the same site.  A lifetime change
  is verified by the suite, not its matrix (loft#1201).
- **A hand-built matrix is not the adversarial gate.**  Run the generative gates
  (`tests/ownership_fuzz_gate`) before believing a lifetime change (loft#1118).

### Fixtures that cannot tell candidates apart

- **Choose each fixture as the value that DISCRIMINATES.**  A value chosen because it is easy to
  type lands where every candidate implementation agrees.
  - *Width*: `7` reads back correctly at 1, 2, 4 and 8 bytes.  Give each declared width a value
    the next-narrower one truncates — `300` for 2 bytes, `70000` for 4 (loft#1409).
  - *Sign*: a `u16` at `300` sits below the signed midpoint where signed and unsigned decodes
    agree.
  - *Edge*: a too-wide read is right at a vector's END (the bytes past it are zero).  Score where
    neighbours exist on BOTH sides (loft#1420).
  - *Position*: one droppable member makes `deps.first()` look right; one cell must carry TWO of
    whatever is being indexed.
  - *Default*: `?? []` answers length 0, as does the bug that took the wrong branch (loft#1120).
    Choose a default the wrong answer cannot imitate (length 2 separates 0, 1 and 2).
  - *Boolean*: `false` prints for every byte that is not `1`, and `!b` reads `true` for garbage.
    Assert the COMPARISON (`b == false`), not the rendering (loft#1254); the same holds for
    `character` 0 and any formatter mapping several bit-patterns to one glyph.
  - *Quantum*: a fixture smaller than the unit being measured (a page, a block, a tick) reports
    the fixture's size, not the code's behaviour (@PLN146 F2: a 19 KB pack in a 64 KiB page).
  - *Count vs identity*: two entries and two entries naming ONE record have the same length.
    Assert a key sum or a real lookup as well (`901-linked-group-fill.loft`, `1159-…`).
- **A CONSTANT index is trusted by contract**, so `v[9]`, `v[k]` under `for k in …` or behind
  `if i < len(v)` read NON-NULL and never produce the `τ?` a nullability cell means to test.
  Index through a variable the compiler cannot prove in range; the receipt is the cell compiling.
- **The cells bind before observing.**  A record BIND COPIES, so `r = f(src); r.n = 99` measures
  the caller's copy.  Mutate THROUGH one call and re-read through another —
  `bump(f(q)); println("{f(q).a}")` (loft#1484, `(F-Ret)`).
- **A cell whose destination starts EMPTY** cannot tell an append that reached the caller's
  collection from one that built a fresh one.  Populate it first and read the OLD key as well as
  the new (loft#1433).

### Controls that cannot discriminate

- **Put the nearest spelling that ALREADY WORKS in the matrix** and re-run it on every build — a
  matrix of broken cells answers "fixed" and cannot see what the fix breaks (the dense and
  vector twins for a `&` fix, the vector for a keyed fix, the plain one for a nullable fix —
  loft#1445).
- **Before believing a green cell, ask what would have to change for it to go red.**  If the
  answer is "nothing the guard is about", the cell measures something else.
- **Build the negative cell first.**  A fixture with only a positive cell passes under both the
  right and the wrong implementation; the one that decides is the cell whose answer must be
  empty.
- **Assert the precondition, so only the guard under test can say no.**  A probe point the
  camera misses anyway answers `-1` with or without the viewport guard.
- **A control can read the same on both trees for OPPOSITE reasons.**  Name what it reads on the
  CONTROL tree and check it against that build; where the readings coincide, move the control to
  where they differ (loft#1241).
- **A predicate needs a reachable negative.**  One that cannot be false in your program cannot
  be wrong in a way anything notices (`document.fonts.check()` answers *true* for a family
  nothing declares, which inverted the browser text bridge's font check).
- **After arranging an absence, ASSERT the absence** (`command -v rustc` in the same
  environment) before scoring anything on it.  A missing toolchain does not fail `--native` at
  all: it prints *"rustc not found … running interpreted instead"* and exits 0.
- **Test a control's cleanliness the way you test a failure** — perturb something the fact
  should not depend on (an unrelated local, a reorder, a rename).  A cell can be clean for no
  reason, and then the axis it establishes is fiction (loft#1201).
- **Name the test a control must redden BEFORE running it.**  When a different test fires, the
  named one cannot fail on its own subject — a dead gate, not a working control.
- **A CONTROL cell scored in the same file as its subject can pin every channel.**  A control
  that fails loudly on both trees fixes the exit code and makes `falsify` read INERT.  Split
  files by the CHANNEL that moves: silent-answer cells in one, already-reporting cells in
  another whose `@falsified-at` records the diagnostic identity (loft#1211, loft#1212).
- **When adding a leg near a `LOFT_NO_*` switch, run that switch's own test.**  Each switch is a
  bisect control whose value is that it can still show its before-half; no value channel reports
  the loss (loft#1482).
- **The before/after oracle has to PREDATE the defect.**  The installed release answers nothing
  for a bug introduced after it shipped.  When both halves agree, prove the channel fires with a
  still-open defect run through it (`D-clo-14`, `D-own-16`).
- **The pair rule: a report saying `ok` proves nothing until the instrument can say `FAIL`** —
  and *"I cannot see what it measured"* is a fact about the report, not the instrument.  Corrupt
  one cell in a scratch copy and look.  (`--tests` on file-level `@EXPECT_ERROR` pins prints
  `0 expected errors`, counting per-function expectations only, while the pins are checked.)

### The probe never ran, or can only confirm

- **A mutation is not applied until you have read it back.**  `grep` the file for the mutant; a
  PASS on an unverified mutation is no result (a stale `old` string, a heredoc eaten by the line
  before, a regex expecting a comma the message lacks).
- **An extraction is not measuring until it reproduces a number you know by another route.**
- **A probe on ONE of two arms that reports nothing is an unarmed instrument, not a negative.**
  Arm it on a known-positive case before believing its silence (loft#1505; WINDOWS.md has the
  platform instance).
- **A probe that FAILS TO PARSE reads as a silent pass**, and a RAN column grepping the
  program's own marker is fooled because the parse error echoes the source line.  Score RAN on
  something the program cannot forge — an `^error` line or the exit code.
- **A probe whose TEST is looser than its question accepts the wrong answer.**  Substring where
  exact was meant (`"0 citation"` matches `"10 citation(s)"`) fails toward "nothing found".
  Assert on the structured thing, and check that the SIGN of a disagreement is possible.
- **A probe whose SETUP contains the thing it tests for measures the setup.**  When probe and
  subject share a channel (a command line, a directory, a cache, a process), run one cell per
  invocation.
- **A reproduction that hits a WARM CACHE measures nothing, and the clock is the tell.**  State
  the failure's cost (wall-clock, *did it compile?*) and check the repro matches before reporting
  a negative.  The artifact cache lives in `<script dir>/.loft/cache/`, not under `TMPDIR`.
- **Replacing a SELECTOR is a set change.**  Print the selected set under both selectors and
  diff; an item that leaves the set is a decision to state.  Likewise a suite running the wrong
  subjects reports green (loft#1520).
- **`cargo test --test <name>` names a TARGET** (a `tests/*.rs` stem), never a function:
  `cargo test --release --test native native_scripts`.  A run that measured nothing emits no
  `test result:` line, so **wait on the SUCCESS sentinel and read the `N passed` count**, never
  the absence of an error.
- **Racing a race is not a falsifiable guard; assert the PROPERTY it violates.**  A publish race
  is guarded by the inode (`rename` swaps a new one in, `fs::copy` truncates the old one), with
  the old implementation kept in the test as a positive control.  A racing test kept for its
  end-to-end shape says in its header that it does not reproduce the defect.
- **Several `@EXPECT_ERROR`s in one file all report only if the compiler reaches each.**  The
  annotation does not stop — the compiler does: a refusal that desynchronises the parser ends
  the run before later phases (loft#1453).  **Split a refusal guard by the PASS its check runs
  in**: a pass-1 error stops the file before anything gated on `!first_pass` runs.
- **Two `@EXPECT_ERROR` cells declaring the IDENTICAL substring make each other vacuous**
  (declarations match the union of every parse round, loft#1242).  Give each cell a
  distinguishable subject.  Prove a refusal guard by hand: replace each expected substring in
  turn with a word the compiler never prints, check the suite fails, restore.

### The channel read is the wrong one

- **A use-after-free reads the bytes the freed slot still holds.**  loft's arena free marks the
  record dead and leaves its bytes, so every value assertion passes until something reuses the
  slot.  A lifetime guard is a POISON-gate guard and its file says so; falsify it under
  `LOFT_POISON=1` and record the unpoisoned count too.  `LOFT_STRICT_STORES=1` names the store,
  type and ops but REPORTS rather than gates — use it to diagnose.
- **A lifetime matrix scored without `LOFT_STRICT_STORES=1` is not scored** (loft#1143).
- **Undoing a lowering means undoing its TYPE.**  A guard over a lowering that can be undone
  asserts value AND leak; a value-only matrix cannot see a dep left on a backing whose copy is
  gone.
- **A VALUE assertion is vacuous when the value cannot witness the defect.**  When the symptom
  is layout-fragile, assert what the fix DETERMINES — the emitted IR (`OpFreeRef(_tuphold` must
  not appear, loft#1361) — **paired with a positive assertion that the construct is still BUILT**,
  since "must not contain" cannot tell *fixed* from *gone*.
- **Identical calls differing by POSITION say a slot is unsound, not which mechanism.**  Turn the
  suspect OFF and measure the same repro on the same binary before believing any reading of the
  IR (loft#1441 was a definite-assignment hole, not a free).
- **Which half of `LOFT_POISON` fires names the class:**

  | site | fills | catches |
  |---|---|---|
  | `database/allocation.rs` `free_named` | a freed store's payload, past the 8-byte header | a stale `DbRef` read **after free** (loft#1361) |
  | `state/mod.rs` `reserve_frame` | the freshly-reserved frame region | a read of a slot **this call never wrote** (loft#1441) |

- **The leak line carries TWO counts** — `kt=81 C×2` is one STORE of two RECORDS.  A leak that
  appends into a reused slot keeps the store count at 1 however far it grows.  Scale the shape
  (loop at 2, 3, 5, 9; calls at 1 and 50) before calling a leak bounded (loft#1483, loft#1487).
- **A leak freed at FRAME exit is invisible to the exit check.**  Reproduce the entry's own
  measurement (`LOFT_ALLOC_SITES=1` peak), not the one your instrument offers (`D-clo-14`).
- **A bare `--native` run does not leak-check.**  Arm `LOFT_NATIVE_LEAK_CHECK=1` before any native
  leak cell, and treat `make falsify` (which arms it) as right when it disagrees with your matrix
  (loft#1344).  A leak fix is checked with `LOFT_STRICT_STORES=1` on `--interpret` AND
  `LOFT_NATIVE_LEAK_CHECK=1` on `--native`; silencing one backend is not a fix (loft#1225).
- **A leak the DRIVER reports beats one inferred from memory growth.**  Look for the API that
  refuses to proceed while the resource is held (`sqlite3_close` → `SQLITE_BUSY`, `PQfinish`)
  and assert its return code.
- **A fix that makes a dropped statement start EXECUTING is invisible on the diagnostic
  channel.**  A sweep for new diagnostics finds only programs newly refused; compare STDOUT on
  both binaries, like-for-like profiles, and remember `test_`-only files diff clean under
  `--interpret` (`tests/wrap.rs` covers them — loft#1221).
- **Fixing a write can move the silence.**  When a fix REDIRECTS a write, assert the read through
  the OLD name as well as the new one (loft#1160).
- **An assertion can read RED while nothing is wrong.**  An over-approximating checker's false
  abort hides every real finding behind it on a stop-at-first gate.  Check that the path it names
  actually reaches the site (`check_text_return_path`: a `Break` hands its frees to what follows
  the loop, a `Return` hands them nowhere).
- **The gate's verdict line is a channel too.**  `CI-RESULT: ALL GATES PASSED` has printed beside
  `error: could not compile`; check `grep -c "^error" result.txt` as well
  ([CI_BUDGET.md § `CI-RESULT`](CI_BUDGET.md)).
- **The column a reviewer reads can be the broken part.**  A scraper matching only the plural
  "expected errors" scored every one-expectation guard `FAIL/1`.

### The harness is not the program

- **`loft --tests` is not `loft prog.loft`.**  The script classifier never runs and a store leak
  is not reported, so a `tests/scripts` guard can be vacuous for a whole class (loft#1271,
  loft#1273).  Homes that can fail:

  | what you are guarding | where it can fail |
  |---|---|
  | a leak | a `.loft` file in `tests/leak_cases/clean/` — `tests/leak_cases.rs` runs it as a plain program on BOTH backends |
  | script classification | `src/script.rs`'s unit tests (`is_script` / `split_top_level`), plus a CLI test in `tests/script_mode.rs` |
  | anything else the CLI decides before parsing | a Rust test that spawns the binary, as `tests/panic_halts_both_backends.rs` does |

  A `tests/scripts` file kept beside those falsifies by SITTING THERE (a corpus sweep names it),
  and its header says so.  `make falsify` reads INERT for these, correctly.
- **A `tests/scripts/` file with no `main` runs nothing under `--interpret`** and still exits 0;
  read the assertion COUNT.  The leak channel is scored only by the `wrap.rs` harness.
- **Reproduce with the gate's command.**  `LOFT_POISON=1 loft --interpret --tests <file>` passes
  on a build the nightly (`wrap` under nextest, leak-checked, one process per test) fails.
- **A `--lib <dir>` override is DROPPED when the cwd has its own `lib/`** (first-wins,
  `parser/mod.rs::lib_path`).  Prove the flag is honoured by pointing it at a copy that CANNOT
  run; work from a directory without `lib/` (loft#1352, loft#930, loft#963).
- **An ad-hoc `--native` run LINKS `libloft.rlib`, which `cargo build --bin loft` does not
  rebuild.**  The two backends then "agree" because one is not being tested.  Run
  `make check-rlib` before treating any ad-hoc native result as evidence; iterate with
  `cargo build --release --lib --bin loft`.
- **A stale test binary masks a diagnostic-message change.**  Run the affected binary from a
  build you watched relink, or `cargo clean -p loft` first.
- **A temp probe named after a HASH of its source shares a truncation window** with every test
  using the same source.  Name a probe after the TEST.
- **A per-item marker on stdout cannot attribute a fault printed on stderr** (stdout is
  block-buffered off a tty).  `LOFT_TRACE_SCRIPT=1` puts `wrap.rs`'s marker on stderr (loft#920).
- **`raise()` is not a fault.**  With `SA_RESETHAND`, only a faulting instruction re-raises after
  the handler.  Fault for real in a forked child
  (`std::ptr::write_volatile(std::ptr::null_mut::<u8>(), 1)`) and assert `WIFSIGNALED`.
- **A doc-comment lands on the NEXT item.**  Insert a function after the previous item's `}` and
  before the next item's doc, or it steals that doc.

### The oracle is not independent

- **A round-trip is not an oracle when the writer is the reader's TWIN.**  Write through the
  product's own setter, never a test-local mirror of the reader; when a test asserts two
  implementations agree, say which is the oracle (loft#1431).
- **Score a new spelling against a working sibling, not a hand number** — the sibling freezes the
  language's answer, a hand number freezes your model (`1159-…`, `1160-…`).
- **A "did this copy too much?" control depends on the element type.**  An undisturbed view must
  ALIAS for a RECORD tail and must NOT for a COLLECTION one (`(B-Copy)`).  Settle it with the
  plain spelling of the same tail on the same build, and pin both boundaries as cells
  (loft#1399).

### What the guard owns

- **A cell your new guard fails may be somebody else's defect.**  Revert your fix by inverse edit
  and re-run the cell: identical output both ways means the defect is older than you — file it
  with that A/B (loft#1418, loft#1421, loft#1422).
- **A guard must not assert what it does not own.**  Take such a cell out and name its issue in
  the header; asserting today's broken answer freezes it into the contract
  (`1415-a-null-arm-source-that-views-a-parameter.loft`).
- **A reverted change is a MEASUREMENT.**  Record the channel that failed it and the cure that
  looked right and was not; "did not work" tells the next person nothing.
- **Turn the suspect off before reading the IR a fourth time.**  A coherent explanation read three
  times is still a hypothesis; the cheap experiment is the same repro, same binary, suspect off.
