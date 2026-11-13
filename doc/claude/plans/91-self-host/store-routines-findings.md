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
| loft2 | `../loft2`, branch `tuxedo-pln204-int32` | the use-after-free cases; `src/bind_lowering.rs`; the records' false findings outside closures |

The records' false findings are split: loft3-c1 takes the closure cells, loft2 the rest.  A
finding that turns out to be a real defect goes to loft-c9, which owns the cures (leaks
included).

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

**Coverage after the fixes** (2026-11-11, the suite at `8778c1e` plus this arc's cells): no use
after free under `LOFT_STRICT_STORES=1` on the interpreter, C or rustc-native.  Every failure
left is a cell that lowers `@STORE_LIMIT` to 1310 (interpreter: 3, native: 6), and each is clean
with the ceiling lifted.  The 574 `tests/scripts` programs: no use after free; 45 could not be
checked from a scratch copy (they fail without strict stores too: imports and files the copy
lacks).  Not swept: the libraries and the consumer programs.

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

**Built since** (2026-11-11): the fault log (`LOFT_FAULT_LOG`, all three backends), read by
`own_sweep.py`, which also gained `--compare`; and `scripts/store_why.py`, the strict run →
the first fault's store → its traced life, as one command.  The records' STATIC noise is
left.

The highest-leverage change was: **a fault reported as data, not text** — one machine-readable line
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

## The records' false findings (STATIC): gap or real

The checker (`LOFT_OWN_ORACLE=check-records`) is swept over the suite with every cure on.
A cell it reports (STATIC) is worked to the end, one cell at a time, and is one of two kinds:

- **gap**: the records model the program wrongly.  The fix goes in `store_records.rs`, the
  cell reads CLEAN, and the sweep then shows which other cells the same fix cleared.
- **real**: the compiler is wrong and the checker found it first.  The cure goes to loft-c9
  (the three-place rule: compiler, records, checker CAUGHT with the cure off).

A cell counts as real only after a probe makes the defect happen on a backend: the suite
cell often never takes the path (a coalesce whose fallback never runs).

```bash
LOFT_OWN_ORACLE=check-records LOFT_NO_CACHE=1 loft --tests <cell>          # one cell's findings
LOFT_STORE_TABLE=<fn> LOFT_NO_CACHE=1 loft --tests <cell>                  # that function's stores
LOFT_NO_CACHE=1 python3 scripts/own_sweep.py --loft <bin> --oracle check-records \
    --suite <loft-suite> --jobs 8 --out cand.tsv --compare best.tsv        # the whole sweep
```

| batch | STATIC before → after | gap | real |
|---|---|---|---|
| patches7 | 133 → 113 | 5 | 1 (B-Copy record update: `__retbuf = u` freed the caller's buffer; loft-c9 09ecad0cf) |
| patches8 | 113 → 94 | 5 | 2 (below) |
| patches9 | 93 → 88 (rebased) | 1 | 1 (3 below) |
| patches10 | 83 → 80 (base merged main) | 1 | 1 (4 below) |
| patches11–12 | 80 → 72 | 5 | 1 (5 below) |
| patches13 | 69 → 53 (base took loft3-c1's closure batch) | 3 | 0 (+ two checker MISSes for loft-c9 now CAUGHT) |
| patches14 | 53 → 50 | 1 | 1 (6 below) |
| series 2026-11-12 (8 commits on 91-self-host 8f7c1c9e0) | 50 → 36 | 6 | 2 (7, 8 below) |
| series 2026-11-12, continued (loft2 and loft3-c1, on 91-self-host 7e2d72225 + the snapshot patch) | 36 → 6 | 17 (the 16 store_records.rs fixes 15caa3c50..7e2d72225, and the snapshot patch) | 3 (9–11 below) |
| loop fixes 2026-11-12 (91-self-host f1df5b9a6 + B + D) | 6 → 5 (1643 runs; 4 new cells) | 1 (stale names, loft3-c1) | 4 (12–15 below) |

The real defects the checker found first:

1. **A keyed collection a coalesce's fallback calls, read inline by key, is never freed** —
   verified on both backends (`(g ?? nr_h(i))[i].v`; bound to a local first, or a vector or
   struct fallback, it is freed).  No temporary took the call inside the coalesce arm.  Fixed
   by loft-c9 (faa069e72, switch `LOFT_NO_COALESCE_ARM_LIFT`).
2. **A vector grown on append after a fn-ref call answered null keeps its buffer** —
   verified on the C backend (`v = h(k); v += [1000]` with `h` a lambda answering null):
   `__vdb_1` has no free in the IR.  The interpreter frees it only through its frame-exit
   release at `Return`.  A direct call is clean.  Open, loft-c9.
3. **An element record copied out of its work-ref ran its drop hook on a freed slot** —
   `S2 { hs: [H { id: 4 }] }` with a hook on `H` printed `dh0` once per element before the
   real releases, both backends: the work-ref's reset after the source-freeing copy read as a
   rebind displacing an owned record.  The copy now arms its hand-off where it runs
   (`scopes/scan.rs`); suite cell H-Drop/a-record-built-for-an-element-of-a-field-literal….
4. **A heap member of a tuple argument handed back was freed twice** —
   `fn first(p: (R, integer)) -> R { p.0 }`: the @P290 witness set skipped a tuple, the dep
   converter gave a vector result no dep, and a member built inside the literal had no name.
   Silent on main (its runtime does not report a free of a freed store).  Switches
   `LOFT_NO_TUPLE_ARG_COVER` / `_DEPS` / `_NAME`; suite cell O-Move/a-member-of-a-tuple-argument….
   Left: a tuple VARIABLE to a callee that may answer its member or mint fails the C backend
   (loft#2471).
5. **A call result stored by reference in a literal argument was freed before the call** —
   `readk(Linked { link: mk(9), n: 1 })` answered 71 where 91 is right on main: the lift that is
   the record's home was scoped to the literal's block.  It now lasts as long as the holder
   (`LOFT_NO_REF_HOME_LIFT`); suite cell H-RefHome/a-call-result-in-a-literal-argument….
6. **A sub-rule run freed a borrowed answer and leaked an answer that consumed nothing** —
   `(xs: view)*` with `view` answering `p.nodes[0]` freed the caller's whole store; a rule
   answering without consuming leaked at the stop.  The answer is released only where it is the
   rule's own or names no argument's store (`LOFT_NO_SUBRULE_OWNED_ANSWER`,
   `LOFT_NO_SUBRULE_STOP_FREE`); suite cell P-Rule/a-sub-rule-run-releases…  The rustc-native
   backend still re-frees the run's temporary through its own `_own_store_` shadow (loft-c9).
7. **A vector local a forwarding fn-ref may rebind was freed twice where it never was** —
   `x: vector<integer> = [5, 6]; for _ in 0..n { x = fwd(q) }` with `fwd` a lambda forwarding to
   another: the scope-end free `OpFreeRefIfDistinct(x, q)` (loft#1257's join witness) ran while
   `x` still named its literal backing, which freed it too.  Latent on main, both backends.  The
   scope-end free takes the rebind's view-temp guard (`LOFT_NO_VIEW_TEMP_SCOPE_END_GUARD`); suite
   cell O-Latest/a-vector-local-a-forwarding-fn-ref-may-rebind-is-released-once.
8. **A binding arm answering the subject freed the caller's vector** —
   `match v { [h, ..t] => t, whole => whole }` delivered `whole` into the return buffer and freed
   its dependency, the parameter `v`.  Latent on main, both backends, every element type.  The
   arm delivery skips a parameter dependency (`LOFT_ARM_FREES_PARAM_DEPS`); suite cell
   P-Rest/a-binding-arm-answering-the-subject-leaves-the-callers-vector.
9. **An element read at a constant index past the end, returned as `-> D?`, answered a default
   record** — `r = v[0]; r` on an empty vector: silent-wrong on both backends.  The local may be
   absent, so the return answers null (`LOFT_NO_ELEMENT_VIEW_ABSENT`); suite cell
   N-Road/a-returned-element-read-at-a-constant-index-past-the-end-is-null.
10. **A work buffer handed null was read after its guarded free at each tail jump** — the
   interpreter left a variable its `OpFreeRefIfDistinct` released naming the freed store, where
   C resets it; `(R-TailSelf)` then cleared it in place on the next pass.  The interpreter now
   resets it as C does (`LOFT_NO_GUARDED_FREE_RESET`); suite cell
   R-WorkBuffer/a-work-buffer-handed-null-is-reset-at-each-tail-jump.
11. **A closure rebuilt through a snapshot released a capture it takes again** — a capture type
   with a drop hook rebuilds the record through `__disp_N`, whose cascade releases every slot; a
   leaving closure rebuilt twice in a loop over the frame's `e` freed `e` twice, both backends.
   The build detaches an old slot naming its new fill first (`LOFT_NO_SNAPSHOT_KEEPS_REFILL`);
   suite cell L-CapOwn/a-rebuild-in-a-loop-keeps-the-capture-it-takes-again.  Found only once the
   records modelled the snapshot's `OpCopyRecord`.
12. **A staying record rebuilt in one pass freed its capture twice** — after exactly one pass of
   `for … { g = fn() {..a..}; a = [i] }` the declaration's backing still named the record's
   capture, and the frame's end freed both.  Let-go point 5 hands the backing's store to a record
   whose live cascade frees it (`LOFT_NO_STAYING_RECORD_TAKES_BACKING`); suite cell
   L-CapKeep/a-staying-record-rebuilt-in-one-pass-frees-its-capture-once.  L-Apply's three
   findings and 2357's `after_local` / `after_in_frame` were this.
13. **A pass-scoped fn-ref beside a kept one freed its record twice** — a second loop keeping a
   fn-ref gates the frame (`closure_keep`), and the pass-scoped fn-ref then released its confined
   record too (`LOFT_NO_CONFINED_RECORD_STAND_DOWN`); suite cell
   L-CapKeep/a-pass-scoped-fn-ref-beside-a-kept-one-frees-its-record-once.  L-CapRebind
   vector's findings were this and 12.
14. **A call rebind in a loop refilled the vector a closure captured** — `a = mkv(i)` after the
   build answered the value assigned after it: silent-wrong, both backends.  The call's buffer is
   a backing the rebind detaches (`LOFT_NO_CALL_BUFFER_REBIND_DETACH`); suite cell
   L-CapRebind/a-call-rebind-in-a-loop-leaves-the-record-its-build-time-vector.  Not a lifetime
   fault, so the records cannot report it; found by the over-reach probe beside 12.
15. **A capture rotated through call buffers was freed under a buffer** — `v = bump(v)` with a
   kept closure: a use after free on the C backend at three passes.  Such a capture is the
   buffers' to release (`LOFT_NO_ROTATED_CAPTURE_LEFT_TO_BUFFERS`); suite cell
   L-CapRebind/a-capture-rotated-through-call-buffers-is-the-buffers-to-release.

16. **A closure re-bound to a plain lambda leaked the record it displaced** — a fn-ref holding a
   frame-built record, re-bound on a branch to a lambda that captures nothing, then returned: the
   frame took the record as leaving on every run, so the re-bound run released neither the record
   nor its captures, and skipped their hooks; both backends.  Such a frame is decided by store
   identity (`LOFT_NO_PLAIN_REBIND_KEEP`); suite cell
   L-CapOwn/a-closure-rebound-to-a-plain-lambda-releases-the-record-it-displaced.  Found probing
   L-CapOne/2281's `lv` with the closure built on one branch only.

**A loop finding is re-run at 0, 1, 2 and 3 passes before it is called false.**  A suite cell's
loop runs a fixed count, and 12, 13 and a leak a first version of 12's cure introduced showed
only at ONE pass: at two or more the stores rotate apart, at none nothing is built.  The records
walk every path; the cell's run walks one.

The gaps fixed, by the representation they corrected:

- **Paths the guards rule out** — `OpDistinctStore` narrows both arms (a fresh store is
  never the caller's; the SAME arm keeps only what the witness may also name, where a
  parameter may be null or another parameter's store); a boolean ownership flag written a
  constant decides the guard that reads it.
- **Frees that are no-ops on some paths** — a guarded free (`OpFreeRefIfDistinct(x, w)`) or
  a stashed free whose other operand may name the store ends it only where that operand does
  not, and the store then stays live exactly where it does; the stores the witness cannot name
  always end.
- **Values handed on** — a variable's value returned or captured by a leaving closure hands
  on the stores it keeps live (`handed_by`); an owned value written through a slot link is
  given up (`OpSetSlotRef` with `COPY_FREE_SOURCE`).
- **Store generations** — a reassignment reads the store it displaces after the value, and
  keeps it in the state while the copy mints the next generation at the same site; a buffer
  call handed null or its own earlier store answers that store; `OpInitRefSentinel` resets its
  variable; a nested tuple literal in a tuple member answers its stores; a store re-initialised
  in place stays correlated with its variable.
- **What a value is** — an IR `n_protect_store_frees` bracket declines a source-free; a source
  that is no variable frees what its value names; a `-> T?` call may answer null, and a call
  answer's collection null test decides (a local's may be a live absent-marked store); a
  stored-reference read answers its referent, not the holder's store; a tuple-literal argument
  is read, not handed on, and a tuple parameter's member is the caller's store; a nested member
  moved out of another tuple is that member; a put into a local tuple's member assigns it.
- **Who holds a store, path by path** — a store may be held by one of several names on different
  paths (a two-buffer rotation): beside its must-holders it keeps ALTERNATIVES, name sets of
  which one holds it wherever it is live; a free or a narrowing on a name drops the alternatives
  naming it, and a copy (`OpPutRef`, `x = y`, `OpRefAlias`, a call answering its buffer or
  argument) adds its destination where its source holds.  A loop head that joins a pass which
  freed a buffer and reset it keeps that site's store as the buffer's; an `OpDatabase` re-init
  keeps a maybe-ended store the variable must-holds; `x = f(…, x)` answering `x`'s own store
  keeps `x` its holder.  `LOFT_STORE_TRACE=<fn>` prints the state after each statement.
- **Guards with a single witness** — the DISTINCT arm of `OpDistinctStore(v, w)` rules out a store
  `w` alone must-holds (where it cannot have ended), and an arm left with nothing cannot run: the
  join of arms answering different buffers closes with it.
- **Owners a stored reference leaves in place** — a compiler temporary the function frees keeps
  the record a `reference<T>` field names, and a holder handed to a call reads those records.
- **Closure records' slots** — a capture slot holds a store; an owner slot (`__own_x`) makes the
  record a holder of it, and an identity test of a slot (`OpDistinctStore(OpGetDbRef(w, off), x)`)
  narrows the slot on each arm.  A record's cascade (`<record>_OpDropAll`, `OpDropFnRef`) ends
  only the slots its type's drop frees, follows a record a slot holds, and is typed by any holder
  of a `__closure_*` type, a rebuild snapshot (`__disp_N`) included.  `OpCopyRecord` gives the
  snapshot the source record's slots.  A join ignores the slots of a record that does not exist
  on one side.  `LOFT_STORE_TRACE` prints the slots.
- **A fn-ref's closure as an operand** — `OpDistinctStore(OpFnRefClosure(g), w)` narrows and
  equates `g` itself, as a plain variable operand does: the stand-downs that hand the frame's
  closure record to a fn-ref read right.
- **Host stores and guarded frees of a capture** — `R-HostStore`'s result clause
  (`OpReleaseRecIfDistinct` a guarded free, `OpStoreBufOnly` may be null, a minting callee never
  answers null); a guarded free of a capture slot leaves the store live exactly where the witness
  names it.

Open (four cells, all records gaps; every function in them is runtime-clean at 0..3 passes on
both backends):
- **One key for every earlier pass.**  A site minting each pass keeps its current store and ONE
  older generation, so the stores of passes 1..n-1 merge: a slot's correlation with the store it
  holds cannot survive (C's cell `one_loop`, 2357 `after_returned_fn`'s older store), and the
  zero-pass path's "this store exists only where the record does" is lost at the loop's exit
  (2357 `record`).  15's records MISS was the same shape; the first three passes are now analysed
  one by one, which caught it.  Owner: loft3-c1's pass design.
- **A liveness test of a capture slot** (C's cell `two_loops`, its `#0`): a let-go point frees a
  slot's store behind `OpStoreLive(OpGetDbRef(w, off))`, and the records narrowed that test only
  for a plain variable, so a store the frame had already freed stayed in the slot.  Both backends
  reset a freed variable, so the identity test beside it is no witness; the liveness test is what
  keeps them clean.  Fixed by loft3-c1's cl-patches13 (the slot narrowed on the live arm).
- **A store's existence correlated with a record variable across a branch join** (L-CapOne/2281
  `lv`: `h` names the second record exactly where it was built; the frame end's stand-downs walk
  an arm where `h` names it and its record variable is null).  Same relation as 2357 `record`.
  Owner: loft3-c1's design.
- L-CapKeep/1636: loft3-c1's.
