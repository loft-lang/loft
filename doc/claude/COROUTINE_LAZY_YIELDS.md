# Coroutines — lazy loop yields (CL-9)

Part of [COROUTINE.md](COROUTINE.md), whose Known Limitations table named CL-9.  Its timeline,
and the lowering the retired rustc writer needed, are in
[COROUTINE_LAZY_YIELDS-history.md](COROUTINE_LAZY_YIELDS-history.md).

## The rule

**A `yield` returns control to the consumer BEFORE the next generator statement runs — in a
loop body no less than in straight-line code** (`(G-Next)`,
[formal/coroutines.md](formal/coroutines.md)).  The generator's observable step sequence is
`next()`-driven, one body slice per advance; a loop's cursor and every loop-carried local
persist across advances.  An endless generator consumed with an early `break` stops when its
consumer does.

## How each backend keeps it

- **The interpreter** serialises the whole frame at each `yield` and resumes from it.
- **The C backend** writes a generator as a frame struct, a creator, a resume function and a
  release ([C_BACKEND.md](C_BACKEND.md) § Generators).  The resume function restores the
  locals and jumps to the label after the `yield` that suspended, so a `yield` in a nested
  loop, in a branch or inside an expression (`x = i + { yield i; 3 }`) resumes where it
  stopped.  No shape takes an eager buffer.

## Guards

The interleaving, which a value-only comparison cannot see:
`loft-suite/rules/coroutines/G-Next/a-generator-resumes-at-the-yield-it-suspended-at.loft`
(control-flow shapes), `loft-suite/rules/coroutines/G-Next/a-record-yielded-from-a-loop-is-handed-over-lazily.loft`
(every channel), `loft-suite/rules/coroutines/G-Next/a-conditional-yield-in-a-loop-is-lazy-on-both-backends.loft`,
`loft-suite/attic/scripts/836-lazy-loop-yields.loft`,
`loft-suite/attic/scripts/1586-a-while-loop-generator-yields-before-its-next-iteration.loft`
and `tests/oracle/26-coroutine-laziness.loft`.  A record yielded from a loop is the value at
the `yield`: `loft-suite/attic/scripts/1356-a-record-yielded-from-a-loop-body-is-the-value-at-the-yield.loft`.
