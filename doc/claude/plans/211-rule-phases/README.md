<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 211 — Every rewrite rule at its proper phase

Tracker: [@PLN211](https://github.com/loft-lang/plans/issues/211).

## Status

Open, `status:next` — the inventory below is measured and the plan is DESIGNED
([design.md](design.md): every arc's files and checks, the first verdict for each rule, and the
removal of the facts routine's three deviations); A0, A2 and A1 are built (the census reads
both writers and the interpreter's generator; every rule reports to it or says why not; the
phase table `bench/portal/rule_phases.tsv` is measured and gated).  It was to start once
#1743 (every routine within 3×) and #2351 (the facts routine) were fixed; #1743 is fixed on
`laptop-c-perf`, and on 2026-11-03 the owner asked to concentrate on this plan, so #2351's
three deviations (D-fact-1 … 3) became part of it rather than a precondition.  Four arcs: **A** keeps the register honest about where
each rule runs, **B** decides per rule whether it can run earlier, **C** moves the ones that
can, **D** widens the rules we already have to the shapes they refuse today — the general
performance fixes, by class, behind the per-routine issues of the 3× bug line.

`formal/rewrites.md` `(R-Phase)` says every rule is APPLIED BY one phase and that the phase
decides its oracle: the IR phase (both backends run the rewritten IR), a writer (the
interpreter is the oracle) or the interpreter's generator and runtime (a compiled backend is
the oracle).  The arcs of @PLN157 and @PLN210 optimised for compiled code: most of their
rules were built in a writer, so the interpreter runs the IR they rewrite away.  A rule that
removes WORK — an allocation, a copy, a store round trip — belongs in the IR phase wherever
its conditions can be decided there, because then both backends stop doing that work.

## Goal

Every rule's phase stated in the register and checked against what the compiler does,
every rule that removes work applied in the IR phase (or an interpreter-runtime clause)
unless a stated reason keeps it in a writer, and each rule's most frequent refusals on the
benches either admitted by a generalised clause or recorded as the rule's boundary.

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
   probe routine.
3. **Then deleted.**  The writer clause is removed, its switch becomes the IR clause's, the
   rule's applied-by changes in the table (A1's bless), and the rewrite census shows no
   drop on the C side.

Each phase can go red alone: an emission that changes where the census did not, a guard
cell, a falsified plant.  A rule whose comparison shows the IR clause admitting FEWER sites
than the writer did is not moved until the gap is closed or recorded as a decline.

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

## Phase ordering

A0 → A1 → A2 → A3 (the table first, so every later move is a blessed change of it) · B1 →
B2 · D-fact-1 → D-fact-3 → D-fact-2 (design.md § The facts deviations) · C in three waves —
existing operators, then one stdlib function or flag each, then the IR twins after D-fact-3 —
each rule after its B2 verdict · the ports to the C as their own track · D1, then D2…, each
clause in its rule's phase.  A, B
and D1 are independent; C needs A1 (the bless) and its rule's B2 row; a D clause of a rule
that arc C moves is built after the move, in the IR phase.

## Open design questions

1. **Representation rules and the interpreter.**  `(R-Range)` and `(R-NonSentinelParam)`
   make an operation plain where a check is proven redundant; the interpreter could carry a
   plain operator variant.  Is that a fused operator in disguise (forbidden) or the same
   operator without its check (allowed)?  B2 asks it for these two before closing them as
   STAYS.
2. **`(R-Transient)` and `(R-Apart)`** build values outside the store as C structs.  The
   interpreter's counterpart would be a different representation, not this rule moved;
   B2 records it as STAYS unless a store-level form (an arena store) shows a gain in its
   probe.

## Cross-arc dependencies

- **#2351 / `formal/facts.md`** — its three deviations are removed inside this plan
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
