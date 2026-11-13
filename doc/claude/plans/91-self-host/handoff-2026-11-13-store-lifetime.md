<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 — handoff, 2026-11-13: the elaborator's store-lifetime cases

Branch `tuxedo-pr2474-next` (pushed through `ba445643c`) = main `efbb7b00e` + `91-self-host`
through `cf0483c23` (the store-records rework) + the fixes below.  The owner's goal for the
stream: *fix the remaining lifetime cases and the store-records checker with it*, then gather
every active branch into one PR.

## Where it stands

At `0653ade1e` (later than the rows below): gate E 617 of 2062 agree, 26 differ, 0 freed twice;
the strict-stores C elaborator leaks **0 stores** over the 2062 programs (no fault); the records
sweep reads CLEAN on all 1679 suite cells; the records report 13 findings on the elaborator
itself — 7 after `4524dea4c`, the other 6 being real leaks the corpus never reached, fixed above
(`n_index`'s re-free is a false one: the checker loses that `x`, a field of `__lift_3`,
names #4 only where `__lift_3` does once the `if` joins — path-correlated holders are owed).
The suite runner fails a cell arming `LOFT_DOUBLE_FREE` that reports a re-free.  The earlier
state, kept for reference:

- **Gate E** (`selfhost/front/elab_gate.py`, the C-built elaborator): 617 of 2054 agree, 25
  differ, **0 freed twice** at `8255edd6c`.
- **Suite** (suite `91-self-host`, both-c, dev binary): 3316 passed; the 2 failures are the
  F-Interp hole-spec cell, whose fix (`cf3bc130e`) is on loft2's `tuxedo-pln204-int32`.
- **Store records**: every fix below reads CAUGHT with its switch off and CLEAN with it on; the
  suite sweep under ten fix switches reads 0 MISS, 0 STATIC
  (`scripts/own_sweep.py --oracle check-records --suite <suite> --switch '' --switch <S> …`).
- **Strict stores over gate E's corpus** (`LOFT_STRICT_STORES=1` C elaborator, never reusing a
  freed slot): no use after free and no double free in any of the 2054 programs; what is left
  is leaks.  `LOFT_LEAK_SITES` names each leaked store's maker and its caller.
- **Records on the elaborator** (`LOFT_OWN_ORACLE=check-records loft --interpret
  selfhost/front/elab.loft`): 38 findings.

## Fixed today (each with a switch that restores the defect)

| commit | the defect | switch |
|---|---|---|
| `e5929aa8f` | the copy bracket protected an `&` argument's pre-call store: a callee re-pointing it leaked, or freed the answer under it (`70 0`, interpreter and rustc) | `LOFT_NO_PROTECT_AFTER_CALL` |
| `b5772e1c7` | a branch arm reading a field of a lifted call (`d = if c { mk().x } else { f() }`) — freed by the lift and by `d` (`coalesce`) | `LOFT_NO_LIFT_VIEW_GUARD` |
| `0ae922a64` | a discharged projection (`def(e, n).attrs[i]!.t`) read as a mint, so the local took no owner witness (`variant_arm`) | `LOFT_NO_NCR_TEMP_VIEW` |
| `c67f11314` | a rebind whose value reads a view of a view of the local freed it first (`parse_code`; silent wrong answer on the interpreter) | `LOFT_NO_TRANSITIVE_VIEW_READ` |
| `c4ba47b68` | C: a null answer copied into a promoted return buffer freed the caller's buffer (`rewrite_unknown`; 142 of 149 programs) | `LOFT_C_NULL_FREES_ENTRY` |
| `4f2a3f141` | a rebind from a choice handing another variable over whole (`class = bound ?? Owned`) aliased it (`classify_var`, `tag_in`) | `LOFT_NO_CHOICE_VAR_JOIN` |
| `b83ab0844` | a join arm's owning call bound into its own buffer (`__ref_N = without_deps(x, __ref_N)`) named the argument's store where the call answered it (`arm_delivery`) | `LOFT_NO_S1_JOIN` |
| `8255edd6c` | a first bind from a choice over a `??` (`chain = if p { f() ?? NdNull } else { NdNull }`) aliased the `??`'s temporary; the rebind freed it under the temporary (`enum_field`) | `LOFT_NO_NCC_CHOICE_JOIN` |
| `7b1d8f614` | an early `return` written before a later rebind paired a literal's buffer free against its adopter, which the rebind had already freed (`tuple_match`; interpreter, seen under strict stores) | `LOFT_NO_LATE_OWNER_DISARM` |
| `684dc8504` | a comprehension body on a line of its own kept its block, so the slot leaked every store its call made (`with_deps` over tuples; main, both backends) | `LOFT_NO_COMP_LINE_PEEL` |
| `6c9a4ff68` | C: a first bind joined on a choice's variable into a view, and the copy leaked (`call_args`' `last_t`: ~100k TypeT; came with `8255edd6c`) | `LOFT_C_FIRST_CHOICE_JOIN_ANY` |
| `ba445643c` | a witnessed local bound from a choice of buffer calls lost the store a call made into a null buffer (`member`: ~71k Ex; both backends) | `LOFT_NO_WITNESS_NULL_BUFFER` |
| `c89a8b1f4` | the cargo build-dir opt-out retired (release-blocking); `9152f7fd3` fixes the tests that assumed `deps/` | — |
| `a31844d73` | loft#2476: a view bound from a choice of buffer calls leaked the store a call made into a null buffer (`vector_return`) | — |
| `324035eb8` | a `!`-checked argument read as a mint, so the copy of the answer lost its source-free (`classify_tail`, ~1.1k Own) | `LOFT_NO_NCR_ARG_ROOT` |
| `32d6b49bd` | `out = out[0..at] + [g] + …` typed `out` with the slice's scratch store, never freed (`sorted_groups`; main, both backends) | `LOFT_NO_SELF_SLICE_DEPS` |
| `094f56067`, `058453a3d` | loft#2477: a rebind reading a lift's view freed the store twice; the lift lets go, for an owner only (a view leaked: `member`) | `LOFT_NO_LIFT_LET_GO`, `LOFT_LIFT_LET_GO_VIEWS` |
| `77ab3fa86` | a call argument choosing a join arm's call read as a mint, so the copy lost its source-free (`place_assignment`, ~520 Node); only for call arguments — a `??` arm witness keeps the old reading | `LOFT_NO_JOIN_ARM_ARG_ROOT` |
| `70fa71fe5` | an early-leaving return arm's source (`_ret_src`) was never freed, though it owned a fresh record (`pattern_cond`) | `LOFT_NO_RET_SRC_OWNS` |
| `65c48e585` | a scalar vector built inline as an argument made the bracket incomplete (`match_expr`'s `arm_typed`) | `LOFT_NO_SCALAR_VECTOR_EXEMPT` |
| `c5febb2ef` | C: a copied answer was compared with the destination's freed old store; a reused slot matched and the answer leaked (`fill_instance`) | `LOFT_C_STALE_HELD_KEEP` |
| `916d55b2e` | `70fa71fe5` freed an early-leaving arm's LITERAL twice (its record is the literal buffer's); the source owns only a non-literal tail | `LOFT_NO_RET_SRC_OWNS` |
| `fd2e88c85` | records: a store a hidden buffer parameter holds at an exit is the caller's (false leaks after `ba445643c`) | `LOFT_RECORDS_NO_BUFFER_HANDS_BACK` |
| `0653ade1e` | C: a literal field read into a skip-free temporary was copied, and leaked (`sink_ncc`) | `LOFT_C_SKIPFREE_ELEMENT_COPY` |
| `2c952b749` | a call argument's block (`last(match c { … => b.ops, _ => [] })`) read as a mint, so the copy kept the callee's fresh answer (`text_instance_carried`, `fill_instance`; main) | `LOFT_NO_BLOCK_ARG_ROOT` |
| `b314c1d47` | `t = f(b)!` typed `t` as a view of `b`; the discharge's copy leaked (`default_image`, `row`; main) | `LOFT_NO_NCR_CALL_OWNS` |
| `4524dea4c` | `f(b)!.name` read in place left the discharge's copy unowned (`default_into`; main) | `LOFT_NO_NCR_LIFT` |
| `260e87512` | a projection into a KEPT local (`cond = mc.cond` beside `cond = NdIf { a: cond!, … }`, in a block): a wrong answer on the interpreter (main), a double free, and a pooled buffer an early return freed unguarded (`tuple_match`) | `LOFT_NO_KEPT_ELEMENT_COPY`, `LOFT_NO_KEPT_DISPLACED_FREE`, `LOFT_POOL_UNGUARDED_FREE_OK` |
| `377796e5b`, `32585c2b6` | records: a view of a variable names its store or none of its stores (`State::views`); `coalesce`'s false re-free gone | `LOFT_RECORDS_NO_VIEW_PAIRS` |

### The store records' findings on the elaborator itself: 5

Each was run down until it reproduced or was shown unreachable:

| function | verdict |
|---|---|
| `index` | **infeasible path**: the arm where `x` views the lift (`[…][i]`, a literal) never reaches the rebind at line 20165 — a sentinel with a positive control never fires there; the records cannot correlate the two conditions |
| `fill` | **infeasible path**: `self_type` answers a fresh record only for an ENUM, and the `TyVector { inner }` arm that views it cannot match one; the records do not know a value's variant |
| `vector_return`, `match_expr`, `parse_function` | not reproduced: probes of each shape run clean and the records say nothing on them |

The kept-local suite cell (`B-Copy/a-projection-into-a-kept-local-in-a-loop-is-a-copy`) reads
STATIC (two false findings): the absent-tag null test that would settle it is opt-in
(`LOFT_RECORDS_TAG_NULL_TEST=1`) because a call's answer through a buffer CAN be an absent-tagged
store; the precise answer needs the variant a mint writes.

The C re-free census (`LOFT_DOUBLE_FREE`) now names a same-site re-free (first 20); it only
counted them.

## Open

1. **loft#2476**: a VIEW bound from a choice of buffer calls leaks the store a call made into a
   null record buffer (main, both backends; `vector_return`'s `tail`, ~2k Node).  Needs a
   design: the buffer names its callee's answer, or such a view gets an owner witness.
2. **Leaks still on the corpus**: rebuild the leak-sites elaborator
   (`LOFT_LEAK_SITES=1 LOFT_STRICT_STORES=1 … --native-c --out <dir> selfhost/front/elab.loft`)
   and total the `[leak-site]` lines per maker and caller.
3. **CI reds of run `43390150731`**: `tests/swap_rebind.rs` `n_r1` and `tests/rebind_buffer.rs`
   `n_b1` — the rustc writer's plain bind (`daf8dbd40`) skipped the hand-over of
   `R-RebindBuffer`; a fix that hands the displaced store over after a plain bind is in the
   working tree, waiting on a rustc-native run.  The Heavy duration gate (four
   `rust_writer::html_debug_*` tests, ~260 s → ~410 s since `c89a8b1f4`) runs on dispatch and
   nightly only.
4. **The rustc writer and the choice join** (from the laptop side; not re-measured on this tree): `generation/dispatch.rs` joins only a CALL's answer
   (`call_bind_taken`), so a rebind `re_bind` joins on a choice (`class = bound ?? Owned`) is
   bound plainly on `--native` alone, and `class` and `bound` free one record twice (the
   interpreter and C are right).  Repro: the suite cell
   `rules/binding/B-Copy/a-branch-handing-back-a-local-copies-it-into-a-binding-assigned-before.loft`
   under `--native`.  The C writer's `join_stmt` is the model.

## The next PR

Gather every active branch once the open items are closed: `91-self-host` (the records rework
and cherry-picks of the first four fixes), laptop-c-perf `11651a5f5..b1043e270` (including the
Linux daily's version string, `41235157e`), loft2's branch (the five `LOFT_NO_OWNER_WITNESS`
switch-ab cells), and this branch.  The full CI run on the new cargo layout is `43390150731`.

## How to reproduce one program

```sh
LOFT_EMIT_PARSED=std.store target/debug/loft empty.loft       # an empty program's image
LOFT_EMIT_PARSED=p.store target/debug/loft <program>
LOFT_NO_NATIVE_LIBS=1 LOFT_NO_AUTO_REBUILD=1 LOFT_REFREE_FAILS=1 \
  target/debug/loft --interpret selfhost/front/elab.loft p.store <program> std.store
```

A long check runs a COPY of the dev binary (`cp target/debug/loft <scratch>/loft-<sha>`, passed as
`--loft`): a `cargo build` for the next fix otherwise replaces the binary under a running gate,
and the result mixes two builds.

`LOFT_STRICT_STORES=1` names a use after free; it also never reuses a freed slot, which hides a
free through a stale reference — the plain build with `LOFT_DOUBLE_FREE=1` shows those.  For the
C-built elaborator: `LOFT_DOUBLE_FREE=1 target/debug/loft --native-c --out <dir>
selfhost/front/elab.loft` (about 6 minutes).  A `--native` run whose runtime rlib came from
another rustc rebuilds `target/release` from the tree: set `LOFT_NO_AUTO_REBUILD=1` in gates.
