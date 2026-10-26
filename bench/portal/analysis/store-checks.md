<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The per-access store checks, priced (the gap sweep's findings 1 and 2)

The gap sweep (`handoff-2026-10-20.md`) named two runtime costs inlined into almost every
routine over 3×: `strict_stores()` inside `Stores::store` / `store_mut`, and the store's own
per-access tests (`offset_in_bounds` on every field access, the lock and init-shadow tests in
`begin_write_inner` on every write).  Each variant below is a RUNTIME change, so it was priced
by the rlib swap of `bench/portal/hand_price.sh`: the routine emitted once, compiled against
the base and the variant rlib, and the two binaries run interleaved (11 rounds, the first
dropped, pinned to the fastest core, median ns/op).  Every hash agreed with the base.

## What each variant answered (median change against base)

| variant | panel_build | truncate_to | cbor decode | forms_upto | binary_read | binary_map | worst |
|---|---|---|---|---|---|---|---|
| strict check REMOVED (ceiling) | −9.6 % | −11.4 % | −8.4 % | −4.6 % | — | — | insert_text +6 % |
| strict: `s.free` tested first | −2.2 % | −7.3 % | −1.2 % | +1.1 % | — | — | keys_near +7 % |
| strict: report behind one cold call | −2.9 % | −7.2 % | +3.4 % | +0.1 % | — | — | invert +61 % |
| strict: report diverges (`-> !`) | −3.6 % | −4.2 % | +0.4 % | −1.5 % | — | — | materialise +26 % |
| bounds + lock + shadow REMOVED (ceiling) | −16.4 % | −20.7 % | −11.6 % | −14.9 % | −15.2 % | −24.6 % | format_specs +3 % |
| lock + shadow removed, bounds kept | −9.0 % | −9.2 % | −6.5 % | −6.3 % | −11.2 % | −19.3 % | materialise +21 % |
| lock + shadow folded into one `write_slow` byte | −4.1 % | −4.4 % | −2.2 % | +3.3 % | +3.2 % | −9.1 % | locate +19 % |

None of the realisable variants was built.  The cold-call row repeats a measurement already on
record (`formal/rewrites-history.md`, R-ElementPath: outlining the strict report was slower).

## What the prices say

The tests themselves are cheap.  What costs is an out-of-line call on a path that REJOINS the
hot path: after it LLVM must assume the store's pointer and size changed, so it reloads them
and cannot keep them in registers across the loop.  The strict report, the lock refusal and
the shadow hook are all such calls.  Moving them out of line, testing a cheaper flag first,
or folding two flags into one leaves the call and keeps most of the cost.  It also moves
inlining, which is where the large one-routine swings come from (`invert`, `locate`,
`materialise`).  Equal instruction counts, with `n_slice_runs` stalled on a `DbRef` written as
two 4-byte halves and read back as one 8-byte load, are a store-forwarding stall:
`ld_blocks.store_forward` is the counter to read (INTERPRETER_PERFORMANCE.md).

Two cures follow, one per kind of check:

- **Debug switches (`LOFT_STRICT_STORES`, the init shadow) leave the shipped tier.**  Only
  removal pays.  The shipped and performance tiers (`--ship`, every library cdylib)
  would link a runtime built without them, and the semantics tier keeps them.  This is a
  decision for the owner: it is a second rlib in the distribution, and a lean binary must say
  so when the switch is set rather than ignore it.
- **The lock and the bounds stay, once per record or vector instead of once per access.**  The
  emitter already has the forms (`(R-PushFill)`'s window, `(R-RecPtr)`, the hoisted element
  reads).  Widening where they admit is the lever.  The first such widening, the window
  beside a scalar vector stored apart from the return buffer, took `binary_map` −41.5 %,
  more than the whole runtime ceiling above.
