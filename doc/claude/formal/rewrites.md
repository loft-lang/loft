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
                 conditions allow.  (C121, owner 2026-09-15.)
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
library, the live-reload arm, and a record's layout in a store (C121's corollary).
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
  (R-Base)       in a loop that GROWS no store — no push, no mint push, no
                 null-discharge buffer minted in its body; in-place sets, store-free
                 ops, store-free or in-place-only callees and frees may run — a
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
                 form re-derives it at every use.

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
                 owner's line to move.

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
                 Every other buffer keeps its per-call mint.
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
                 elements per skipped iteration); nothing between them mentions
                 `out`; the temp's whole-function mentions reconcile to its build
                 plus the one copy (a later read or a second consuming append
                 declines — though an INTERVENING append merely reads the slot and
                 the later one may pair); `out` an owned never-rebound plain
                 vector of a plain-struct element.  The interpreter keeps the
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
                 takes a record declines the callee.  The record form's three guards
                 then read as the tuple says: a free of a value local is nothing, a
                 store-identity test against one is always distinct (so the free it
                 guards is unconditional — the ownership change a selecting tail
                 carries: the record form declined that free exactly when the buffer
                 was the result), and a copy FROM one MATERIALISES the tuple into the
                 destination with one typed write per field, which is how a builder
                 delivered into a push slot lands without a call or a buffer; and a
                 buffer local whose every mention is one of those drops — a dropped
                 argument, a free, a test against a value local — is DEAD, and its
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
`State::mark_stale_handles`, `Stores::watch_oob_text_report`.

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
                 path that reads the result after the store (the destination owns it
                 then, and B-Copy would show); an argument that reaches the
                 destination's store (source and destination alias); a callee that
                 may hand back a store it did not mint (O-Opaque: empty deps license
                 nothing); a callee that on some exit answers a store other than the
                 buffer it was handed (the destination would hold the writes of the
                 exit not taken); a `?`/`??` discharge on the result.  Every other
                 call keeps its own buffer.
  (R-MoveLast)   a record-literal field or a field/element assignment whose source is
                 a LOCAL the ownership oracle marks OWNED (O-Owner: never a parameter,
                 a view, a `&` link or a witnessed local) and DEAD on every path after
                 the assignment (O-Complete: no read, no rebind, no hand-off, no
                 free-guard that names it, in this frame or through a callee it is
                 passed to) takes the source by RELOCATION when source and destination
                 share a store — the record's bytes move, its heap handles keep their
                 claims, the source is zeroed and its scope-exit free finds nothing
                 (R-MoveAppend's mechanics for one record) — and keeps B-Copy's deep
                 copy when they do not: a cross-store move copies every claim anyway,
                 so the rewrite has no gain there, and R-Place is what brings the
                 source into the destination's store first.
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
                 see declines.
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
the staging clause), (R-Prefill) is C4.  What each needs from the IR is in the plan's
table: the def-use classes of a value (its owning destinations against its read-only
uses), per-path liveness at a set (the `avoidable-copy` lint already computes it and
codegen does not yet read it), the disturbance walk `B-Ref-Reshape` runs for `&`,
and `R-Callee`'s writer summary.  (R-Prefill) is built (C4): its ONE site is
`Stores::prefill_from_image` in `src/database/structures.rs`, the image is READ BACK
from the walk's first run over a zeroed span rather than computed a second way, the
switch is `LOFT_NO_PREFILL_IMAGE=1` and the falsifier `LOFT_PREFILL_VERIFY=1` (the walk
re-run after every image write, a panic where they disagree).  The other three have no
switch, site or cell yet: each lands with its phase in the @PLN157 shape (a
`LOFT_NO_<unit>` switch, cells with hand-computed values on both backends under
`LOFT_STRICT_STORES`, `LOFT_POISON` and the leak gate, a guard with its
`@falsified-at:` receipt), and until then the copy each rule replaces is what runs.
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

**OPEN: 0** (2026-09-09).

- **D-rw-1 — CLOSED 2026-09-09.**  `one_op_wrapper` asked the body's SHAPE and not
  the definition's ORIGIN, so a user function whose body is one op (`fn reader(w:
  W) -> integer { w.a }`) was emitted as its op; the wasm live-dispatch probe
  flipped it to the interpreter and counted 0 dispatches where it expected 2.  The
  rule was always (R-Wrapper)'s "stdlib"; the code now asks `def.source() ==
  STD_SOURCE`, and cell c11 of `V-o-wrapper-op-cells.loft` pins the user call.
  Found by the GitHub gate on the rebased tree (run 34323456806); the local
  curated set never runs the wasm probe.
