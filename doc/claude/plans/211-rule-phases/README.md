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
a verdict per rule in § Verdicts.  Arc C has moved its first rule,
`(R-SliceArg)`'s range clause (§ Arc C's moves).  It was to start once
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

**Wave 1 — the rules that can move with what exists**, probed.  Weight is B1's routines' worth
([b1-weights.md](b1-weights.md)); the probe is `probes/<case>/` run by `probes/run.py`:
instructions per iteration on `--interpret`, the shape as written against the same work in the
form the IR rule would leave, both printing the same answer, the rule admitted in the case's
function before and not after.

| rule | decidable on the IR | expressible | weight | probe (before → after) | verdict |
|---|---|---|--:|---|---|
| `(R-Wrapper)` | yes — `ir_shape::one_op_wrapper` | `Call(op, operands)` | 40.9 | `wrapper`: 920 → 308, **−66.5 %** (upper bound: the operator itself is left out) | MOVE.  The interpreter calls `t_6vector_len` every pass; `(R-InlineLeaf)` does not take stdlib wrappers |
| `(R-PushFill)` fill form | yes — IR facts | `OpPush` + `OpAppendCopy`, as `[c; n]` lowers | 17.2 | `push_fill`: 10 580 → 3 017, **−71.5 %** | MOVE.  The C admits this shape too (the census, not "rustc only") |
| `(R-ElemFirst)` | yes — `opt::hoist::element_first` | existing operators | 1.0 | `elem_first`: 9 666 → 7 299, **−24.5 %** | MOVE |
| `(R-SliceArg)` range | yes — IR facts | `text_from_byte_range` | 0.5 | `slice_arg`: 7 771 → 3 351, **−56.9 %** (the hand form skips the slice's bound normalisation); moved: 7 779 → 5 385, **−30.8 %** | **MOVED** (`slice_text::rewrite`, arc C's first) |
| `(R-LoopBuffer)` | yes — `opt::hoist::loop_buffers` | `OpClearVector` on a held store | 0.6 | `loop_buffer`: 7 803 → 6 508, **−16.6 %** | MOVE |
| `(R-LoopRecord)` | yes — `opt::loop_records` | the record's null to the prelude, one free after the loop | 0.1 | `loop_record`: 3 150 → 1 545, **−51.0 %** | MOVE (light) |
| `(R-SliceInto)` | yes — IR facts | `OpSliceVector` onto the place | 0.5 | no hand form ([probes/slice_into/NOTE.md](probes/slice_into/NOTE.md)): a field place is not spellable; a LOCAL place `p += v[a..b]` already compiles to one `OpSliceVector` | MOVE, as that parser path widened to field places; measured on arc C's beside build |
| `(R-RetAdopt)` | yes — `opt::ret_adopt` | the guarded witness | 0.3 | no hand form ([probes/ret_adopt/NOTE.md](probes/ret_adopt/NOTE.md)) | owed: measured on arc C's beside build |

**Wave 2 — the rules that need one new piece.**

| rule | the new piece | weight | evidence | verdict |
|---|---|--:|---|---|
| `(R-KeyedEntry)` | find-or-insert answering a reference, a stdlib function through one `OpStaticCall` | 4.2 | `keyed_entry`: an overwriting append of a present key 16 068–16 835 → 1 790 instructions, **−89 %** over two runs (the before form moves with the hash seed; the hand form looks up, then writes or inserts) | MOVE |
| `(R-RefillText)` | the refill twin | 0.6 | not a DROP: with `LOFT_NO_REFILL_TEXT=1` the rustc build of game_protocol runs **+148.6 %** instructions (`msg_ping`, same hash), so `(R-Reuse)`'s pool reset does not take its cost.  The C takes that cost another way: `ping_op` calls `n_msg_ping__tr0r`, an `(R-Transient)` twin.  No hand form for the interpreter | MOVE (twin), after wave 1; measured on arc C's beside build |
| `(R-RefillBuffer)` | a REFILL bit on `OpDatabase` | 4.4 | its switch A/B on the rustc build: bench 17 **+0.0 %**, game_protocol **−0.2 %** instructions — it saves nothing measurable where it fires most | STAYS — no flag for a rule with no measured effect; whether to drop it is the owner's call |
| `(R-FoldCompare)` | `fold_compare`, a stdlib function | 0.0 | admitted in no function a bench routine runs | DEFER until a consumer reaches it |
| `(R-HandOver)` rotate | `rotate_into` + the `__nk` twin | 0.1 | two routines | DEFER by weight |
| `(R-AppendTwin)` base | the `__ap` twin | 0.2 | two routines | DEFER by weight |
| `(R-Destination)` | the `__d` twin + a failure-path release | 0.5 | one routine | DEFER by weight |

**Interpreter clauses** — the interpreter's generator or runtime applies the same fact.

| rule | weight | evidence | verdict |
|---|--:|---|---|
| `(R-CompleteWrite)` | 30.9 | the heaviest work rule the interpreter lacks; the skipped prefill cannot be spelled in loft | INTERP-CLAUSE, the first to build; measured on arc C's beside build |
| `(R-TextBorrow)` | 13.7 | `text_borrow` (the parameter-text clause, 16 borrows a pass): 62 800 → 57 791, **−8.0 %**, about 310 instructions per borrowed text | INTERP-CLAUSE (`mark_text_views` widened); the risk is a view outliving its source on an early exit |
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
then `(R-CompleteWrite)` and `(R-TextBorrow)` as interpreter clauses; `(R-SliceInto)`,
`(R-RetAdopt)` and `(R-RefillText)` measured on their beside builds.  The ports to the C
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
