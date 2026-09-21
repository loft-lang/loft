<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Native rewrites — the emitter's cheaper forms, and what each one assumes

**Scope.** The forms the NATIVE emitter (`src/generation/`) substitutes for the
template emission when a side condition holds: a vector header derived once for a
loop, a record scalar read once, a view's header taken at its binding, a stdlib
wrapper emitted as its op, a callee handed the values its caller already holds.
Each is a REWRITE — the program's observable behaviour is unchanged — and the
interpreter applies none of them, which is what makes it the oracle for every cell.
The runtime-side units of the same arc (the per-allocation and per-record
bookkeeping, the append path) are not rewrites and are not here; the IR lowerings
both backends share (the fused scalar append, append in place) belong to the
chapters of the constructs they lower.

**Why a chapter.** Every rewrite rests on ONE invariant, and until now each lived
as prose in its own design section (`doc/claude/plans/157-native-4x-drawing/
DESIGN.md`), with no name a code site could cite.  Measured: `src/generation/
hoist.rs` cited zero rules while enforcing eight, and the first defect of the
family — a user function inlined as a stdlib wrapper (D-rw-1 below) — was the
rule's own qualifier ("stdlib") that the code had never asked.  A rewrite that
COMPOSES others (R-Inputs takes R-Callee's admission, R-Scalar's and R-Header's
candidates, and moves them through a call) needs them as citation targets.

**How a rule is written.** Shape ⟹ emitted form, under a side condition; then the
switch that restores the previous emission and the falsifier that catches a stale
assumption.  A site enforcing a rule cites its `@FR-R-…` tag
([README.md § Rule tags](README.md)).

## Rules

### Every rewrite is switchable and falsifiable

```
  (R-Switch)     a rewrite ships with a GENERATION-TIME switch that emits the previous
                 form, and with a falsifier: a checking form that re-derives what the
                 rewrite assumed and panics when it is stale (LOFT_HOIST_VERIFY=1), or
                 an emission pin where there is nothing to re-derive.  The interpreter
                 applies no rewrite and is the oracle of every cell.
  (R-Escape)     the contract is SEMANTICS — what a program computes and can observe —
                 never a representation: how many stores or copies a value takes, or
                 where it lives, is the compiler's to change wherever the rule's
                 CONDITIONS are validated on the IR, and where they cannot be the
                 rewrite declines to the unrewritten form.  The one boundary is a
                 library's exported API, whose callers the compiler cannot see: a
                 construction that ESCAPES the unit — answered to, stored by, or
                 handed to code outside it — keeps the representation the API
                 promises, and the boundary materialises whatever an internal rewrite
                 made of it; one that does not escape may be rewritten in any way its
                 conditions allow.  (C122, owner 2026-09-15.)
```

**`(R-Escape)` in words.** Every rule below asks for permission from nobody: it states
the conditions under which a program cannot tell the rewritten form from the written one,
and the compiler validates those conditions or declines.  What it may never do is change
a value, a fault or an effect (C120 is the same principle from the other side), or hand a
representation of its own choosing across a library's API, where the callers that would
have to validate the conditions are not in the compilation.  `(R-ValueRecord)`'s bridge
clause is the existing instance — a value tuple inside the library, the promised record
at the boundary — and `(O-ViewField)` takes its scope from here.  A user PROGRAM is a
closed unit (every use is in the compilation; a fn-ref call is a match over known
definitions), so every condition is decidable in it and the only boundaries left are
where a representation is turned back into the promised one: a call into a `use`d
library, the live-reload arm, and a record's layout in a store (C122's corollary).
And the UNIT is a build decision: a release copy of a program (`--native-release`)
emits its `use`d loft libraries' reachable functions into the one program it compiles,
so there a loft-to-loft library API is no boundary at all — a library is recompiled
with the game rather than reused as a binary — and what remains is a package's `#rust`
native (a C ABI), the live-reload arm, a stored record's layout and a placed library
(PLACEMENT.md § 4 is this rule at the wire).  A library's own published cdylib is the
build in which its API keeps the promised representation for callers it cannot see.

**In words.** The switch is the before-half of an A/B on one binary and the first
bisect step for a native-only wrong answer; the falsifier is what makes "the values
agree" a proof rather than a coincidence — a header-served read and a runtime read
of an unmoved vector answer the same number, so agreement alone cannot tell a sound
rewrite from a lucky one.  `PERFORMANCE.md § Design: P2` and `NATIVE.md` list the
switches; `hoist_verify` in `Output` is the one flag every checking form reads.

## The hoist state — what the rewrites compose through

A loop's rewrites do not compose pairwise.  They compose through ONE object, the loop's
HOIST STATE: the map from a vector path to the holder the loop keeps for it, plus the record
scalars it read once.  Each rewrite below is a claim on that state — what it may add, what it
may read, what it must keep current — and the three rules here are what make a combination of
rewrites decidable by a lookup instead of by the order their collectors happened to run in.
Measured: the first emission of a push loop beside a twin call gave one path TWO holders,
because the push collector ran before the twin-input collector; the verifier caught it and
the ordering closed it, but nothing in the per-rewrite rules had said it was wrong
([rewrites-history.md](rewrites-history.md)).

### One path, one holder

```
  (R-State)      a loop's hoist state maps each vector path to AT MOST ONE holder per
                 frame — a plain header (R-Header), a push header (R-Push) or a value
                 handed in from outside (R-Inputs, a twin's parameter) — and every read,
                 write, length and push of that path in the loop serves from that
                 holder.  A nested loop re-uses an enclosing frame's holder for the
                 same path rather than deriving a second; a view of a held path copies
                 the holder (R-View).  Two holders for one path is a defect whatever
                 the values say.
```

**In words.** The holder is the ONE definition of the path's `(store, record, length)` for
the loop; a second one can only ever be the same or stale, and stale is silent.  The state
is built by `hoist::hoistable` — the read candidates, § V-p's callee inputs, then the push
paths, which REMOVE themselves from the read list — and `Output::begin_vector_hoist` binds
one local per entry; today the rule is held by that order, and the closure by construction is
the state-builder refactor queued in @PLN157 (one map, one insert path).  Sites:
`hoist::hoistable` (the push paths leave the read list), `Output::begin_vector_hoist` (the
frames), `Output::bind_view_header` (a view copies a held path's holder).

### The op that changes a held fact refreshes the holder at its own site

```
  (R-Refresh)    a held fact — a header's record and length, a push header's capacity,
                 a hoisted scalar — may be changed inside the loop ONLY by an op that
                 the gate admitted for that purpose, and that op refreshes the holder
                 at its own site before anything else can read it: a push bumps its
                 header's length and re-derives the whole header after a growth; a
                 scalar hoist is evicted at analysis time by the (type, offset) its
                 writes reach (R-Scalar).  An op that could change a held fact and
                 does not refresh it blocks the loop.
```

**In words.** This is why a push may be admitted and an `OpAppendVector`, a remove or a
resize may not: not because they grow, but because only the push's emission refreshes what
it changes.  The refresh includes the RECORD, not only the local: the length is written
back per push so a runtime reader inside the loop (an admitted callee's `len(v)`) sees every
push.  Sites: `Stores::push_hoisted` (bump, write-back, re-derive), `hoist::WriteSet::evicts`
(the scalar half).

### Two paths may name one vector only where ownership cannot rule it out

```
  (R-Alias)      a rewrite that MOVES a held vector (a push's growth) is admitted beside
                 other holders only when no other held path can name the same vector,
                 decided by OWNERSHIP: a root that is a local owning its store (its dep
                 list empty or naming only its own `__vdb_N` witness, not a `&` link,
                 not captured, not a parameter) or the function's return buffer is
                 EXCLUSIVE; two exclusive roots are two stores.  A parameter cannot
                 alias an owned local but can alias a return buffer the caller offered
                 (§ V-d); a `&` link or a view can alias anything.  A single moving
                 rewrite beside no other holder has nothing to alias with and is
                 admitted whatever its root.
```

**In words.** The ownership facts are the ONE spelling (`@FR-O-Proxy`, the `__vdb`
witness, `is_argument`, `is_captured`, the hidden return-buffer attribute); this rule reads
them and no rewrite re-derives them.  A rewrite that fails it declines the WHOLE loop — a
mover left to its template would move a record a kept holder still describes.  Today only a
push moves; the rule is written for the next mover too.  Sites: `hoist::owned_local`,
`hoist::retbuf_var`, the admission block in `hoist::hoistable`.

### A vector reached by a pure path has one header for a loop that cannot move it

```
  (R-Base)       in a loop that GROWS no store — no push, no mint push; in-place
                 sets, store-free ops, store-free or in-place-only callees, frees
                 and a null-discharge buffer's mint (a FRESH store, or a clear of
                 the buffer's own — neither moves an element any header names;
                 admitted 2026-09-18, the crossing loop's `pg_table[i]?` had kept
                 its loop on headers alone) may run — a
                 hoisted header (R-Header) is accompanied by the address of its
                 vector's element 0, derived once beside it, and every fused element
                 read and write of the path in the loop is one bounds test against
                 the header's length and one load or store through that address.
                 The base is the header's address, never a second derivation of the
                 path (R-State counts it with its header), and an inner growth-free
                 loop may derive one from a header an enclosing, growing loop holds,
                 for the inner loop's extent alone: the enclosing loop's growth
                 happens outside it.  A base is valid exactly while no store's buffer
                 is reallocated, which is what "grows no store" secures; the verify
                 form re-derives it at every use.  A callee TWIN (R-Inputs) takes,
                 beside each header input, that header's base — the caller's held
                 base where its loop holds one, else a base derived from the held
                 header at the call — and every fused read and write of the path
                 inside the twin, a view of the path (R-View) included, goes through
                 it: an admitted callee (R-Callee) is store-free or writes only in
                 place, so no store reallocates for the call's duration, which is the
                 same condition the loop's base rests on.

  (R-Counter)    a counted range's counters — its `#index`, the `next` counter of a
                 computed start, and the loop variable — are never the sentinel: an
                 exclusive range steps its counter to at most its end, so the step
                 cannot overflow, and an inclusive range is admitted when its end is
                 a literal below the type's maximum.  The counters are seeded into
                 the non-sentinel proof from the one parser that reads a range's
                 shape, so the step emits as the non-null checked add and every
                 index built from the counter alone loses its operand pre-tests.  The
                 INTEGER CLOSURE — the result of `+`/`-`/`*` over non-sentinel
                 operands counted non-sentinel — is deliberately NOT taken: C85 makes
                 an overflow's sentinel propagate onward as null on both backends,
                 and a native closure would answer a number there, a divergence after
                 a reported fault.  Its price is measured (§ V-aj), and it is the
                 owner's line to move.  THE ITERATION CLAUSE: the `#index` of a
                 `for e in v` loop is a counter of the same kind — it starts at the
                 parser's literal seed, its one step is the head's `idx + 1`, and
                 the loop's own bound `if len(v) <= idx { break }` stands between
                 every two steps, so it never exceeds a vector's length (a u32)
                 plus one, whatever the body does to the vector.  It is seeded
                 when the loop's first two statements are that head and that bound
                 over ONE vector operand, nothing else in the loop assigns it,
                 every other assignment in the function is a literal, and its
                 address is never taken.

  (R-LitDiv)     a division or remainder by a LITERAL that is neither 0 nor -1 emits
                 as one sentinel test and the plain operator — `if x == MIN { MIN }
                 else { x / k }` — which is the guarded template's exact value: the
                 null a `/` mints comes from a zero divisor, from `MIN / -1`, or from
                 a null operand, and the literal rules out the first two at
                 generation time.  It needs no proof of the dividend, which is what
                 lets it fire on an arithmetic result the proof never trusts; the
                 template's fault note is dropped with it, since it fires only when
                 the result is the sentinel while neither operand is.

  (R-LoopBuffer) a per-site vector buffer (`__vdb_N`, the store a vector local
                 declared `[]` is backed by) whose mint stands INSIDE a loop keeps
                 its store and its vector across iterations: the first pass mints,
                 every later pass resets the vector's length to 0 and keeps its
                 record and capacity, and the literal's zero of the vector field is
                 not emitted.  Observably the same vector as a fresh one — a read is
                 bounded by the length, a push writes from 0 — with the capacity
                 retained, as a Rust `Vec` cleared in a loop retains its own.  Admitted
                 only where the record is exactly one vector of elements that OWN NO
                 HEAP (a length reset releases nothing; the clear is what releases
                 what an element owns, `@FR-H-ClearRelease`), where every mention of
                 the buffer is its own init family or a free (a callee reaching the
                 buffer could keep a handle into the record the reuse keeps), and
                 where the buffer's null declaration stands outside the loop (a
                 re-declaration would orphan the kept store).  A buffer another
                 rewrite owns — an invariant literal, an element-first pair, a move
                 host, the adopted result's witness — is left to that rewrite.

  (R-PushFill)   a counted loop (`for … in lo..hi`, `lo..=hi`) whose body pushes k
                 scalars to ONE pure path at its top level every iteration — nothing
                 in the body can leave the loop early or loop again, no push to the
                 path stands under a branch, no other write reaches the path, the
                 range's end is a simple invariant — RESERVES k times its trip count
                 once before it runs, over the push header the loop holds, and
                 re-derives that header (the reserve may move the record).  Observably
                 nothing: a reservation is capacity, and the pushes write as before.
                 When the body is that one push of a SIMPLE INVARIANT and nothing
                 else, the loop is ONE fill of the vector's tail — the room reserved,
                 the elements written with one bounds check at each end, the length
                 bumped once — guarded so that a range the fill declines (empty,
                 negative, an absent owner, an element width not the value's) runs the
                 per-element loop instead, and the counters are left as that loop
                 would leave them (`R-Fill`'s own tail).  The trip count is the range's
                 end less its start — the `next` counter's current value or `#index +
                 1` — plus one for an inclusive range, taken at loop entry.

  (R-Invariant)  an integer chain — `+`, `-`, `*`, negation, `&`, `|`, `^` (their
                 `Nullable` twins included) over literals and variables — that a loop
                 neither REBINDS (a `Set` or `TuplePut` anywhere in the loop, its own
                 counters included) nor lets ESCAPE (a bare or `OpCreateStack`-spelled
                 argument to a by-reference parameter or a fn-ref call, a tuple
                 destination, an iterator variable) holds one value for the loop's whole
                 extent, so it is evaluated at its FIRST use and answered from a memo
                 after.  The first-use evaluation is exactly the per-use one: the same
                 value on every path, the overflow note fired at the point the first
                 evaluation stands (once, where the per-use form notes once per use; a
                 zero-trip loop notes nothing).  The memo is declared at the INNERMOST
                 loop that spells the chain, so its flag is clear on every entry and the
                 test peels out of the loop.  A shift, a division and a remainder are
                 not chain ops (their templates raise through `stores`); a chain of
                 literals alone is the constant folder's; a field read is not a leaf (a
                 callee could write the record — R-Scalar answers that question); a body
                 that yields or runs arms in parallel memoises nothing.  Switch
                 `LOFT_NO_INVARIANT_HOIST`; falsifier `LOFT_HOIST_VERIFY=1` (every use
                 re-evaluates the chain and compares).  Sites: `hoist::invariant_chains`,
                 `hoist::arith_chain`, `non_sentinel::collect_escapes` (the one home for
                 the escape question, the proof's and this rule's), the emitter's
                 `begin_vector_hoist` and `emit_invariant_use`.

  (R-Header)     in a loop body that writes no store, a vector reached by a PURE
                 PATH P — a variable, or const-offset fields over one — has one
                 header (store, record, length) for the whole loop: the emitter
                 derives it once before the loop, and every element read, element
                 write and len(P) in the body uses it (R-State).  A rebind of P's root
                 inside the body removes P; a nested loop's prelude skips a path an
                 outer one already holds.
```

**In words.** loft#885 stage 1 (the header) and stage 2 (the element read fused onto
it), with @PLN157 P4d's path key (`lay.px` is `(lay, [32])`) and the hoisted loop
bound (`len(P)` served by the header, queue item 1).  The path is pure, so evaluating
it once in the prelude is exactly the evaluation the body repeats.  Switches
`LOFT_NO_VECTOR_HOIST` (no header at all) and `LOFT_NO_ELEM_FUSE` (the header, but
the element read unfused); falsifier `LOFT_HOIST_VERIFY=1`, whose checking form
re-derives the header at every access.  Sites: `hoist::vector_path`,
`hoist::vector_candidates`, `Output::begin_vector_hoist`, the registry's
`FusedElementReadEmitter`, `FusedElementWriteEmitter` and `HoistedLengthEmitter`.

### A record local declared in a loop keeps its store

```
  (R-LoopRecord) a plain no-heap record local declared by a statement INSIDE a
                 loop and minted by a literal (`sub = Spec {…}` per pass) keeps
                 its store and its record across the loop's iterations: it is
                 declared at the loop's prelude, the per-pass mint is taken only
                 where the local holds no store (the first pass), a complete
                 literal writes every field over the kept record, a partial one
                 re-establishes the omitted fields' defaults first, and one free
                 follows the loop.  Observably the same record as a fresh one.
                 Admitted where the local's every mention is its init family —
                 the declaration, the mint, a fusable scalar field read or
                 in-place write, a free — or a hand-off to a loft callee whose
                 return does not borrow THAT parameter (O-Borrow: a callee keeps
                 a caller's record only by copy or through its return's deps,
                 which name the parameters they alias by index; a record
                 function's hidden buffer is named there too and is no borrow of
                 the argument); a heap-owning
                 or nullable type, a second binding, a copy to another local, a
                 return, a capture, a native op taking it otherwise, `par` and
                 `yield` decline.  A free on a `return`/`continue` path stays.
```

**In words.** 2026-09-18, the `(R-LoopBuffer)` shape for a record.  A record literal
bound inside a loop cost the free of its store at the body's end and a fresh store next
pass — `free_named`, the free-slot search, the re-init, the claim, the zero, the tag: 39 ns
per pass against 10 ns for a vector loop buffer's reset — where a local declared outside
the loop already kept its store through `OpDatabase`'s clear arm.  Measured on the probe
(`s = S1 { a: k }; t += s.a`, 200 000 passes): 7 600–8 500 → 480–540 µs per 200 000 passes, 39 → 2.4 ns per pass (the kept record's field write and read no longer resolve a store per pass either), value unchanged.  Switch `LOFT_NO_LOOP_RECORD`
(and `LOFT_NO_LOOP_BUFFER_REUSE`, one family); trace `LOFT_TRACE_LOOP_RECORD=1`; falsifiers
`LOFT_STRICT_STORES` / `LOFT_POISON` / `LOFT_POISON_CLAIM` / `LOFT_NATIVE_LEAK_CHECK` (a kept
store must still be freed exactly once).  Sites: `hoist::loop_records`, the `Value::Loop`
emission in `emit.rs` (prelude and postlude), `output_block` (the two dropped statements),
`ops::misc_ops` (the kept-record mint).  Cells `tests/scripts/157-loop-record.loft` l1–l15,
pins `tests/loop_record.rs`.

### An in-place scalar set disturbs no header

```
  (R-InPlace)    a fixed-width scalar set through an EXISTING address — an element,
                 a record field, a local — moves no record and changes no length,
                 so it cannot invalidate a header: a loop whose store writes are all
                 such sets keeps (R-Header).  What may run in such a loop is an
                 ALLOW-LIST (the store-free ops, IN_PLACE_SET_OPS); an op missing from
                 it costs the rewrite, never correctness.  THE COPY CLAUSE: a
                 flag-free OpCopyRecord(src, dst, tp) over a record type that owns no
                 heap is the same write at a larger width — size(tp) bytes stored
                 at dst's address, nothing claimed, released or moved — and is on
                 the list; (R-Scalar) reads it as a write of tp WHOLE, and
                 (R-RecPtr) reads a copy FROM a view as a read of it.  A copy of a
                 heap-owning type walks claims, and a flagged one frees its source
                 or marks a fresh destination: both stay store writers.
```

**In words.** @PLN157 P4a.  Aliasing is free for headers under this rule — an
in-place write moves nothing, so no header, aliased or not, can go stale; that is
why the header needs no write set while a scalar (R-Scalar) does.  Switch
`LOFT_NO_WRITE_HOIST`.  Sites: `hoist::IN_PLACE_SET_OPS`,
`hoist::blocks_header_hoist`.
**The hidden-buffer allowance** (@PLN157 § V-ad): the allow-list also admits
`OpDatabase`/`OpDatabaseNP` into a null-discharge buffer — the hidden `__ref_p2_N` that
`e = tbl[i]?` mints an ABSENT record element into — when the record is all-scalar.
The allocation takes a store of its own from a null slot or clears the buffer's OWN
store, and that store hosts no vector, text or reference, so no header can name
anything in it; the field sets that follow are (R-InPlace) sets and walk on their own,
and the scalar tier reads the allocation as the record's type whole (R-Scalar).  Only
the pass-2 discharge buffers qualify: a `__ref_N` work-ref may be a return buffer, and
a return buffer may be a record the caller offered (R-Callee's second half carries
that case).  Switch `LOFT_NO_NULL_BUFFER_HOIST`; falsifier `LOFT_HOIST_VERIFY=1`.
Site: `hoist::null_buffer_alloc`.
**The copy clause** (2026-09-21, @PLN158 R5).  A consumer writes the copy-out / mutate /
write-back a Rust or C author writes — `e = ents[i]?; e.energy += e.speed; …; ents[i] = e`.
In loft `e` is a VIEW of `ents[i]` (`(B-View)`), so the last statement copies the element
onto itself, which both backends already make a no-op (`data == to`).  But the whole-record
copy was an unclassified store writer: the loop held no header, `e` no address (*"the
remainder may grow a store"*), and each of the ~20 field accesses per pass resolved the
store.  Found by a five-variant one-statement bisect under `LOFT_TRACE_RECPTR=1`; priced by
deleting the write-back (−26 %); built, `entity_tick` 205.4 → 157.7 µs (−23 %), 3.19× →
2.45× of the Rust reference (4.67× before this arc).  The copy itself STAYS.  Eliding it
statically would need more than the alias proof: on the path where `ents[i]?` discharged an
absent element, `ents[i] = e` is an out-of-range store with a fault note of its own, and a
rewrite does not decide what is reported after a fault (C120).  Two cells could not fail
when first sabotaged and were replaced by ones that can: with the copy contributing nothing
to the write set, a loop that also discharges with `?` stayed green — the discharge buffer
is itself a whole-type write of the same type and evicted the scalars on the copy's behalf —
while `c3b`, where the copy is the loop's ONLY whole-type write, answers `195 1 40` for
`255 1 40` and `LOFT_HOIST_VERIFY=1` panics *"hoisted record scalar is stale (hoisted 30,
now 40)"*.  Switch `LOFT_NO_COPY_IN_PLACE`.  Cells `tests/scripts/158-copy-in-place.loft`,
pins `tests/copy_in_place.rs`.  Sites: `hoist::in_place_copy`, `hoist::blocks_header_hoist`,
`hoist::body_writes`, `hoist::view_extent_verdict`.

### A callee is admitted by what its body writes, one call deep

```
  (R-Callee)     a user callee whose store writes are all (R-InPlace) sets — through
                 any address, transitively through callees it admits the same way —
                 or whose only writes land fixed-width scalars in its own hidden
                 return buffer, is admitted under (R-InPlace) as a direct set is.  A
                 CallRef, a Parallel, a Yield and a recursion keep the WRITER
                 verdict.
```

**In words.** @PLN157 § V-l (`in_place_only_writer`) and § V-c
(`retbuf_only_writer`): the P4a argument applied to a whole body.  The arguments
still walk below the call node, so a growing op inside one blocks on its own.
Switches `LOFT_NO_INPLACE_CALLEE_HOIST`, `LOFT_NO_RETBUF_HOIST`.  Sites:
`hoist::in_place_only_writer`, `hoist::retbuf_only_writer`,
`hoist::call_writes_store`.

### A record scalar the body cannot write is read once

```
  (R-Scalar)     under the (R-InPlace) gate, a scalar field read v.f — v a plain-
                 struct record variable the body does not rebind — holds one value
                 for the whole loop unless the body's WRITE SET reaches
                 (type(v), f): every (record type, offset) its in-place sets target,
                 what an admitted callee writes (R-Callee) — an in-place-only
                 writer's own set, a return-buffer writer's record type WHOLE —
                 and the whole type of any record the body frees.  A write the walk
                 cannot type empties the loop's scalar candidates.  Keys carry the
                 TYPE, never the variable, because aliasing is decided by what a
                 write can reach.
```

**In words.** @PLN157 P4c.  The getter's own `#rust` template is emitted once in
the prelude — the `rec == 0` sentinel included — so the local holds exactly what a
per-iteration read would.  A `vector<integer>` element written at offset 0 reaches
no record's field; a `&`-bound alias, an element view and a callee's parameter all
write the same `(Lay, 0)`.  Switch `LOFT_NO_SCALAR_HOIST`; falsifier
`LOFT_HOIST_VERIFY=1` (`hoisted_scalar_verify` re-reads and compares).  Sites:
`hoist::hoistable`, `hoist::body_writes`, `hoist::callee_writes`,
`hoist::WriteSet::evicts`, `hoist::setter_target`, the registry's
`emit_hoisted_scalar_or_default`.

### A view's header is taken at its binding

```
  (R-View)       a view d = &P bound from a pure path P fixes d's DbRef, so d's
                 header may be derived once, right after the binding, and serve
                 every d[i] and len(d) in the rest of the block — when the remainder
                 passes (R-InPlace), never rebinds d, and indexes d at least once.
                 A rebind of P's ROOT afterwards does not matter: d keeps the DbRef
                 it was given.  Where P is already held by an enclosing frame, d's
                 header is a COPY of that holder, not a second derivation (R-State).
```

**In words.** @PLN157 § V-n: the loop hoist's promise applied to the statements
after a `Set(d, P)` instead of a loop body.  A length read alone earns no header (it
is cheaper through the runtime).  Switch `LOFT_NO_VIEW_HOIST`; falsifier
`LOFT_HOIST_VERIFY=1`.  Sites: `hoist::view_def_header`, `Output::bind_view_header`.

### A record view carries its address

```
  (R-RecPtr)     a plain-record local r bound by a statement — `e = tbl[i]?`,
                 `s = o.inner`, a copy of another view — has its DbRef FIXED at
                 the bind (B-View), so the address of r's first byte is one
                 address for as long as the place lives.  It may be derived once,
                 right after the binding, and serve every fusable scalar field
                 read and in-place field write of r in the rest of the block —
                 and the scalar inputs of a twin (R-Inputs) r is handed to, read
                 through it at the call — when the remainder grows no store
                 (R-Base's condition; a null-discharge buffer's mint is not a
                 growth), frees no record before a later use of r, never
                 rebinds r — a `Set`, and a NATIVE op taking r as its first
                 operand that is not a read (`OpGet…`) or an in-place scalar set
                 (R-InPlace; a kind the address does not serve keeps its store
                 write, to the same bytes): a mint into it, a copy into it, a
                 free — and reads, writes or hands r at least once.  A binding to null (a buffer's pre-init,
                 minted later) is never a view.  A NULLABLE view — `e = v[i]`
                 without `?`, the loop variable of `for e in v`, whose null is the
                 loop's end signal — is admitted as its inner record: the null
                 record keeps its sentinel, the address is null and every read
                 tests it.  An enum payload and a synthetic `__nullable<S>` are
                 not plain records and are never bound.  THE BASE CLAUSE: where
                 the binding is the head of `for e in v` — `e = { idx = idx + 1;
                 v[idx] }` — over a path whose header AND element base the loop
                 holds (R-Base), the address is `base + idx * size` when `idx` is
                 in range of the header's length, and null past the end (the null
                 record that ends the loop): no DbRef is consulted and no store is
                 resolved.  THE PATH CLAUSE: a scalar field of r reached through
                 INLINE sub-records — `r.pos.x`, OpGet<K>(OpGetField(r, off(pos), _),
                 off(x)) — is r's field at the SUMMED offset, a fusable read or
                 write like a direct one: OpGetField adds a constant to the DbRef's
                 position and leaves its record alone.  The clause serves the
                 ADDRESS only; (R-Scalar)'s hoisted scalars keep their
                 bare-variable key.  THE MINT CLAUSE: a minted plain-record
                 element `e = OpNewRecord(…)` holds its address over its own
                 WINDOW — from the mint (after its growth step, if it had one) up
                 to its `OpFinishRecord(…, e, …)` in the same block — when the
                 window meets the conditions above in the remainder's place.
                 THE ORDER CLAUSE: "frees no record before a later use of r" is
                 asked in EXECUTION order through the extent's structure — a
                 block its statements in sequence, an `if` its condition then
                 either arm, a loop its body twice when the body frees — and a
                 block that ends in `return` and holds no `break` or `continue`
                 leaves the function on every path that reaches a free in it, so
                 it carries none past itself.  THE ENUM CLAUSE: a view of a USER
                 struct-enum value is a record view too — the tag byte at 0, read
                 through the address as one byte (0, no variant, for the null
                 record), and the variant's fields behind it at the offsets the
                 parser resolved for the arm that reads them.  The exclusion of
                 enum payloads is (R-Scalar)'s: a hoisted scalar is keyed by (type,
                 offset), and two variants put different fields at one offset.  The
                 synthetic `__nullable<S>` stays out: its absence is a discriminant,
                 not a null record.
```

**In words.** 2026-09-18, the drawing library's crossing loop (`pg_cur = pg_table[i]?`,
then six field reads of `pg_cur` and three more inside `edge_x` per edge per row): each
read resolved the store — `stores.store(&db).get_int(db.rec, db.pos + off)` — where the
Rust reference's `let cur = table[i]` reads registers.  (R-View) hoists the HEADER of a
vector view; this is the same promise for a RECORD view, whose derivation is an address
rather than a header.  Sound for the reason (R-Base) is: nothing in the remainder can move
the record's bytes, and a freed record's read is excluded by order (the releases a block
ends with follow the last use).  Measured on the `wide_line` probe: 7 800 → 5 060 ns/op
(−35 %), 2.9× → 1.9× the reference; hand-priced first at −25 % for the reads and −15 % for
the callee's inputs.  The first build admitted a return BUFFER's null pre-init
(`__ref_p2_N = null`) as a binding and did not read the literal's mint into it
(`OpDatabaseNP(buf, tp)`, a native op writing its first operand, not a `Set`) as a rebind,
so the literal's field writes went through a null address and were dropped — a library's
`mk()` answered a record of zeros; the suite caught it the same day (the 47 golden, the
#672 parity test) and the two clauses above are its receipt, with cells r14/r15.  Switch
`LOFT_NO_RECORD_PTR` (and `LOFT_NO_VECTOR_BASE`, one rule);
falsifier `LOFT_HOIST_VERIFY=1` (`vector::rec_get` re-derives the address and re-reads
the store at every use); trace `LOFT_TRACE_RECPTR=1`.  Sites: `hoist::record_view_ptr`,
`Output::bind_record_ptr`, `Output::rec_ptr_read` (the twin input),
`ops::vector_ops::emit_hoisted_scalar_or_default` and `FusedElementWriteEmitter`
(the read and the write), `vector::rec_ptr` / `rec_get` / `rec_set`.  Cells
`tests/scripts/157-record-ptr.loft`, pins `tests/record_ptr.rs`.  The `for e in v` loop
variable is a `Set(e, Iter…)` of the NULLABLE element type (its null ends the loop), so
the same gate admits it once the `Optional` is peeled — the loop body reads every field of
`e` through one address per iteration (`polygon_generic`'s first loop, `thin_line`; the
`forview` probe — `for e in v { t += e.a * 2 + e.b - (e.f as integer) }` over 100 000 records — 562–584 → 389–405 µs per pass, −31 %, the value hand-checked).  No lane row moved: the one such loop on the drawing bench (`polygon_generic`'s `for e in edges`) appends to `pg_table` and so declines by design.

*The base clause (2026-09-21, @PLN158 R2).*  A `for e in v` loop built a `DbRef` for its
element and then resolved the store a second time to turn that `DbRef` back into an address
— per element, in a loop that already held the address of element 0.  Hand-priced on the
emitted Rust before anything was built (`record_walk`, 32.0 µs per call): the address from
the base and the index −40 %, the element's `DbRef` no longer built −49 %, the index stepped
unchecked −62 % (1.12× the Rust reference).  Built, the first two arrive together — once the
address stops depending on the `DbRef`, LLVM drops the `DbRef` where nothing else reads it —
and the third is `(R-Counter)`'s iteration clause: `record_walk` 31.97 → 16.15 µs (2.98× →
1.50×), `tuple_kernel` −20 % (2.02× → 1.62×), `entity_tick` −30 % (its inner scan).  The
form matters and was measured: a first build took the address from the `DbRef` — an
identity (`rec_ptr` is the store's pointer plus `rec * 8 + pos`, the base the same pointer
plus `rec * 8 + 8`) guarded by a test of its store and record — and ran **+46 % slower**
than the store resolution it replaced, because the tests and the fallback hid the induction
from the optimiser; the index form is what ships.  An explicit index binding (`e = v[i]?`)
is a JOIN — the element, or a discharge buffer in another store — and keeps `rec_ptr`.
Switch `LOFT_NO_BASE_RECPTR`; falsifier `LOFT_HOIST_VERIFY=1` (`rec_get`/`rec_set` compare
the address with a fresh `rec_ptr` at every use — a sabotaged `index + 1` panics there, and
answers `0 0 162 135 243` for `0 -7 155 135 250` without it).  Cells
`tests/scripts/158-iteration-base.loft`, pins `tests/iteration_base.rs`.  Sites:
`hoist::iteration_head`, `Output::held_iteration_base`, `Output::bind_record_ptr`, the
`BIND_RECPTR_BASE` rule in `scripts/emission_audit.py`.

*The path clause (2026-09-21, @PLN158 R3).*  Only a DIRECT field of a view counted as a
fusable access, so a loop over records of records — `for v in m.verts { … v.pos.x … }` —
bound no address at all, and each of its reads rebuilt a `DbRef` with two offset additions
and resolved the store (`mesh_aabb`: twelve per vertex, 9.2× the Rust reference).  Priced
first by hand-binding the sub-record (`p = v.pos`, −52 %); built, 70.3 → 37.4 µs (−47 %),
4.7×.  What is left there is the null-aware float comparison, which is the language's
semantics on operands no proof says are non-null.  **Emitter-local by design:** the fold
is sound for an ADDRESS, which loads the bytes where they are each time.  Done in the
parser it would turn `v.pos.x` into `OpGetFloat(v, 8)`, a `(R-Scalar)` candidate typed
`(Vertex, 8)` — and a write through a sub-record view (`p = v.pos; p.x = …`) is typed
`(V3, 0)`, which never evicts that key: a stale hoisted value, silently (the cell n4 is
that program).  The falsifier had a hole the sabotage found: `rec_get`'s own check re-reads
the store at the offset it is given, so an offset summed wrongly (`n1` answering
`0 57 0 15` for `5 21 15 15`) passed `LOFT_HOIST_VERIFY=1` untouched; the checking form of
a nested read now walks the path the unrewritten way and compares
(`vector::path_read_verify`), and the same sabotage panics naming both values.  Switch
`LOFT_NO_NESTED_FIELD`.  Cells `tests/scripts/158-nested-field.loft`, pins
`tests/nested_field.rs`.  Sites: `hoist::view_field`, `hoist::record_view_ptr` (the fusable
use), `ops::vector_ops::emit_hoisted_scalar_or_default` and `FusedElementWriteEmitter`,
`vector::path_read_verify`.

*The mint clause (2026-09-21, @PLN158 R4).*  An address is promised for the whole
remainder of a block, and a minted element never got one: its remainder holds the NEXT
append, which grows a store (`_elm_N`: *"the remainder may grow a store"*).  So every field
of an appended record resolved the store again — a store lookup, a record-validity read and
two bounds checks per field, 143 instructions for a three-field record against the Rust
reference's 18, even through a push header that had just produced the slot.  The element
needs the address only while it is filled.  Hand-priced on the emitted Rust (−38…46 %),
then built: `record_append` 126.7 → 69.1 µs (3.55× → 1.95×), `mesh_emit` 189.3 → 107.2 µs
(4.98× → 2.83×; it stood at 18.3× before `(R-Mint)`'s field clause).  The window is judged
by the verdict a remainder gets (`hoist::view_extent_verdict`, one definition for both
extents): a text set, a nested mint, a builder that appends and a delivery copy each
decline it; a field value that names the container runs BEFORE the mint (loft#1548) and so
leaves the window clean; an element handed to its builder as the return buffer has no write
left for the caller to serve.  Any mint form qualifies — a keyed collection's claimed
record is one `DbRef` too, and its finish files it by the key just written.  The shared
verdict also stopped reading an in-place set of a kind the address does not serve
(`OpSetBoolean`, the narrow integers) as a REBIND of the view: it is a fixed-width write to
the same bytes, and counting it had declined every record with such a field.  **The first
sabotage of this clause changed nothing** — with the window's verdict blinded to everything
but the element's own writes, every cell stayed green, because a stale address needs the
store's memory to MOVE inside the window and nine small elements never reallocate: the
cells could not fail, so they proved nothing.  `m4b` (sixty nested points per element, 120
elements) is the cell that can: under the sabotage it answers `120 125094 21420` for
`120 127140 21420`, and `LOFT_HOIST_VERIFY=1` panics naming the stale address.  Switch
`LOFT_NO_MINT_WINDOW`.  Cells `tests/scripts/158-mint-window.loft`, pins
`tests/mint_window.rs`.  Sites: `hoist::mint_window`, `hoist::view_extent_verdict`,
`Output::bind_record_ptr`, `Output::close_ptr_windows_before`.

*The order clause (2026-09-21, @PLN158 R6).*  The free-before-use question was asked per
top-level STATEMENT of the extent, and a lookup loop's whole body is one statement — so
`for c in m.chunks { if c.cx == cx && … { return c.hexes[i]?.h } }`, where the `return`
releases the `?` discharge buffer after the last read of `c`, read as *"frees and uses in
one statement"* and `c` held no address (*"the remainder frees a record before a use of the
view"*).  It is the shape of every find-and-answer helper.  Asked in execution order it is
no free before a use at all.  `chunk_lookup` 1,417 → 1,097 µs (−23 %), 3.67× → 2.84× of the
Rust reference.  A `break` or `continue` keeps running this function, so a block holding
one carries its frees on; and a loop variable's extent is ONE pass — it is rebound, and its
address re-derived, at the top of the next — which is why a returning block that also holds
a `break` still leaves `c` its address (cell q9: the prediction said decline, the rule and
the values said otherwise, and the rule was right).  **No program can make this clause
answer wrong**: the only frees that reach it are discharge-buffer frees, which the scope
pass places after the last use on every path, so a free-before-use inside an extent is
what a scope-pass defect would produce, not what a test can write — three cells written to
decline through it declined EARLIER, as a store growth (the literal that mints their
record local).  The walk is therefore falsified where it lives, over synthetic IR
(`hoist::free_order_tests`, five tests): with a returning block made to drop its frees even
when it holds a `break`, and a loop body's second pass removed, exactly the two tests that
state those facts fail.  No switch of its own — it refines `(R-RecPtr)`'s admission, which
`LOFT_NO_RECORD_PTR` turns off whole.  Cells `tests/scripts/158-leaving-free.loft`, pins
`tests/leaving_free.rs`.  Sites: `hoist::free_before_use`, `hoist::free_before_use_by`,
`hoist::view_extent_verdict`.

*The enum clause (2026-09-21, @PLN158 R7).*  `hoist::plain_record_type` answers `None` for
every enum, and its reason — *"their payload offsets are a layout question this key does not
model"* — is `(R-Scalar)`'s: a hoisted scalar is keyed by (type, offset), and two variants
put different fields at one offset.  But the predicate was read by every rewrite, so a
`vector<Edit>` got no push header on append and no address in the `match` that walks it:
each element paid `record_new` and `record_finish` (~420 of 969 instructions per edit), and
each arm re-read the TAG and then its fields through the store.  Neither question needs a
layout: the literal's lowering writes the tag and the variant's fields at explicit offsets,
and the parser resolves each arm's reads behind that arm's own tag test.  A minted element's
WINDOW (the mint clause) therefore asks only that the element be a record — a struct-enum
element's variable is typed by a placeholder record, not by the enum — and the iteration
head is seen through the `OpGetField(element, 0, _)` a struct-enum element is bound behind.
`enum_match` 111.6 → 40.4 µs (−64 %), 12.4× → 4.5× of the Rust reference.  The tag's answer
for the null record is falsified by the ORACLE, not the checking form — flipped to the first
variant, a view bound past the end matches it and the cell answers `null 0` for `120 2`
under `LOFT_HOIST_VERIFY=1` as without it, since that form compares an address with a fresh
derivation and has nothing to compare an absent answer with.  The slot's zero is not
falsifiable by a program (an unwritten tail is bytes no arm reads, and a release walks a
slot by its tag); it is kept as the prefill it replaces.  Switch `LOFT_NO_ENUM_RECORD`.
Cells `tests/scripts/158-enum-record.loft`, pins `tests/enum_record.rs`.  Sites:
`Stores::is_struct_enum`, `hoist::mint_push_qualifies`, `hoist::struct_enum_view`,
`hoist::TAG_GETTER`, `hoist::iteration_head` (the wrapper), `hoist::mint_window`, the
registry's `NewRecordEmitter`, `Output::write_elem_first_mint`.

### A stdlib one-op wrapper is its op

```
  (R-Wrapper)    a call to a STDLIB function whose whole body is one native op over
                 its own parameters and constants, with leaf arguments (a variable,
                 a literal) and no text operand or result, is emitted as that op
                 with the arguments in the operand positions.  A USER function's
                 call is observable — the live tier may flip it to the interpreter,
                 and its frame is on the shadow call stack — and stays a call.
```

**In words.** @PLN157 § V-o.  The wrapper's compiled Rust body IS the op's template,
so for a frameless, unflippable stdlib method the op is the call; `len(d)` after a
view binding then reads the header.  Leaf arguments only, because the op's operands
are emitted from a fresh list the pre-evaluation map (keyed on node addresses)
cannot see.  Switch `LOFT_NO_WRAPPER_INLINE`; pin `tests/wrapper_op.rs`.  Sites:
`hoist::one_op_wrapper`, `Output::wrapper_op`.

### A callee's invariant inputs cross the call

```
  (R-Inputs)     a callee admitted by (R-Callee), of a parameter p it never
                 rebinds, has INVARIANT INPUTS: for a plain-struct p, the scalar
                 fields p.f its own write set does not reach and the vector paths
                 p.g it views (R-View) or indexes (R-Header); for a vector p, its
                 own header when it indexes p (g empty).  A caller loop that holds,
                 for the argument a it passes for p — a leaf variable c for a scalar
                 input, a pure path (R-Header) for a header input — the value of
                 (c, f) under (R-Scalar) and the header of a.g under (R-Header) —
                 EVERY input of the callee — calls the callee's TWIN: the same body
                 emitted with those values as extra parameters, read in place of
                 the record — the caller's holders handed in (R-State: a path held
                 by a push header hands in that header, current at the call).  A
                 call missing any input keeps the plain form; the twin exists
                 beside the original, never instead of it.
```

**In words.** @PLN157 § V-p.  The pixel methods read `self.width` / `self.height`
and view `self.data` per call — single reads, so (R-Scalar) cannot hoist them
inside the callee, and the caller's hoisted values do not cross the call.  Handing
them over is sound for exactly the reason each was hoisted: the caller's loop proved
them invariant across the call (its write set includes what the callee writes), and
the callee's own write set proves it does not write them either.  Measured before
the code by hand-editing the emitted Rust: the twin and a full inline of the
callee cost the same (`composite` 4.2× → 2.33× of Rust), because rustc inlines the
small twin — so the rewrite passes values and never clones IR; shipped, the consumer's
`composite` row measures 2.34× (446k → 237k ns/op), `render_lock` / `render_marks` −8 %.  Switch
`LOFT_NO_CALLEE_INPUTS`; falsifier `LOFT_HOIST_VERIFY=1`, which inside the twin
re-reads every input against the record.  Sites: `hoist::callee_inputs`,
`hoist::hoistable` (the mapping through the argument), the twin's emission in
`Output::output_function`, the twin call in `Output::user_fn_call_body` and in the
return-buffer delivery site of `dispatch.rs`.
**The path-argument half** (@PLN157 § V-ac): a header input is keyed on the
ARGUMENT's pure path, so `brush_sample(br.img, …)` over `img: const vector<integer>`
takes the header of `(br, img)` the loop already holds, and `rd(h.cv, i)` over
`c.data` takes `(h, cv, data)`; the callee's path is re-spelled over the argument by
`hoist::substitute_path` (a walk of its own — `map_nodes` descends into the
replacement, whose variable numbers are the caller's), the key by
`hoist::input_header_at`, the ONE definition both the loop's candidate and the twin
call ask.  A scalar input still needs a leaf variable — its key carries a variable,
not a path.  A return-buffer writer (R-Callee's second half) is admitted like any
other: its write set is its buffer's type whole, which no parameter's field shares.
**The address half** (2026-09-18, (R-RecPtr)): a scalar input the caller holds no VALUE
for — the record changes per iteration — but whose record ADDRESS the caller's block holds
is read through that address at the call (`Output::rec_ptr_read`), so `edge_x(pg_cur, y)`
takes its twin with three loads for arguments where the plain call read three store
resolutions inside; and handing `r` to such a twin counts as a use of `r`'s address for
(R-RecPtr)'s admission.
**The base half** (2026-09-18, (R-Base)'s twin clause): a header input alone left the
twin resolving the store per element — `composite`'s `get_pixel`/`set_pixel` twins ran
`get_elem_hoisted` through `allocations[k].ptr` on every pixel, and LLVM cannot hoist that
load (the pointer is loaded from memory the twin's own writes may alias; LTO and a
`noalias` ABI were both measured as no gain).  So the twin's signature carries `__ib_k:
*const u8` after its headers, `twin_call_inputs` passes the loop's `__vb_N` or
`vector::vec_base(&hdr, …)` at the call, `push_twin_frames` binds each base under the
header's key, and `bind_view_header` shares a held base with the view it binds
(`__vb_N = __ib_k`), so `ops::vector_ops`' fused read and write take `get_elem_at` /
`vec_set_at` through the base exactly as inside a growth-free loop.  Switch
`LOFT_NO_TWIN_BASE`; falsifier `LOFT_HOIST_VERIFY=1` (the base re-derived at every use);
cells `tests/scripts/157-twin-base.loft`, pins `tests/twin_base.rs`.

### A push keeps its own header current

```
  (R-Push)       in a loop whose store writes are (R-InPlace) sets and pushes of a
                 fusable scalar kind to pure paths, a pushed path P keeps a PUSH
                 header — (R-Header)'s triple plus the record's capacity — that the
                 push itself keeps current: a push that fits is a bounds test, one
                 store and a length bump written to the header AND the record; a
                 growth step is the runtime's own append followed by a fresh header
                 (R-Refresh).  Every read of P in the loop serves from that header
                 (R-State).  The push is a MOVER, admitted beside other holders under
                 (R-Alias); the OpPreAllocVector the parser emits before a push to a
                 local claims a record only for an absent vector, never moves one,
                 and is emitted as nothing under a held push header.
```

**In words.** @PLN157 § V-q.  A growth moves the pushed vector's RECORD and nothing else
— the container record, every other vector's record and every scalar keep their numbers —
so the only holder a push can invalidate is one naming the SAME vector (R-Alias), and the
push's own is refreshed at the site, the record's length included (R-Refresh).  Switch
`LOFT_NO_PUSH_HOIST`; falsifier `LOFT_HOIST_VERIFY=1` (the push re-derives its header
before each fast-path store).  Sites: `hoist::FUSABLE_PUSHES`, `hoist::fused_push`,
`hoist::pre_alloc_path`, `Output::begin_vector_hoist`, the registry's `HoistedPushEmitter`
and `PreAllocEmitter`, `Stores::push_hoisted`.  Shipped: the consumer's `lock_curved` 5.64× → 3.84×, `lock`
4.39× → 3.51× of Rust, hashes unchanged.

### A minted element is a record no holder can name

```
  (R-Mint)       in a loop admitted under (R-InPlace), the record MINT group over a
                 plain vector P — OpPreAllocVector(P) · e = OpNewRecord(P) · writes
                 into e · OpFinishRecord(P, e) — is a MOVER like a push, admitted
                 beside other holders under (R-Alias); P itself earns no holder.  P
                 has two spellings and both are this one notion: a BARE variable that
                 types as a plain vector (OpNewRecord(v, vector_tp, MAX)), and a vector
                 FIELD of a plain record (OpNewRecord(R, parent_tp, fld)) — R a pure
                 path to a variable that types as a plain struct, parent_tp a plain
                 struct, fld a plain vector no sibling collection shares — whose key is
                 R's path extended by the field's position: the key a read of R.fld
                 (OpGetField(R, pos, tp)) already has, so the append and the reads
                 share ONE holder (R-State).  A write
                 whose target is the FRESH variable e — the literal element's
                 in-place sets, and § V-d's OpCopyRecord delivery of a builder's
                 result into e (its source-free included) — contributes nothing to
                 (R-Scalar)'s write set: a record minted inside the body cannot be
                 named by a variable whose getter the prelude already ran.  The
                 same op names over a KEYED collection are a keyed insert — a mover
                 of OTHER records — and stay blocking: admission asks the TYPE,
                 never the op shape.  A mover whose ROOT the body REBINDS takes no
                 holder (a holder describes what the root named on the way in);
                 it declines the loop unless every rebind is alias-free — a null,
                 or the projection OpGetField(b, …) of a buffer var b the EMITTER
                 owns the mint of (a § V-al loop buffer, a § V-z element-first
                 witness), which names a fresh store's root or a fresh element's
                 field slot that no kept header can describe.  Such a mover's ops
                 keep their templates or take a GROUP header (R-GroupPush), and the
                 loop is not growth-free.  The same clause admits OpDatabase(b, …)
                 on such a b (a fresh store, a length reset of its own, or nothing
                 at all) and the § V-z paired copy into the element's field slot
                 (emitted as nothing) as statements that move no held record.
```

**In words.** @PLN157 § V-s.  Before it, `OpNewRecord`/`OpFinishRecord` were
unclassified writers, so a loop appending a record element hoisted NOTHING — `smooth`'s
hot line reads eight loop-invariant parameter fields per sample, none lifted.  The fresh
window is tracked in the same preorder the write-set walk uses (`e` marked at its
`Set(e, OpNewRecord)`, cleared by any other assignment and by the group's own
`OpFinishRecord`), which is execution order for the straight-line group the parser emits
— the only producer of these ops.  Freshness over-applied is the falsified failure:
the sabotage turns the write-through-parameter and write-through-alias cells red and
`LOFT_HOIST_VERIFY=1` panics naming the stale scalar.  Switch `LOFT_NO_MINT_HOIST`;
falsifier `LOFT_HOIST_VERIFY=1`.  Sites: `hoist::mint_path`,
`hoist::blocks_header_hoist`, `hoist::body_writes`, the mint arm in `hoist::hoistable`.
*The rebound clause (2026-09-18):* the drawing library's `fronds` declares its point and width
vectors per pass of the side loop, and every rebound mover declined the WHOLE loop — so
neither the side loop nor the `for i` around it hoisted anything (the `for i` was declined a
second way, by its per-pass `fd_sides` loop buffer's `OpDatabase`).  A rebind from the
emitter's own buffer is the one rebind that cannot alias a held vector, so it costs the mover
its holder and nothing else.  Falsified on the alias the clause refuses: a mover rebound to a
VIEW of a held vector (`w = &big; w += [...]` beside `len(big)`) must still decline the loop,
or the hoisted length answers the pre-push count (`tests/scripts/157-group-push.loft` g7).
Switch `LOFT_NO_REBOUND_MOVER`; trace `LOFT_TRACE_HOIST_DECLINE=1` names the first statement
that declines a loop.  Sites: `hoist::HoistOwned`, `hoist::rebinds_alias_free`, the
`buffer_alloc`/`elided_copy` arms in `hoist::blocks_header_hoist`, the rebound arm in
`hoist::hoistable`, `Output::hoist_owned`.  *The same clause reaches `(R-Scalar)`'s write set
(the same day):* the loop the clause admitted still hoisted no scalar, because the write-set
walk answered "untyped" at the very statements the admission let through — the loop buffer's
`OpDatabase`, the `(R-LoopRecord)` local's mint, and the element-first copy into a fresh
element's field.  Each is now typed: a vector buffer's mint writes no record a hoisted scalar
can name, a loop record's mint is a write of its whole TYPE (evicting scalars hoisted off that
type — the type-keyed conservatism `(R-Scalar)` already has), and a copy into a fresh
element's field is `(R-Mint)`'s own exemption.  `fronds`' outer loop then reads its `sp`
parameter's twelve fields once per activation: 69.8 → 64.9 µs per call (`tests/scripts/
157-group-push.loft` g11, g12; trace `LOFT_TRACE_HOIST_DECLINE=1` now also names the node
that left a write set untyped).  Sites: the owned arms in `hoist::body_writes`.
*The field clause (2026-09-21, @PLN158 R1):* every builder a consumer writes appends to a
vector that is a FIELD — `m.verts += […]`, `sc.ops += […]`, `world.items += […]` — and that
append names the PARENT record and a field number, so the gate, which asked whether the
operand typed as a vector, never admitted it: the loop held no header, and every element
paid `record_new` and `record_finish`, each a walk of the type table (`mesh_emit`, the
portal's worst row: ~1,500 of 2,314 instructions per vertex).  For a plain, unlinked vector
field those two calls ARE `vector_append` and `vector_finish` on the field's `DbRef` — the
calls the push header wraps — so the clause changes the gate and the operand the emitters
write, and nothing in the runtime.  The schema is asked (`Stores::plain_vector_field`): an
`array` member of a linked group and every keyed kind keep the general append, a struct-enum
variant's field and a nullable element's payload form no plain parent.  Two things the
matrix found: the § V-z element-first early mint looked its header up by the bare-variable
key, so a field-form element would have been minted by its template and finished through
the header — `0` elements for `20`, silently; both halves now resolve the holder through
`hoist::mint_target`.  And a NULL root handed to a parameter (`fill(h.ms[5], 3)`) is a
dropped append on the general path, where the header's finish indexed store 65535:
`Stores::push_record_finish` returns on an absent owner, as `vector_finish` does.
Measured: `mesh_emit` 708.7 → 189.5 µs per call (−73 %, the figure its paired-variant
probe priced), 18.3× → 4.9× of the Rust reference, hashes unchanged.  A field-form group
OUTSIDE a held header keeps its templates: the parser reserves only for a local vector, and
`(R-GroupPush)` starts at the reservation.  Switch `LOFT_NO_FIELD_MINT`; falsifier
`LOFT_HOIST_VERIFY=1`.  Cells `tests/scripts/158-field-mint.loft`, pins
`tests/field_mint.rs`.  Sites: `hoist::mint_target`, `Stores::plain_vector_field`, the
registry's `NewRecordEmitter` and `FinishRecordEmitter`, `Output::write_elem_first_mint`,
`Stores::push_record_finish`.

### A record append emits through its push header

```
  (R-PushRec)    a mint group admitted under (R-Mint) whose element type is a plain
                 STRUCT owning no heap, on a path that holds a PUSH header, emits
                 through it: the fresh element e is the header's next slot — a
                 bounds test against the capacity, no record_new dispatch and no
                 default prefill — the group's writes fill e as before (the IR's
                 literal lowering writes every field explicitly, omitted fields'
                 declared defaults and null sentinels included, and a declined
                 delivery lands as a whole-record OpCopyRecord), and the
                 OpFinishRecord is the length bump written to the header AND the
                 record — the one visibility step, exactly where record_finish's
                 vector_finish was.  A growth step is the runtime's own append
                 followed by a fresh header (R-Refresh), taken BEFORE the element's
                 writes so they land in the moved record.  Admission asks the TYPE:
                 the element must be a plain struct — a __nullable element's
                 discriminant is not a field the literal writes.  An element that
                 OWNS heap (text, a nested collection) takes the header too, its
                 slot ZEROED at the mint (push_record_hoisted_zero): the zero is the
                 whole of what the prefill did for its handles, and a raw slot's
                 stale handle is exactly what a reused buffer's slot carries.  The
                 § V-z element minted at its temp's declaration emits through the
                 same header when the container holds one.  THE ENUM CLAUSE: a USER
                 struct-enum element qualifies as a plain struct does — its literal
                 writes its own tag (OpSetEnum(e, 0, variant)) beside the variant's
                 fields — with its slot always zeroed, a narrower variant leaving a
                 wider one's tail unwritten.
```

**In words.** @PLN157 § V-t.  Before it, an admitted mint still paid per element a
`record_new` dispatch, a `set_default_value` type walk over fields the group was about to
write, a `zero_range` and a `record_finish` dispatch — ~30 % of `smooth`'s row after every
other unit.  The prefill is redundant exactly where the write set is complete, and the IR
makes it complete: a partial literal (`P3 { a: x }`) emits explicit writes for the omitted
declared default, the omitted zero AND the omitted null sentinel, at the caller and in a
retbuf-delivered builder alike — the cells pin this from both sides.  The skipped refresh
is the falsified failure: the sabotage empties every growth cell (`c1 100 0 25 9801 2475`
→ `0 0 0 0 0`, the elements landing in the dead pre-growth record) and
`LOFT_HOIST_VERIFY=1` panics naming the stale header.  Switch `LOFT_NO_RECORD_PUSH`;
falsifier `LOFT_HOIST_VERIFY=1` (the header re-derived and compared at the slot and at
the finish).  Sites: `hoist::mint_push_qualifies`, the mint arm in `hoist::hoistable`,
`Output::begin_vector_hoist`, `Output::active_mint_push`, the registry's
`NewRecordEmitter`, `FinishRecordEmitter` and `PreAllocEmitter`,
`Stores::push_record_hoisted`, `Stores::push_record_finish`.  Shipped: standalone
`smooth` −32 % (3 070 → 2 090 ns/op), consumer `smooth` 11.0× → 4.6× and `fronds` 11.1× →
8.4× of Rust on the measuring box, 14/14 hashes unchanged.
*The heap clause (2026-09-18):* `fronds`' element is `Frond { fpts, fwid }`, two vector
handles, so every one of its 1 272 mints per call paid the dispatch and a `set_default_value`
walk whose whole effect was two zero words.  Falsified by sabotage: `push_record_hoisted_zero`
made to skip its `zero_range` turns the reused-buffer cell red on `--native` under
`LOFT_POISON_CLAIM=1` — the slot's poisoned handle is read as the element's `xs` vector, a
store guard panic naming record `3735928559` — while the plain run (zero-on-claim) stays
green, which is why that falsifier and not the plain run guards the clause.  Switch
`LOFT_NO_HEAP_RECORD_PUSH`; cells `tests/scripts/157-group-push.loft` g2, g3, g8–g10; pins
`tests/group_push.rs`.  Sites: `hoist::mint_push_qualifies` (`heap`), `NewRecordEmitter`,
`Output::write_elem_first_mint`, `Stores::push_record_hoisted_zero`.

### A mint group outside any held header binds its own

```
  (R-GroupPush)  a mint GROUP — OpPreAllocVector(P, n, size) · n × (e = OpNewRecord(P)
                 · writes into e · OpFinishRecord(P, e)) — over a bare-variable plain
                 vector P (the (R-Mint) shape) whose path holds NO record-push header
                 where it stands, whose every mint qualifies under (R-PushRec), and
                 whose every OTHER statement up to the n-th finish is one the header
                 admission lets through (an in-place set, a store-free builder, a
                 § V-d fresh delivery, a nested mint on another path) with P never
                 rebound and never minted or reserved outside the count, binds a
                 push header of its own right after its reservation and emits its
                 mints and finishes through it (R-PushRec), the header dying at the
                 n-th finish; every read of P inside the group stays a store read.
                 The reservation STAYS — it is what gives the header its capacity —
                 and a growth inside the group is (R-Refresh)'s.  A builder that may
                 write a store, a mint or finish on P nested where the count cannot
                 see it, and a group that runs out before its n-th finish each keep
                 the templates, which resolve per element and are always right.
```

**In words.** @PLN157 (2026-09-18).  `(R-PushRec)` gave the header to a mint INSIDE a loop
that holds one; every other append — a literal group at function level, a group inside a
loop that declined, a group on a per-pass local — paid the `record_new` dispatch, the prefill
and the `record_finish` dispatch per element: the drawing library's `fronds` built its 1 296
points per call that way (`fd_pts += [pt(…), pt(…)]`, `fd_pts` declared per pass), 15 µs of a
102 µs call.  The group's own reservation already guarantees the slots, so the header derived
after it is exact for the whole group, and the group is straight-line code the parser emits —
the same producer `(R-Mint)` trusts.  Hand-priced first on the lean emission (H7 −15 µs, H8
the heap clause −12 µs, together −33 µs), then built: `fronds` 101.7 → 69.6 µs per call
(2.71× → 1.86× of the Rust reference), hash `ebcfd875` on every run, the hand ceiling met.
The audit reads the group's close marker (`// @FR-R-GroupPush group __ph_N closed`) because
the header's Rust binding outlives the group.  Switch `LOFT_NO_GROUP_PUSH`; falsifier
`LOFT_HOIST_VERIFY=1` (the header re-derived at the slot and the finish; the growth cell g6
is the one that moves the record mid-group).  Cells `tests/scripts/157-group-push.loft`
g1, g3–g7, g10; pins `tests/group_push.rs`, and `tests/record_push.rs` re-derived where the
group reaches a cell the loop declined (c8, c11, c15).  Sites: `hoist::mint_group`,
`Output::bind_group_push`, `Output::close_groups_before`, `Output::hoist_tiers`, the
`GROUP_CLOSE` rule in `scripts/emission_audit.py`.

### An integer operator whose result provably fits emits the plain operator

```
  (R-Range)      an integer `+`, `-`, `*`, negation, `/` or `%` whose RESULT lies in a
                 range the emitter can prove — a literal by its value; `a & lit` (a
                 non-negative literal) in `0 ..= lit` when `a` is NON-SENTINEL; `&`
                 and `|` over two non-negative ranges; `>> k` of a range; `+ - *` and
                 negation by interval arithmetic over ranged operands whose result
                 fits `(i64::MIN, i64::MAX]`; `/` and `%` by a ranged divisor whose
                 range excludes zero; a text's size and a vector's length in `0 ..=
                 u32::MAX` (the store's length word); a text byte in `0 ..= 255`; an
                 `if` merge as the union of its arms; a counted range's counters from
                 ranged ends; a LOCAL whose every assignment is ranged, never handed
                 out by reference and never a self-step; a ONE-EXPRESSION callee
                 evaluated over its arguments' facts, three calls deep at most —
                 emits the processor's operator (`wrapping_*`, bare `/` `%`): no
                 operation can fault, so the plain answer IS the checked template's.
                 A range implies non-sentinel: the sentinel is never inside one, and
                 a parameter or a record/element read is never ranged (C80).  The
                 fallback is the checked template, always right.
```

**In words.** 2026-09-20.  C120 declined the CLOSURE of the non-sentinel proof over
arithmetic — an overflow's null must propagate on both backends — and named the admissible
successor: *"a PROOF, not a policy: arithmetic whose operands carry ranges such that the
result CANNOT overflow may emit the plain operator, because no fault can occur and no value
can differ."*  `(R-BoundedNest)` built it for one loop shape with a run-time guard; this is
the same proof for straight-line arithmetic, decided at generation time
(`generation::range`, mirroring the non-sentinel proof's per-function fixpoint).  The one
rule that needed its own falsifier is the mask: `null & 255` is null
(`op_logical_and_int` propagates the sentinel), so `a & lit` is ranged only over a
non-sentinel `a` — the a9 cell holds a null read from past a vector's end, and the mask
rule without that requirement answers `1` for null.  Beside it the non-sentinel proof gained
a callee's RETURN summary (`callee_returns_non_sentinel`: every exit non-sentinel under the
callee's own facts, parameters trusted for nothing), which is what lets `color_a(get_pixel(…))`
range at all — the pixel is any integer, the accessor's `?? 0` makes it non-null, the mask
makes it a byte.  Measured on `composite_layer`: 24 of its 36 checked operators take the
plain form and the row does NOT move — the twelve that remain (`j * lw + i`, `x0 + i`,
`y0 + j`, the accessors' `by * width + bx`) have record-scalar or parameter operands no
static proof can bound, and the ceiling (every operator plain, −33 %) is carried by exactly
those (class split: those six −35 %, the counters −5 %, the divisions −3 %).  That is what
`(R-GuardedChain)` is for.  Switch `LOFT_NO_RANGE_ARITH`; falsifier `LOFT_HOIST_VERIFY=1`
(`ops::range_verify` compares the plain answer with the checked one at every admitted
operator).  Cells `tests/scripts/157-range-arith.loft` a1–a9, pins `tests/range_arith.rs`.

**The counted-counter clause was INERT until 2026-09-21** (loft#1558).  It is listed above
and was written, but no counted loop ever got a range: the seeding looked for the counter's
SEED inside the loop, and the parser emits `index = <start>` as the statement BEFORE it, so
`seeds` was always 0.  The seed is now looked for in the whole FUNCTION, which is what makes
it sound rather than merely wider — `v_seed` counts every non-step `Set` to that counter
anywhere, so a counter written from a second place declines on `seeds != 1` instead of taking
the first value it meets.  Nothing made the gap visible, because the one pin that would have
shown it (a4's `wrapping_mul` on `i * i`) was recording a plain form supplied by
`(R-GuardedChain)`'s duplicated copy, not by this proof: **a pin can borrow another
rewrite's evidence and read as proof that its own clause works.**  It surfaced only when the
guard's profitability gate declined that loop and took the borrowed evidence with it.
Measured against the clause inert, same lane and invocation: `smooth` +12.9 %, `fill_circle`
+2.9 %, `hash` +1.8 %, `render_marks` +1.5 %, `resize` +1.1 %, every other row within its
swing.  Cells b1–b6, of which b3/b4 are the BOUNDARY PAIR: they differ only in the range's
end (`0..=10` against `0..=9`, `i * 1e18` crossing i64::MAX between them), so the multiply's
verdict is a claim about the counter's top bound and nothing else — a top taken one too LOW
is the only unsound direction, and it turns b3 red and makes `LOFT_HOIST_VERIFY=1` panic
naming the operator.
Sites: `generation::range::{range, op_range, range_vars, plain_form}`, the range arm in
`ops::int_arith`, `Output::op_range`, `non_sentinel::callee_returns_non_sentinel`.

### A counted loop's index chains run plain behind a guard

```
  (R-GuardedChain) a counted loop whose body holds CHAINS of `+`, `-`, `*` and negation
                 over literals, the loop's own counters, the counters of a counted loop
                 NESTED in it, integer locals the loop never writes (nor takes the
                 address of) and record scalars an enclosing frame hoisted, runs those
                 chains with the processor's PLAIN operators when a guard, evaluated
                 once at the loop's entry, proves that none can fault: the range's
                 ends are not the sentinel; every invariant leaf is not the sentinel
                 (`checked_abs` fails on `i64::MIN`); every nested loop's seed and end
                 are bounded over the same leaves; and the MAGNITUDE BOUND of every
                 chain — a literal by its value, a counter by the larger of its
                 range's ends plus one, an invariant by its absolute value, `+`/`-`
                 summing and `*` multiplying, each step checked — fits the type.  A
                 bound that fits means every true intermediate fits, so the plain
                 operator answers exactly what the checked template would; every
                 OTHER operator of the body is untouched.  An INNERMOST loop (no
                 loop inside it) is emitted twice, the guarded copy plain and the
                 checked loop as the `else` arm; a loop with loops inside is emitted
                 once, each admitted operator branching on the guard, so nothing below
                 it is duplicated — its nested loops guard their own chains.  A body
                 carrying a `Yield`, a `Parallel` or a `CallRef` is never copied: it
                 is not this rewrite's to run twice, and a generator's loop body
                 carries the native collector's REFUSAL, which copied reaches the
                 author twice.  Inside a COROUTINE's state machine the guard is
                 not emitted at all: a persistent local is spelled `self.var_…`
                 there, so a guard naming one emits an identifier that does not
                 exist, and the machine RE-ENTERS its loop across a `next_*`
                 call, so a fact proved once at entry is not proved for the
                 resumes after it (`R-BoundedNest`'s guard declines there for the
                 same second reason).  A loop with ONE admitted operator declines on
                 PROFITABILITY: the guard is a fixed cost per loop ENTRY and the
                 saving is one null test per operator per ITERATION, and an
                 innermost loop is emitted twice — in a small hot function that
                 doubling costs the inline.  Measured against the guard off, the
                 one-operator loops in `pil_hline` and `matches_at` cost
                 `fill_circle` and `fill_star` ~50 %, `wide_line` 22 % and `parse`
                 18 %, while six-operator `composite_layer` gains 30 %.  An
                 UNDER-approximation, and the residual is stated rather than hidden:
                 `hair_brush` has the same six operators as `composite_layer` and
                 LOSES ~8 %, so operator count does not separate them — trip count
                 does, and that is not a compile-time fact here.  The
                 `*Nullable` twins of `+ - *` are chain operators too: a chain the
                 guard admits has no fault for them to be silent about.  A chain any
                 of whose leaves is not one of these is declined whole and its
                 sub-chains asked again.  A loop already admitted as a bounded nest is
                 not guarded twice.
```

**In words.** 2026-09-20.  `(R-BoundedNest)`'s method — a fact ESTABLISHED before the
arithmetic runs, in the same magnitude-bound arithmetic the rule states — for the index
chains of any counted loop, where the static `(R-Range)` proof stops: `composite_layer`'s
`j * lw + i`, `x0 + i`, `y0 + j` read record scalars (`lay.lw`, `lay.x0`) that a program
cannot bound at generation time and a guard can bound at the loop's entry in six checked
operations.  Measured, `composite` 103 → 68 µs per call (−34 %, the hand ceiling for those
six operators met), hash exact; the trace shows the guard admitting the graphics library's
`fill_rect`, `fill_triangle`, `resample`, `mat4_mul`, `sphere` loops as well.  A leaf may be
a PARAMETER — the guard tests its value — where the static proofs never trust one.  The
two emission forms were measured against each other: the branch-per-operator form alone
keeps `composite` at −21 % (LLVM does not unswitch a body that calls), the innermost
loop's duplicated copy gives the full −35 % at +7 % emitted lines on the drawing probe;
duplicating every guarded loop cost +16 % and doubled every per-function pin, which is
why only the innermost loop — where the time is and the body is small — is copied.  Found by
the cells: a `(R-LoopRecord)` local declared at the loop's prelude must be declared before
the guard and freed after BOTH arms, or the plain arm neither sees it nor frees it (the
`Value::Loop` emission's order was corrected for the bounded nest too).  Switch
`LOFT_NO_GUARDED_CHAIN`; trace `LOFT_TRACE_CHAIN=1`; falsifier `LOFT_HOIST_VERIFY=1`
(`ops::range_verify` at every admitted operator).  Cells `tests/scripts/157-guarded-chain.loft`
c1–c6, pins `tests/guarded_chain.rs`.  Sites: `Output::chain_fast_path`,
`Output::in_plain_chain`, `hoist::nested_counted_loops`, `hoist::written_vars`, the chain
arm in `ops::int_arith`, the loop hook in `emit.rs`.

### A dying temporary's elements move into the append that consumes them

```
  (R-MoveAppend) in `for f in call(…) { V += [f] }` where the call MINTS its result
                 (a borrowed-view return declines), the loop variable's ONLY use
                 after its binding is that one append (a read after it, a second
                 append, an append under a further loop all decline), V is an owned
                 local plain vector of a PLAIN STRUCT element never rebound in the
                 function, and the call's hidden buffer serves nothing else — the
                 buffer is PLACED as a record inside V's own `__vdb` store at the
                 loop (V is in scope there, so its store exists on every path), the
                 append relocates the element's bytes and ZEROES the source when
                 source and destination share that store (anything else keeps the
                 deep copy, which is always correct), and the buffer's free is a
                 record-level release — the deep walk of what the loop did not
                 move, then the record's block — inside the store that lives on.
                 Three lifetime conditions bound the placement.  (1) The placed
                 record dies WITH ITS HOST STORE: the release is injected before
                 EVERY free of the host `__vdb` — the early dead-after-last-read
                 site as much as scope end, since a later store can recycle a dead
                 host's slot and a deferred release then deletes a record inside
                 whatever owns it — and the buffer attr's own scope-end free is
                 the null-guarded backstop that covers an ADOPTED `__retbuf` host,
                 which no in-function site frees.  Between host deaths an
                 enclosing loop REUSES the placement (the callee's entry clear
                 resets its length).  (2) A host store CLEARED whole re-arms the
                 guard: `OpDatabase`'s reuse arm reclaims the placement with the
                 clear, so the buffer var is nulled at that site or the next call
                 delivers into unclaimed bytes.  (3) The buffer's type must be V's
                 ELEMENT's own wrapper — the lookup is by name, and a struct named
                 like a stdlib type variable finds the GENERIC template whose
                 field still carries the typevar, which a record-level walk
                 misreads (such a name declines the pairing).
```

**In words.** @PLN157 § V-j.  The deep copy's cost was never the bytes: each appended
element re-CLAIMED its inner vectors in the destination's store, copied them, and freed
the source's — two claims and two frees per element (23 % of the `fronds` row).  Placing
the buffer where the elements must end up makes the callee's own delivery put them there,
and the append is then a shallow relocation whose heap handles never change store.  The
zeroed source is what the temporary's clear and free are allowed to walk.  The gates are
under-approximations on purpose; the one that carries values is use-after — falsified by
removing it, the read-after-append cell answers the zeroed source (`3 409 45` →
`3 409 0`).  All three lifetime conditions were falsified on builds that lacked them
(2026-09-11): the scope-end-only release corrupted a recycled store slot on a6f30ae7 —
values right, teardown walked another store's live records (the red `native_scripts`
gate) — the host-death build WITHOUT the OpDatabase re-arm corrupted c21 (a store panic
at rec 472: the reuse clear had reclaimed the placement the guard still trusted), and a
corpus struct named `T` was walked through `main_vector<T>`'s typevar field (the
c19–c22 cells).  The host-death form reuses the placement across an enclosing loop's
entries — `fronds` 351k → 295k ns/op (−16 %), the hand ceiling met exactly.  Composes with `(R-PushRec)`: a no-heap element's slot comes from the
push header and the move lands in it (the c16 cell).  Switch `LOFT_NO_MOVE_APPEND`; the value
cells run under `LOFT_POISON=1` and both leak checks.  Sites: `hoist::move_appends`,
`hoist::pair_for_block`, `Output::move_pair_for_block`, `Output::active_move_pair`, the
placement in `Output::output_block`, the move arm in `OpCopyRecordEmitter`, the record
free in `OpFreeRefEmitter`, the null decl in `Output::emit_null_dbref`,
`Stores::place_record_in`, `Stores::move_record_shallow`, `Stores::free_record_in`.
Shipped: standalone `fronds` −11 % (396–400k → 354–359k ns/op), the hand-measured
ceiling reached exactly; hash `ebcfd875` on every run.

### A loop over a split takes its pieces from the text

```
  (R-LazySplit)  in `for p in T.split(c) { … }` where `split` is the standard library's
                 `split(self: text, separator: character)`, `c` is a character CONSTANT
                 other than the null character, and the loop is the plain forward
                 walk the parser gives a text vector — the hidden vector the call's
                 result is bound to is read by the loop's element read and by its
                 length test and by NOTHING else (a `rev`, a vector bound to a name
                 first, a third mention all decline) — the vector is never built:
                 the loop iterates the pieces of T as slices, in the order and with
                 the content `split` answers (none for an empty text; otherwise one
                 per separator plus the trailing piece, empty when T ends in a
                 separator; a NULL text is its sentinel's one character and answers
                 itself as its only piece), and binds each to `p` exactly as the element
                 read did, so `p` is still the loop's own text.  T is evaluated
                 ONCE, where the call stood.  A plain text PARAMETER the function
                 never writes is borrowed for the loop; every other T — a local, a
                 by-reference text, a field, a call, a slice — is iterated as a COPY
                 taken at that point, so no write in the body can reach what is
                 being walked: the pieces are those of T as it was, which is what
                 the vector held.  The call's hidden buffer, when it serves that
                 call alone, is never minted, and its frees release nothing.  A
                 generator declines whole: its loops are re-entered across `next`,
                 and the iterator is a local of the activation.
```

**In words.** A parser's outer loop is `for line in src.split('\n')`, and the vector it
walks has no other reader: `split` claimed a record and a text per line, the loop copied
each out again, and the buffer was freed at exit — the drawing bench's `parse` row spent
2.1 µs of 19.4 there, where its Rust reference's lazy `split` spends none.  `(R-Escape)`
is the licence: the vector never leaves the `For` block, so how many stores its pieces
take is the compiler's to change.  The conditions are the vector's TWO mentions (counted
over the whole function, so a shape this rule has not met declines rather than compiles to
a read of a vector nobody filled) and the separator's constancy — a null separator
compares equal to a NUL inside the text, which the iterator does not model.  The NULL
text is the edge the first build got wrong: it answered no pieces, by analogy with the
empty text, where `split` answers one — `len(null)` is 1, so its trailing-piece rule
fires — and lazy native disagreed with the interpreter AND with its own switch-off form
(`count=0` for `count=1`).  The cell that found it passes a real null (s2, s21); the
first s2 passed `nothing ?? ""`, an empty text, and saw nothing.  The source
needs no gate because the copy is always sound; the borrow is the one case where the
copy is provably unnecessary.  Priced by hand patch first (−1.85 µs), then built: `parse`
19.21 → **17.44 µs (−9 %), 298k → 268k instructions per parse**, 2.93× → 2.63× its
reference on x86-64, hash `33f6d2b8`; with the switch set the bench's emission is
byte-identical to the build before the rule.  Two spellings of the `For` block reach it —
the buffer minted at function entry, and minted at first use inside the block
(`(O-LazyBuffer)`), where a `#count` seed may also stand before the bind — and both are
one shape to the matcher, which finds the bind and requires it, the index seed and the
loop to close the block.  Switch `LOFT_NO_LAZY_SPLIT`; trace `LOFT_TRACE_LAZY_SPLIT`
(each loop admitted, each declined with its reason, and whether the buffer is minted).
The falsifier is the emission pin with the value cells beside it — there is no assumption
to re-derive at run time, so `(R-Switch)`'s second form applies: cells
`tests/scripts/157-lazy-split.loft` s1–s21 (falsified by dropping the trailing piece:
`s1 3 7 ab|cde|` for `s1 4 6 ab|cde||f`), pins `tests/lazy_split.rs`.  Sites:
`hoist::lazy_splits`, `hoist::lazy_split_block`, `Output::lazy_split_reader`,
`Output::lazy_split_borrows`, the bind in `Output::output_set`, the element read in
`LazySplitNextEmitter`, the length test in `IntCompareEmitter`, the pre-eval exemption
in `Output::collect_pre_evals_inner`, the null decl in `Output::emit_null_dbref`,
`codegen_runtime::lazy_split`.

### A lookup by one integer key takes the typed entry

```
  (R-TypedKeyed)  c[k] where c's type is `hash<T[f]>` — a HASH, with exactly ONE key
                  field, of an integer width the hash's pre-resolved equality lists
                  (`integer`, `long`, `i32`, the unsigned 4-byte form) — is emitted
                  as the typed lookup `OpGetHashLong(c, type, k)` with `k` handed over
                  as the integer it is.  It answers what `OpGetRecord(c, type, [k])`
                  answers, by construction: the same digest of the same value under
                  the table's seed, the same home bucket, the same walk to the first
                  empty bucket, the same equality on the key field; and everything
                  that is not that plain answer — a holder with no record, an ABSENT
                  collection, a miss on a collection a lazy source is bound to — is
                  handed to the general entry with the same key, so there is no case
                  the two can answer differently.  Every other collection kind
                  (`index`, `sorted`, a vector), a text key, a compound key, and a
                  width the pre-resolved forms do not list keep the general call.
```

**In words.** `(R-Escape)` is not needed here — nothing is removed or reshaped, the
lookup is the same lookup.  What changes is how much of it is re-derived per call.  The
general entry is handed a slice of tagged key values and a type number, and learns from
them, per lookup, what the schema said once at generation time: that the collection is a
hash (a dispatch over its type row), that the key is one integer (a `Content` built to be
matched apart, a pre-resolved key chosen and then re-dispatched per bucket).  After the
walk itself was cut to ~1.2 buckets a lookup (`bench/portal/analysis/keyed.md`, L5) that
prologue had become most of a lookup: 613 instructions, of which the hash and the walk
are about 250.  The typed entry is the table's header reads, the digest inline, and the
walk compiled for the key's kind: **613 → 444 instructions a lookup**, `hash_find` −13 %,
`hash_update` −14 % on x86-64.  The condition is read off the schema the emitter already
bakes type numbers from (`Output::stores`), and the runtime re-reads the key's position
and kind from the same row, so the emitter asserts only the SHAPE (hash, one key, integer)
and never an offset.  Switch `LOFT_NO_TYPED_KEYED`.  The falsifier is
`LOFT_KEYED_VERIFY=1`, which answers every typed lookup through the general entry as well
and panics where the two differ (a typed lookup sabotaged to hash the wrong value fails
the native run of the cells, panics under the verify naming both answers, and passes
under the switch).  Cells `tests/scripts/158-keyed-fast-paths.loft`; pin
`tests/keyed_fast_paths.rs::a_hash_lookup_by_one_integer_key_takes_the_typed_entry`.
Sites: `OpGetRecordEmitter` (`src/generation/ops/key_ops.rs`), `Output::emit_long_key`,
`codegen_runtime::OpGetHashLong`, `hash::find_long`.  The APPEND half — a keyed append
that skips the per-element type-table walk the same way — is not built: its emission
runs through the mint and push rewrites (`hoist::mint_path`), and it is priced at ~300 of
an insert's 2,100 instructions.

### A result vector adopts the return buffer

```
  (R-RetAdopt)   a function VALUE-returning a plain vector through a hidden buffer
                 (the shape-A ABI: a separate `__retbuf` attr; a borrow return
                 delivers nothing and declines), whose EVERY delivery into that
                 buffer sources ONE result local — bound once from its own
                 witness, never rebound, never captured, the buffer serving
                 nothing else, every Clear+Append delivery inside its
                 `one_buffer_vec_copy` block — has that local ADOPT the buffer:
                 the declaration aliases it (allocating one exactly as the
                 witness would have been when the caller offered none), the
                 witness store is never allocated, the delivery pair inside the
                 block emits as nothing while the block's OTHER statements — the
                 scope-exit frees, the returned value — stay, the bare ENTRY
                 clear stays (it is the buffer's reuse contract across calls),
                 and `OpReplaceVector` deliveries stay as emitted — they are
                 aliasing-safe at run time and self-detect the adopted no-op.
                 The witness-promoted ABI (the buffer parameter IS the witness)
                 already delivers copy-free through that same self-detection and
                 declines here.  A § V-j placement into the adopted local's
                 witness re-targets the return buffer.
```

**In words.** @PLN157 § V-u.  Before it, a shape-A function copied its WHOLE result
vector into the caller's buffer at every exit — per element a claim in the buffer's
store plus a deep copy, at every recursion level (`vector_add` → `copy_claims` was the
top inclusive chain of `fronds`' profile, carrying most of the claim/free-tree time
with it).  Adoption builds the result where it must end up, so the exits deliver
nothing and the buffer's backing capacity survives across calls.  Falsified LIVE,
twice, at the scoping that carries the rule: blanking every `OpClearVector(buf)`
(the entry clear included) corrupts the fronds probe — the reused buffer accumulates
across calls — and collapsing the delivery BLOCK whole drops its scope-exit frees
(2 stores leaked in the V-j corpus, caught by `LOFT_NATIVE_LEAK_CHECK`).  Switch
`LOFT_NO_RETBUF_ADOPT`; `LOFT_TRACE_ADOPT=1` names the gate that declined.  Sites:
`hoist::ret_adopt`, the init arm in `Output::output_set`, the witness skip in
`OpDatabaseEmitter`, the scoped pair blanking in `TextDispatchEmitter`, the
`in_adopt_delivery` scope in `Output::output_block`, the placement re-target in the
§ V-j hook.  Shipped: standalone `fronds` −9.5 % (381–396k → 350–357k ns/op) and
`smooth` −14.4 % (2 205 → 1 887), hashes exact, cells leak-free under poison.

### A filling loop is one slice fill

```
  (R-Fill)       a counted loop `for i in lo..hi { v[base + i] = c }` — the body ONE
                 in-place scalar set (R-InPlace) at field 0 of a plain scalar vector
                 reached by a pure path a header is held for (R-Header), the index
                 the loop variable or an invariant plus it, the value and the bound
                 invariant — runs as ONE range test and one slice fill when every
                 index lands in [0, len) with no overflow on the way and the range is
                 non-empty; otherwise the per-element loop runs, unchanged.  The
                 counters are left where the loop leaves them.
```

**In words.** @PLN157 § V-ae.  The elements the fill writes are exactly the elements the
loop would write, in the only case the fill takes: a contiguous run inside the vector.
Every other case — a negative index (which counts from the end), a range past either
end, an empty range, an overflow in `base + i` — is the loop's own, so the fill declines
at run time and the loop runs; the emitter never has to know which case it is.
`Stores::fill_hoisted` is the guard and the fill, `Store::fill` the primitive (one bounds
check at each end, an unaligned store per element the optimiser vectorises).  A body
that reads the vector, a value that depends on the loop variable, a strided index or a
second statement keep the loop.  Switch `LOFT_NO_FILL_HOIST`; falsifier
`LOFT_HOIST_VERIFY=1` (the fill re-derives the header) and the count sabotage recorded in
`tests/scripts/157-fill-hoist.loft`; `LOFT_TRACE_FILL=1` names the check that declined a
loop.  Sites: `hoist::fill_loop`, `Output::fill_fast_path`, `Stores::fill_hoisted`.

### A nest whose arithmetic cannot fault runs plain

```
  (R-BoundedNest) an innermost counted loop whose body is ONE accumulate over
                 `?`-discharged element reads — `acc = acc + term`, the term a
                 chain of `+`, `-`, `*` and negation over literals, the loop's
                 counters, variables the loop does not write, and reads
                 `v[chain]?` of `integer` vectors — runs with the processor's
                 PLAIN operators when a guard, evaluated once at the loop's
                 entry, proves that no operation in it can fault: the range's
                 ends are not the sentinel and the trip count is positive; no
                 invariant is the sentinel; every vector read has a known
                 element bound (no stored null); and the MAGNITUDE BOUND of every
                 chain and of `|acc| + trips × bound(term)` — literals by value,
                 counters by the larger range end, reads by their vector's bound,
                 `+`/`-` summing and `*` multiplying, each step checked — fits
                 the type.  A bound that fits means every true intermediate
                 fits, so the plain operator answers exactly what the checked
                 template would; otherwise the checked loop runs, unchanged.
                 An element bound is a fact about VALUES, taken once at the
                 outermost loop whose body neither writes the vector (an element
                 or field set through it, an append, a rebind of its root) nor
                 hands it to a call — never at the nest's own prelude.
                 When every read's chain names the counter at most ONCE (so it
                 is affine, and its extremes over the range are its values at
                 the two ends), the guard also requires both ends in [0, len)
                 for every read; then each read is one load through the held
                 base with no bounds test and no null select — the element
                 bound has already ruled out a stored null.  A read outside the
                 range at either end declines the nest whole.
```

**In words.**  @PLN157's guarded plain nest, the admissible successor C120 names: the
integer closure is not taken (R-Counter) because an overflow's null must propagate on both
backends, so the plain form is admitted only where a fact ESTABLISHED before the arithmetic
runs says no overflow can occur — which is exactly what the guard checks, in the same
magnitude-bound arithmetic the rule states, with `checked_*` operators whose failure is the
decline.  The reads stay what they were (a bounds-tested load and the discharge's null
test); what goes plain is the index chain, the product and the accumulate — the taps of a
resample, where they were 12 checked operators per tap and ~55 checking instructions in
front of the arithmetic.  The bound's placement is the cost model: one linear pass per pass
of the nest's enclosing loop, against the nest's taps under it; at the nest's own prelude
it would be the nest's cost again, which is why a function-level nest with no enclosing
loop declines.  A stored null, an accumulator near the maximum, a null invariant and a
vector written by the enclosing loop are the cells that must DECLINE
(`tests/scripts/157-bounded-nest.loft` n2, n3, n7c, n8, n16, n17); a read one past the end
and a chain that goes negative decline the RAW form and answer through the checked loop
(n19, n23), while the last element, a negative coefficient and a constant index are in range
(n18, n20, n22) and a counter named twice is not affine (n21).  Switches
`LOFT_NO_BOUNDED_NEST` and, one step finer, `LOFT_NO_NEST_RAW_READS` (the plain arm keeps its
bounds-tested reads); falsifier `LOFT_HOIST_VERIFY=1` (every plain operator's answer is
compared with the checked template's at the operator — `ops::nest_verify` — and every raw read
with the checked read); trace `LOFT_TRACE_NEST=1`.  Sites: `hoist::bounded_nest`,
`hoist::nest_read_paths`, `Output::nest_fast_path`, `nest_bound_expr`, `nest_chain_at`,
`ops::int_arith::nest_form`, `ops::vector_ops` (the raw read), `Output::output_if_inner` (the
select), `vector::abs_bound_i64`.

### A witnessed buffer is allocated once, not minted per call

```
  (R-Reuse)      a hidden return buffer whose result local is WITNESSED (O-Buffer) is
                 allocated ONCE, right after its null-init, so a callee that builds
                 into it reuses one record per call SITE instead of minting a store
                 per CALL.  The witness is the whole condition: an allocated buffer
                 outlives the call, so a site that frees the result plainly would
                 release it and the next turn would write a record back in the pool.
                 Also required — the buffer is used ONCE and its result local is
                 assigned ONCE, since a second use has one guarded site and one this
                 did not read, and a reassignment frees the store it displaces.
                 A buffer whose callee MINTS the store its result adopts (O-Move at
                 a plain local's first bind, @PLN164 B1) is paired for the guarded
                 free alone and NOT allocated here: handed non-null to a callee that
                 rebinds its promoted local from a call, it is freed by that rebind.
                 Every other buffer keeps its per-call mint.  A reused record is
                 REFILLED, so each reuse releases what the record held first
                 (H-ClearRelease, its record clause).
```

**In words.** @PLN157 § V and § V-af.  `scopes::reuse_record_buffers` inserts the
`OpDatabase` and `scopes`'s pairing supplies the witness (`Scopes::minted_pairs` names the
adopt-at-bind pairings it skips — @PLN164 B1's matrix measured the use-after-free that
pooling one produces on the interpreter, plan 51 cluster 3's shape); the positive control
`LOFT_NO_RETBUF_WITNESS_GATE=1` allocates every buffer, guarded or not, and
`LOFT_STRICT_STORES=1` then reports the use-after-free at exactly the sites the gate
declines — which is how the condition is falsified rather than asserted.  Switches
`LOFT_NO_RETBUF_REUSE`, `LOFT_NO_JOIN_BUFFER_WITNESS`.  Sites:
`scopes::reuse_record_buffers`, `scopes::tail_calls`.

### A leaf carries no frame

```
  (R-Leaf)       a function whose body calls no user function and no fn-ref — a LEAF
                 — is emitted without a shadow-stack frame push and without a fn-ref
                 buffer guard: it cannot recurse, cannot reach stack_trace / assert /
                 panic, and cannot sit between the frame that pushed a buffer and
                 the one that releases it.  The live-flip check stays.
```

**In words.** @PLN157 N4.  A runtime fault inside a leaf keeps its exact position
and loses only the innermost frame NAME from the chain.  Switch
`LOFT_NO_LEAF_PRELUDE`.  Site: `Output::is_elidable_leaf` and its use in
`Output::output_function`.

```
  (R-LeafChain)  in the LEAN tier, a function whose whole call tree is FRAMELESS is
                 emitted as a leaf is: every user function it reaches, transitively,
                 has a loft body, none of them lies on a cycle, and none calls a
                 fn-ref, runs `parallel` or yields.  A native user-level callee
                 (no loft body) makes the tree opaque, so the caller keeps its frame.
                 The named tiers keep every non-leaf frame.
```

**In words.** @PLN157 queue row 11.  Such a function cannot be re-entered while it runs, so
the depth cap needs no entry for it: a recursion that passes through it is still counted at
the recursive function's own frame, and the cap's report names that function — the nearest
frame — where the interpreter names the innermost call.  Nothing beneath it can push a fn-ref
buffer, so its buffer guard would always drop empty.  The lean frame carries no name, which is
why the rule is lean-only: in a named tier the frame is also what `stack_trace()` and a
panic's frame block read.  Switch `LOFT_NO_LEAF_CHAIN` (one step finer than
`LOFT_NO_LEAF_PRELUDE`).  Site: `Output::is_frameless_chain`.

### A fast path inlines; its cold half is outlined

```
  (R-LitHoist)   a loop-body vector LITERAL whose parts are invariant builds ONCE
                 per activation: `v: vector<σ> = ℓ` under a `for`, σ a no-heap
                 scalar, ℓ's parts literals, pure scalar ops, never-reassigned
                 by-value scalar parameters, or scalar-getter reads of value-const
                 record parameters — the emitter pre-declares `v` at function top
                 and guards the declaration (the one wrapped Set, or the flat
                 `OpDatabase · Set · pushes` run) on `v` being UNBOUND, so the
                 build runs once and every later iteration and re-entry reuses the
                 store.  `(Const-Value)` is per-NAME, so const alone cannot carry
                 cross-iteration invariance: the alias gate requires every variable
                 whose type can reach a record type ℓ reads to be a fresh-store
                 local (its record is this activation's, never the caller's) or a
                 value-const parameter itself used ONLY as a scalar-getter base.
                 The local's other uses must be reads the analysis can see whole:
                 For-iteration binds, `len`, its scope-exit free — an indexed use
                 declines, because the same OpGetVector node is the lvalue base of
                 an element WRITE (context-blind); an append, an escape, a rebind,
                 a heap element, and two admitted locals sharing a sanitized name
                 (they would share the one fn-top binding) all decline.

  (R-CompleteWrite) a record built by a COMPLETE literal group skips the default
                 prefill: the parser's lowering writes EVERY field explicitly — a
                 named value, the declared default, the interned empty text, the
                 null sentinel of a nullable, `false`, the variant TAG — so where
                 the emitter proves coverage of every schema field position by the
                 group's contiguous `OpSet*`s (the tag through `OpSetEnum` at 0),
                 `set_default_value`'s walk (or its all-zero `zero_range`, which
                 duplicates the zero-on-claim) writes nothing that survives, and
                 the site calls the no-prefill twin (`OpDatabaseNP` /
                 `OpNewRecordNP`).  An uncovered field — a nested struct arriving
                 by `OpCopyRecord`, a vector field bound by an append, a
                 `__nullable` element whose discriminant no `OpSet` names — keeps
                 the prefill: the check can only DECLINE the elision.  `db_vars`
                 is keyed by the local (every `OpDatabase` site must cover);
                 `mint_tps` by the element type (every mint group in the function
                 must).  The interpreter keeps the prefill and is the oracle.

  (R-ElemFirst)  a local vector consumed EXACTLY ONCE as a record-literal field of
                 an append (`out += [R { f: v, … }]`) is built INSIDE the appended
                 element: the element is minted at the FIRST paired temp's
                 declaration site (the mint claims the slot; the LENGTH BUMP stays
                 at the append's finish, so the element is invisible until then —
                 and a re-mint overwrites the same invisible slot), each temp is
                 bound to the element's own field slot (a vector value IS the ref
                 to its handle slot), the build targets it in place — an adopted
                 callee then delivers its record DIRECTLY into the element — and
                 the append site keeps its scalar sets and finish while the
                 reservation, the mint, the paired handle-zeros and the paired
                 copies vanish.  The invariant: every field is written before the
                 finish — by the prelude mint's prefill, a paired build, or the
                 kept sets.  Gates: declaration and append are top-level
                 statements of ONE block (an if-arm append strands unfinished
                 elements per skipped iteration) — or (@PLN164 E-2b) the append
                 stands in EACH arm of an `if` that block holds, into the same
                 destination with the same temps, and the two arms share ONE early
                 element: the second arm's mint becomes an alias of the first's,
                 each arm keeping its own writes and its finish; no statement
                 between them jumps out of it (a `return`, `break` or `continue`
                 strands the element the same way, holding what the temp built);
                 nothing between them names `out` in a way that reaches its
                 collection (for a FIELD destination a naming that reaches only
                 another field moves nothing, `namings_avoid_place`); and nothing
                 between them READS A VIEW the early mint may have moved — the
                 mint is the append's GROWTH brought forward, and `(B-Disturb)`'s
                 copies were placed against the growth where the IR has it
                 (loft#1553): a local whose deps close over `out`'s store, and,
                 where that store is a caller's, any heap parameter (a caller may
                 hand an element in beside its container), decline — except a
                 local whose every binding views a SIBLING field, whose
                 collection the growth does not move; the temp's declaration is its ONLY
                 binding (a rebind points the temp at another store, and the
                 element keeps what the declaration built); the temp's
                 whole-function mentions reconcile to its build plus the one copy
                 (a later read or a second consuming append declines — though an
                 INTERVENING append merely reads the slot and the later one may
                 pair), where an admitted function's own EXIT copies are not
                 mentions (the value form drops them, `(O-ViewField)`); `out` is
                 never rebound, and its element a plain struct stored inline.
                 `out` is a local vector, or (@PLN164 E-2) a collection FIELD of a
                 record variable — a parser appending into `sc.ops` — and the temp
                 is a local built from a literal, or (E-2) the result of a call
                 handed a hidden buffer that serves it alone: the call is then
                 handed the element's field as that buffer, which is `(R-Place)`'s
                 "the buffer IS the place" with the place made to exist by the
                 early mint, and it needs `(R-Place)`'s callee condition — a
                 callee that FILLS its buffer and never mints into it — and its
                 argument condition — no other argument names `out`.  A value the
                 list does not declare keeps its copy into the early element.  The interpreter keeps the
                 temp-store build and is the oracle.

  (R-ValueRecord) a function whose result is a PLAIN NO-HEAP RECORD of at most six
                 scalar fields (an `integer` only at its 8-byte width) returns those
                 fields BY VALUE — a Rust tuple, in registers — instead of writing them
                 into a return buffer the caller then reads back.  Admission is a
                 FIXPOINT over two gates, because a tail may forward another admitted
                 function's result and a site may bind a branch of admitted calls:
                 the BODY gate asks that every result position be a VALUE LEAF — an
                 `Object` build of the function's own record (the tuple of its writes),
                 a call to an admitted function (its tuple, forwarded), a value local,
                 or a borrowed VIEW of the record (the tuple of its field reads; a view
                 is never freed, so reading it is all the value form owes) — a heap
                 field of the record that (O-ViewField) admits is a VIEW LEAF, the
                 reference to the place it views delivered in the tuple instead of a
                 claim of its own, and a record whose every heap field is such a leaf
                 counts as no-heap here — and that
                 the return buffer be mentioned only where the value form drops the
                 mention (a converted `Object`, a dropped buffer argument, a free); the
                 SITE gate asks that every admitted call stand where a tuple is
                 consumed as one: a result position of an admitted body, the right of
                 a VALUE LOCAL (a local whose every assignment is a value shape and
                 whose every use is a field read, a free, a store-identity test, a copy
                 FROM it, or a `return`), or a dropped statement; an argument, a field
                 value, a return from a non-admitted function or a local that also
                 takes a record declines the callee — except a FORWARD: a call handed a
                 buffer `b` whose answer is bound back into `b` (the lowering of
                 `return g(…)` in a function that keeps its record, and of a call arm of
                 a value branch whose join takes a record) writes the tuple
                 into `b` at the site exactly as `g`'s own exit would have — `b` minted
                 where it is absent, every scalar set, a view part's vector copied —
                 and evaluates to `b`, so `g` stays admitted everywhere else.  The record form's three guards
                 then read as the tuple says: a free of a value local is nothing, a
                 store-identity test against one is always distinct (so the free it
                 guards is unconditional — the ownership change a selecting tail
                 carries: the record form declined that free exactly when the buffer
                 was the result), and a copy FROM one MATERIALISES the tuple into the
                 destination with one typed write per field (a view part: the field
                 emptied, then the deep copy of what it views), which is how a builder
                 delivered into a push slot lands without a call or a buffer; and a
                 buffer local whose every mention is one of those drops — a dropped
                 argument, a free, the pool's release before a reuse, a test against a
                 value local — is DEAD, and its
                 mint and its frees emit as nothing.  An OWNED
                 record at a tail declines: the value form would have to mint per call
                 what the buffer form reuses.  Every function a fn-ref dispatch can
                 reach declines, read from the emitter's own arm scan
                 (`fnref::dispatch_arms`) so the two cannot drift.  Two boundaries keep
                 the record contract: the LIVE-RELOAD arm answers a `DbRef` and so
                 reads the fields back out of it, and a library's CDYLIB BRIDGE
                 materialises the tuple into the destination record it already owns —
                 so the C ABI is unchanged while loft-to-loft calls inside the library
                 take the value path.  A compiler `__lift_` temp is never a VIEW leaf:
                 it owns the store its whole-record bind mints (the record form hands
                 that store up as the result), so read as a view it leaks one record
                 per call; bound from a bare view it is a VALUE LOCAL — the tuple of the
                 view's reads — and a whole-value read of a value local at a value
                 position (the tail of a branch arm, the right of a value local, a
                 return) is a use the tuple serves.  A body's value locals are admitted
                 TOGETHER — the join local of a selecting branch and the lifts its arms
                 bind justify each other — by an optimistic growth from the body's
                 views, admitted calls and `Object` builds, pruned to a consistent set.
                 And a whole-record bind INTO a value local takes the plain assignment:
                 the mint and the deep copy `@FR-B-Copy` spells for a record local are
                 the store the value form exists to drop (a generic instance's
                 selecting tail lowers as a statement join whose arms each bind the
                 join local from a parameter's view).

  (R-Cold)       a runtime helper on the per-element fast path — an element read or
                 write through a holder, a length, a bounds test, a fault note, a
                 diagnostics hook — must INLINE into the emitted code, and whatever
                 shares its body but not its frequency (an error report, a growth
                 step, a watch/verify hook, the off-fast-path re-derivation) is
                 OUTLINED as its own `#[cold]` `#[inline(never)]` function.  The
                 split changes no observable behaviour; what it protects is rustc's
                 SIZE decision — a generic `#[inline]` body that carries its cold
                 half loses the inline at every call site, and the whole fast path
                 pays a call for a compare and a load.  `#[inline(never)]` on the
                 cold half is load-bearing, not a hint: without it rustc folds the
                 halves back together.
```

**In words.** @PLN157 § V-h found the class (`hash` paid a third of its row for the
un-inlined fault note BESIDE its checks, not for the checks); loft#1508 and its write
twin re-found it inside `get_elem_hoisted`/`vec_set_hoisted` (7.2 % + 3.2 % of the
`loft_planet` profile, out of line at all 1,302 call sites of one program); the
always-off watch hook cost 1.6 % of a real program the same way.  No switch — this is
a layout discipline, not a semantics rewrite, so (R-Switch) does not apply; the checks
are the two instruments: `scripts/native_call_census.py` (which fast-path symbols
still cross the rlib boundary as calls) and `scripts/inline_audit.py` (which
`#[inline]` functions rustc declined — an `#[inline]` function with an out-of-line
symbol and self time is the candidate).  Sites: `vector::get_elem_hoisted_cold`,
`Stores::vec_set_hoisted_cold`, `Stores::note_format_fault`'s split,
`Store::raise_out_of_bounds`, `Store::shadow_write`, `State::verify_slot`,
`State::mark_stale_handles`, `Stores::watch_oob_text_report`, and `ops::text_character`
(the ASCII byte is answered inline; the multi-byte snap-back and decode are
`text_character_wide` — a `for c in text` walk paid a call per character, 580 per parse
on the drawing bench).

### A result is built where it will live, moved at its last use, and written over a place

```
  (R-Place)      a call whose result has ONE owning destination on every path that
                 keeps it — a field or element of a record living in store S, whether
                 or not that record exists yet — is handed a return buffer CLAIMED IN
                 S (R-Callee: a buffer may be a record the caller offered), so the
                 result is never minted in a store of its own and the later store into
                 the destination is a relocation within S (R-MoveLast).  When the
                 destination place EXISTS at the call and no argument of the call
                 reaches it, the buffer IS the place and nothing moves.  Declines: a
                 path that reads the result after a RELOCATING store (the destination
                 owns it then, and B-Copy would show — where the buffer IS the place, a
                 read of the result reads what was written and needs nothing); an
                 argument that reaches the destination's store (source and destination
                 alias); a callee that may hand back a store it did not mint (O-Opaque:
                 empty deps license nothing), and, where the buffer IS the place, a
                 callee that MINTS into its buffer on some exit (a returned vector
                 literal does), which would mint over the place it was handed; a callee
                 that on some exit answers a store other than the buffer it was handed
                 (the destination would hold the writes of the exit not taken); a
                 `?`/`??` discharge on the result; a destination that does not
                 unconditionally EXIST at the call — a field of a struct-enum VARIANT
                 exists only while the value holds that variant.  A member of a linked
                 collection group is NOT a decline: the group's maintenance brackets
                 the fill (Col-Group), and a fill that arrives through the buffer
                 instead of an append is one it must see.  Every other call keeps its
                 own buffer.
  (R-MoveLast)   a record-literal field or a field/element assignment whose source is
                 a LOCAL the ownership oracle marks OWNED (O-Owner: never a parameter,
                 a view, a `&` link or a witnessed local) and DEAD on every path after
                 the assignment (O-Complete: no read, no rebind, no hand-off, no
                 free-guard that names it, in this frame or through a callee it is
                 passed to) takes the source by RELOCATION when source and destination
                 share a store — the record's bytes move, its heap handles keep their
                 claims, the source's block is released at the move, and the exit of a
                 path that moved it frees NOTHING, because the state of the local at
                 every exit is a fact the rewrite establishes and writes into the IR;
                 no runtime stand-in (a zeroed source, a null rebind) is substituted
                 for it — and keeps B-Copy's deep copy when they do not: a cross-store
                 move copies every claim anyway, so the rewrite has no gain there, and
                 R-Place is what brings the source into the destination's store first.
  (R-InPlaceLiteral) an assignment of a record LITERAL to an existing place — an
                 element `v[i] = R { … }` or a field `o.f = R { … }` — writes the
                 literal's fields into that place instead of building the literal in
                 a temporary store and copying it: every field expression is
                 evaluated BEFORE the first write (a field may read the old value
                 through a view of the same place), the old value's owned heap is
                 released before its slot is reused (H-ClearRelease, per field), and
                 every field the literal omits is set to its declared default
                 (loft#914) — so the place holds exactly what the copy would have
                 left.  An absent element keeps the path the copy takes today; a
                 literal whose field reads the place through a call the walk cannot
                 see declines.  The place is written once per FIELD, so the ELEMENT
                 clause holds only where re-deriving the receiver for each write is
                 the same place with no effect the program can see — a repeatable base
                 and an index that is a literal or a bare variable (`v[bump()]` would
                 call `bump` per field; `v[len(v) - 2]` re-reads a length the writes
                 may move).  An element whose type owns a COLLECTION field declines:
                 staging a field read of the slot's own collection binds a VIEW
                 (B-View-Base), and the write releases what that view names before the
                 value is stored back.  A member of a linked collection group declines:
                 its unlink and relink bracket a whole-record write (Col-Group), and a
                 write straight into the slot leaves the keyed member holding the
                 record under its old key.  A NESTED literal that initialises an embedded record
                 field of a FRESH record — an appended element or a construction
                 temporary, `sc.ops += [Op { paint: Paint { … } }]` — is written into
                 that field the same way; nothing can read a fresh record while it
                 is built, so it needs no staging of its own.
  (R-Prefill)    the default prefill of a minted record — the declared defaults, the
                 null sentinels and the variant tag a partial literal leaves to the
                 type — is ONE block write of a per-type IMAGE computed once from the
                 declaration, never a field-by-field walk; a field whose default is
                 not a fixed byte pattern keeps the walk for itself alone.  The image
                 is what the walk writes, so no value changes.
```

**In words.** @PLN164's rewrite list (its README § *The rewrite list*), derived by
reading the drawing library's natural `parse_poly` beside the version a programmer
who knows the store model would write, and asking what the compiler must PROVE to
reach the second from the first without a line of the first changing.  (R-Place)
is C2 and the half of B2 that matters: `pp_paint = read_paint(s)` whose only owning
destination is `Op.paint` inside the scene gets a buffer claimed in the scene's
store, and `paint: pp_paint` at its last use is then (R-MoveLast)'s relocation
instead of a deep copy; `pts: smooth_pts(…)` with the op already appended is the
"buffer IS the place" clause.  (R-InPlaceLiteral) is C1 (`sc.elems[idx] = Elem {
… }` in `acc_pts`, whose `ename: ap_e.ename` reads the very slot it overwrites —
the staging clause) and C6 (the nested clause, `Parser::nested_literal_place`, switch
`LOFT_NO_NESTED_IN_PLACE`), (R-Prefill) is C4.  What each needs from the IR is in the plan's
table: the def-use classes of a value (its owning destinations against its read-only
uses), per-path liveness at a set (the `avoidable-copy` lint already computes it and
codegen does not yet read it), the disturbance walk `B-Ref-Reshape` runs for `&`,
and `R-Callee`'s writer summary.  (R-Place)'s callee clause — *a callee that on some
exit answers a store other than the buffer it was handed* declines — is what
`Parser::literal_exits_into_buffer` makes true for every callee whose exits are ALL
literals or CHAINS: once the tail's delivery is decided and the buffer is still the
unpromoted `__retbuf` — or a buffer only chains name (`return no_mk()`, a tail `no_mk()`,
which rename it after the chain's work ref; `Parser::chain_return_buffer_var`) — each
mid-body `return S { … }` builds into it as the tail literal does, and so does a literal
TAIL beside chain exits (switch `LOFT_NO_LITERAL_EXIT_BUFFER=1`), where before it minted a
store per exit and the caller adopted whichever came back.  A chain exit answers what its
callee wrote into the same buffer, and a chain always returns, so no literal exit meets a
value another statement put there.  A callee with a promoted named local beside a literal
exit keeps its per-exit stores.  (R-Prefill) is built (C4): its ONE site is
`Stores::prefill_from_image` in `src/database/structures.rs`, the image is READ BACK
from the walk's first run over a zeroed span rather than computed a second way, the
switch is `LOFT_NO_PREFILL_IMAGE=1` and the falsifier `LOFT_PREFILL_VERIFY=1` (the walk
re-run after every image write, a panic where they disagree).  (R-Place), (R-MoveLast) and (R-InPlaceLiteral) are built too, each
in the @PLN157 shape with its own switch and cells: the callee clause above
(`LOFT_NO_LITERAL_EXIT_BUFFER`), the relocation of a call's result into an appended element
(`LOFT_NO_PLACE_RESULT`, `OpPlaceRecord`/`OpMoveRecord`), the buffer that IS the place
(`LOFT_NO_BUFFER_IS_PLACE`), the element written in place (`LOFT_NO_ELEMENT_IN_PLACE`) and
the nested literal (`LOFT_NO_NESTED_IN_PLACE`).  Each implementation admits less than its
rule — the B2 relocation only a destination in an element appended to a PARAMETER's
collection, outside a loop, with no second destination or host — and a narrower admission
costs the rewrite and never a value.  The declines written into the rules above are the
ones a MEASUREMENT bought (@PLN164's README § C1, § C2 *The restrictions, re-derived*).
The rules are written BEFORE the phases so that a question met while building one is
answered here rather than decided in the code.

## Validating the emitted routines against their assumptions

Every rule above is an ASSUMPTION the emitted Rust makes about the loop it sits in, and the
emitted routines grow with each rewrite — a push header beside a twin call beside a view.
Two instruments check the assumptions, and the chapter is not complete without both:

- **at run time**, the checking forms under `LOFT_HOIST_VERIFY=1` (R-Switch): every
  holder-served read, write and push re-derives what it assumed and panics on a stale
  value — the cell corpora and the script guards run under it;
- **at emission time**, the EMISSION AUDIT (`scripts/emission_audit.py`, @PLN157 § V-r):
  over a `--native-emit` output it reads each function's preludes and holder uses and
  checks R-State (one `let __vh_N` / `__ph_N` per path expression per frame, every
  `get_elem_hoisted` / `vec_set_hoisted` / `push_hoisted` / `.len` naming the holder bound
  for its path), R-Refresh (no template append or growing op on a path while a holder for
  it is live in an enclosing frame) and R-Inputs (a twin call hands in exactly the holders
  its parameters name).  It runs over the cell corpora and the consumer bench in the gate
  and is the instrument that would have flagged the double holder before any run.

## Deviations

**OPEN: 0** (2026-09-17).

- **D-rw-3 — OPENED AND CLOSED 2026-09-17 (loft#1553).**  `(R-ElemFirst)` moves the append's
  mint — its GROWTH — to the temp's declaration, and the gate asked only whether a statement in
  between NAMED `out`.  A view of `out`'s element is another variable: `e = out[0]; ws = [];
  ws += […]; w = e.fid; out += [F { fpts: ws }]` left `e` a view (the scope pass saw the growth
  at the append, after the read), and on `--native` it read the vector record the early mint
  had relocated — `4609434218613702656` for `100` at the eleventh element, silently, since
  @PLN157 § V-z.  Found by @PLN164 E-2b's growth-boundary sweep.  Closed at the gate: the
  window may not read a local whose deps close over `out`'s store or, where that store is a
  caller's, a heap parameter; a view of a sibling field is spared by the scope pass's own place
  model.  Guard `tests/scripts/an-element-view-read-before-an-element-first-append-is-not-moved.loft`;
  cells `g25`–`g27` of `164-element-place.loft` for the field destination.

- **D-rw-2 — OPENED AND CLOSED 2026-09-17 (loft#1552).**  `(R-ElemFirst)` says the temp is consumed
  exactly once, and the gate counted its READS but never its BINDINGS: `p: vector<Pt> = [];
  if c { p = mk(n) }; out += [Op { pts: p }]` bound `p` to the early element's field at the
  declaration, the conditional rebind pointed `p` at the call's store, and the suppressed copy
  left the element holding the empty vector — `len(out[0].pts)` 0 where the interpreter says 3
  (`1000` for `1093` in the guard), silently, on `--native` since @PLN157 § V-z.  Found by
  @PLN164 E-2's cell `g13`, which widened the same gate.  Closed at the gate: the declaration
  must be the temp's only binding.  Guard
  `tests/scripts/a-rebound-element-first-temp-keeps-its-copy.loft`.

- **D-rw-1 — CLOSED 2026-09-09.**  `one_op_wrapper` asked the body's SHAPE and not
  the definition's ORIGIN, so a user function whose body is one op (`fn reader(w:
  W) -> integer { w.a }`) was emitted as its op; the wasm live-dispatch probe
  flipped it to the interpreter and counted 0 dispatches where it expected 2.  The
  rule was always (R-Wrapper)'s "stdlib"; the code now asks `def.source() ==
  STD_SOURCE`, and cell c11 of `V-o-wrapper-op-cells.loft` pins the user call.
  Found by the GitHub gate on the rebased tree (run 34323456806); the local
  curated set never runs the wasm probe.
