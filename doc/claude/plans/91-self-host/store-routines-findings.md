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

Measured 2026-11-11 on `tuxedo-pln204-int32` @ `15a2291fb` (suite `8778c1e`): **1442 passed,
4 failed, with the reset on and with it off — the same four cells.**  The reset hides no UAF the
suite reaches.

| Case | Cause | State |
|---|---|---|
| `R-HostStore/a-recursion-answering-its-own-results-keeps-adopts-and-releases-them.loft` | see below | cause found |
| `O-NoDiverge/1328-a-fn-ref-rebind-frees-the-store-it-displaces.loft` (3 of 6 tests) | not yet known: passes when run alone | open |
| `O-NoDiverge/1329-a-fn-ref-vector-rebind-frees-the-store-it-displaces.loft` (2 of 8) | not yet known: passes when run alone | open |
| `O-Proxy/1320-a-branch-joined-binding-frees-the-arm-that-minted.loft` (1 of 11) | not yet known: passes when run alone | open |

**R-HostStore: the adopted local's null reset frees the host store.**  `simplify` builds the
`Add` node in the store of its first child's result and adopts both children by number:

```text
__ref_p2_1 = OpStoreBuf(a);                     // the node goes into a's store
OpDatabase(__ref_p2_1, Add);
OpAdoptChildRec(OpGetField(__ref_p2_1, l), a);  // the node takes a's record
a = null;                                       // a no longer holds it
OpAdoptChildRec(OpGetField(__ref_p2_1, r), _result_20);   // reads the node: freed
```

`a = null` is a user variable's reassignment, so `bind_lowering::re_bind` gives it a displaced
free (`LOFT_TRACE_BIND`: `b2=pre … R3`), and `OpFreeRef(a)` releases the whole store the node
was just built in.  The rule "a compiler temporary reset to null frees nothing" (`re_bind`,
`LOFT_TEMP_NULL_RESET_FREES`) covers only compiler temporaries.  The null written after an
adoption has the same meaning for any variable: the store was handed over.

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

1. One report per runtime fault that names the source line and variable of the free and of the
   access (the pcs are already known; `introspect` has the map).
2. The records keyed to the same store identity the runtime reports, so a strict-stores fault
   can be looked up in `LOFT_STORE_TABLE`.
3. The records modelling host stores (`OpStoreBuf`, `OpAdoptChildRec`), so this cell reads
   as a use after end instead of 8 leaks.

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
