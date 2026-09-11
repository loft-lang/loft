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
```

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

### An in-place scalar set disturbs no header

```
  (R-InPlace)    a fixed-width scalar set through an EXISTING address — an element,
                 a record field, a local — moves no record and changes no length,
                 so it cannot invalidate a header: a loop whose store writes are all
                 such sets keeps (R-Header).  What may run in such a loop is an
                 ALLOW-LIST (the store-free ops, IN_PLACE_SET_OPS); an op missing from
                 it costs the rewrite, never correctness.
```

**In words.** @PLN157 P4a.  Aliasing is free for headers under this rule — an
in-place write moves nothing, so no header, aliased or not, can go stale; that is
why the header needs no write set while a scalar (R-Scalar) does.  Switch
`LOFT_NO_WRITE_HOIST`.  Sites: `hoist::IN_PLACE_SET_OPS`,
`hoist::blocks_header_hoist`.

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
  (R-Inputs)     a callee admitted by (R-Callee), of a plain-struct parameter p it
                 never rebinds, has INVARIANT INPUTS: the scalar fields p.f its own
                 write set does not reach, and the vector paths p.g it views (R-View)
                 or indexes (R-Header).  A caller loop that holds, for the leaf
                 argument variable c it passes for p, the value of (c, f) under
                 (R-Scalar) and the header of (c, g) under (R-Header) — EVERY input
                 of the callee — calls the callee's TWIN: the same body emitted with
                 those values as extra parameters, read in place of the record — the
                 caller's holders handed in (R-State: a path held by a push header
                 hands in that header, current at the call).  A call missing any
                 input keeps the plain form; the twin exists beside the original,
                 never instead of it.
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
`Output::output_function`, the twin call in `Output::user_fn_call_body`.

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
                 pure bare-variable path P that TYPES as a plain vector —
                 OpPreAllocVector(P) · e = OpNewRecord(P) · writes into e ·
                 OpFinishRecord(P, e) — is a MOVER like a push, admitted beside
                 other holders under (R-Alias); P itself earns no holder.  A write
                 whose target is the FRESH variable e — the literal element's
                 in-place sets, and § V-d's OpCopyRecord delivery of a builder's
                 result into e (its source-free included) — contributes nothing to
                 (R-Scalar)'s write set: a record minted inside the body cannot be
                 named by a variable whose getter the prelude already ran.  The
                 same op names over a KEYED collection are a keyed insert — a mover
                 of OTHER records — and stay blocking: admission asks the TYPE,
                 never the op shape.
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
                 an element owning heap (text, a nested collection) keeps the
                 templates, because a raw slot carries stale bytes where a heap
                 handle's zero matters; a __nullable element's discriminant is not
                 a field the literal writes.
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

### A fast path inlines; its cold half is outlined

```
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
`State::mark_stale_handles`, `Stores::watch_oob_text_report`.

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

**OPEN: 0** (2026-09-09).

- **D-rw-1 — CLOSED 2026-09-09.**  `one_op_wrapper` asked the body's SHAPE and not
  the definition's ORIGIN, so a user function whose body is one op (`fn reader(w:
  W) -> integer { w.a }`) was emitted as its op; the wasm live-dispatch probe
  flipped it to the interpreter and counted 0 dispatches where it expected 2.  The
  rule was always (R-Wrapper)'s "stdlib"; the code now asks `def.source() ==
  STD_SOURCE`, and cell c11 of `V-o-wrapper-op-cells.loft` pins the user call.
  Found by the GitHub gate on the rebased tree (run 34323456806); the local
  curated set never runs the wasm probe.
