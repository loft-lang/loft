<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 — handoff, 2026-11-13: the elaborator's store-lifetime cases

Branch `tuxedo-pr2474-next` (pushed through `3eb82c694`) = main `efbb7b00e` + `91-self-host`
through `cf0483c23` (the store-records rework) + the fixes below.  The owner's goal for the
stream: *fix the remaining lifetime cases and the store-records checker with it*, then gather
every active branch into one PR.

## Where it stands

- **Gate E** (`selfhost/front/elab_gate.py`, the C-built elaborator): 617 of 2049 agree, 27
  differ, **2 freed twice** at `3eb82c694` (357 freed twice that morning).
- **Suite** (suite `91-self-host` `c1358dd`, both-c, dev binary): 3304 passed, 0 failed.
- **Store records**: every fix below reads CAUGHT with its switch off and CLEAN with it on
  (`scripts/own_sweep.py --oracle check-records --cell <probe> --switch '' --switch <S>`).

## Fixed today (each with a switch that restores the defect)

| commit | the defect | switch |
|---|---|---|
| `e5929aa8f` | the copy bracket protected an `&` argument's pre-call store: a callee re-pointing it leaked, or freed the answer under it (`70 0`, interpreter and rustc) | `LOFT_NO_PROTECT_AFTER_CALL` |
| `b5772e1c7` | a branch arm reading a field of a lifted call (`d = if c { mk().x } else { f() }`) — freed by the lift and by `d` (`coalesce`) | `LOFT_NO_LIFT_VIEW_GUARD` |
| `0ae922a64` | a discharged projection (`def(e, n).attrs[i]!.t`) read as a mint, so the local took no owner witness (`variant_arm`) | `LOFT_NO_NCR_TEMP_VIEW` |
| `c67f11314` | a rebind whose value reads a view of a view of the local freed it first (`parse_code`; silent wrong answer on the interpreter) | `LOFT_NO_TRANSITIVE_VIEW_READ` |
| `c4ba47b68` | C: a null answer copied into a promoted return buffer freed the caller's buffer (`rewrite_unknown`; 142 of 149 programs) | `LOFT_C_NULL_FREES_ENTRY` |
| `4f2a3f141` | a rebind from a choice handing another variable over whole (`class = bound ?? Owned`) aliased it (`classify_var`, `tag_in`) | `LOFT_NO_CHOICE_VAR_JOIN` |
| `c89a8b1f4` | the cargo build-dir opt-out retired (release-blocking); `9152f7fd3` fixes the tests that assumed `deps/` | — |

The C re-free census (`LOFT_DOUBLE_FREE`) now names a same-site re-free (first 20); it only
counted them.

## Open

1. **`tag_in` → `enum_field`** (suite C-Var `2441-a-field-every-variant-declares-…`): `tag_in`
   frees a store at its exit (el_expressions.loft 8007) that `enum_field`'s `__ncc_3` — the
   `field_get(…, recv) ?? ir::NdNull` temporary, `recv = v_if(tag_in(…), place, …)` — frees
   again (line 7997).  `tag_in`'s own binds are right since the choice join (`cond` copies
   `test`); the store reaches `enum_field` through `v_if`'s and `field_get`'s child records.
   `classify_var`'s three programs closed with the choice join.
2. **`arm_delivery`** (suite H-Move `a-file-local-moved-into-a-structure-keeps-its-handle`):
   two adjacent frees in its exit sweep (el_expressions.loft 2283) release one store.  The
   interpreter shows it with `LOFT_REFREE_FAILS=1`; the first free's variable is not named.
3. **Early-return leak** (main, both backends): a `return` inside a loop skips a local whose
   deps the scope pass strips LATER in the scan (a materialised view across a container's
   rebind), because `get_free_vars` reads the deps at the return (`dep_empty=false`).  The fix
   decides an early return's frees after the whole function is scanned; the strip in
   `scan_set` (the materialise arm) is conditional, so a pre-scan fact is not safe.  The records
   already flag it (a STATIC leak in `tuple_match`).  Repro — `1034 -1` and one `NIf` leaked:

   ```loft
   enum N { NB { b: boolean }, NI { i: integer, s: text }, NIf { a: N, c: N } }
   struct MC { cond: N?, refused: boolean }
   fn member_cond(k: integer) -> MC { MC { cond: NI { i: k, s: "c{k}" }, refused: k == 3 } }
   fn v_if(a: N, c: N) -> N { NIf { a: a, c: c } }
   fn sz(n: N) -> integer { match n { NI { i, s } => i * 10 + len(s), NIf { a, c } => 1000 + sz(a) + sz(c), NB { b } => if b { 1 } else { 0 } } }
   fn go(n: integer) -> integer { cond: N? = null; for pe in 1..n + 1 { mc = member_cond(pe); if mc.refused { return -1; } if mc.cond != null { cond = if cond == null { mc.cond } else { v_if(cond!, mc.cond!) }; } } if cond == null { 0 } else { sz(cond!) } }
   fn main() { println("{go(2)} {go(3)}"); }
   ```
4. **Records findings on the elaborator** (`LOFT_OWN_ORACLE=check-records loft --interpret
   selfhost/front/elab.loft`): 53 leaks and 12 re-free / use-after-end findings, mostly on
   paths gate E does not run — each a real defect or a records false positive to settle
   (`struct_match`'s `__ref_2` re-free reads as a false positive).
5. **The rustc writer and the choice join**: `generation/dispatch.rs` joins only a CALL's answer
   (`call_bind_taken`), so a rebind `re_bind` joins on a choice (`class = bound ?? Owned`) is
   bound plainly on `--native` alone, and `class` and `bound` free one record twice (the
   interpreter and C are right).  Repro: the suite cell
   `rules/binding/B-Copy/a-branch-handing-back-a-local-copies-it-into-a-binding-assigned-before.loft`
   under `--native`.  The C writer's `join_stmt` is the model.
6. **Suite cells**: the choice join's is `rules/binding/B-Copy/a-branch-handing-back-a-local-copies-it-into-a-binding-assigned-before.loft`
   (`LOFT_NO_CHOICE_VAR_JOIN`).  The other fixes' cells are on suite
   `91-self-host` with receipts; two score only under an instrument (O-Detach under
   `LOFT_POISON=1`, O-Buffer under `LOFT_STRICT_STORES=1`), as their CHANNEL lines say.

## The next PR

Gather every active branch once the open items are closed: `91-self-host` (the records rework
and cherry-picks of the first four fixes), laptop-c-perf `11651a5f5..4a53e3373` (including the
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
