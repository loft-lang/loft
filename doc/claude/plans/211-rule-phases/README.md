<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 211 — Every rewrite rule at its proper phase

Tracker: [@PLN211](https://github.com/loft-lang/plans/issues/211).

## Status

Open, `status:next` — the inventory below is measured and the plan is DESIGNED
([design.md](design.md): every arc's files and checks, the first verdict for each rule, and the
removal of the facts routine's three deviations); arc A is built (the census reads both
writers and the interpreter's generator; every rule reports to it or says why not; the phase
table `bench/portal/rule_phases.tsv` is measured and gated, and `rewrites.md` § Applied by is
rendered from it); B1 and B2 are done: weights in [b1-weights.md](b1-weights.md),
a verdict per rule in § Verdicts.  Arc C has moved `(R-SliceArg)`'s
range clause, `(R-Wrapper)`, `(R-PushFill)`'s fill form, `(R-KeyedEntry)`, `(R-ElemFirst)`'s literal-built temp, call-filled temp and joined arms, and
`(R-LoopBuffer)` (§ Arc C's moves); **arc C′ is next** — those moves brought to ONE place each,
no writer clause or writer compensation left (§ Arc C′), before any further rule moves; **arc C′
is closed** (C′1–C′4), with `(R-ElemFirst)`'s view-exit form deferred to the `(R-ValueRecord)`
view-clause move (§ What arc C′ skips); then `(R-LoopRecord)` and `(R-CompleteWrite)` moved (§ Arc C's moves; the latter's design
in [cw-plan.md](cw-plan.md)), and `(R-ValueRecord)`'s view clause moved (c4-view S1, S2);
the forms of `(R-ValueRecord)` the IR declines stay in the writers, closed builds only, since
spelling them in the IR would not speed the interpreter (owner ruling, [c4-view-plan.md](c4-view-plan.md)
§ S2); c4-view S3 deleted both writers' element-first clauses (D-phase-4 closed).  D-fact-2
gave the C `(R-RangedCall)` (its gain test now counts the operators a variant saves), and
`(R-TextBorrow)` and `(R-RetAdopt)` moved to the interpreter and `(R-SliceInto)` into the IR
(§ Arc C's moves); `(R-RefillText)` stays.  It
was to start once
#1743 (every routine within 3×) and #2351 (the facts routine) were fixed; #1743 is fixed on
`laptop-c-perf`, and on 2026-11-03 the owner asked to concentrate on this plan, so #2351's
three deviations (D-fact-1 … 3) became part of it rather than a precondition.  Four arcs: **A** keeps the register honest about where
each rule runs, **B** decides per rule whether it can run earlier, **C** moves the ones that
can (and **C′** leaves each moved rule in one place), **D** widens the rules we already have to the shapes they refuse today — the general
performance fixes, by class, behind the per-routine issues of the 3× bug line.

`formal/rewrites.md` `(R-Phase)` says every rule is APPLIED BY one phase and that the phase
decides its oracle: the IR phase (both backends run the rewritten IR), a writer (the
interpreter is the oracle) or the interpreter's generator and runtime (a compiled backend is
the oracle).  The arcs of @PLN157 and @PLN210 optimised for compiled code: most of their
rules were built in a writer, so the interpreter runs the IR they rewrite away.  A rule that
removes WORK — an allocation, a copy, a store round trip — belongs in the IR phase wherever
its conditions can be decided there: then it is decided once, for every backend, and both
backends stop doing that work.

## Goal

**Harmonisation** (the owner's goal for this plan): every optimisation implemented ONCE, at
the front, and every backend reading that one decision — no rule built separately in each
writer, no writer patched to recognise what another phase produced, no decision recomputed per
consumer — [GOALS.md § Don't tolerate re-derivation patterns](../../GOALS.md#dont-tolerate-re-derivation-patterns--engineer-the-class-away),
applied to the optimisations.  In detail: every rule's phase stated in the register and checked against what the
compiler does; every rule that removes work applied in the IR phase (or an
interpreter-runtime clause) unless a stated reason keeps it in a writer, and then in that one
writer alone; each moved rule left in one place (§ Arc C's finishing condition, arc C′); and
each rule's most frequent refusals on the benches either admitted by a generalised clause or
recorded as the rule's boundary.

**What counts as a duplicate** (owner, 2026-11-06): a rule implemented TWICE INSIDE ONE BACKEND —
an IR pass and that backend's own writer clause both reaching its output — and a rule that would
speed the interpreter but lives only in a writer.  The same clause in the rustc writer and the C
writer is not, in itself, the problem: most of the rustc backend will be shelved, so the C writer's
clauses are what this plan judges.  The one known duplicate left by ruling: `(R-ValueRecord)` on the
C, the IR pass beside the C writer's clause for the forms the IR declines ([c4-view-plan.md](c4-view-plan.md) § S2).

A faster interpreter follows from it — work a rule removes in the IR, every backend stops doing
— and is measured per move, but it is the CONSEQUENCE, not the aim: a move that speeds the
interpreter while leaving the rule in two places is not finished, and a cleanup that only
removes a writer's duplicate is in scope even when it speeds nothing.

**Why now** (the owner's timing): the arcs of @PLN157 and @PLN210 built the CORPUS of speed
optimisations that brought every bench routine within 3× of its hand-written Rust twin — the
bug line, [`formal/performance.md`](../../formal/performance.md) § The bug line.  Measured, that is a
position; ingrained, it is a guarantee.  A rule decided once at the front, its phase stated and
gated (`rule_phases.tsv`), its admissions counted per writer (the rewrite census), its sites
cited (`@FR-R-…`) and its cells falsified, cannot be lost by one backend drifting from the others
or by a writer refactor that forgets a copy — so harmonising the corpus is how the 3× bar
becomes something the compiler holds rather than something a bench run finds.  It is also the
project's own order ([ROADMAP.md § The order of the large arcs](../../ROADMAP.md#the-order-of-the-large-arcs)):
the rustc backend in good shape before the C, the language stable before loft-in-loft — the
corpus harmonised at the front is what the C backend inherits and what the self-hosted
compiler ports ONCE rather than once per writer.

**What a move also buys.**  A rule moved to the front meets the whole IR and all three backends
at once, so each move re-verifies the optimisation, retests the language features its cells
touch, and carries what rustc learned over to the C.  Found by the moves of arc C: the lost-write
lint and three analyses that matched call names only (`(R-Wrapper)`); the REPL's source 0 and a
dropped tuple the C emitter had no arm for (#2400); a moved first assignment rustc declared
inside a block (`(R-SliceArg)`); every IR pass skipping a library function with a native name;
a `Set` between record variables binding a copy (`(R-ElemFirst)`'s joined arms); a generator
holding the vector the next pass would clear (`(R-LoopBuffer)`).  And the C gains rules it never
had — `(R-Wrapper)`, the fill form, `(R-LoopBuffer)`, `(R-ElemFirst)`'s joined arms and call-filled
temps.  Each finding is fixed in its move, never filed, and its cell stays.  The deepest check
is still to come: [@PLN91](../91-self-host/README.md) § The path's first stage judges the
loft-written front end by a byte-identical image against the Rust one, before and after the
`close_program` passes — so every rule this plan moves into the IR gets a second, independent
implementation compared with it at the IR, where a duplicate verified on every level exposes
what one implementation alone hides.  A rule left in a writer waits for stage 3 (the C
generator in loft); a rustc-only one is never compared.

## Effort + design

- **Effort:** H (A: S, B: M, C: one S–M phase per rule moved, D: one M phase per clause)
- **Design:** ~ (arc A and the move protocol designed here; each move's own design in its
  phase)

## The inventory (measured 2026-11-02)

Every bench program of the suite and every library bench (58 programs), emitted with
`LOFT_REWRITE_CENSUS_FN`, which credits each admission to the phase that decided it
(`parse`, `ir`, `c`, `native`).  The C emission covered all 58, the rustc emission 41 (17
declined to emit), so the rustc-only list is a floor.

**The register.**  6 of the 123 rule entries state their APPLIED BY line.  `(R-Transient)`
reports to no census at all.

**Corrections found while designing** ([design.md § What the code already has](design.md#what-the-code-already-has)):
the census reads only the rustc emission, so no C admission reaches a baseline; five rules
fire on the C under no census name (four of them listed below as rustc-only); census names
disagree between the writers (`R-Kept`, `R-HandOver/rotate`, `R-LoopRecord/refill`); and most
rules listed as decided in the C writer DECIDE in a shared analysis (`src/opt/`, `src/lower/`)
and only EMIT in the writer, which makes them cheaper to move than this table suggests.

**Decided before any writer** (both backends already run them), 38 rules — among them
`(R-InlineLeaf)`, `(R-WorkBuffer)`, `(R-ValueRecord)`, `(R-ValueLocal)`, `(R-ExitVector)`,
`(R-Reuse)`, `(R-MoveLast)`, `(R-Place)`, `(R-Rebind)`, `(R-CopyView)`, `(R-VecCopy)`;
`(R-Counter)`, `(R-Header)` and `(R-FormatAppend)` have clauses in both.

**Decided only in the C writer**, 40 rules, in two kinds:

| kind | rules | first verdict |
|---|---|---|
| REPRESENTATION — a value kept in a register or a C local, or code shaped for the C compiler | `(R-Range)`, `(R-NonSentinelParam)`, `(R-Base)`, `(R-RecPtr)`, `(R-Header)`'s held clauses, `(R-Inputs)`, `(R-Scalar)`, `(R-Invariant)`, `(R-View)`, `(R-Callee)`, `(R-Version)`, `(R-AliasVersion)`, `(R-Peel)`, `(R-CharWalk)`, `(R-CompleteWrite)`, `(R-SplitTable)`, `(R-Tabulate)`, `H-SwapRebind` | STAYS, reason stated: the interpreter's counterpart would be a new fused operator, and the owner's standing rule is that an interpreter hot spot is never fixed with one (a kernel goes through one `OpStaticCall` instead) |
| WORK — an allocation, a copy or store traffic removed | `(R-TextBorrow)`, `(R-RetAdopt)`, `(R-Destination)`, `(R-HandOver)`, `(R-ElemFirst)`, `(R-Mint)`, `(R-FlatCopy)`, `(R-AppendTwin)`, `(R-Push)`, `(R-PushFill)`, `(R-PushRec)`, `(R-FieldAppend)`, `(R-WorkPark)`, `(R-FnRefBuffer)`, `(R-Kept)`, `(R-KeptTwin)`, `(R-LoopRecord)`, `(R-SliceArg)`, `(R-SliceInto)`, `(R-KeyedEntry)`, `(R-Fill)`, `(R-Apart)`, `(R-Transient)` | arc B decides |

**Decided only in the rustc writer**, 21 rules — lost with that writer unless moved:
`(R-FaultFlag)`, `(R-FnHeader)`, `(R-FnRefValue)`, `(R-FoldCompare)`, `(R-GroupPush)`,
`(R-GuardFree)`, `(R-LazySplit)`, `(R-Leaf)`, `(R-LeafChain)`, `(R-LitDiv)`,
`(R-LoopBuffer)`, `(R-RangedCall)`, `(R-RebindBuffer)`, `(R-RefillBuffer)`,
`(R-RefillText)`, `(R-SliceBorrow)`, `(R-StreamWalk)`, `(R-TextClear)`, `(R-TypedKeyed)`,
`(R-TypedRead)`, `(R-Wrapper)`.  Arc B gives each the same verdict; one that cannot move to
the IR phase is a C-writer port or a recorded loss.

## Arc A — the administration

**A0 — the census reads the C.**  `rewrite_census.py` emits every bench with `--emit-c` beside
`--native-emit` and fails a drop on either writer (design.md § Arc A).  Effort S.

**A1 — the phase table, derived.**  `scripts/rule_phases.py` runs the census over the bench
set on both writers and writes `bench/portal/rule_phases.tsv`: per rule, the phases that
decided it and the programs it fired in.  `make rewrite-census` compares a fresh run with the
committed table: a rule whose set of phases CHANGES fails, named, as a drop does today; a
deliberate move is blessed in its own commit.  *Validated by:* a plant that credits one
rule's admission to another phase must fail the check (and the plant is the receipt).
Effort S.

**A2 — no silent rule.**  Every rule in the register either reports to the census or is on
an explicit list of the three the CENSUS CLAUSE exempts (`(R-Prefill)`, `(R-Cold)`,
`(R-FlatCopy)`).  `(R-Transient)` and every other silent rule get their `fired` call.  The
rules defined but admitted on no bench are listed with why (unreached by the benches,
or dead).  *Validated by:* `rule_tags.py` gains `phases --check`: a rule with no census call
site and no exemption fails; planted by deleting one `fired` call.  Effort S.

**A3 — the register says it.**  The APPLIED BY of each rule comes from the table, not
from prose written beside it: an `## Applied by` index in `rewrites.md`, generated from
`rule_phases.tsv` and checked by `make rewrite-census` like the table itself; `(R-Phase)`
names the index as where the line lives.  A rule with clauses in two phases lists both, as
`(R-Phase)`'s one-FACT clause allows.  *Validated by:* the generated index drifting from the
table fails the doc-drift check.  Effort S.

## Arc B — can it move?

**B1 — the weight.**  `scripts/interp_gap.py --writer c` takes its rewrite census from the C
emission (phase `c` counted as writer-only) beside the interpreter's operator census
(`LOFT_OP_CENSUS`, the `op-census` build).  Per rule it answers the interpreter's share of
the time in the functions the rule fires in.  *Validated by:* for a rule both writers apply
(32 today), the functions it fires in under `--writer c` and `--writer native` agree
where both emitted.  Effort S.

**B2 — the verdict, per rule.**  For each WORK and rustc-only rule, three questions, each
answered with evidence in a row of [§ Verdicts](#verdicts):

1. **Decidable on the IR?**  Every condition the writer checks is a fact of the IR and of
   the facts routine (`formal/facts.md`), not of writer state (a held header, a C local's
   shape).  A condition that reads writer state is restated on the IR or the rule STAYS.
2. **Expressible in the IR?**  The rewritten form is IR the interpreter already executes —
   existing operators, a stdlib kernel through one `OpStaticCall` — or a store/runtime
   policy the interpreter's runtime can apply.  A form that needs a new fused operator
   STAYS.
3. **Worth it?**  The B1 weight, and a hand-priced probe: the routine where the rule fires
   most, rewritten BY HAND in loft to the form the IR would carry, run on the interpreter —
   its instructions per op against the unrewritten program, same answer.

Verdict: **MOVE** (IR phase), **INTERP-CLAUSE** (the interpreter's runtime or generator
applies the same fact, `(R-Phase)`'s one-fact clause), **STAYS** (the reason recorded in the
rule's entry), or **PORT** (a rustc-only rule that cannot reach the IR becomes a C-writer
clause).  *Validated by:* each MOVE verdict's probe shows a gain on the interpreter; a probe
that shows none turns the verdict to STAYS — the phase can go red on its own.  Effort M.

## Arc C — moving them

One phase per rule, in B1's weight order.  Each is a parallel run:

1. **Beside.**  The IR-phase clause built under its own switch, the writer clause untouched.
   With both on, the writer admits nothing the IR clause already took.
2. **Compared.**  The C emission of every bench with the IR clause on and the writer clause
   off is byte-identical to the writer clause alone, or differs only where the census says
   the IR clause admitted MORE (each such site read by hand).  The interpreter: the
   rule's guard cells on both backends, the switch A/B against hand-computed values, the
   C sweep and the language suite on both backends, and the instruction count of the
   probe routine.  An IR clause changes what EVERY writer receives, rustc's included, so the
   cells also run on `--native` with `LOFT_NATIVE_BACKEND` unset, and `find_problems.sh
   --subject codegen` runs once with it unset: rustc's own pins (`apart_value`,
   `decode_view`, `mint_window`, …) and its declaration-at-first-assignment are what the C
   checks cannot see (a moved assignment declared inside a block was an E0425 there).  Two
   more reaches an IR clause has that a writer clause did not, both met on #2400: the REPL
   (its input keeps source 0, so "is this the stdlib?" reads `is_stdlib()`, never the source
   number — a breakpoint stopped pausing), and every caller of a function the move makes a
   LEAF (the leaf inliner then exposes a value kind the C only ever saw behind a call — a
   dropped tuple).  Run the REPL tests and the library gate's `--native-c` leg for such a move.
3. **Then deleted.**  The writer clause is removed, its switch becomes the IR clause's, the
   rule's applied-by changes in the table (A1's bless), and the rewrite census shows no
   drop on the C side.

**A move is finished when the rule lives in ONE place** (the owner's goal for this plan: the
optimisations harmonised, implemented cleanly at the front, not patched at the writers'
fringes).  Three things the comparison can leave behind, each a reason the move is NOT done:

- **a writer clause for a form the IR does not take yet** — the rule then runs in two or three
  places, and the writers' copies drift (`(R-ElemFirst)`'s comprehension row);
- **a writer compensation for the IR form** — code in a writer that recognises one pass's output
  shape so another writer rule keeps applying.  The fact it needs is stated once in `opt`, in
  general terms, for every writer to read;
- **a decision recomputed per consumer** — an IR pass and each writer deriving the same answer
  (who owns a buffer) on their own.  It is computed once, in `opt`, and read by all.

A shared `opt` analysis taught the IR form (`hoist::hash_fill_loop`, `frame_calls::is_store_entry`
for `(R-KeyedEntry)`'s call) is the front doing its job, not a compensation: the question it
answers belongs to every consumer.

**How the checks run, so a step never waits on the corpus.**  Per STEP, the targeted checks:
the rule's cells on the interpreter, the C and rustc, the pins it touches, the census, and
`c_emit` (with the `push_*` / `locked_writes` / `foreign_*` binaries) when the C writer changed —
minutes.  The two CORPUS checks — `switch_ab.sh` and `c_sweep.py`, ~20 and ~25 minutes — run
once per CHUNK (a C′ item, a PR-sized group of moves), in the background, against a COPY of the
binary (`LOFT=` / `LOFT_BIN=` name it), so the next step builds and tests on the real one
meanwhile.  The copy lives IN `target/release/` (`target/release/loft-check`), beside the
`libloft.rlib` a library's native part is built against ([C_BACKEND.md](../../C_BACKEND.md),
the C sweep): a copy elsewhere fails the interpreter side of every program that uses a native
library, and the sweep reports those as `differ` — C′1's and C′3's sweeps from
`target-exp/` showed four, each a `match` from `target/release/loft`.  While a check runs, a
library rebuild (`cargo build --lib`) would rewrite that rlib beneath it, so the next step builds
the `loft` binary and runs its cells, and the test binaries wait.  One heavy job at a time on the laptop (both at once ran it out of memory).  A
chunk's cut goes to a PR only once its corpus checks are in.

**A rule that needs the closed program is checked closed.**  A whole-program rewrite that
changes a signature (`(R-ValueRecord)`'s IR pass) does not run in a semantics build, which is
an open program (NATIVE.md § Optimisation tiers), and no writer keeps a copy of it there.  Its
cells, falsifiers and corpus checks therefore run `--lean` on the C and rustc; a plain
`--native` / `--native-c` run passes without reaching it.

Each phase can go red alone: an emission that changes where the census did not, a guard
cell, a falsified plant.  A rule whose comparison shows the IR clause admitting FEWER sites
than the writer did is not moved until the gap is closed or recorded as a decline.

## Arc C′ — harmonising the moves made

Before any further rule moves (`(R-LoopRecord)`, `(R-CompleteWrite)`, the ports), the six made
are brought to "one place", in this order.  Each is a phase that can go red alone, with the
move protocol's checks (cells on the interpreter, the C and rustc; switch_ab; c_sweep; the
census, every move attributed; rustc's pins).

| # | what is left | the clean form | done when |
|---|---|---|---|
| C′1 **DONE** (`opt::empty_slot`) | `c::body::emptied_either_way`: the C's `known_empty` taught `(R-LoopBuffer)`'s `if … else OpClearVector` shape, so its push-run reservation applies | a front fact, `opt`: a vector slot is EMPTY after a statement that empties it on every path — its zero, an `OpClearVector` of it, an `if` whose arms both do — read by the C's run clause and by any writer that asks; no knowledge of the loop-buffer shape | the C function is gone; the C's `R-PushFill/run` census unchanged |
| C′2 **DONE** (`opt::buffer_owner`) | buffer ownership derived per consumer: `opt::loop_buffer` recomputes five rules' analyses (invariant literal, element-first pair, move host, adopted witness, inline-buffer companion) and each writer its own | one `opt::hoist::buffer_owner(data, stores, d)` answering, per `__vdb` witness, the ONE rule that owns it, in a stated priority; the IR passes and the writers read it | no consumer computes another rule's analysis to decide ownership; the census unchanged |
| C′3 **DONE** (`opt::keyed_entry`) | `(R-KeyedEntry)`'s subscript clause (`coll[k₁, …] = T{…}`) still in both writers (`output_keyed_entry_subscript`, the C's `keyed_place` arm) | the IR form: the literal's values staged, the entry answered by the stdlib probe, the fields written — `opt::keyed_entry`'s own staging, one more host shape | both writer clauses deleted; `R-KeyedEntry/subscript` applied by `ir` |
| C′4 **DONE, one form deferred** (`opt::elem_first`) | `(R-ElemFirst)`'s comprehension row and view-plan exits still the writers' | the comprehension row as an IR form — done; the view-exit form deferred with `(R-ValueRecord)`'s view clause (§ What arc C′ skips) | the comprehension row applied by `ir`; the view-exit form registered as D-phase-4, waiting on D-phase-5 |

This is [STABILITY_METHOD.md § The rule-led walk](../../STABILITY_METHOD.md#the-rule-led-walk--the-standing-practice)
on a rewrite rule: the question its sites share is WHERE it is decided, and each row is a
second home.  The four rows are registered as `formal/rewrites.md`'s D-phase-1 … 4, deviations
of `(R-Phase)`'s ONE PLACE (D-phase-1 closed by C′1, D-phase-2 by C′2, D-phase-3 by C′3).  The measurable "done" for a rule: `python3 scripts/rule_tags.py
sites @FR-R-<rule>` names no writer site beyond the ones that spell the IR's decision, each
saying so in its comment.  The position when arc C′ opens, citing sites per layer (front /
rustc writer / C writer): `R-ElemFirst` 4 / 4 / 4, `R-KeyedEntry` 5 / 6 / 2, `R-LoopBuffer`
4 / 1 / 1.

C′4's design: [c4-view-plan.md](c4-view-plan.md) — the view leaf is `(R-ValueRecord)`'s own
writer clause, so the front fact is that clause moved into the IR (a tuple element that is a
reference), after which the element-first form needs no plan.  The comprehension row is done.

**Arc C′ is closed (owner, 2026-11-04)**, with one form deferred by decision: every rule arc C
moved now takes in the IR every form its conditions can be decided on there, and the one form
left in the writers depends on a rule arc C has not moved yet.

### What arc C′ skips

The owner chose, between doing c4-view-plan.md's S1–S3 now (several days) and closing the arc
with the remainder reclassified, the second — on the ground that the remainder is no duplicate
of a moved rule but a rule not yet moved, which belongs in arc C's queue ranked by its weight
like every other move rather than inside a harmonisation arc.  What that leaves, all of it
registered (`formal/rewrites.md` D-phase-4, D-phase-5):

- **`(R-ElemFirst)`'s view-exit form stays a writer clause.**  A temp its function also returns
  inside its record (`164-element-place.loft` g1, g2, g4, g10, g16 — the `parse_circle` shape —
  and g20) is built in the appended element only by rustc and the C, which pair it through
  `element_first`'s view plan; the interpreter keeps the temp's own store and its copy there.
  Both writers' element-first clauses (`generation::emit`'s statement handling, `generation::calls`'
  buffer place, `c::body::elem_first`) therefore stay, with this ONE job: the IR takes every
  other pair first, so they find nothing else.  On the benches they apply nowhere.
- **`(R-ValueRecord)`'s view-leaf clause stays the writers' alone.**  The IR's value-record pass
  carries FLAT records only (two or more fields, each an 8-byte integer, a float, a single or a
  boolean) and declines a view in a result position (`value_record::body_shape`); rustc
  (`lower::plan`) and the C (`FnWriter::value_records`) each compute `opt::value_records` with
  its view leaves and deliver the reference themselves.  The interpreter returns those records
  through a buffer.  Weight, measured (`R-ValueRecord/view`): 3 bench functions — drawing's
  `parse_lock` and `parse_poly` (the parser behind drawing's `parse` routine, over 2× its twin
  on both lanes) and two in the moros/dryopea consumer.
- **The plan is still computed three times** (the IR pass, rustc, the C), keyed by node address
  for the view leaves; D-phase-5's removal makes the IR the one place.

**Why it is not done here.**  The view leaf is a vector field delivered as a REFERENCE to a
place, so the IR's tuple needs an element kind it lacks: the interpreter's tuple layout must
carry a `DbRef` part that a frame never frees and that a copy out of it copies as the place it
views (`formal/binding.md` `(B-View)`); the value-record call-site gates must admit it; and the
C and rustc must spell it.  Several days, on all three backends — an arc C move, not a cleanup.

**What the deferral risks, and what holds it.**  The writers' element-first code, kept for one
form, could drift from the IR pass that took the rest; the cells pin both (`164-element-place.loft`
for the writers' form, `tests/element_first.rs` counting the IR trace and the writers' marks
together).  The register states both deviations, so neither reads as finished.

**When and how it resumes.**  `(R-ValueRecord)`'s view clause is a wave-1 move below (§
Verdicts), taken in arc C's order by its weight; its steps are c4-view-plan.md S1 (a reference
element in the IR tuple), S2 (the writers read the IR's tuple instead of recomputing view
leaves, emission byte-identical elsewhere) and S3 (`(R-ElemFirst)` takes the view-exit form;
both writers' element-first clauses deleted; D-phase-4 and D-phase-5 closed).

## Arc D — the class remainders: widening the rules we have

The 3× bug line files one issue per slow routine (`formal/performance.md` § The bug line).
The routines over a lower line share causes, and the cause is usually a rule that already
exists but REFUSES their shape.  So the general fixes come from widening those rules, not
from new ones (the owner's standing rule: general optimisations, no construct-specific
rewrite), and each widened clause lands in the phase arc B gives its rule.

**D1 — the remainder, by class.**  Every routine over 2× on either compiler (stats.py, both
compilers, the current library mains), grouped by the rule that would serve it and the reason
that rule declines there: `make decline-census` ranks each rule's refusals by reason, and the
per-routine profile names the rule.  A routine no existing rule could serve is listed as such
(a candidate for a new rule, outside this plan).  *Validated by:* each grouping read against
the emitted code of the routine it names — a class is a decline reason seen at the hot site,
not a guess from the profile alone.  Effort S.

*Done 2026-11-10:* [d1-classes.md](d1-classes.md) — the 33 routines over 2× in eleven classes.
The widest is `(R-Range)`'s INDEX clause proving only the lower bound (nine routines); the next
are `(R-Version)` refusing a leaf that calls a leaf (three) and copies where the twin borrows or
moves (six).  D2 takes the first: its provable part with the select form, priced on `boundary_loops`, is 11.18 →
9.29 ms (gcc) and 18.24 → 9.28 ms (clang) against the twin's 6.05.

**D2 … — one generalised clause per phase,** in order of the time the class carries.  Each is
priced by hand first (the hand form of the routine where the class is heaviest, same answer,
instructions per op), then built as a clause of its rule under the rule's switch, then judged:
its decline count falls in `make decline-census`, the census rises only at the sites it names,
the class's routines move (every one of them, `stats.py --routine …`), the guard cells and a
falsified plant on both backends, the C sweep and the suite.  A clause that moves only the
routine it was written against has treated a symptom and is reworked or dropped.

Seeds measured 2026-11-02 (to be ranked by D1):

| rule | refuses | routines it would reach |
|---|---|---|
| `(R-Version)` | a leaf that calls a leaf (an index helper: "calls hs_index", "calls hz_index"), and element reads of `float`, `u8`, `boolean` | hex_roof `roof_match` (3.0× by instructions), the hex libraries' cell accessors |
| `(R-MoveLast)` | a PARAMETER moved into a record field (its source must be a local today); the caller's argument a temporary that dies at the call | the `form_new` family where the call is not already a destination |
| `(R-Transient)` | a producer that pushes into its own vector field ("OpPushInt of a transient value"), roots whose lives overlap | the producers its trace declines across the library benches |
| `(R-Peel)` | a trip count from a parameter known at every call site | loops over `0..halo_k` and similar |

## Arc C's moves

**`(R-SliceArg)`'s range clause — moved.**  `src/slice_text.rs`, in the scope pass after
`vec_copy`; switch `LOFT_NO_SLICE_TEXT` (the writer's, now both backends'), trace
`LOFT_TRACE_SLICE_TEXT`.  It rewrites the STATEMENT that holds the call — an assignment, or an
append into a plain variable (the parser's text build) — because a text call as a block's bare
value has no destination to be written into: the first form, a block whose value was the range
call, answered `0` for `1365` on its probe (the WIP that showed it was kept opt-in until fixed).
The protocol's evidence: the C writer, left on, admitted nothing the IR clause took; every
bench's C with the IR clause on and the writer's off is byte-identical except cbor and
pluginabi, where one check-tier `loft_frame_site(260)` moves within line 260; the corpus switch
A/B (`switch_ab.sh`, 2910 programs, the clause firing in 8): 0 defects; the C sweep of those 8:
8 match; the guard cells on both backends, the clause on and off, under `LOFT_STRICT_STORES` and
`LOFT_POISON`, two new cells (a source with an effect runs once; the append host); falsified by
swapping the range call's bounds (t1 fails on both backends).  The census blessed: the phase
table's `R-SliceArg/range` is `ir`; the C rows count sites rather than emitted copies (cbor 2 → 1,
pluginabi 3 → 1), and `(R-CompleteWrite)` loses the slice temporaries' own groups, one per
emitted copy (cbor −2 C / −3 rustc, pluginabi −3 each) — their store no longer exists.
On the interpreter: 7 779 → 5 385 instructions per pass of the probe, −30.8 %.

**`(R-Wrapper)` — moved.**  `src/wrapper_inline.rs`, LAST in the scope pass, so every rewrite
before it sees the call it saw; switch `LOFT_NO_WRAPPER_INLINE` (the rustc writer's, now both
backends'), trace `LOFT_TRACE_WRAPPER`.  A rustc-only rule, so the C gains it too.  The
protocol's evidence: the rustc writer, left on, admitted nothing the IR clause took (its 2744
emitted copies against the IR's 1989 sites; residual 0 once a reference variable counts as a
leaf — first excluded, measured unneeded on the guard's w4 and benches 17 and 18); the corpus
switch A/B found two defects, both the lost-write lint's: it counts a collection length as no
read of the elements but matched only the method's name, `t_*_len` — it now names the length
operators too, and with the clause off no corpus program and no bench changes a diagnostic;
rerun: 0 defects in 2911 programs; the C sweep: 2578 match, one differ that is `main`'s own
(below); the guard's cells on the interpreter, the C and rustc, the clause on and off; the pin
`tests/wrapper_op.rs`; falsified by reversing `pow`'s operands (w6 fails on both backends).  The
analyses after the scope pass that recognised a length or a clear by the call's name read both
spellings now — `lower::apart_vector`'s clear (`R-Apart/vector` had fallen 30 → 0 on hex_body),
`lower::hand_over`'s rotate length (`R-HandOver/rotate` had fallen to nothing), the C's
transient length.  The census blessed: `R-Wrapper` applied by `ir`; `(R-InlineLeaf)` loses the
stdlib `cos`/`sin` calls this pass takes first (same op, without the argument locals), so
`(R-Leaf)` gains what `(R-LeafChain)` loses; `(R-Counter)`'s small drops are its count repeated
per inlined call (`non_sentinel` reruns after each); on the C `(R-Inline)`, `(R-RefParam)` and
`(R-NonSentinelParam)` lose the wrappers themselves, which nothing calls any more.  On the
interpreter: 921 → 479 instructions a pass of the probe, −48.0 %.

**`(R-PushFill)`'s fill form — moved.**  `src/opt/fill_loop.rs`, in `compile::close_program`
after `discharge_into` (the shape is `opt::push_window::push_loop`'s, which the scope pass, a
layer below `opt`, cannot ask); `if 0 < n { the push; OpAppendCopy(v, n, tp) } else { the loop
as written }`, so every range the fill declines runs the loop exactly; admitted onto a
non-nullable vector VARIABLE only (a field path's owner may be absent).  Switches: the rule's
`LOFT_NO_PUSH_FILL` (every backend) and the clause's `LOFT_NO_FILL_LOOP`; trace
`LOFT_TRACE_IR_FILL`.  rustc's fill (`push_fast_path`, which counted under no census name) is
deleted; the C gains the fill.  The protocol's evidence: switch_ab.sh over 2912 programs, 0
defects; c_sweep over 2948 programs, 2579 match, the one differ main's own; the cells
`a-counted-loop-pushing-one-value-is-one-fill.loft` on the interpreter, the C and rustc, the rule
on and off, falsified by a trip count one short (f1, both backends); `tests/push_fill.rs`
re-pinned on the IR form; c_emit 71/71; decline_census, enum_record, emission_audit,
iteration_base.  Declines recorded rather than closed, each measured on the C ship lane with
the clause off and on: zttext's `char_styles` loses the outer loop's push window (`R-PushFill`
−2, `R-RecPtr` −2/−1) — zttext's routines 1.48× → 1.43× median; hex_terrain's
`terrain_surface_at` keeps `wm` in the store instead of apart (`R-Apart/vector` −1) — the
routine 1.51× → 1.50×, `terrain_hydrology` 1.32× → 1.17×; hex_draw's `footprint` loses one
`R-ValueLocal` — `surface_fitted_spread` 1.34× → 1.32×.  So the writers' forms bought nothing
over the fill there, and teaching their analyses `OpAppendCopy` waits for a case that does.
The fill declines a `single` value (`simple_invariant` lists no single literal, as before):
arc D.  On the interpreter: 9 847 → 2 776 instructions a pass of the probe, −71.8 %.

**`(R-KeyedEntry)` — moved.**  The analysis (`site`) moved from `lower` to `src/opt/keyed_entry.rs`
(the interpreter's `close_program` may not name `lower`), its buffer test counting the
buffer's bindings instead of comparing nodes, so it answers the same on the copy the pass
rewrites.  The pass stages every value that is not a constant into a fresh temporary, typed by
its setter's value parameter, in program order; then `B = keyed_entry_ints(c, tp, fld, k₁…k₄)`
or `keyed_entry_text(c, tp, fld, key)`; then the other fields written into `B`.  A text key is
read at the probe, so it is admitted as a literal or a variable no later value rebinds;
generators are skipped.  The two functions are stdlib declarations (`default/01_code.loft`):
one `OpStaticCall` on the interpreter (`native::n_keyed_entry_*` → `Stores::keyed_entry` /
`keyed_entry_text`), `KeyedEntryEmitter` on rustc (`keyed_entry_long_at`'s inline walk for one
part, the offsets resolved at generation), the C's `keyed_native` case
(`loft_keyed_entry_ints` / `_text`).  Both writers' insert clauses are deleted; the subscript
clause stays theirs.  Three things the move had to carry, found by the census: the hash
reservation (`hoist::hash_fill_loop`) and the leaf rules (`frame_calls::is_store_entry`) read
the entry call as they read the claim; `R-CompleteWrite` falls at every keyed site because the
claim it trimmed is gone (a miss claims inside the store call), and the writers' own count fell
from emitted copies (a guarded loop's two, the C's `__fr` twin) to the IR's source sites.  The
protocol's evidence: switch_ab.sh over 2913 programs, 0 defects; c_sweep 2949 programs, 2580
match, the one differ main's own; the cells — the suite's three, the four in `tests/scripts`
and `a-keyed-insert-stages-its-values-before-the-entry-is-found.loft` (s1–s7, falsified by the
values unstaged: s2's effect order fails on the interpreter and the C) — on the interpreter, the
C and rustc, rule on and off; c_emit 71/71; `tests/mint_window.rs` re-pinned (m7's entry
reference takes the view clause's address); gridmesh's bench rows agree interpreter / C.
On the interpreter: 15 550 → 2 073 instructions a pass of the probe, −86.7 %.

**`(R-ElemFirst)`'s literal-built temp — moved.**  `src/opt/elem_first.rs`, last in
`compile::close_program`: `opt::hoist::element_first`'s pairs (asked without a view plan, which
only the writers build) whose every temp is literal-built and whose append stands once.  At the
first temp's declaration: the reservation, `elm = OpNewRecord(out, tp, fld)` and a zero per
paired vector handle (a fresh C slot is not zeroed, so the zero is the IR's on every backend);
each declaration becomes `t = OpGetField(elm, off, ftp)`; the append keeps its scalar sets and
finish.  Still the writers': the call-filled temp (@PLN164 E-2), the joined arms (E-2b), the
comprehension row and a function whose exits copy a temp — each asks the same analysis of the IR
it receives, which no longer shows a pair the pass took.  The writers' clauses stay for them,
so this move deletes no writer code; the C's clause now admits nothing on the benches.
Switches: the rule's `LOFT_NO_ELEMENT_FIRST` (every backend), the clause's
`LOFT_NO_IR_ELEM_FIRST`; trace `LOFT_TRACE_IR_ELEM_FIRST` (one line per pair and its
destination, which `tests/element_first.rs` now counts beside rustc's mint comments).  The
protocol's evidence: switch_ab.sh over 2913 programs, 0 defects; c_sweep 2949 programs, 2580
match, the one differ main's own; the suite's four cells, `164-element-place.loft` and the V-z
cell file on the interpreter, the C and rustc, the clause on and off (the pass takes the five
V-z pairs the pin predicts, and none of 164's, which are call-filled or need the view plan);
`tests/element_first.rs` 3/3; falsified by the append's paired copy kept (the suite's
`157-element-first.loft` fails on the interpreter and the C).  The census's declines sit in drawing's `fronds` and
`lock_ribbons` (rustc `R-Scalar` −27, `R-Mint` −17, `R-CompleteWrite` −12, `R-PushRec` −4,
`R-RangedCall` −5; C `R-CompleteWrite` −12): the IR now shows the temp's pushes growing the
destination's store, where the writer applied the rule after its analyses had read a temp with
a store of its own.  Timed on drawing's 14 routines, the clause off and on: the C ship lane
level or better (`fronds` 64 650 → 62 280 ns); rustc native `fronds` 50 626 → 64 134 ns
(1.35× → 1.71×), every other routine level.  rustc native is outside the perf targets; the
follow-up is `R-Scalar`'s decline in `fronds`' build loops.  On the interpreter: 8 252 → 5 886
instructions a pass of the probe, −28.7 %.

**`(R-ElemFirst)`'s call-filled temp and joined arms — moved.**  The call-filled temp's lazy-buffer
guard becomes the mint, and the call is handed `OpGetField(elm, off, ftp)` for its buffer (`ftp`
read from the append's paired copy).  The joined arms are NOT aliased: a `Set` of one record
variable from another binds a COPY (`(B-Copy)`; the C wrote `loft_bind_copy`, and the arm wrote and
filed the copy — two leaked records, the arm's values lost), so every use of the other arm's
element is renamed to the minted one, and that element's mint and pre-init go.  Most pairs of
these forms also need the view plan (a function returning the temp in a record), which only the
writers build, so the benches show no census move; `164-element-place.loft` takes 4 pairs (its
record-collection cells).  Cells:
`an-element-is-built-in-place-by-its-call-and-in-either-arm.loft` (e1 joined arms, e2 a call, e3
both), falsified by the rename left out (the interpreter stops naming the element, the C exits
101); the rule's other cells and the four pins pass on the interpreter, the C and rustc, the clause
on and off.  Taking the view-plan forms needs that plan in `opt` read AFTER this pass — the writers
recompute it on the rewritten IR, and a plan read before could disagree.

**`(R-LoopBuffer)` — moved.**  `src/opt/loop_buffer.rs`, the LAST `close_program` pass: a
declaration `OpDatabase(vdb) · t = OpGetField(vdb, 0, ftp) · OpSetInt4(vdb, 0, 0)` of a
`hoist::loop_buffers` buffer becomes `if OpRefIsNull(vdb) { mint; zero } else {
OpClearVector(OpGetField(vdb, 0, ftp)) }` then the bind, so the first pass mints and every
later one clears the vector, keeping its store and capacity; rustc's clause (its mint emitter
and its dropped field zero) is deleted and the C gains the rule.  It leaves the buffers another
rewrite owns, asked of the IR the writers see (the reason it runs last): an invariant literal's
flat group AND the buffers a one-`Set` literal is built from (`R-LitHoist` builds it once,
reading those declarations as invariant — drawing's `fd_sides`), an element-first pair, a move
host, the adopted witness, and the companions of a vector `R-Apart` keeps inline
(`opt::apart_vector`, moved from `lower` for this; the inline buffer is no store at all).  A
function that yields or calls a generator is declined: a live frame may hold the store, which
rustc tested at run time (`is_frame_held`).  The C's push-run reservation reads the form as an
empty vector (`known_empty`, now `opt::empty_slot` — C′1).  Found on the way, in every IR pass of
this plan: a definition with a native NAME (a library export once its artifact is built) was
skipped, though a native build emits its loft body — hex_shape with a warm library lost
`form_read`'s 16 buffers; the filter is gone.  Census: rustc counted once per emitted copy (the
`__kh` / `__rg` twins), the IR once per site — hex_shape 42 → 30 is exactly the two twins; the
rustc declines left in drawing (`R-Scalar` −4 / `R-Header` −3 / `R-Callee` −1 in `draw_sky`)
measured level.  Timed on drawing's 14 routines, the old binary against the new: rustc native
level on every routine; the C ship lane median 1.61× → 1.55× (`render_lock` 1.67 → 1.56,
`render_marks` 2.03 → 1.83, `resize` 1.73 → 1.56).  Evidence: switch_ab.sh and c_sweep (in
the commit), the rule's cells and `a-loop-buffer-kept-in-the-ir-reads-as-a-fresh-vector.loft`
(b1–b5, falsified by the reset arm left empty: b1 fails on both backends) on the interpreter, the
C and rustc, the rule on and off; c_emit 71/71; `tests/loop_buffer.rs` re-pinned on the IR form;
apart_value, element_first, mint_window.  On the interpreter: 7 311 → 6 106 instructions a pass
of the probe, −16.5 %.

**`(R-LoopRecord)` — moved.**  `src/opt/loop_record.rs`, after `opt::loop_buffer` in
`close_program`: an admitted record (`opt::loop_records`, the nest and refill clauses with it)
is declared `= null` before the outermost loop (`Function::move_scope`, new for this), its
per-pass null and end-of-body free go, a complete literal's mint becomes `if OpRefIsNull(v) {
mint; vector zeros } else { OpClearVector of each vector field }`, an early exit's free nulls
the local, and one free follows the loop.  rustc's clause (its prelude declaration, postlude
free, dropped statements and kept-record mint) and the C's refill port are deleted; the C keeps
only `(R-FnRefBuffer)`'s records in `c/body/loop_record.rs`, and now keeps plain records too
(drawing, 2 — the IR's kept pass mints nothing, where the C's port had measured a store reset as
no gain).  Ownership goes through `opt::buffer_owner`, which gained the apart RECORD owners
(`opt::apart_records`, moved from `lower` for this, with `ident_part` and `apart_disabled`
given their one home there): an apart record holds no store to keep.  `opt::empty_slot`'s
block scan learned that a write to ANOTHER field of the record leaves a vector slot empty —
without it the C's push-run reservation lost hex_recover's `forms_upto` (8 → 6), whose kept
record's other fields are written between the clear and the push.  Census: the rustc rows
count sites rather than emitted copies (hex_fit and hex_recover refill 5 → 3); the
`(R-CompleteWrite)` drops (drawing −2, hex_fit −5 and hex_recover −5 on rustc, −2 / −3 on the C)
are exactly the kept complete records, whose mint now sits in the guard's arm away from its
literal group, so the writers prefill that one store on the first pass — the order to fix when
`(R-CompleteWrite)` moves (its IR pass runs ahead of this one; the IR has no no-prefill mint
operator yet, `OpDatabaseNP` is a writer spelling).  Evidence: the switch A/B
(`LOFT_NO_LOOP_RECORD`, 2 916 programs): 0 defects; the C sweep (2 952 programs) 2 583 match and under `--strict-stores` 2 582, the one differ in both an attic cell that expects a parse error and parses (the same with the rule off); the rule's cells
(`157-loop-record.loft`, `157-a-loop-record-keeps-its-vectors.loft`, the nest cells,
`a-function-value-fills-its-callers-record.loft`) and the new
`a-loop-record-kept-in-the-ir-reads-as-a-fresh-record.loft` (r1–r6: `continue`, `break`,
`return`, a partial literal, the nest) on the interpreter, the C and rustc, the rule on and off;
the new cell falsified by the early exit's null left out (caught under `LOFT_STRICT_STORES=1`
only — the plain answer stays right by accident); `tests/loop_record.rs` re-pinned on the IR
(it counts the pass's trace), c_emit 71/71, push_window, push_fill, work_buffer, heap_sound,
locked_writes, loop_buffer.  On the interpreter: 3 058 → 1 613 instructions a pass of the
probe, −47.3 %.

**`(R-CompleteWrite)` — moved.**  `src/opt/complete_write.rs`, in `close_program` ahead of
`opt::loop_record` ([cw-plan.md](cw-plan.md) S0–S4).  The decision is a STORED fact, not an IR
spelling: `Definition::no_prefill` holds the locals and the mint parent types whose literal
group writes every field, keyed so a later pass cannot stale it, and it crosses the program
image (`selfhost/src/ir.loft`, both codecs).  The interpreter's code generator emits the twin
opcode (`OpDatabaseNP`, `OpNewRecordNP`, new operators); rustc and the C read the fact and lose
their own computation.  The twins first stood in the IR (S2), which made ~126 later readers of
`OpDatabase` / `OpNewRecord` decline (the census under it: 40+ rules, `R-RetAdopt` −76,
`R-Mint` −74, …); the stored fact leaves those readers unchanged.  The fresh-field null arm
is spelled in the IR instead: the copy is flagged `COPY_FRESH_DEST`, whose null source every
runtime writes as the type's defaults — S1, which fixed loft#2414 on the way (a vector-literal
slot's nullable field read 0, not null).  `opt::writes::in_place_copy` admits a fresh-flagged
copy of a type that owns no heap.  Census: the rule applied by `ir`; rustc and the C counted per
emitted copy, the IR once per function; emission against the writer-clause binary: no bench
program writes a prefill it did not (rustc's kept-record mints no-prefill again, its
`set_default_value` null arms in the flagged copy's runtime arm, the C fewer `loft_fill`).
Evidence: switch A/B (`LOFT_NO_COMPLETE_WRITE`, 2 917 programs): 0 defects; the C sweep (2 953 programs) 2 584 match plain and under `--poison`, the one differ the attic cell that expects a parse error; the 2414 / 2044 / loop-record cells on the interpreter, the C and rustc in
both switch states; the pins (`complete_write`, `field_append`, `field_mint`, `group_push`,
`place_result`, `prefill_image`, `record_push`, `ir_schema_roundtrip`, the codec units).  On
the interpreter: a push loop of complete literals 3 922 → 3 746 instructions a pass, −4.5 %.

**`(R-TextBorrow)` — moved to the interpreter.**  An interpreter clause, not an IR spelling: the
decision already lived once in `opt::text_borrow` / `opt::store_text_borrow`, and only the
writers applied it.  `State::mark_text_views` (`src/state/codegen.rs`) now marks every variable
that decision admits, asked as the C asks it (walks, discharge temps, the store-read, local-root
and format clauses); the tested clause (a run-time test) and the call clause (the compiled
writers' calling convention — on the interpreter `j = join(v, ", ")` read `""`) keep the copy.
A view slot holds a `Str`: a rebind skips `OpClearText`, and the empty literal is put on the
first bind and on a rebind.  Every reader of a text slot's layout asks `holds_str` — the halted
frame's release did not, which aborted main's interpreter on a failed assert inside a borrowed
walk (loft#2432, fixed on the way).  Census: `R-TextBorrow` and `R-SliceBorrow` applied by
`interp` too.  Evidence: the switch A/B (`LOFT_NO_TEXT_BORROW`, 2 925 programs) 0 defects (its
first run found the call clause and loft#2432); the C sweep (2 961 programs) 2 592 match, the
one differ the attic cell that expects a parse error; `find_problems --subject` c, docs,
runtime, codegen: 2 526 passed; a 17-cell probe matrix under `LOFT_POISON` and
`LOFT_STRICT_STORES`; the cell
`a-borrowed-text-rebound-takes-each-bind-whole.loft` (falsified by leaving the empty rebind's
put out: r1 = 663) and `tests/scripts/2432-…`.  On the interpreter: 62 784 → 51 344
instructions a pass of the probe, −18.2 %.

**`(R-SliceInto)` — moved.**  `src/slice_into.rs`, in the scope pass after `(R-SliceArg)`'s range
clause and on its slice-block test (`slice_text::admit` / `split` / `outside_mentions`, now one
home for both rules; the C's `slice_fill` was a second copy of it).  `OpAppendVector(P, <slice
block>, tp)` becomes the bounds' statements and `OpSliceVector(P, src, lo, hi, row)` — the form
the parser already gives a local place.  The bounds now run before P is read, so P is admitted
only as variables and constant field offsets that no bound statement assigns (the C writer
evaluated P first and took any expression; no corpus site needed more).  Switch
`LOFT_NO_SLICE_INTO` (the C's, now every backend's), trace `LOFT_TRACE_SLICE_INTO`.  The C
writer's clause (`slice_fill`, `slice_into`) is deleted; the transient writer's clause reads the
IR form through an `OpSliceVector` arm.  Evidence: with both on, the C writer admitted nothing;
the C of the 3 036 corpus and bench programs, old binary against the IR clause, differs in 8,
each read: one pair of parentheses the writer's raw place wrapped, and the cell's new cases;
the switch A/B (2 931 programs) 0 defects; the C sweep (2 967 programs) 2 597 match, the two known
differs, and the eight changed programs clean under `--poison`; `find_problems --subject` c and
codegen with rustc native: 917 passed; the cell on the interpreter, the C and
rustc, the switch on and off, under `LOFT_POISON` and `LOFT_STRICT_STORES`, four new cases (the
field its own source, a field with elements, a bound with an effect, `i32` elements), falsified
by a lower bound of 0 in the IR form (8 of 12 fail on both backends).  Census: `R-SliceInto`
applied by `ir`; the C rows count sites, not emitted copies (cbor and pluginabi 3 → 1), and
rustc gains it (0 → 1); `(R-CompleteWrite)` loses the temporary's own group (−1 each), whose
store no longer exists; on rustc three `R-Header` sites become `R-Header/function`: with the
temporary's store gone the decoder holds its source's header for the whole function.  On the
interpreter: 10 292 → 7 536 instructions a pass of the probe, −26.8 %.

**`(R-RetAdopt)` — moved to the interpreter.**  An interpreter clause, not an IR spelling, as
`(R-TextBorrow)` moved: the decision already lived once in `opt::ret_adopt`, and only the
compiled writers applied it.  An IR spelling would also reach the twins those writers build of
the same body (`__ap`, `__d`), whose return buffer is the CALLER's vector and which must not
adopt it.  `State::retbuf_adopt` (`src/state/codegen.rs`) takes the decision once per function
in `def_code`; `adopted_bind` writes the result's bind as the buffer (minted when the caller
offered none, cleared otherwise) and then the bind to its vector field; `generate_call` skips
the witness's mint and zero; `adopted_delivery` skips the delivery's clear and append.  It reads
nodes through `IrNode`, so warm runs (the body in the program store) take it too.  Switch
`LOFT_NO_RETBUF_ADOPT`, now every backend's.  The plan's probe never reached the rule — the scope
pass promotes its result local onto the buffer — so it was priced on the libraries where it
fires, the interpreter with a throwaway prototype on and off: 46 routines, every hash equal;
drawing `fronds` −7.3 %, mesh3d `mesh_to_floats` −2.6 %, nothing else past 2 %.  Built: `fronds`
935–944 → 865–874 µs, cold and warm.  Evidence: the switch A/B (2 931 programs) 0 defects; the C sweep (2 967 programs, the interpreter its oracle) 2 597 match, the two known differs;
the drop gate's interpreter cells clean;
the six adoption cells on the interpreter and the C, the switch on and off, under `LOFT_POISON`
and `LOFT_STRICT_STORES`; falsified by keeping the delivery's clear (`157-retbuf-adopt`'s `main`
fails on the interpreter).  A first plant, dropping the reuse arm's clear, changed nothing: the
function's entry clear already empties an offered buffer.  `find_problems --subject` codegen
and runtime: 2 329 passed, the one pin that moved being `adopt_buffer_reuse`'s store census
(3 interpreter mints fewer on and off the pool, the switch giving the old pair back).  Census:
`R-RetAdopt` applied by `interp` too.

## Phase ordering

A0 → A1 → A2 → A3 (the table first, so every later move is a blessed change of it) · B1 →
B2 · D-fact-1 → D-fact-3 → D-fact-2 (design.md § The facts deviations) · C in three waves —
existing operators, then one stdlib function or flag each, then the IR twins after D-fact-3 —
each rule after its B2 verdict · the ports to the C as their own track · D1, then D2…, each
clause in its rule's phase.  A, B
and D1 are independent; C needs A1 (the bless) and its rule's B2 row; a D clause of a rule
that arc C moves is built after the move, in the IR phase.

## Open design questions

Both answered by B2 (§ Verdicts, "The open design questions, answered"): a check-free operator
variant is no fusion but saves little where dispatch is the cost, and `(R-Transient)` /
`(R-Apart)` get no store-level form — both STAY, their interpreter gains go to arc D.

## Cross-arc dependencies

- **`formal/facts.md`** (was #2351, closed into this plan) — its three deviations are removed inside this plan
  (design.md § The facts deviations); a moved rule's callee facts come from the facts routine;
  D-fact-2 (interval entry facts) is `(R-RangedCall)`'s precondition to leave the rustc
  writer.  The two plans share B2's "decidable on the IR" question.
- **@PLN210** — the source of most C-only rules.
- **The per-routine issues of the 3× bug line** — each is a symptom D1 assigns to a class;
  a D clause is measured on every routine of its class, and closes their issues when it brings
  them under the line.
- **@PLN205** (lean interpreter) — the interpreter-side measurements and their layout pin
  (`INTERPRETER_PERFORMANCE.md` § Measuring an interpreter change).

## Verdicts

Filled by B2: one row per WORK and rustc-only rule — rule · decidable on the IR · expressible
· weight · probe result · verdict · phase that moves it.  The first verdicts, from reading the
code and each still owed its probe, are in [design.md § The first verdicts](design.md#the-first-verdicts).

**Wave 1 — the rules that can move with what exists**, probed.  Weight is B1's routines' worth
([b1-weights.md](b1-weights.md)); the probe is `probes/<case>/` run by `probes/run.py`:
instructions per iteration on `--interpret`, the shape as written against the same work in the
form the IR rule would leave, both printing the same answer, the rule admitted in the case's
function before and not after.

| rule | decidable on the IR | expressible | weight | probe (before → after) | verdict |
|---|---|---|--:|---|---|
| `(R-Wrapper)` | yes — `ir_shape::one_op_wrapper` | `Call(op, operands)` | 40.9 | `wrapper`: 920 → 308, **−66.5 %** (upper bound: the operator itself is left out); moved: 921 → 479, **−48.0 %** | **MOVED** (`wrapper_inline::rewrite`, arc C's second) |
| `(R-PushFill)` fill form | yes — IR facts | `OpPush` + `OpAppendCopy`, as `[c; n]` lowers | 17.2 | `push_fill`: 10 580 → 3 017, **−71.5 %**; moved: 9 847 → 2 776, **−71.8 %** | **MOVED** (`opt::fill_loop`, arc C's third).  The C's admission on the probe was its window clause, and rustc's fill had no census name |
| `(R-ElemFirst)` | yes — `opt::hoist::element_first` | existing operators | 1.0 | `elem_first`: 9 666 → 7 299, **−24.5 %**; moved: 8 252 → 5 886, **−28.7 %** | **MOVED: the literal-built temp, the call-filled temp, the joined arms** (`opt::elem_first`, arc C's fifth); the comprehension row and the view-plan exits are still the writers' |
| `(R-SliceArg)` range | yes — IR facts | `text_from_byte_range` | 0.5 | `slice_arg`: 7 771 → 3 351, **−56.9 %** (the hand form skips the slice's bound normalisation); moved: 7 779 → 5 385, **−30.8 %** | **MOVED** (`slice_text::rewrite`, arc C's first) |
| `(R-LoopBuffer)` | yes — `opt::hoist::loop_buffers` | `OpClearVector` on a held store | 0.6 | `loop_buffer`: 7 803 → 6 508, **−16.6 %**; moved: 7 311 → 6 106, **−16.5 %** | **MOVED** (`opt::loop_buffer`, arc C's sixth) |
| `(R-LoopRecord)` | yes — `opt::loop_records` | the record's null to the prelude, one free after the loop | 0.1 | `loop_record`: 3 150 → 1 545, **−51.0 %**; moved: 3 058 → 1 613, **−47.3 %** | **MOVED** (`opt::loop_record`, arc C's seventh) |
| `(R-SliceInto)` | yes — IR facts | `OpSliceVector` onto the place | 0.5 | no hand form ([probes/slice_into/NOTE.md](probes/slice_into/NOTE.md)): a field place is not spellable; a LOCAL place `p += v[a..b]` already compiles to one `OpSliceVector`; moved, the switch off against on, `slice_into`: 10 292 → 7 536, **−26.8 %** | **MOVED** (`slice_into::rewrite`, the scope pass) |
| `(R-ValueRecord)` view clause | yes — `opt::value_records`' view leaves, decided on the IR already (by each writer) | a value-record tuple element that is a REFERENCE to a place — new to the IR's tuple | — (not in B1) | no probe yet; S0's count: `R-ValueRecord/view` 3 bench functions, drawing's `parse_lock` / `parse_poly` among them | MOVE — c4-view-plan.md S1–S3; carries `(R-ElemFirst)`'s view-exit form with it (D-phase-4, D-phase-5) |
| `(R-RetAdopt)` | yes — `opt::ret_adopt` | the guarded witness | 0.3 | no hand form ([probes/ret_adopt/NOTE.md](probes/ret_adopt/NOTE.md)); the probe's shape is promoted by the scope pass, so it was priced where the rule fires: drawing `fronds` −7.5 %, mesh3d `mesh_to_floats` −2.6 % on the interpreter | **MOVED to the interpreter** (`State::retbuf_adopt`: an IR spelling would reach the compiled writers' twins) |

**Wave 2 — the rules that need one new piece.**

| rule | the new piece | weight | evidence | verdict |
|---|---|--:|---|---|
| `(R-KeyedEntry)` | find-or-insert answering a reference, a stdlib function through one `OpStaticCall` | 4.2 | `keyed_entry`: an overwriting append of a present key 16 068–16 835 → 1 790 instructions, **−89 %** over two runs (the before form moves with the hash seed; the hand form looks up, then writes or inserts); moved: 15 550 → 2 073, **−86.7 %** | **MOVED** (`opt::keyed_entry`, arc C's fourth) |
| `(R-RefillText)` | the refill twin | 0.6 | not a DROP: with `LOFT_NO_REFILL_TEXT=1` the rustc build of game_protocol runs **+148.6 %** instructions (`msg_ping`, same hash), so `(R-Reuse)`'s pool reset does not take its cost.  The C takes that cost another way: `ping_op` calls `n_msg_ping__tr0r`, an `(R-Transient)` twin.  No hand form for the interpreter | **STAYS** in the rustc writer (2026-11-06, by § Goal's test of a duplicate): no C clause beside it, and no interpreter gain to move it for |
| `(R-RefillBuffer)` | a REFILL bit on `OpDatabase` | 4.4 | its switch A/B on the rustc build: bench 17 **+0.0 %**, game_protocol **−0.2 %** instructions — it saves nothing measurable where it fires most | STAYS — no flag for a rule with no measured effect; whether to drop it is the owner's call |
| `(R-FoldCompare)` | `fold_compare`, a stdlib function | 0.0 | admitted in no function a bench routine runs | DEFER until a consumer reaches it |
| `(R-HandOver)` rotate | `rotate_into` + the `__nk` twin | 0.1 | two routines | DEFER by weight |
| `(R-AppendTwin)` base | the `__ap` twin | 0.2 | two routines | DEFER by weight |
| `(R-Destination)` | the `__d` twin + a failure-path release | 0.5 | one routine | DEFER by weight |

**Interpreter clauses** — the interpreter's generator or runtime applies the same fact.

| rule | weight | evidence | verdict |
|---|--:|---|---|
| `(R-CompleteWrite)` | 30.9 | the heaviest work rule the interpreter lacked; the skipped prefill cannot be spelled in loft | **MOVED** (`opt::complete_write`, arc C's eighth: a stored fact, the twin opcodes; [cw-plan.md](cw-plan.md)) |
| `(R-TextBorrow)` | 13.7 | `text_borrow` (the parameter-text clause, 16 borrows a pass): 62 800 → 57 791, **−8.0 %**, about 310 instructions per borrowed text; moved: 62 784 → 51 344, **−18.2 %** | **MOVED** (`mark_text_views` reads the writers' decision; § Arc C's moves) |
| `(R-FlatCopy)` | 1.1 | a runtime fast path, not spellable | INTERP-CLAUSE, by weight after the two above |
| `(R-FieldAppend)` | 2.6 | the interpreter's `Stores::record_new` / `record_finish` already apply it (`plain_field_vector`); the census counts only the writers' clause | done — no move |
| `(R-Apart)` instance 4 | 2.4 | `(R-Reuse)`'s reset widened to the owner local | arc D, as that widening |
| `(R-FnRefBuffer)` | 0.0 | — | DEFER |

**Stays in its writer** — representation, each reason checked:

| rule | weight | why |
|---|--:|---|
| `(R-Push)`, `(R-PushRec)` header, `(R-PushFill)` window / run / record, `(R-AppendTwin)` window / loop | 17.8 / 6.7 / 17.2 | a vector's length and base held in C locals.  The growth they save is not worth moving: with the whole vector reserved by hand, the interpreter's `push` loop ran 280–283 ms either way and `f32_build` 595–640 → 592–596 ms (INTERPRETER_PERFORMANCE.md § the extra store work); dispatch is the cost.  `push_fill`'s −71.5 % comes from FEWER OPERATORS (the fill form), which is why that form moves and these stay |
| `(R-Header)` kept / grown / function | 19.7 / 2.7 / 10.7 | a header held across reads; on the interpreter `OpGetVector` derives it per read, and holding one would be a new operator |
| `(R-Mint)` | 8.8 | classifies mint groups for the representation rules; its one piece of work is already in the interpreter's runtime |
| `(R-WorkPark)` | 13.8 | a per-thread runtime park; an operator variant would buy one realloc chain per call |
| `(R-SliceArg)` carrier | — | the interpreter's slot reuse with an in-place `init()` is its analogue |
| `(R-Transient)` emission, `(R-SliceInto)` transient | 0.6 / — | arena structs and views.  The site PLAN moves to `opt/transient.rs` so the census credits the decision; the interpreter's gains go to arc D (`(R-Place)`, `(R-Destination)`, `(R-DecodeView)` widened) |
| `(R-Apart)` instances 2, 3 | — | a dynamically indexed value outside the store; the interpreter's tuple is read at constant indices only |
| `(R-Fill)` | 0.4 | needs a fill kernel; spans under eight lost on `fill_triangle` — the owner decides whether a kernel is wanted |
| `(R-Leaf)`, `(R-LeafChain)` | 42.4 / 60.6 | a call without its shadow frame.  The interpreter's frame IS its call.  On the C it is inherent: `c_driver` compiles `-DLOFT_CALL_CHAIN` only in the check and debug tiers, so the release tier's `loft_frame_push` / `loft_frame_pop` compile to nothing for every function |

**The rustc-only rules.**

| rule | weight | evidence | verdict |
|---|--:|---|---|
| `(R-Wrapper)`, `(R-LoopBuffer)`, `(R-PushFill)` fill form | — | wave 1 | MOVE |
| `(R-LazySplit)`, `(R-TypedKeyed)`, `(R-TypedRead)` | 3.5 / 2.7 / 0.6 | on the C, counted since A2 | on the C |
| `(R-LitDiv)` | 25.5 | inherent on the C: `loft_div_at` is always inlined, and its zero test of a literal divisor folds | on the C |
| `(R-SliceBorrow)` | 0.4 | the C admits it nowhere: markdown's `render_inline`, five slices the analysis admits, kept as `loft_text` copies (A2) | PORT — the C writer's borrow of a slice |
| `(R-GuardFree)` | 15.1 | the C's release-tier wrapper still marks and checks the fn-ref list around every call (`size_t fm = loft_nfnrefs; … loft_fnref_release(fm)`), for a function that registers none | PORT |
| `(R-FnHeader)` | 10.7 | a C prologue on `fn_header_params` | PORT, hand-priced on drawing first |
| `(R-RangedCall)` | 5.8 | needs the interval entry facts | PORT after D-fact-2 |
| `(R-FaultFlag)` | 1.0 | — | PORT only if `06_newton_sqrt`'s C ratio shows the gap (one `stats.py --only 06 --lanes c,rust`) |
| `(R-TextClear)` | 7.8 | both other backends reset a text in place (`loft_text_set` on the C, `OpClearText` on the interpreter) | DROP |
| `(R-FnRefValue)`, `(R-StreamWalk)`, `(R-RebindBuffer)`, `(R-GroupPush)` | 0.0 / 0.1 / 0.0 / 0.2 | — | DEFER by weight (`(R-RebindBuffer)` also waits on `(R-RefillBuffer)`, which stays) |

**The open design questions, answered.**  (1) A check-free operator variant for `(R-Range)` /
`(R-NonSentinelParam)` merges nothing, so it is not a fused operator; but the interpreter's cost
is DISPATCH, not the work inside an operator (INTERPRETER_PERFORMANCE.md: `vector_append` is 5 %
of `push`'s cycles, the dispatch of its 13 ops per element the rest), so removing a
compare-and-branch inside an op saves little against a second operator per checked one: both
STAY.  (2) `(R-Transient)` and `(R-Apart)` weigh 0.6 and 2.4 on the interpreter; a store-level
arena would be a new representation for that, and the gains go to arc D's widenings instead:
both STAY.

**Arc C's order**, by measured gain and weight: `(R-SliceArg)`'s range clause first (the
protocol's proof, −56.9 %), then `(R-Wrapper)` (40.9, −66.5 %), `(R-PushFill)`'s fill form (17.2,
−71.5 %), `(R-KeyedEntry)` (4.2, −89 %), `(R-ElemFirst)`, `(R-LoopBuffer)`, `(R-LoopRecord)`;
then `(R-CompleteWrite)` and `(R-TextBorrow)` as interpreter clauses; `(R-SliceInto)` (moved),
`(R-RetAdopt)` (moved, an interpreter clause) and `(R-RefillText)` (stays).  The ports to the C
(`(R-GuardFree)`, `(R-FnHeader)`, `(R-SliceBorrow)`) are their own track.

## Documentation

- `doc/claude/formal/rewrites.md` — `(R-Phase)` points at the generated `## Applied by`
  index; each moved rule's entry says its phase; each STAYS rule says why.
- `doc/claude/BOTH_BACKEND_SWITCHES.md` — each moved rule's switch, moved from
  `C_BACKEND.md` / `NATIVE_SWITCHES.md`.
- `doc/claude/C_BACKEND.md`, `doc/claude/NATIVE_SWITCHES.md` — the moved switches removed.
- `doc/claude/INTERPRETER_PERFORMANCE.md` — what the interpreter now runs in their place.
- `bench/README.md` § Interpreter against native — `--writer c`.
- `doc/claude/PROFILING.md` — the census phases and `rule_phases.tsv`.
- `doc/claude/CHANGELOG_TECHNICAL.md` — per moved rule and per widened clause.
- Each widened rule's entry in `formal/rewrites.md` — the new clause, its guard, its switch.
- `doc/claude/ROADMAP.md` — this plan's row.

## See also

- [`formal/rewrites.md`](../../formal/rewrites.md) `(R-Phase)`, `(R-Escape)`.
- [`formal/facts.md`](../../formal/facts.md) — the facts routine.
- [`../210-c-worst-to-2x/README.md`](../210-c-worst-to-2x/README.md).
- [`INTERPRETER_PERFORMANCE.md`](../../INTERPRETER_PERFORMANCE.md).
