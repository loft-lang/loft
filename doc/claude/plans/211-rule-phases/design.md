<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 211 — the design

The plan is [README.md](README.md): four arcs that put every rewrite rule at its proper phase,
so a rule that removes WORK — an allocation, a copy, a store round trip — runs in the IR,
where the interpreter and every compiled backend stop doing that work.  This file is the
design behind it: what each arc builds, in which files, how each step is checked, and the
first verdict for every rule the inventory names.  The verdicts come from reading the code
(2026-11-03); every one is still owed its B2 probe before a phase is built on it.

## The invariant

**A rule is applied where its conditions are first decidable, and moving it never changes
what any backend computes.**  Two consequences carry the design:

1. A move is proven by a parallel run, not by argument: the IR clause is built beside the
   writer clause, the compiled emission is compared byte for byte, the interpreter's answers
   are compared with hand-computed cells, and only then is the writer clause deleted
   ([§ The move protocol](#the-move-protocol)).
2. A rule's phase is a FACT the compiler reports, not prose: the census credits every
   admission to the phase that decided it, and the register is generated from that
   ([§ Arc A](#arc-a--the-administration)).

## What the code already has

| piece | where | what it gives this plan |
|---|---|---|
| per-body IR passes | `scopes::check` (`src/scopes.rs`): `pure_reuse`, `bound_append`, `byte_copy`, `text_run`, … | the home of a moved rule that rewrites one body |
| whole-program IR passes | `compile::close_program` (`src/compile.rs`): `arg_copy`, `value_record`, `leaf_inline`, `ctor_literal`, `in_range`, … | the home of a moved rule that changes a signature or makes a twin |
| an IR twin | `value_record::add_twin` (`src/value_record.rs`) | the precedent for every twin a moved rule needs |
| shared analyses | `src/opt/`, `src/lower/` | most WORK rules already DECIDE here; only their emission is writer-only |
| the census | `src/rewrite_census.rs`: `fired`, `declined`, `InBody::enter(phase, fn)` | admissions per rule, per phase, per function |
| the census gate | `scripts/rewrite_census.py`, `make rewrite-census` | a drop against `bench/portal/rewrite_census.tsv` fails |
| the decline census | `scripts/decline_census.py`, `make decline-census` | each rule's refusals by reason |
| interpreter weight | `scripts/interp_gap.py`, the `op-census` build (`LOFT_OP_CENSUS`) | interpreter ops per routine and function, joined to the rewrite census |
| the facts routine | `src/opt/facts.rs`, `formal/facts.md` | the callee facts a moved rule asks |
| interpreter measurement | `INTERPRETER_PERFORMANCE.md` § Measuring an interpreter change | two pinned-layout builds, `perf stat -e instructions,cycles` |

**Four gaps the inventory did not show** (found while reading for the verdicts):

1. **The census only reads the rustc writer.**  `rewrite_census.py` emits `--native-emit`; the
   C writer's admissions — today's default compiled backend — reach no baseline.  A C clause
   can drop silently.
2. **Rules on the C under no census name.**  `(R-LazySplit)` (`c/body.rs` `lazy_split_bind`),
   `(R-TypedKeyed)` (`c/body/keyed.rs`), `(R-SliceBorrow)` (counted as `R-TextBorrow`),
   `(R-TypedRead)` (inherent in `c/body/files.rs`) and `(R-Transient)` fire nothing, so the
   inventory listed four of them as rustc-only.
3. **Census names that disagree between writers.**  `R-HandOver/rotate` (C) against
   `R-HandOver` (rustc); `R-LoopRecord/refill` against `R-LoopRecord`; `R-Kept`/`R-KeptTwin`
   name the KEPT clause on rustc and the GROWN-header clause and `__kh` twin on the C — two
   different clauses of `(R-Header)` under one name, and neither is a register entry;
   `R-FnHeader` is `(R-Header)`'s function clause without an entry.
4. **Decided on the IR, credited to a writer.**  `(R-Apart)` decides in `lower::plan`
   (`apart_value`, `apart_vector`, `apart_records`) but fires in the writers, so it reads as
   writer-only; so do `(R-Mint)` and the shared `opt::hoist` admissions.

So the phase table needs two columns where the inventory had one: **decided in** (a shared
analysis, or a writer's private state) and **applied by** (the IR rewritten, the
interpreter's generator or runtime, a writer's emission).  A rule decided in a shared analysis
and applied by a writer is the cheapest move: only its emission changes home.

## Arc A — the administration

**A0 — the census reads the C.**  `rewrite_census.py` emits every bench with `--emit-c` as
well as `--native-emit`, writes one baseline row per (program, writer, rule), and fails a
drop on either writer.  `LOFT_REWRITE_CENSUS_FN` already splits by phase; the C path enters
`InBody::enter("c", …)` (`src/c/mod.rs`), so the change is in the script and the baseline
format.  *Check:* a plant that removes one C `fired` call must fail the gate on the C row
alone.  Effort S.

**A1 — the phase table.**  `scripts/rule_phases.py` runs the census over the bench set on
both writers and writes `bench/portal/rule_phases.tsv`, one row per register entry:

```
rule   clause   decided-in   applied-by   programs   admissions(ir)   admissions(c)   admissions(native)
```

- *decided-in* is read from the code, not the census: the module of the function that
  answers the admission (`opt/…`, `lower/…`, `c/…`, `generation/…`).  A rule's `fired` call
  site names its clause; a small `decided_in` table in `src/rewrite_census.rs`, one line per
  rule, says where the decision lives, and the script checks it against `rule_tags.py
  sites @FR-R-…`.
- *applied-by* is the set of phases whose census row is non-zero.
- `make rewrite-census` fails when a rule's applied-by set CHANGES; a deliberate move
  lands with `--bless` in its own commit.

*Check:* a plant that credits one rule's admission to another phase fails the gate (that
plant is A1's receipt).  Effort S.

**A2 — no silent rule, one name per clause.**
- Every register entry either reports to the census or is on the exemption list
  (`(R-Prefill)`, `(R-Cold)`, `(R-FlatCopy)`).  Missing calls today: `(R-Transient)` (the
  site plan), `(R-LazySplit)`, `(R-TypedKeyed)`, `(R-SliceBorrow)` and `(R-TypedRead)` on the
  C.
- Census names are `R-<Entry>` or `R-<Entry>/<clause>` where `<Entry>` is a register entry:
  `R-Kept` becomes `R-Header/kept` (rustc) and `R-Header/grown` (C), `R-KeptTwin` becomes
  `R-Header/kept-twin`, `R-FnHeader` becomes `R-Header/function`; the writers' clause names
  agree (`R-HandOver/rotate` on both, `R-LoopRecord/refill` on both).
- `rule_tags.py phases --check`: a register entry with no census call and no exemption, or a
  census name whose root is no entry, fails.  *Check:* delete one `fired` call; the check
  fails naming it.  Effort S.

**A3 — the register says it.**  `rewrites.md` gains `## Applied by`, generated from
`rule_phases.tsv` (`scripts/rule_phases.py --render`), checked like every generated doc;
`(R-Phase)` names it as the home of every rule's applied-by line, and the prose `APPLIED BY`
lines go.  *Check:* the index drifting from the table fails the doc-drift check.  Effort S.

## Arc B — can it move?

**B1 — the weight.**  `scripts/interp_gap.py --writer c` takes its rewrite census from the C
emission.  Per rule it reports the interpreter's share of the time in the functions the rule
fires in, beside the op families the interpreter runs there (`records`, `store`, `frame`).
*Check:* for a rule both writers apply, the functions it fires in agree between `--writer c`
and `--writer native` where both emitted.  Effort S.

**B2 — the verdict, per rule.**  Three questions, each answered with evidence:

1. *Decidable on the IR?*  Every condition is an IR fact or a fact of the facts routine.
2. *Expressible in the IR?*  The rewritten form is IR the interpreter already executes:
   existing operators, a stdlib function through one `OpStaticCall`, an IR twin, or a flag on
   an existing operator's constant operand that the shared runtime reads.  A new fused
   operator means STAYS.
3. *Worth it?*  B1's weight, and the PROBE: the routine where the rule fires most, written
   by hand in loft (or in the IR form, through `LOFT_LOG=static` and a patched body) as the
   moved rule would leave it, run on `--interpret` under the pinned-layout builds —
   instructions per op against the unrewritten program, same answer.

A probe that shows no gain on the interpreter turns MOVE into STAYS: the verdict can go red
on its own.

### The first verdicts

From reading the code; every row still owes its probe.  "Decided in" is where the admission
is computed today.

**Can move with what exists** — existing operators, nothing new:

| rule | clause | decided in | the IR form | oracle cells |
|---|---|---|---|---|
| `(R-SliceArg)` | range | C `call` (IR facts) | the bounds' statements and `text_from_byte_range(src, lo, hi)`; `text_run::replace_read` is the precedent | `a-text-of-a-slice-reads-the-span-in-place.loft` |
| `(R-SliceInto)` | main | C `slice_fill` (IR facts) | the bounds' statements and `OpSliceVector(P, src, lo, hi, tp)`; `byte_copy` emits it already | `a-slice-appended-to-a-place-is-sliced-into-it.loft` |
| `(R-Wrapper)` | — | `ir_shape::one_op_wrapper` (IR) | `Call(op, operands)` for `Call(wrapper, args)`; each parameter once and in order, else bound first | the pin `tests/wrapper_op.rs` only: its cells are written with the move |
| `(R-LoopRecord)` | plain, refill | `opt::loop_records` (IR) | the record's null to the loop's prelude, the per-pass free dropped, one free after the loop; refill: `if OpRefIsNull(v) { OpDatabase } else { OpClearVector per field }` | `157-loop-record.loft` l1–l15, `157-a-loop-record-keeps-its-vectors.loft` |
| `(R-LoopBuffer)` | — | `opt::hoist::loop_buffers` (IR) | the in-loop mint guarded: a held store's vector field cleared (`OpClearVector`), else minted | its cells, named in B2 (the entry cites none by file) |
| `(R-ElemFirst)` | — | `opt::hoist::element_first` (IR) | the append group's `OpPreAllocVector` + `OpNewRecord(out)` moved to the temp's declaration, `t = OpGetField(elm, …)`, the `OpCopyRecord` dropped | suite `R-ElemFirst/*`, `157-a-vector-element-is-built-in-place.loft`, `164-element-place.loft` |
| `(R-RetAdopt)` | — | `opt::ret_adopt` (IR) | the witness `OpDatabase` becomes `if OpRefIsNull(buf) { OpDatabase(buf) } else { OpClearVector(buf) }`, `v = buf`; the delivery's clear and append dropped | `157-retbuf-adopt.loft`, `tests/retbuf_adopt.rs` |
| `(R-PushFill)` | fill form | rustc only (IR facts) | `if n > 0 { OpPush(v, c); OpAppendCopy(v, n, tp) }`, as `[c; n]` already lowers; brings the clause to the C too | `157-push-fill.loft`, `a-repeated-literal-is-one-fill-of-its-copies.loft` |
| `(R-PushFill)` | hash | `hoist::hash_fill_loop` (IR) | `OpReserveHash(h, OpLengthHash(h) + per_pass · trip, tp)`; only if the probe gains | `a-counted-hash-fill-reserves-its-table.loft` |

**Can move with one new piece** — a runtime flag, a stdlib function, or an IR twin:

| rule | the new piece | its kind |
|---|---|---|
| `(R-RefillBuffer)` | a REFILL bit on `OpDatabase`'s type operand, read by the shared `Stores::take_spare` (precedent: `POOL_RESET` on `OpClear`) | flag |
| `(R-KeyedEntry)` | find-or-insert answering a reference, a native stdlib function through one `OpStaticCall` | stdlib function |
| `(R-FoldCompare)` | `fold_compare(op, fold, x, y)`: an ASCII fast path falling back to the written form | stdlib function |
| `(R-HandOver)` | rotate clause: `Stores::rotate_into` exposed as a stdlib function, and the `__nk` twin | stdlib function + twin |
| `(R-AppendTwin)` | base clause: the `__ap` twin (the buffer's reset dropped, its mint guarded) and the caller's `f__ap(…, X)` | twin |
| `(R-Destination)` | the `__d` twin, and a release of what the twin placed into an unfinished element on the failing path (a stdlib function) | twin + stdlib function |
| `(R-RefillText)` | its text clauses need the twin; its vector clause moves with `(R-RefillBuffer)`.  Re-measure first against `(R-Reuse)`'s pool reset, which may already take its cost: if the `LOFT_NO_REFILL_TEXT` A/B shows little, DROP | twin, or drop |

**An interpreter clause** — the interpreter's generator or runtime applies the same fact:

| rule | the clause |
|---|---|
| `(R-TextBorrow)` | the store-read, discharge, local-root, format, call and slice clauses: `mark_text_views` (`state/codegen.rs`) extended from the walk clause to `store_text_borrow`'s borrows minus the tested clause.  Risk: a view outliving its source on an early exit |
| `(R-FlatCopy)` | the codegen runtime's early return at the top of `State::do_copy_record`, after the null and alias tests |
| `(R-CompleteWrite)` | an `OpNewRecord` whose fields the following statements all write skips `set_default_value`: a bit on its constant operand (precedent `OpCopyRecord`'s `0x8000`), set from `opt::ret_adopt::complete_writes` — the prefill `(R-PushRec)` and `(R-FieldAppend)` leave behind |
| `(R-FieldAppend)` | exists: `plain_field_vector` in `Stores::record_new` / `record_finish`; the register lists both phases |
| `(R-Apart)` | instance 4: `(R-Reuse)`'s reset clause widened from hidden buffers to the owner local, priced on dryopea `truncate_to` |
| `(R-FnRefBuffer)` | a candidate only: the binding's store as an operand of `OpCallRef`; first if B1 shows mint and free time in `flow_layout_full` |

**Stays in its writer**, each with the reason its entry will state:

| rule | why |
|---|---|
| `(R-Push)`, `(R-PushRec)` header, `(R-PushFill)` window / run / record, `(R-AppendTwin)` window / loop | a vector's length and base held in C locals: representation.  The growth they save was measured not worth moving on the interpreter (`INTERPRETER_PERFORMANCE.md`) |
| `(R-Header)` grown / kept clauses | a header held across reads: representation; the interpreter's `OpGetVector` derives it per read, and holding one is a new operator |
| `(R-Mint)` | classifies mint groups for the representation rules; its one piece of work is already in the interpreter's runtime |
| `(R-WorkPark)` | a per-thread runtime park; an op variant would buy little (one realloc chain per call) |
| `(R-SliceArg)` carrier | the interpreter's slot reuse with an in-place `init()` is already its analogue |
| `(R-Transient)` emission, `(R-SliceInto)` transient clause | arena structs and views: representation.  Its site PLAN moves to `opt/transient.rs` (as `(R-ValueRecord)` splits plan and emission) so the census credits the decision; its interpreter gains go to arc D as widenings of `(R-Place)` / `(R-Destination)` (a vector built in its destination's store, a view return) and `(R-DecodeView)` (a dead tree's shape twin) |
| `(R-Apart)` instances 2, 3 | a dynamically indexed value outside the store; the interpreter's tuple is read at constant indices only |
| `(R-Fill)` | needs a fill kernel; spans under eight lost on `fill_triangle`.  The owner decides whether a kernel is wanted |

**The rustc-only rules**, which are lost when that writer retires unless ported:

| rule | verdict |
|---|---|
| `(R-Wrapper)`, `(R-RefillBuffer)`, `(R-LoopBuffer)`, `(R-FoldCompare)`, `(R-PushFill)` fill form | MOVE (above) |
| `(R-LazySplit)`, `(R-TypedKeyed)`, `(R-SliceBorrow)`, `(R-TypedRead)`, `(R-GroupPush)` work half, `(R-Leaf)`, `(R-LeafChain)`, `(R-LitDiv)` | already on the C (under no census name, inherent in its emission, or folded by its inlining); A2 gives each its census line.  `(R-LitDiv)` gets one emit-and-look to confirm |
| `(R-TextClear)` | DROP: both other backends already clear in place (`loft_text_set`, `OpClearText`) |
| `(R-RangedCall)` | PORT to the C, after D-fact-2 puts the interval `entry` facts in the routine |
| `(R-FnHeader)` | PORT: a C prologue on `fn_header_params`; hand-priced on drawing first (the C's per-loop headers take much of it) |
| `(R-GuardFree)` | PORT: `registers_buffers` / `is_guard_free` into `opt/frame_calls.rs`; the C's wrappers skip the fn-ref mark where the closure is free |
| `(R-FnRefValue)` | PORT; first step: the C asks for the value-record plan with fn-ref groups CLOSED.  Today `Cx::value_records` (`src/c/mod.rs`) drops the WHOLE plan when any group would answer a tuple, so zttext loses every `(R-ValueRecord)` admission on the C — a loss to fix in A2, not a port |
| `(R-StreamWalk)` | PORT (the plan in `lower/` is shared; no IR form without a capture mechanism) |
| `(R-RebindBuffer)` | PORT after `(R-RefillBuffer)` reaches the C |
| `(R-FaultFlag)` | PORT only if `06_newton_sqrt`'s C ratio shows the gap |

## Arc C — moving them

### The move protocol

One phase per rule (or per clause), in B1's weight order within the waves below.

1. **The IR clause, beside.**  A pass in the phase the form needs: per body in
   `scopes::check` when it rewrites one body; in `compile::close_program` when it makes a
   twin or changes a signature, after the passes whose output it reads (`(R-LoopRecord)` and
   `(R-ElemFirst)` before `ctor_literal`, which reads `loop_records`).  It enters
   `InBody::enter("ir", fn)`, calls `fired` at each admission and `declined` at each refusal,
   and has its own switch (`LOFT_NO_IR_<RULE>`) and trace (`LOFT_TRACE_<RULE>`).  The writer
   clause stays untouched.
2. **Compared.**  With both on, the writer clause admits nothing the IR clause took (its
   census row falls to zero, or to the sites the IR clause declines).  The C emission of every
   bench with the IR clause ON and the writer clause OFF is byte-identical to the writer
   clause alone, or differs only where the census says the IR clause admitted MORE — each
   such site read by hand.  On the interpreter: the rule's cells on both backends, the switch
   A/B against hand-computed values (`(R-Switch)`'s BOTH-BACKEND CLAUSE: no interpreter
   oracle is left, so the A/B and the cells are the oracle), `LOFT_STRICT_STORES=1`,
   `LOFT_POISON=1` and the leak check, the C sweep and the language suite on both backends,
   and the probe routine's instructions per op on the pinned builds.
3. **Then the writer clause goes.**  Its code is deleted, its switch becomes the IR clause's,
   A1's table is blessed with the new applied-by, and the census shows no drop on either
   writer.

A writer clause whose decline protected WRITER state — `(R-ElemFirst)`'s "the writer holds a
header on `out`", `(R-Destination)`'s transient twins — does not move that decline into the
IR: the IR has no such state.  Instead the writer must accept the rewritten IR: step 2's
byte-identical comparison is what shows it does.

### The waves

- **Wave 1 — existing operators, no new mechanism:** `(R-SliceArg)` range, `(R-SliceInto)`
  main, `(R-Wrapper)`, `(R-LoopRecord)`, `(R-LoopBuffer)`, `(R-ElemFirst)`, `(R-RetAdopt)`,
  `(R-PushFill)` fill; and the interpreter clauses `(R-FlatCopy)`, `(R-CompleteWrite)`'s
  prefill bit, `(R-TextBorrow)`'s store clauses.  `(R-SliceArg)` range goes first: it is the
  smallest (one call rewritten, `text_run` writes the same form) and proves the protocol.
- **Wave 2 — one stdlib function or flag each:** `(R-RefillBuffer)`, `(R-KeyedEntry)`,
  `(R-FoldCompare)`, `(R-PushFill)` hash (if its probe gains).  Each stdlib function gets its
  row in `KERNELS.md` with its removal trigger.
- **Wave 3 — IR twins, after D-fact-3's registry** ([§ D-fact-3](#d-fact-3--one-twin-registry)): `(R-AppendTwin)` base, `(R-HandOver)`
  rotate, `(R-Destination)`, `(R-RefillText)` (or its drop).
- **The ports** — a separate track whose goal is the rustc writer's retirement, not the
  interpreter: `(R-FnRefValue)`'s closed-groups plan first (it is a loss on the C today), then
  `(R-GuardFree)`, `(R-FnHeader)`, `(R-RangedCall)` after D-fact-2, `(R-StreamWalk)`,
  `(R-RebindBuffer)`, `(R-FaultFlag)` if measured.

## The facts deviations, removed (D-fact-1 … D-fact-3, #2351)

`formal/facts.md` carries three open deviations; this plan needs all three gone, because a
moved rule must read its callee facts from the routine (`(Fact-One)`) and wave 3's moves need
twins made in the IR (`(Fact-Twin)`).  Order: **D-fact-1, then D-fact-3, then D-fact-2.**

| moves it unblocks | needs |
|---|---|
| `(R-Destination)` `__d`, `(R-HandOver)` `__nk`, `(R-RefillText)`'s text clause, `(R-AppendTwin)` `__ap` | D-fact-3 (IR twins) |
| `(R-StreamWalk)` port, view returns (`__vr`) | D-fact-1 + D-fact-3 |
| `(R-RangedCall)` port | D-fact-2 (on D-fact-1's handle and D-fact-3's key) |

### D-fact-1 — one long-lived handle

**Today.**  `Facts<'m>` (`src/opt/facts.rs`) is built per QUESTION (`Facts::fresh`, `over`,
`with`) over memos the writers keep at three different scopes: per program (native's
`Output.hoist_cache` / `scalar_write_cache` / `input_cache`, the C's `Cx.hoist_memo`), per
function (the C's `FnWriter.hoist_cache`), and fresh per question (every `Facts::over` starts
an empty `WriteCache`).  The scope matters: `StoresWritten` and `InPlaceOnly` memoise by
function while their answer depends on the asking callee, and `Writes` keeps the answer it gave
on a cycle, which depends on the order calls are entered — so merging the scopes CAN change
admissions.  The three analyses outside the routine are context-free and follow no call:

| analysis | answers | kind on the handle |
|---|---|---|
| `lower::view_return::callee` (native's `vr_memo`) | the body writes only stores it minted, calls no user function, ends in view exits | bool, MUST, UP, key the function, reach `own`, memo always |
| `opt::ret_adopt::callee_reinits_buffer` | the callee runs `OpDatabase` on its own return buffer (read by `(R-MoveAppend)`, not `(R-RetAdopt)`) | bool, MAY, UP, `own` |
| `lower::stream_walk::written_params` | the return buffer, hidden buffers and `&` parameters — the signature | a parameter set, MAY, UP, `own` |

**The removal, three steps.**
1. The handle OWNS its memos (`Facts<'static>`; `ask` already takes `&Data` per call): native
   keeps `Output.facts`, the C `Cx.facts: RefCell<Facts>`.  The writers' scopes become NAMED
   memo slots with explicit resets — `enter_fn()` clears the per-function slot, `fresh(|fx|
   …)` stands for what `over` does today — so the emission is byte-identical by construction.
2. The three analyses become kinds of reach `own` on it; `vr_memo` goes.  Byte-identical.
3. Separately, census-measured: the scopes collapse, which is the fix `facts-engine.md` §
   Found 1–2 names.  This step may move admissions and is judged by the census, rule by rule.

*Check:* the facts-engine gate — `scripts/introspect_diff.sh` on bytecode and Rust, `--c` with
and without `LOFT_RELEASE_PASS_PROBE=1`, the library benches (`--extra`), `make
rewrite-census` (now on both writers, A0).  Effort: steps 1–2 S, step 3 M.

### D-fact-3 — one twin registry

**Today.**  Three keying schemes:
- the C: one `u64` mask per twin (`src/c/mod.rs`) — the place mask in the low bits, EIGHT named
  bits (`INV`, `NK`, `APR`, `APV`, `KH`, `AP`, `DS`, `VF`; `facts.md` says seven), `NN_MASK`
  in bits 32–55; `twin_name` spells the suffixes in a fixed order.  Outside the mask: the
  transient `__in`, `__wr`, `__vp`, `__fr` twins;
- native: name suffixes in a different order (`__inv __rg [__rt|__ap|__d|__vr|__apv|__apr|__nk|
  __kh] __ps __sw<k>`), six request queues, and bit keys of its own OR'd into a definition
  number (`DV_BIT`, `src/lower/decode_view.rs`; `KH_BIT`, `src/generation/kept.rs`) that
  `facts.md` does not list;
- the IR: `value_record::add_twin` (`<name>_tuple`), `shared_call` (`<f>__given_<g>`, through
  `Data::add_variant_fn` and `variant_origin`), store-text instances (`<key>@st<mask>`).

**The registry.**
- `TwinKey { of: u32, assume: Assumptions }`, `Assumptions` a sorted set of `Place(p)`,
  `Inputs`, `Kept`, `NonSentinel(p)`, `Interval(p, bound)`, `AppendBuf`, `Destination(off)`,
  `NoKeep`, `Apart`, `ApartValue`, `ViewReturn`, `Refill`, `DecodeView`, `Positional(k)`,
  `StreamWalk(k)`, `Fast`.
- `Twins { entries: Vec<(TwinKey, TwinBody)>, index: HashMap<TwinKey, usize> }` — a `Vec` so
  the emission order is reproduced.
- `TwinBody::Writer` (the same IR, the assumptions seeding the writer's facts: `__rg`, `__nn`,
  `__inv`, `__kh`, `__vf`, places) or `TwinBody::Ir(def_nr)` (a definition whose IR body
  differs: the existing `_tuple`, `@st`, `__given_`, and the moved rules' `__d`, `__nk`,
  `__ap`, refill-text).  **A writer twin never becomes a `Data` definition**: the interpreter
  compiles every definition and would pay for identical bytecode.
- IR passes in `close_program` register `Ir` twins; writers register `Writer` twins as
  `request_twin` does now.  The gain test stays per kind; there is no budget today, and adding
  one is its own measured change.
- Spelling: `spell(TwinKey)` per writer, reproducing each writer's suffix order exactly.  IR
  twin names use the `@` convention, which `trace_name` / `source_name_of_key` (`data.rs`)
  already decode for traces and profiles — after `is_store_text_instance` is narrowed from any
  `@` to `@st`.  `variant_origin` folds into the registry: one origin map.
- An `Ir` twin's admissions are credited to the twin's name under `ir`; A1's table maps them
  back to the origin function.  Open-world (live) builds make no `Ir` twins, as `emit_live`
  takes no plan twins today.

**Steps.**  (1) the registry type and the C re-keyed (mask → `TwinKey`, `twin_name` → `spell`);
(2) native re-keyed (the queues, `DV_BIT` / `KH_BIT`, the suffix chain); (3) the existing IR
twins registered, names unchanged; (4) `Ir` materialisation for wave 3.  *Check:* steps 1–3
byte-identical emission on both writers and the interpreter's bytecode (the facts-engine
gate); step 4 is each wave-3 move's own protocol.  Effort L overall, S–M per step.

### D-fact-2 — interval entry facts in the routine

**Today.**  `src/generation/ranged_call.rs`: a fixed box seed (every integer parameter in
[−2³⁰, 2³⁰], no widening), a gain test (`checked_ops`, ≥ 3 checked operators up to three calls
deep; `variant_gains` comparing `op_range` under the seed and without), the call routing
(`ranged_call`, `args_ranged`), and a guarded copy of a counted loop behind `|x| <= 2²⁰`.  The
seeding and the gain test are IR facts over the already-shared `opt::range`, `opt::non_sentinel`
and `callee_inputs`; the guarded copy and the request queues are writer state.  So the `entry`
interval kind is a seed and a gain test over `opt::range` — no new domain.

**The interpreter gains nothing** (it has no plain integer operator; the checked ops stay
checked), so the guarded copy never becomes an IR rewrite: it would cost the interpreter a
duplicated block and a test.

**The falsifier is missing on the C.**  Native checks a plain range operator under
`LOFT_HOIST_VERIFY=1` (`ops::range_verify`); the C's `range_plain` emits a raw `((a) + (b))`
that `-DLOFT_VERIFY` does not cover, and on C a wrong interval is signed-overflow UB, not a
wrap.  `(Fact-Oracle)`'s `LOFT_FACT_VERIFY` does not exist in the code at all.

**Steps.**  (1) The seed and the gain test move to `opt/` as the `entry` interval kind, its memo
on D-fact-1's handle — native byte-identical; (2) a `LOFT_VERIFY` checked form of every plain
range operator in the C (the falsifier first); (3) `Interval` as a `TwinKey` assumption with
the C's routing and seeding (`nn_facts` already passes a seed, `Ranges::default()` today);
(4) the C guarded copy.  The C census RISES (native 384 admissions, the C 0): a port, every new
admission read by hand.  Effort M, toward L with the guarded copy.

**`(Fact-Oracle)` itself** — the interpreter checking a MUST `entry` fact on entry and a MAY
`writes` summary as a superset of what the call did — is built with step 2, under
`LOFT_FACT_VERIFY=1`, so a widened reach has a run-time falsifier on the interpreter as the C's
verify form gives one on the C.

## Arc D — widening the rules we have

As the README states; the design adds where the classes come from.  D1 runs `stats.py` on both
compilers over the library mains, takes every routine over 2× on either, and for each reads
the decline census and the hot site's emission to name the rule that refuses it and the reason.
The seeds the verdicts above add:

| rule | the widening | routines it would reach |
|---|---|---|
| `(R-Place)` / `(R-Destination)` | a vector a decoder builds and moves into a field, built in the destination's store (no cross-store `vector_add`) | cbor `decode`, pluginabi's three rows, every recursive builder |
| `(R-DecodeView)` | its structure clause (a shape twin of a decode whose tree is dead) emitted in the IR phase, so the interpreter skips the tree too | `check_request` |
| view return | a read-only reader (`pa_get`) answering a view of a field instead of `OpCopyRecord` of the subtree, its source proven unwritten (`opt::body_writes`) | `check_request`, `request` |
| `(R-Version)` | a leaf that calls a leaf; element reads of `float`, `u8`, `boolean` | hex_roof `roof_match` |
| `(R-Transient)` | a producer that pushes into its own vector field; roots whose lives overlap | the producers its trace declines |

## Side findings

Recorded here so none is lost; each is fixed in the phase named.

- **The C ship tier has no call-depth cap.**  `loft_frame_push`'s depth test compiles to
  nothing without `LOFT_CALL_CHAIN` (`src/c/loft_rt.h`), so a ship binary recursed 20 000 deep
  where the interpreter and the C check tier halt at 10 000 frames (probed 2026-11-03).  That
  is the production behaviour `@PLN177` aims for, but `CAVEATS.md` § Recursion depth says
  every build halts, and a recursion past the machine stack crashes instead of reporting.
  The caveat is corrected, or the ship tier keeps a cap — the owner's call — in A2.
- **The C drops the whole value-record plan** when any fn-ref group would answer a tuple
  (`Cx::value_records`, `src/c/mod.rs`): `(R-FnRefValue)`'s first step, in A2.
- **A guard named but missing:** `(R-Header)`'s grown-header text cites
  `a-vector-the-loop-grows-is-read-through-the-header-derived-after-each-growth.loft`, which
  `tests/scripts/` does not have; A2 finds it or writes it.

## Validation, per arc

| arc | each step can go red alone because |
|---|---|
| A | each tool has a planted defect it must catch (a removed `fired` call, a phase credited wrong, a drifted index) |
| B | a probe that shows no gain overturns its verdict |
| C | a byte difference in the compiled emission the census does not explain, a guard cell, a switch A/B difference, a leak or a strict-stores report, a falsified plant |
| D | a clause that moves only the routine it was written against is reworked or dropped |

## Effort

A: S each (A0–A3).  B: S (B1) + M (the probes).  C: wave 1 S per rule, wave 2 S–M per rule,
wave 3 M per rule after the registry, the ports S–M each.  D: S (D1) + M per clause.
