<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 — handoff, 2026-11-13: the elaborator's store-lifetime cases

Branch `tuxedo-pr2474-next` (pushed through `ac6dd4a2c`) = main `efbb7b00e` + `91-self-host`
through `cf0483c23` (the store-records rework) + the fixes below.  The owner's goal for the
stream: *fix the remaining lifetime cases and the store-records checker with it*, then gather
every active branch into one PR.

## Where it stands

- **Gate E** (`selfhost/front/elab_gate.py`, the C-built elaborator): 617 of 2054 agree, 25
  differ, **0 freed twice** at `8255edd6c`.
- **Suite** (suite `91-self-host`, both-c, dev binary): 3316 passed; the 2 failures are the
  F-Interp hole-spec cell, whose fix (`cf3bc130e`) is on loft2's `tuxedo-pln204-int32`.
- **Store records**: every fix below reads CAUGHT with its switch off and CLEAN with it on; the
  suite sweep under ten fix switches reads 0 MISS, 0 STATIC
  (`scripts/own_sweep.py --oracle check-records --suite <suite> --switch '' --switch <S> …`).
- **Records on the elaborator** (`LOFT_OWN_ORACLE=check-records loft --interpret
  selfhost/front/elab.loft`): 58 findings — 49 leaks, 6 re-frees, 3 uses after end.

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
| `c89a8b1f4` | the cargo build-dir opt-out retired (release-blocking); `9152f7fd3` fixes the tests that assumed `deps/` | — |

The C re-free census (`LOFT_DOUBLE_FREE`) now names a same-site re-free (first 20); it only
counted them.

## Open

1. **Records findings on the elaborator** (58): each is a defect on a path gate E does not
   run or a records error (a gap) to cure in `src/store_records.rs`.  `tuple_match`'s `mc`
   cluster was a gap (`ac6dd4a2c`); `struct_match`'s `__ref_2` re-free reads as one too.
2. **CI reds of run `43390150731`**: `tests/swap_rebind.rs` `n_r1` and
   `tests/rebind_buffer.rs` `n_b1` (the plain bind `daf8dbd40` skips H-SwapRebind /
   R-RebindBuffer; `LOFT_NATIVE_OWN_CALL_BIND=1` restores them), and the Heavy test-duration
   gate (four `html_wasm` `rust_writer::html_debug_*` tests near 410 s).

## The next PR

Gather every active branch once the open items are closed: `91-self-host` (the records rework
and cherry-picks of the first four fixes), laptop-c-perf `11651a5f5..28408ab57` (including the
Linux daily's version string, `41235157e`), loft2's branch (the five `LOFT_NO_OWNER_WITNESS`
switch-ab cells), and this branch.  The full CI run on the new cargo layout is `43390150731`.

## How to reproduce one program

```sh
LOFT_EMIT_PARSED=std.store target/debug/loft empty.loft       # an empty program's image
LOFT_EMIT_PARSED=p.store target/debug/loft <program>
LOFT_NO_NATIVE_LIBS=1 LOFT_NO_AUTO_REBUILD=1 LOFT_REFREE_FAILS=1 \
  target/debug/loft --interpret selfhost/front/elab.loft p.store <program> std.store
```

`LOFT_STRICT_STORES=1` names a use after free; it also never reuses a freed slot, which hides a
free through a stale reference — the plain build with `LOFT_DOUBLE_FREE=1` shows those.  For the
C-built elaborator: `LOFT_DOUBLE_FREE=1 target/debug/loft --native-c --out <dir>
selfhost/front/elab.loft` (about 6 minutes).  A `--native` run whose runtime rlib came from
another rustc rebuilds `target/release` from the tree: set `LOFT_NO_AUTO_REBUILD=1` in gates.
