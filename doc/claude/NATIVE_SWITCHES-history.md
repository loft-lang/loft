<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# NATIVE_SWITCHES-history.md — the timeline of [NATIVE_SWITCHES.md](NATIVE_SWITCHES.md)

[NATIVE_SWITCHES.md](NATIVE_SWITCHES.md) states what each switch does now; this file keeps how each got there.

## Older dated lines moved from NATIVE_SWITCHES.md on 2026-10-01

- **`LOFT_NO_VECTOR_BASE`.**  Since 2026-09-21 a mint left on its templates counts as growth: a
  base bound beside one answered `null` on native.  Since 2026-09-18 a null-discharge buffer's
  mint does not count.
- **`LOFT_NO_PUSH_WINDOW`.**  The window took RECORD mint groups from 2026-09-22.
- **`LOFT_NO_INVARIANT_HOIST`'s verify form** found loft#1534, a by-reference argument the
  non-sentinel proof's escape collector could not see.
- **The range proof (`LOFT_NO_RANGE_ARITH`).**  Its counted-range-counter clause was INERT until 2026-09-21 (loft#1558):
  the seeding looked for the counter's seed INSIDE the loop, so no counted loop was ever ranged.
  Nothing showed it, because the pin that should have (`range_arith` a4) was recording a plain
  form the CHAIN GUARD supplied.  The proof read the STATIC TYPE from 2026-09-22, once loft#1593
  made every store into a narrow slot refuse an unprovable value.
- **`LOFT_NO_VALUE_RECORD`** went default-ON 2026-09-14.  It was opt-in for two days because its
  call-site gate did not hold over the script corpus (376 compile errors, 218 of them the
  fn-ref DISPATCH); a `__lift_` temp bound from a CALL was declined by shape until
  `(R-ValueLocal)` made it a value local.  A `__lift_` temp bound from a VIEW read as a view
  leaf leaked the store its copy mints, one record per call; found 2026-09-15 with the generic
  instance's statement join, whose arms bind the join local the same way and did not compile.
- **`LOFT_NO_VALUE_LOCAL`** went default-ON 2026-09-25.
- **`LOFT_NO_VIEW_FIELD`** and **`LOFT_NO_FORWARD_TUPLE`** went default-ON 2026-09-17.
- **`LOFT_NO_ELEMENT_FIRST` / `LOFT_NO_ELEMENT_PLACE`.**  A read through an element view between
  the declaration and the append was loft#1553; a local declared `[]` and rebound under an `if`
  left the element empty (loft#1552).
- **Null-discharge buffers (`LOFT_NO_NULL_BUFFER_HOIST`).**  The buffer's allocation declined every header in the loop around
  it — the drawing bench's polygon crossing loop hoisted nothing; since 2026-09-18 it does not
  block the loop's BASES either.

## `LOFT_NO_FN_HEADER` — the function clause's first measurement

Measured on cbor's `read_value` (the byte reads of a decoder's head): the runtime vector
lookup fell from 7 % of the check profile to 2 %, decode 2.65 → 2.53 s per 60 driver rounds.

## `LOFT_NO_ELEM_FUSE` — what the middle rung showed

The unfused rung showed stage 2 (the fused element read) is worth ~3.2× on top of stage 1, the
header hoist (projected ~1.4×) — more than the hoist itself, because the second store
resolution it removes costs more than the arithmetic it saves.

## Measurements moved from NATIVE_SWITCHES.md on 2026-10-23

- **`LOFT_NO_GUARDED_CHAIN`'s profitability floor.**  Measured against the guard off, the
  one-operator loops in `pil_hline` and `matches_at` were costing `fill_circle` and `fill_star`
  ~50 %, `wide_line` 22 % and `parse` 18 %, while six-operator `composite_layer` gained 30 %.
- **`LOFT_GUARDED_CHAIN_ONLY`.**  Built because the drawing `parse` row lost 18 % to ONE of six
  admitted loops: a 3–8 trip loop in a per-byte helper, whose doubled body stopped inlining into
  its caller.
- **`LOFT_NO_VALUE_RECORD`'s site refusals.**  An element read at the site was measured wrong: the
  write landed in the container, reading 109 where the copy holds 9.  A disturbance between the
  bind and the last read was too: a removal made the result read the NEXT element's points.
- **`LOFT_NO_VALUE_RECORD`'s per-path proof** replaced a per-exit test that was measured wrong: 0
  points for 3, and 3,9 for 4,109.
- **The narrow-element scalar hoist** took `hex_edge` `edges_cut` 4.47× → 1.70× and `hex_place`
  `combine_cut` 4.13× → 2.16×.
- **The text-walk header hold** took the stdlib `join` 5.40× → 1.96× of Rust, `split_walk` 1.80× →
  1.01× and `parse_num` 1.46× → 0.98×.
- **The character walk's ASCII arm** took `char_walk` 4.62× → 2.48× of Rust, and the stdlib
  `split`, built on such a walk, 12.5× → 7.8×.
- **The small-record parameter clause** took mesh3d `mat4_transform` 9.9 → 2.64 ms (5.0× → 1.32×
  of Rust) and `sphere` −38 %.
- **`LOFT_NO_NULL_BUFFER_HOIST`'s setter fix** took stage's `pack_instances` 24.3× → 8.4×.
- **The bounded-sum blocks** took the stdlib `sum` 11.9 → 3.7 µs, 6.45× → 2.06× of Rust, pinned.
- **The loop-free `#[inline]` hint**, measured over the suite and three library benches (87
  routines, hashes identical): geomean −2.3 %, hex_field `edgeset_count` 23.1 → 10.1 ms (35.6× →
  ~15.6×), `join` −19 %, `replace` −18 %, `dot_product` −14 %; the largest loss, `char_roundtrip`
  +5 %, was function layout — both arms read the same once `-align-all-functions=6
  -align-loops=64` pins it.

## `LOFT_NO_BASE_RECPTR` — the address it does not derive (moved 2026-11-03)

An address derived from the element's `DbRef` instead of the index measured +46 % SLOWER, so
the clause derives it from the index; the `DbRef` form is not what ships.

## `LOFT_NO_VALUE_RECORD` — the first measurement (moved 2026-11-04)

Returning a no-heap record's fields in registers measured 1.65× on the call, `smooth` −25 %
standalone and `lock_curved` −3 %.

## `LOFT_NO_LOOP_RECORD` — a native generation-time switch until 2026-11-04

`(R-LoopRecord)` and its refill and nest clauses were rustc's (and the refill clause the C's
port) until @PLN211 moved them into the IR; the three switches now turn the IR pass's rule off
on every backend.  The nest clause measured `forms_upto` 761 → 638 µs on native.

## Switches of the rewrites retired with the rustc writer (@PLN225, 2026-11-24)

Each entry below named a switch only the rustc writer read; the writer and its rewrites were
removed by @PLN225 (the rules: [formal/rewrites-history.md § Retired with the rustc writer](formal/rewrites-history.md#retired-with-the-rustc-writer-pln225-r5)).
The text is NATIVE_SWITCHES.md's as it stood, under the section it stood in.

### From § Vector headers, pushes and mints

**`LOFT_NO_FN_HEADER=1`**
keeps `(R-Header)`'s FUNCTION clause off: a body that reads a vector parameter twice or more
and leaves it as it found it (`hoist::param_untouched`, a callee asked the same of the position
the parameter reaches, a recursion answering for itself) derives the header at every read
again instead of once at entry — the first bisect step for a wrong element read OUTSIDE any
loop under `--native`; `LOFT_TRACE_HOIST_DECLINE=1` names the parameter declined and the op
that touches it.  **`LOFT_NO_KEPT_HEADER=1`** keeps the KEPT clause off: a record parameter's
vector field (`m.chunks`) walked by a chain of callees, one of which appends to it, is derived
at every walk again instead of held from the outermost reader and handed down by `&mut` to
`__kh` twins — the first bisect step for a stale length or a missing element after an append
in a callee; `LOFT_HOIST_VERIFY=1` checks every holder read, `LOFT_TRACE_KEPT=1` names each verdict.

**`LOFT_NO_GROUP_PUSH=1`** (`@FR-R-GroupPush`, default-ON, generation time, `--native` only)
makes a record append OUTSIDE any held header keep its templates again — with it off, a
literal group `v += [pt(a, b), pt(c, d)]` that no enclosing loop holds a header for (a group
at function level, in a loop that declined, or on a vector declared per pass) binds a push
header of its own right after its reservation and emits its mints and finishes through it
(`fronds` built its 1 296 points per call that way: 101.7 → 69.6 µs with the two clauses
below, 2.71× → 1.86× of Rust) — and is the first bisect step for a wrong element out of a
literal record append on native; `LOFT_HOIST_VERIFY=1` re-derives the header at the slot and
the finish.

**`LOFT_NO_TWIN_INLINE=1`** makes a twin that reaches no loop (but a literal `0..k`, k ≤ 16)
take the plain `#[inline]` hint instead of `#[inline(always)]`: the A/B for a row that moved
with it (suite + ten `hex_*` benches, 102 routines, geomean −1.1 %, `mesh_emit` −18 %,
`draft_fit_p` −15 %; `char_roundtrip` +7 % is layout).

**`LOFT_NO_MOVE_APPEND=1`** (@PLN157 § V-j) makes `for f in call(…) { v += [f] }` keep the
deep copy — with it on, the call's buffer is PLACED as a record in `v`'s own store, the
append relocates the element's bytes (heap handles included — they never change store) and
zeroes the source, and the buffer's free is record-level — and is the bisect step for a
wrong element, a leak or a double free out of a loop that appends a dying temporary's
elements.  `LOFT_TRACE_MOVE=1` names the gate that declined a pairing.


### From § Text walks

**`LOFT_NO_TEXT_CLEAR=1`** (`@FR-R-TextClear`, default-ON, generation time, `--native`
only) empties a text with a fresh `String` again — with it off, `s = ""` on a declared text
local or a `&text` parameter is `s.clear()`, the buffer's capacity kept (zttext `materialise`
−59 %) — and is the first bisect step for a text that reads leftover content after it was
emptied.


### From § Vector headers, pushes and mints

**`LOFT_NO_TEXT_SET_BORROW=1`** (default-ON, generation time, `--native` only) makes every text
set into a store field copy its value into a `String` first — with it off, the setter reads the
value where it lives and `Store::set_str` / `refill_str` copy only a value whose bytes lie in
the destination store, the one store the claim can move.  First bisect step for a wrong text
written into a field on native; the falsifier is the store unit test
`a_text_from_its_own_store_survives_the_claim_that_moves_it`.


### From § Element bases and record addresses

**`LOFT_NO_FIELD_HEADER=1`** (`@FR-R-RecPtr`'s nested clause, `--native`) resolves a view's
VECTOR field (`len(c.items)`, `c.items[i]`) through the store again, not through the view's
address.  First bisect step for a wrong length or element of a record's vector field.

**`LOFT_NO_INDEX_RECPTR=1`** (`@FR-R-RecPtr`'s index clause, default-ON, generation time,
`--native` only) makes `r = v[IDX]?` over a vector whose header and base the frame holds read
its element through `get_vector_hoisted` and resolve the store for its address again — with it
off, IDX is named once before the statement and, in range, the view is the held record at
`8 + IDX * size` and its address the held base plus `IDX * size`, the binding running as
written out of range (graphics `fill_polygon` −12 % alone).  First bisect step for a wrong
field read through a `v[i]?` view on native; `LOFT_HOIST_VERIFY=1` checks the in-range read
against the runtime's own.


### From § Generators

**`LOFT_NO_REDESCENT=1`** (default-ON, generation time) sends every generator loop
that RE-DESCENT would lower lazily back to the eager buffer — the whole loop runs in the
factory before the first value is handed out — and is the first bisect step for a native-only
wrong answer, leak or rustc error out of a generator loop whose yields are not all on one
straight line (COROUTINE_LAZY_YIELDS.md § Re-descent).  The values must not move; the order of
side effects does, which is the point.  `LOFT_TRACE_REDESCENT=1` names each generator that
still runs eagerly with the switch off, and why: a `yield` inside an expression re-descent
cannot hoist, or a carried local no struct field can hold.


### From § Guarded reads and integer arithmetic

**`LOFT_NO_PUSH_RUN=1`** (`@FR-R-PushFill`'s run clause, default-ON, generation time,
`--native` only) pushes a straight-line run of byte appends (`v += [a]; v += [b]; …`) one by
one through the runtime again — with it on, the run's bytes are built in a stack array and
appended by one `vector::append_bytes` (cbor's `head`: `encode_bytes` −10 % over the earlier
one-window form).  The first bisect step for a wrong byte or length after such a run; cells
`tests/scripts/158-push-run.loft`.

**`LOFT_NO_TYPED_READ=1`** (`@FR-R-TypedRead`, default-ON, generation time, `--native` only)
sends every `f#read` through the generic `OpReadFile` again — with it off, a read that lands a
fixed-width integer whole in an `integer` local (`f#read(2) as i16`) calls
`OpReadFileInt::<W, SIGNED>`, width and sign fixed at the site and the bytes taken by value
from the file's buffer (`binary_read` 2.54 → 1.91 ms).  The first bisect step for a wrong
value or a wrong `#next` out of such a read; the interpreter's read is the reference.

**`LOFT_NO_STREAM_WALK=1`** (`@FR-R-StreamWalk`, default-ON, generation time, `--ship`
and every build that does not ship the live tier) builds every vector a caller walks once —
with it off, a function answering a vector of records that its caller binds and walks by
index runs as its stream twin, handing each element's fields to the walk's body at the
append, and no vector is built.  First bisect step for a wrong value, order or count out of
such a walk on a release build; **`LOFT_TRACE_STREAM_WALK=1`** names every producer and walk
admitted or declined, with why.

**`LOFT_NO_BOUNDED_NEST=1`** (@PLN157 `R-BoundedNest`, `@FR-R-BoundedNest`, default-ON,
generation time, `--native` only) keeps every tap nest checked — with it off, an innermost
counted loop that is one accumulate over `?`-discharged element products
(`acc += a[(yy*w + x)*4 + ch]? * k[base + x]?`, a resample's tap) runs with PLAIN operators
behind a guard evaluated once at the loop's entry: no range end or invariant is the sentinel,
every vector read has a known element bound (taken ONCE at the outermost loop whose body
leaves the vector alone, never at the nest's own prelude), and the magnitude bound of every
chain and of `|acc| + trips × bound(term)` fits, so no operation can fault and the plain
answer is the checked one; the checked loop is the `else` arm.  It is @C120's admissible
successor built — the fact is established before the arithmetic runs, not assumed — and it
took the drawing lane's three resample rows from 4.9–6.1× to 2.4–2.8× their Rust reference
(`render_marks` −54 %).  **Step 2, `LOFT_NO_NEST_RAW_READS=1`** keeps the arm's bounds-tested
reads — with it off, where every read's chain names the counter at most once (affine) the guard
also proves both range ends in `[0, len)` and the arm reads RAW through the held base with no
null select (the bound already ruled out a stored null): the tap becomes the multiply-accumulate
LLVM vectorises (`render_marks` −40 % again, to ~1.7×).  Both are the first bisect steps for a
wrong accumulate or index out of such a loop on native; `LOFT_HOIST_VERIFY=1` compares every
plain operator's answer with the checked template's and every raw read with the checked read,
panicking on a disagreement; `LOFT_TRACE_NEST=1` names every admission and decline and whether
the reads are raw.

**`LOFT_NO_FAULT_FLAG=1`** (`@FR-R-FaultFlag`, default-ON, generation time, `--native` only)
runs every `for` statement as written — with it off, a statement that writes only scalar locals
first runs a fast copy, each fault form setting a flag, and reruns as written when it is set —
the first bisect step for a wrong value out of such a loop; `LOFT_TRACE_FAULT_FLAG=1` names each,
and `LOFT_NO_FAULT_FLAG_UNROLL=1` keeps the fast copy's constant ranges as loops.


### From § Records returned and passed as values

**`LOFT_NO_REFILL_BUFFER=1`** (`@FR-R-RefillBuffer`, default-ON) frees the store a rebind
exchange released and mints every return buffer — with it off, a callee whose buffer is built
by a complete literal of a refillable type takes that store, holding the rebound variable's
previous value, and empties its vector fields in place.  The first bisect step for a wrong
record built into a return buffer on `--native`.  Read where the buffer is emitted and at run
time.

**`LOFT_NO_REBIND_BUFFER=1`** (`@FR-R-RebindBuffer`, default-ON) exchanges every rebind's
fresh result into the destination's store (`@FR-H-SwapRebind`) — with it off, `x = f(…)`
adopts the fresh result and hands `x`'s previous store to the call's hidden buffer for its
next execution.  The first bisect step for a wrong value after a rebind in a loop on
`--native`.  Read where the rebind is emitted; `LOFT_TRACE_REBIND_BUFFER=1` names each
rebind it declines and why.

**`LOFT_NO_REFILL_TEXT=1`** (`@FR-R-RefillText`, default-ON) keeps a pooled call buffer's
release walk before every call and the plain callee, instead of the callee's `__rt` twin
writing each text over the block its slot already owns.  The first bisect step for a wrong
text, or a store that grows, read back from a record-returning call in a loop.
`LOFT_TRACE_REFILL_TEXT=1` names each pooled site's verdict (`refill-text: <caller> →
<callee> admitted`, or the condition that declined).  Guard
`tests/scripts/a-pooled-buffers-texts-are-refilled-in-their-slots.loft` (its `m1` is the
live-record census that sees a text leaked per call); pin `tests/refill_text.rs`.

**`LOFT_NO_REFILL_ELEMENTS=1`** (`@FR-R-RefillText`'s collection clause, default-ON)
releases a heap-element refill buffer whole at entry and claims every element's texts anew,
instead of keeping the elements and refilling their texts in their slots.  The first bisect
step for a wrong text, a panic in a text set, or a store that grows, in a vector a function
builds by appending records and returns.  `LOFT_TRACE_REFILL_TEXT=1` names each such
buffer's verdict.  Guard `tests/scripts/a-kept-buffers-elements-are-refilled-in-their-slots.loft`.

**`LOFT_NO_REFILL_VECTORS=1`** (`@FR-R-RefillText`'s vector clause, default-ON) releases a
pooled call buffer whose heap is vectors of plain elements before every call again, instead
of leaving its callee to empty each vector field in place.  The first bisect step for a stale
or wrong vector field, or a store that grows, in a record a call builds in a loop.
`LOFT_TRACE_REFILL_TEXT=1` prints `admitted (vectors)` for each site it takes.  Guard
`tests/scripts/a-pooled-buffers-vectors-are-emptied-in-place.loft`.

**`LOFT_NO_REFILL_IN_PLACE=1`** (`@FR-R-RefillBuffer`'s in-place clause, default-ON) empties
and refills a refilled buffer's repeat-literal vector field every time — with it off, a kept
vector that already holds the literal's count is overwritten where it stands
(`Stores::fill_exact`).  The first bisect step for a stale element in a vector a refilling
callee starts as `[c; n]`.  Read where the literal is emitted.


### From § Element-first builds, complete writes and return buffers

**`LOFT_NO_SPARE_RESTAMP=1`** / **`LOFT_NATIVE_SWAP_KEEPS_HANDUP=1`** (`@FR-H-FreeTwice`, runtime, rustc
`--native`) are the bisect steps for a store a fn-ref call's buffer leaves unreleased or releases twice: the
first keeps a kept spare's old allocation stamp when a callee builds in it (the result then reads as borrowed
and nobody releases it), the second leaves an exchanged source on the fn-ref hand-up list (released twice).

**`LOFT_NO_VIEW_RETURN=1`** (`@FR-R-ViewReturn`) makes a lookup that returns a view of its
parameter's entry materialise it again — a buffer minted per call, the entry deep-copied — and
is the first bisect step for a freed, leaked or wrong lookup result on native;
`LOFT_TRACE_VIEW_RETURN=1` names each call site's admission or declining condition.


### From § The lean tier: frameless call trees

**`LOFT_NO_LEAF_CHAIN=1`** keeps those frames (one step finer than `LOFT_NO_LEAF_PRELUDE`);
the named tiers never elide a non-leaf.


### From § Library artifacts

**`LOFT_NO_LIB_IDS=1`** writes the generating program's numbers as literals again: the A/B for
a library routine that moved, and the falsifier of `n2_cdylib`'s renumbered-program guard.

`LOFT_TRACE_INVARIANT=1` named each memo of `LOFT_NO_INVARIANT_HOIST`'s rewrite in the writer.

### From § The lean tier: frameless call trees

**A frameless call tree carries no prelude (@PLN157 row 11, `@FR-R-LeafChain`, lean tier
only, generation time):** N4's leaf rule made transitive — a function whose every user
callee is loft-bodied, off any cycle and free of fn-refs drops its depth entry and buffer
guard in `--ship`, since there the frame is only a depth count (parse row −12–14 %).


### From § Inlining hints (the writer's `#[inline]` form)

**Every loop-free loft function carries `#[inline]` (default-ON, generation time, both native
tiers).**  The hint only raises LLVM's inlining threshold for that function; a small helper
whose checked arithmetic pushed it just past the default budget now folds into its caller the
way rustc folds a plain-Rust twin's.  A function that runs a loop gets no hint: pulled into a
caller that loops itself, its loop-carried values spill across the caller's live range
(`newton_sqrt` inlined through `roots` into `main` kept `guess` on the stack across its divide
chain, +16 %).  **`LOFT_NO_INLINE_HINT=1`** emits no attribute (the emission of before) and is the
A/B for a row that moved after a rebuild.  `--names` pins every loft function
`#[inline(never)]` regardless.  One home: `Output::fn_inline_attr`.
