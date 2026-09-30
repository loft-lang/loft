<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 180 — Small records as tuples, decided in the IR phase

Tracker: [@PLN180](https://github.com/loft-lang/plans/issues/180).

## Status

Active.  **Slice 1 built** on `laptop-superinstructions`: tuple return with read-only
call sites, both backends (`src/value_record.rs`).  Slices 2–4 are open.

Building the slice also found and fixed a `--native` defect in the existing rewrite.
Native built the tuple of a value-returned literal in FIELD order, so a literal whose
fields are written out of schema order ran its field expressions reordered, and nothing
reported it.  The fix is in `value_record_parts` and its emitter.

## Goal

A small record a function returns, and a local bound from such a call, are carried as
tuples on the interpreter as well as `--native`.  The interpreter then claims no store for
them, just as native already does not.

## Why

`R-ValueRecord` and `R-ValueLocal` (formal/rewrites.md) are native-only today.  The IR both
backends read still binds `p:ref(V3) = n_v3(…, __ref_1)`.  On the interpreter that claims a
buffer store per call and frees it through `OpFreeRefIfDistinct`.  The store census
(`make interp-gap`, `LOFT_STORE_CENSUS`) counts per routine op, interpreter against native:

| routine | interpreter stores | native stores |
|---|--:|--:|
| resolve_move | 146,756 | 0 |
| slope_path_with_undo | 4,227 | 1 |
| panel_build | 46k | 10k |

A hand-written probe computed the same hash in every cell: records took 29.8 ms and 60,000
stores on the interpreter, tuples 13.6 ms and 0.

## Design

**One admission.** The candidates are exactly what `generation::hoist::value_records`
admits, so the rule keeps one definition.  This pass narrows them to the shapes it can
rewrite.  Native's own pass then finds only the functions this pass declined, so the two
never apply to the same function.  Deleting native's pass is the end state, and it becomes
possible only once every shape it covers has moved here.

**Where it runs.** At the top of `compile::byte_code_from`, not in `scopes::check`, because
the rewrite changes SIGNATURES.  `check` runs once per source load, and a file loaded later
would type a call by the rewritten signature (`p.x` on a tuple does not parse).  A `Data`
that is parsed against again after compiling is marked `Data::open_world` and keeps
records.  That covers the REPL, the debugger (which runs through it), live reload, the
browser debugger panel and a host that calls functions by name.  A warm-cache start also
keeps records, because it reads bodies from the cached store.

**Tuple order is fill order.** The literal writes its fields in source order, and the
tuple is built in that order.  Building it in schema order would reorder the evaluation of
field expressions that have effects.  That reordering is exactly the native defect this
slice found.

**Frame layout.** The rewritten functions are laid out again by the same two steps
`scopes::check` ends with (`compute_function_intervals`, `assign_function_slots`), after
`Function::reset_intervals`.

**Switch.** `LOFT_NO_IR_VALUE_RECORD=1`.

## Slices

| # | Shape | Status |
|---|---|---|
| 1 | Tuple RETURN: the callee is not `pub`, its record is flat (8-byte integer, float, single, boolean), and its buffer appears only in object literals in result positions.  Every call is `local = f(…, __ref_N)` whose local is only read field-wise and freed.  The buffer may be the straight-line one or the loop-hoisted one (a mint guard, its free against the local in either argument order, its own exit free). | built |
| 2 | Tuple LOCALS written to: a field write becomes `TuplePut`.  The `resolve_move` shape, where a local is adjusted and re-read. | open |
| 3 | Tuple PARAMETERS: a by-value or const parameter of an admitted type, and a local handed to one (`blocked(from, to_x)` in the probe). | open |
| 4 | Materialisation: a record is built where one is owed (a field or element store, a library boundary), so one such site no longer declines the whole function. | open |

`resolve_move`, the row that motivated the plan, needs slice 3 and an INLINE sub-record.
Its `vec3(…)` locals are handed to `move_blocked`, and `MoveResult` holds a `Vec3`, which
slice 1's flat-record gate declines.  So slice 3 comes before slice 2, and nested layouts
(native's `layout_into` already flattens them) belong in slice 3.

After this plan, the same move applies to the push family (R-Push, R-PushFill, R-PushRec,
R-Mint), then R-TextBorrow.  Those are separate plans.

## Verification per slice

This is the proportional rule (CI_BUDGET.md § Sizing the checks for a performance change).
Each slice brings: a guard with hand-computed cells, one planted defect recorded in its
`@falsified-at`, the switch, `find_problems.sh --changed`, and the census row it moves.
Slice 1 reaches every program, so it also runs the broader suite once.

Slice 1's guard is `tests/scripts/a-small-record-returned-to-a-reader-is-a-tuple.loft`.
Each plant fails exactly one cell on both backends:

- A tuple built in schema order runs `tick` as 3, 2, 4, 1 and fails `fill order`.
- The old native emitter runs 7, 5, 8, 6 and fails `fill order when handed on`.

The native R-ValueRecord guards (`157-value-record.loft`, `157-value-chain.loft`,
`157-value-tail.loft`, `158-*`, `164-*`) now run through this pass as well.

Probed, clean on both backends:

- a callee reached from a `par` worker (rewritten);
- a caller in a generator body (declined by `value_records`, so both passes keep records);
- a literal with a whole sub-record copied in from a call (the copy's source runs once).

## Open questions

- The rewrite census (`make rewrite-census`) will record native `R-ValueRecord` admissions
  moving to the `ir` phase.  That shows as a native DROP on the benches this slice takes, and
  the move is deliberate: bless it with this plan named.
- The debugger shows a tuple where the program wrote a record, but only on a program it did
  not open, since `open_world` keeps records wherever it parses.  If a debugger ever attaches
  to a closed program, the variable table should remember the record type the tuple stands
  for.

## See also

[formal/rewrites.md](../formal/rewrites.md) R-ValueRecord / R-ValueLocal ·
[PERFORMANCE.md § How the interpreter executes](../PERFORMANCE.md) (store work) ·
[NATIVE_SWITCHES.md](../NATIVE_SWITCHES.md) (`LOFT_NO_VALUE_RECORD`, `LOFT_NO_VALUE_LOCAL`).
