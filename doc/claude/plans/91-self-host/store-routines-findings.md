<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The IR store routines — findings, and the use-after-free cases

A running record of what a review of the store-lifetime work found, and of the
use-after-free (UAF) cases around it.  The design is [STORE_RECORDS.md](../../STORE_RECORDS.md);
the re-free arc's state is [handoff-2026-11-11.md](handoff-2026-11-11.md).  Each finding says
whether it was **verified** (read in the code, or measured) or **inferred** (not yet probed),
and who owns the fix.

## Who works on what

| Session | Checkout | Owns |
|---|---|---|
| loft-c9 | `../loft`, branch `91-self-host` | the re-free cells; `src/store_records.rs`; `src/scopes/{get_free_vars,scan_set,tuple_members,capture_builds}.rs`; the parser sites of the cures |
| loft3-c1 | `../loft3` | the closure re-frees (fn-ref and captured-record stand-downs) |
| loft2 | `../loft2`, branch `tuxedo-pln204-int32` | the use-after-free cases; `src/bind_lowering.rs` |

loft2 sends changes to files it does not own as patches against `91-self-host`'s head
(`git am -3`), kept small so they rebase over the records' churn.

## How the UAF cases are measured

The language suite under `LOFT_STRICT_STORES=1` on the interpreter (a freed store is never
reused, so any access to it is reported), twice:

```bash
# per run: a frozen copy of target/release/loft, ulimit -v 8000000, LOFT_NO_CACHE=1
LOFT_STRICT_STORES=1 loft-suite run --loft <bin> --mode interpret --jobs 8 -v
LOFT_STRICT_STORES=1 LOFT_NO_FREE_RESET=1 loft-suite run --loft <bin> --mode interpret --jobs 8 -v
```

Every backend now resets a variable to null after its free (`Codegen::generate_call`,
`OpInitRefSentinel`).  A stale read through that variable then reads null instead of the
freed store, so strict stores no longer sees it.  A cell that reports a UAF only with
`LOFT_NO_FREE_RESET=1` is a UAF the reset hides; on the C and native backends it was already
a silent null read.

## Use-after-free cases

Measured 2026-11-11 on `tuxedo-pln204-int32` (suite `8778c1e`), the suite under
`LOFT_STRICT_STORES=1`.  The first interpreter sweep read 1442 passed, 4 failed, but it was
partly blind: `loft --tests` counted strict-store violations and never failed a test on them
(the direct run's exit did).  `test_runner.rs` now fails a test that adds one.  With that, and
with the C backend swept too, the suite held **two** use-after-free cases; both are fixed, and
the suite now reads 0 under strict stores on the interpreter and C.

| Case | Cause | State |
|---|---|---|
| `R-HostStore/a-recursion-answering-its-own-results-keeps-adopts-and-releases-them.loft` (interpreter) | the adopted result's let-go freed the host store | fixed; cell armed with strict stores |
| `R-Demote/a-front-push-moves-the-old-node-and-keeps-what-it-owns.loft` (interpreter, C, native) | a link walk over a plain parameter released the caller's list | fixed; cell armed; new cell `F-ParamRebind/a-link-to-a-plain-parameter-walks-…` |
| `O-NoDiverge/1328-…`, `O-NoDiverge/1329-…`, `O-Proxy/1320-…` | not a UAF: each cell lowers the store ceiling (`@STORE_LIMIT: 1310`), and strict stores never reuses a store number, so the run exhausts the table; run without the ceiling they are clean on both backends | not a defect |

Run a cell alone the way the runner does: `LOFT_LEAK_CHECK=1`, the cell's `@STORE_LIMIT` as
`LOFT_STORE_LIMIT`, `loft --tests`, from the suite root.

**R-HostStore: the adopted local's let-go freed the host store.**  `simplify` builds the `Add`
node in the store of its first child's result and adopts both children by number:

```text
__ref_p2_1 = OpStoreBuf(a);                     // the node goes into a's store
OpDatabase(__ref_p2_1, Add);
OpAdoptChildRec(OpGetField(__ref_p2_1, l), a);  // the node takes a's record
a = null;                                       // a lets go of it
OpAdoptChildRec(OpGetField(__ref_p2_1, r), _result_20);   // reads the node: freed
```

`(R-HostStore)`'s result clause says the local LETS GO (null): one owner per record, the store
living on for the answer.  `re_bind` lowered `a = null` as a user variable's reassignment, with
a displaced free.  Fix: `host_result` writes the let-go as a detach (`OpNullRefSentinel()`), and
`data::is_null_sentinel_detach` admits a local whose one bind adopts its callee's record
(`deferred_first_bind`); the predicate is shared, so no backend frees there.  Behind it the
strict child-record check refused record 1 as a child (`is_claimed_record` excludes the store's
first record); a host-store tree's first leaf IS record 1, so the check now admits it when
claimed.  The bytecode differs from before only by the two let-go frees.

**R-Demote: a link walk over a plain parameter released the caller's list.**  `walk(n: N?)`
reads `cur = &n; while … { cur = &cur.next }`.  Each step writes `n = n.next` through the link
(`OpMoveSlotRef`), and the write-back released the displaced store where the new value is not
in it: on the last step, `null`, the whole store, which is `main`'s list.  `(F-ParamRebind)`:
replacing a plain parameter's value only rebinds the callee's name.  Fix: one predicate,
`Function::link_write_releases`, false for a link that reaches a plain parameter, gates the
move form and the displaced release in all three writers (interpreter, rustc, C).

**The records and the checker (the three-place rule).**  Each cure has a switch that restores
its defect — `LOFT_HOST_LETGO_FREES=1`, `LOFT_LINK_PARAM_RELEASES=1` — and
`scripts/own_sweep.py --oracle check-records` reads CAUGHT with it on and CLEAN (R-Demote, the
F-ParamRebind cell) or the cell's known STATIC (R-HostStore) with it off.  The host-store case
needed no records change: `OpStoreBuf(a)` already answers `a`'s store, so the restored free is a
use after end of the node.  The link case needed one: the records now follow a `&` link
installed on one variable as a write to that variable, with the code generator's release.
`own_sweep.py` now counts the strict child-record check as a use after free (under
`LOFT_POISON` it fires first, and R-Demote's cure-off run read CLEAN).  Over the suite: STATIC
366 → 362, no cell worse.  Landed on `91-self-host` as a patch to loft-c9.

**Left open: a whole new record written through a link to a plain parameter leaks.**
`cur = &n; cur = N { … }` puts a fresh store in `n` that nothing frees; the direct spelling
`n = N { … }` is clean because the parser gives a rebound parameter an entry witness
(`ensure_rebind_witness`) and frees it at exit when distinct.  Before the fix above this shape
freed the caller's store first and leaked the new one too.  The cure is to treat a whole write
through such a link as a rebind of its target, so the witness applies.

## Can a store's paths be followed today?

Not easily.  Finding the R-HostStore path took five instruments and hand arithmetic, and the
one instrument built for the question was wrong on this cell:

| Step | Instrument | What it gave | What it lacked |
|---|---|---|---|
| 1 | `LOFT_STRICT_STORES=1` | the access to a freed store, with pcs | the source line, the variable |
| 2 | `LOFT_LOG=store_life:3` | the store's allocation and free, op and function | the source line; the later access |
| 3 | `loft introspect` | the bytecode at those pcs, after subtracting the function's start by hand | the IR line of each op |
| 4 | the IR in the same dump | `a = null` after the adoption | why it frees |
| 5 | `LOFT_TRACE_BIND=1` | the lowering `re_bind` chose (`pre`) | which fact decided it |
| — | `LOFT_STORE_TABLE=simplify` | 8 leaks the runtime never sees | the use after free the runtime finds |

`LOFT_STORE_TABLE` is the designed answer (a store's origin, holders, uses and ends per path),
but it does not model `OpStoreBuf` and `OpAdoptChildRec` (the handoff's "records gaps"), so a
host-store function reads as leaking and the free inside it as harmless.  Its store numbers are
per-function abstractions, not the runtime's, so a runtime report cannot be looked up in it.
What would make the paths easy to follow, in order of leverage:

1. **Built** (`src/store_trace.rs`, [DEBUG_STORES.md](../../DEBUG_STORES.md) § Debugging
   store-ownership bugs): `LOFT_STRICT_STORES` and `LOFT_REFREE_FAILS` name the function, file,
   line (column when a span covers the pc) and the freed variable; `LOFT_LOG=store_life:<N>`
   adds the jumps each frame took from the store's allocation to each free, and reports a frame
   that returns with the store live.  On R-HostStore the strict report now reads
   `freed `simplify` … line 36 by `a`` / `read … line 36`, and the store-life path shows
   lines 34 and 35's conditions false before the free.
2. The records keyed to the same store identity the runtime reports, so a strict-stores fault
   can be looked up in `LOFT_STORE_TABLE`.
3. The records modelling host stores (`OpStoreBuf`, `OpAdoptChildRec`), so this cell reads
   as a use after end instead of 8 leaks.

## The store tooling, judged from use

Measured on this arc's cases (2026-11-11): two use-after-frees found, traced and fixed, and the
checker extended to report them.

| Instrument | Insight | Ease of use | What cost time |
|---|---|---|---|
| `LOFT_STRICT_STORES` | high: found both cases; exact, no false positive | good since `store_trace` (function, line, freed variable) | `loft --tests` never failed on it (fixed); a cell with a lowered `@STORE_LIMIT` exhausts the no-reuse table (three false failures); under `LOFT_POISON` the child-record panic fires first and ends the run |
| `LOFT_LOG=store_life:<N>` | high: the path to each free, the frame that leaves a store live | fair: needs the number from a strict report; `#N` is a SLOT that holds several stores in turn | runs the logged loop only |
| `LOFT_TRACE_BIND` | medium: names the lowering each assignment took | good | the `b2=` column recomputes the free and can disagree with it |
| store records (`LOFT_OWN_ORACLE=check-records`, `LOFT_STORE_TABLE`) | high when it fires on the defect (both cases CAUGHT with the cure off) | poor: 362 STATIC cells over the suite, 20 false reds beside the 2 real ones on R-HostStore; its store numbers are per-function, not the runtime's | a CLEAN cell may be an unchecked (imprecise) function |
| `scripts/own_sweep.py` | high: MISS / CAUGHT / STATIC is the right question | fair | faults recognised by output regex (missed the child-record panic: a hidden MISS); before/after needs a hand-written per-cell diff |
| `loft introspect` | essential for the bytecode and IR | heavy (8,800 lines for one cell) | pc → line by hand, before `store_trace` |

The highest-leverage change: **a fault reported as data, not text** — one machine-readable line
per store fault (kind, store, sites, variable) that `own_sweep`, the suite runner and the
tests read, instead of each matching its own regex against a human message.  Two of this
arc's blind spots (`--tests` ignoring strict faults, `own_sweep` missing the child-record
panic) were the same defect: a fault channel whose consumers parse prose.  After it: cut the
records' STATIC noise (the host-store gap is 20 of it) so a CAUGHT stands out, and give
`own_sweep` a `--compare before.tsv` that prints the per-cell verdict changes.

## Findings: `bind_lowering` (phase 2 of STORE_RECORDS.md)

Phase 2 is not met yet.  The lowering is one function for the interpreter and the records, but
it is not carried on the IR, and the native and C writers decide it in their own code.

1. **The records model the wrong store for a call-copy reassignment** — verified, code read
   (`store_records.rs:966` against `state/codegen.rs:2885–3052`).  The code generator has three
   outcomes for `v = f(…)` with a copy: the old store freed before the call and a fresh one made
   (`!alloc_after`); a fresh store after the call (`fresh_path`); `v`'s own store re-initialised
   in place after the call (every other case).  In place is exactly
   `stash && !(witnessed && rhs_reads_v) && !rhs_may_alias_v`.  The records answer in place for
   `stash || (witnessed && rhs_reads_v)`, so they keep the old store in the two populations the
   generator sends to a fresh one.  The stashed free and every later use-after-end on that store
   are then judged against the wrong store.  Fix: one `ReBind` method both callers read (loft2,
   patch to loft-c9).
2. **Native and C still decide the lowering themselves** — verified.  The displaced free fires
   for any right-hand side in `re_bind` (`bind_lowering.rs:683–697`) but only for a call,
   `CallRef`, `Insert`, block or `if` in native (`generation/dispatch.rs:514–525`) and C
   (`c/body.rs:6336–6345`); native's call-copy test adds `may_hand_back_a_caller_store`
   (`dispatch.rs:1425`), which the interpreter and C do not.
3. **The move decision is written twice** — verified.  F3's move (`codegen.rs:3659–3669`) and
   its copy in the records (`store_records.rs:874–880`); the move also sets `skip_free`
   during generation, after the records have run.
4. **Trace names do not map onto the enums** — verified.  21 `FirstArm` and 7 `ReArm` variants;
   `walk-step`, `loop-slot`, `zero-size` and the F3 move/copy split are decided outside the
   module.  The `b2=` trace line (`codegen.rs:2615`) recomputes the free from the facts and
   prints `pre` where `rb.free` is `None`.
5. **Switches inside the decision** — verified.  `join_own_enabled`,
   `displaced_witness_enabled`, `reassign_copy_enabled`, `temp_null_reset_frees`: cached once per
   process, but native and C read only some of them, so a switch can split the backends.

## Findings: `store_records.rs` (owner: loft-c9)

1. **The records never compare the generator's choice with their own summary** — verified.
   `FirstArm::CallAdopt` binds the value even when the callee's records answer `Join(i)`
   (`store_records.rs:925`); the doc's promise "where the two disagree, the checker says so"
   is not kept.  `VectorNull` makes a store only when `deps` is empty (`:902`), so a local with
   wrong deps cannot be reported as leaking.
2. **`Summary::join` depends on order** — verified (`:219–230`).  `Join(i) ⊔ Arg(j)` answers
   `Join(i)` for any `j`: {Fresh, Arg0, Arg1} joins to `Join(0)` in one order and `Unknown` in
   another.  Fix: `Join(i) ⊔ Arg(j≠i) = Unknown`.
3. **A clean report can hide unchecked functions** — verified.  An imprecise function (a
   `Yield`/`Iter`, a step or loop budget hit) prints no findings (`:1822`), its callers read
   `Unknown`, and the summary counts only red findings.  A gate on it needs an "unchecked: N"
   count.  The call-graph fixpoint stops at 8 rounds without saying so (`:1736`).
4. **Constructs read as null or skipped** — verified.  The default arm (`:799`) evaluates
   nothing inside `Parallel`, `Keys`, `RawExpr`; `free_op` ignores a free of an expression;
   `:1474` assumes every reference-answering op answers into its first argument's store, wrong
   for `OpCastVectorFromText`, `OpCopyClosure`, `OpGetDbRef`, `OpStoreBuf`.
5. **A tuple member that was null is lost** — verified (`:456`, `:775`).  `bind` drops a
   variable bound to null, and `TuplePut`/`TupleGet` need the member present, so
   `t = (null, 0); t.0 = mk()` hides a leak and a later double free.
6. **The conditional free's mixed case ends the witness's store too** — verified
   (`:1330–1338`); the witness's own free then reads as a re-free.
7. **Liveness tests may count as uses** — inferred.  `OpStoreLive`, `OpDistinctStore`,
   `OpRefIsNull`, `OpConvBoolFromRef` go through `use_var` (`:1220`); a guard asking "was this
   freed?" would report a use after end.  To probe from the UAF side.

## Findings: the re-free cure series

1. **Each cure patches one producer of the same invariant** — verified.  Six parser and scope
   sites each give a view a dep on what it views (`collect_iterator_subject`,
   `materialise_slice_leaves`, `emit_nullable_slot_write`, `unboxed_member_types`, the unbox
   bind in `scan_set`, `lift_join_arm_tails`).  `Scopes::scan_set` already asks the oracle
   (`use_analysis::ownership_of`) about every bind and gets `Borrowed { base }` for a
   projection, but writes no dep from it.  Writing the dep there is the chokepoint
   (`@FR-B-View`, `@FR-O-Complete`).
2. **The scope-end stand-downs choose between two owners at free time** — verified
   (`get_free_vars.rs:1264–1489`), by IR shape and name prefix, within one sweep only.
3. **Few cures have a guard in the default gate** — verified.  Only `tests/redundant_free.rs`
   fails when its cure is reverted; the rest are seen only under `LOFT_REFREE_FAILS=1`.
4. **The sweep recipe misses the new switches** — verified.  STORE_RECORDS.md § Measuring
   collects `LOFT_NO_*` names from BOTH_BACKEND_SWITCHES.md; the eleven new switches are
   documented only in STORE_RECORDS.md, and three do not start with `LOFT_NO_`.
5. **The null reset after a free hides re-frees from the instruments** — verified (198 → 148
   cells under `LOFT_REFREE_FAILS=1`).  It hides no UAF the suite reaches: the strict-stores
   sweep fails the same four cells with `LOFT_NO_FREE_RESET=1` as without it.
