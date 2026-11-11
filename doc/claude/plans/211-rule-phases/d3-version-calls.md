<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN211 D3 — class 2: `(R-Version)` for an accessor that calls an index helper

D1's class 2 ([d1-classes.md](d1-classes.md)): `(R-Version)` takes only a LEAF — a function
with no loop and no call but operators — so the hex libraries' cell accessors are refused:

```loft
fn hs_index(s: HexSet, q: integer, r: integer) -> integer {
  dq = q - s.hs_q0;
  dr = r - s.hs_r0;
  if dq < 0 || dq >= s.hs_w || dr < 0 || dr >= s.hs_h { return -1; }
  dr * s.hs_w + dq
}
pub fn hexset_get(s: HexSet, q: integer, r: integer) -> boolean {
  i = hs_index(s, q, r);
  if i < 0 { return false; }
  s.hs_cells[i] ?? false
}
```

`LOFT_TRACE_VERSION=1` says "no leaf: calls hs_index" for `hexset_get` and `hexset_set`, and
"calls hz_index" for `height_get` and `height_set`.  Every cell read in a window walk
(`for r in r0..r0 + h { for q in q0..q0 + w { … hexset_get(s, q, r) … } }`, the form of nearly
every loop in hex_roof and of hex_place `field_union`) then pays the window tests, checked
`loft_sub`, the element's bounds test and a null test.

## Price (2026-11-11, ship C, hex_roof bench, min of 11 interleaved runs, hashes equal)

The hand form, by script on the emitted C (`scratchpad c2/hand.py`, `hand2.py`): per loop nest
(`ridge_at`, and the `__inv` twins of `cone_at`, `dome_at`, `roof_cone`) one guard — the
nest's box is the set's own window, the heights' window contains it, both vectors hold
`w * h` elements, nothing null — then each accessor call is its fast form: the index
`(r - r0) * w + (q - q0)` and the element read or written with no test.  Priced twice: bare,
and with every fast call and its index helper keeping the call wrapper the existing fast twins
keep (`loft_frame_push` / `pop`, the hold mark).  The same numbers both ways:

| routine | gcc | clang |
|---|--:|--:|
| `roof_cone` | 5.46 → 2.41 ms, **−55.8 %** | 3.40 → 2.08 ms, **−38.8 %** |
| `roof_match` | 4.28 → 3.59 ms, **−16.3 %** | 3.17 → 2.84 ms, **−10.3 %** |

`roof_match`'s other 31 % is `seg_distance` (hex_way), refused as "calls pv": the same class
with natives (`sqrt`, `atan2`) and a non-leaf (`ang_wrap`) beside it — not priced, not in this
design.

## What the rule needs — three widenings, one decision each

1. **Element kinds.**  `Walk::access` proves `OpGetInt` / `OpSetInt` only.  The accessors read a
   boolean (`loft_pa_bool` in the C) and a float, and write a float and a boolean.  Each kind is
   a read or write at `(header input, size, linear index)`; the fast twin's writer already has
   the plain forms (`loft_vh_at_float`, `loft_vh_put_float`, `loft_vh_at_addr` with the narrow
   readers).  Site: `opt::version::Walk::access`, `c::body::version::version_access`.
2. **An early exit decided.**  `if c { return x }` (no else, the arm holding no access) followed
   by an access on the path that continues is decided toward NOT taking it: the precondition
   gains `!c`, the fast twin drops the `if`.  Today only an `if` whose own arm holds the access
   is decided.  Site: `Walk::walk`'s `Value::If` arm; the continuation is known because the walk
   is in evaluation order — decide when the rest of the leaf (callee bodies included) has an
   access.
3. **A call to a leaf.**  `i = hs_index(s, q, r)`: the walk enters the callee with its
   parameters bound to the caller's linear arguments (`Local::Lin`) and its record parameter to
   the caller's inputs (the `(R-Inputs)` scalars and headers name the caller's record); a
   `return e` in the callee ends the callee with `e`'s value.  One level of leaves calling
   leaves is what the accessors need; deeper is the same rule recursively, with a depth bound.
   The plan's addresses then name nodes in the callee's IR too, so the C writer emits the
   callee's own fast twin (`hs_index__inv__vf`, the same plan) and the caller's fast twin calls
   it.  Site: `Walk::walk`'s `Value::Call` arm (today `Err("calls …")`),
   `c::body::version` (the twins emitted per callee).

What stays as it is: the guard (the precondition asked at the box's corners, the nest clause),
the floor of eight values, null and overflow refused in the precondition, the call
`vg ? f__inv__vf(…) : f__inv(…)`.

## Matrix, before each part

Cells over a probe accessor pair modelled on hex_field's (hand-computed values, interpreter as
the oracle, then C, `LOFT_CC` verify-C, rustc-native):
- the box inside the window; the box crossing each of the four window edges (fast twin
  refused, values from the written path); an empty window; `w * h` beyond the vector's length
  (refused);
- the heights' window smaller than the set's (refused) and larger (admitted);
- a null cell (`?? false` / `?? 0.0` still answers the default on the fast path);
- a write through `hexset_set` (the count field updated) and `height_set`;
- a loop that also writes the set's window fields (refused: not invariant);
- an index helper with a second early exit, and one whose `if` holds the access itself.

Each part lands as its own commit with its cells, a falsified plant, the census, and the A/B on
the four routines above (and every routine whose C changes, found by diffing the emitted C, not
by timing the unchanged ones).
