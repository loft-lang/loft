<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Native rewrite switches

Which switch turns off a `--native` rewrite, and what checks it.  Every switch here is read at
GENERATION time and changes only what `--native` emits; the interpreter never takes these
rewrites, so it is the values oracle for all of them.  A wrong answer on BOTH backends belongs
to [BOTH_BACKEND_SWITCHES.md](BOTH_BACKEND_SWITCHES.md) instead.

Each entry names its `LOFT_NO_*` switch (the first bisect step: the before-half of an A/B on
one binary), its falsifier (usually `LOFT_HOIST_VERIFY=1`, which emits the checking form) and
its `LOFT_TRACE_*` switch (which names each admission and decline).  The rules the rewrites
enforce live in [formal/rewrites.md](formal/rewrites.md) (`@FR-R-…`); the emitter's design is
[NATIVE.md](NATIVE.md) and [PERFORMANCE.md](PERFORMANCE.md) § Design.

## Vector headers, pushes and mints

**Vector-header hoist (loft#885, `--native` only, both switches read at GENERATION time):**
a loop the emitter proves writes NO store derives each vector's `(store_nr, record, length)`
once before the loop, so an element read is a bounds test plus address arithmetic (~2×).
The gate (`src/generation/hoist.rs`) is an ALLOW-list on purpose — an op missing from it
costs the optimisation, never correctness, which is the opposite of the five drifted
mutation deny-lists in PERFORMANCE.md § Design: P8. **`LOFT_HOIST_VERIFY=1`** emits the
checking form of every hoisted read (re-derives the header, panics on a stale one) — run the
suite under it after touching the gate; **`LOFT_NO_VECTOR_HOIST=1`** emits the pre-885 form,
which is the before-half of an A/B on one binary and the first bisect step for a
native-only wrong answer in a vector loop; **`LOFT_NO_ELEM_FUSE=1`** keeps the hoisted header
but leaves the scalar element read UNFUSED, one bisect step finer, and is the middle rung that
showed stage 2 is worth ~3.2× on top of stage 1 (projected ~1.4×) — more than the hoist itself,
because the second store resolution it removes costs more than the arithmetic it saves.
**`LOFT_NO_SCALAR_HOIST=1`** (@PLN157 P4c) reads every record scalar field per iteration
again — a loop that cannot write `lay.x0` otherwise reads it ONCE into a local, keyed by
(record type, offset) over the body's typed write set — and is the bisect step for a wrong
scalar in a loop over a record; `LOFT_HOIST_VERIFY=1` re-reads each hoisted scalar and
panics on a stale one.  **`LOFT_NO_VIEW_HOIST=1`** (@PLN157 § V-n) makes a `&`-bound vector
view (`d = &cv.data`) derive no header at its binding — with it on, the rest of the block
reads `d[i]` through one header derived once — and is the bisect step for a wrong element
read through a view outside a loop.  **`LOFT_NO_WRAPPER_INLINE=1`** (@PLN157 § V-o) emits a
call to a stdlib one-op wrapper (`len(v)`, `sqrt(x)`) as the CALL again instead of as its
op — the bisect step for a wrong length or libm value on native.  **`LOFT_NO_CALLEE_INPUTS=1`**
(@PLN157 § V-p, § V-ac) emits no callee TWIN — with it on, a callee that reads a record
parameter's scalar fields, views or indexes its vector fields, or indexes a plain VECTOR
parameter gets a `<fn>__inv` twin taking those as extra parameters, and a loop that hoisted
them for the argument — a variable, or for a header any pure path such as `br.img` — calls
the twin; a callee answering a scalar record through its return buffer qualifies too — and
is the bisect step for a wrong value read through a record or vector parameter inside a
callee a hoisting loop calls.
**`LOFT_NO_PUSH_HOIST=1`** (@PLN157 § V-q) makes a loop that PUSHES to a vector (`v += [x]`,
a comprehension) hoist nothing — with it on, the pushed path keeps a PUSH header carrying the
record's capacity, a push that fits is one store and a length bump, and every read of the path
serves from it — and is the bisect step for a wrong element or length out of an appending loop.
**`LOFT_NO_MINT_HOIST=1`** (@PLN157 § V-s) makes a loop that appends a RECORD element
(`v += [Pt{…}]`, `v += [pt(…)]`) hoist nothing — with it on, the mint group is admitted as a
mover and the loop's invariant record scalars are read once before it — and is the bisect step
for a wrong scalar read out of a record-appending loop.
**`LOFT_NO_RECORD_PUSH=1`** (@PLN157 § V-t) makes an admitted record append keep its
mint-group templates — with it on, a no-heap struct element is built IN the push header's
next slot (no `record_new` dispatch, no default prefill: the group's writes fill every field
explicitly) and the finish is the length bump — and is the bisect step for a wrong element,
default value or length out of a record-appending loop.
**`LOFT_NO_FIELD_MINT=1`** (`@FR-R-Mint`'s field clause, default-ON, generation time,
`--native` only) keeps a record append to a vector FIELD (`m.verts += [Vertex { … }]`) on
its templates — with it off, such an append in a loop holds the push header a bare-variable
append holds, keyed by the path a read of the field already has (`mesh_emit` −73 %, 18.3× →
4.9× of Rust); a linked group's member, a keyed field, a variant's field and a nullable
element's payload keep the general append — and is the first bisect step for a wrong, missing
or extra element out of a loop that appends records to a record's vector field;
`LOFT_HOIST_VERIFY=1` re-derives the header at the slot and the finish.
**`LOFT_NO_GROUP_PUSH=1`** (`@FR-R-GroupPush`, default-ON, generation time, `--native` only)
makes a record append OUTSIDE any held header keep its templates again — with it off, a
literal group `v += [pt(a, b), pt(c, d)]` that no enclosing loop holds a header for (a group
at function level, in a loop that declined, or on a vector declared per pass) binds a push
header of its own right after its reservation and emits its mints and finishes through it
(`fronds` built its 1 296 points per call that way: 101.7 → 69.6 µs with the two clauses
below, 2.71× → 1.86× of Rust) — and is the first bisect step for a wrong element out of a
literal record append on native; `LOFT_HOIST_VERIFY=1` re-derives the header at the slot and
the finish.  **`LOFT_NO_HEAP_RECORD_PUSH=1`** (`@FR-R-PushRec`'s heap clause) keeps an element
that OWNS heap (a `Frond { fpts, fwid }`) on its templates — with it off, such an element goes
through the header too, its slot ZEROED at the mint, which is the whole of what the prefill did
for its handles — and is the bisect step for a wrong or stale handle in an appended record
whose fields own heap; `LOFT_POISON_CLAIM=1` is the falsifier (the plain run's zero-on-claim
hides a missing zero).  **`LOFT_NO_REBOUND_MOVER=1`** (`@FR-R-Mint`'s rebound clause) makes a
loop whose body REBINDS a pushed or minted vector decline every hoist again — with it off, a
mover rebound to a § V-al loop buffer's or a § V-z element slot's projection simply takes no
holder, the loop keeps its other headers and scalars, and the buffer's own mint, a loop
record's mint and the elided element-first copy are admitted as statements that move nothing
— by the header admission AND by the scalar write-set walk (a loop record's mint evicts the
scalars of its own type) — and is the bisect step for a wrong element or scalar read in a
loop that declares a vector or a record per pass.
**`LOFT_TRACE_HOIST_DECLINE=1`** names the FIRST statement that declines a loop's hoist, and
the node that left its scalar write set untyped — reach for it when a loop that should hoist
does not, before reading the admission.
**`LOFT_NO_MOVE_APPEND=1`** (@PLN157 § V-j) makes `for f in call(…) { v += [f] }` keep the
deep copy — with it on, the call's buffer is PLACED as a record in `v`'s own store, the
append relocates the element's bytes (heap handles included — they never change store) and
zeroes the source, and the buffer's free is record-level — and is the bisect step for a
wrong element, a leak or a double free out of a loop that appends a dying temporary's
elements.  `LOFT_TRACE_MOVE=1` names the gate that declined a pairing.

## Text walks

**`LOFT_NO_LAZY_SPLIT=1`** (`@FR-R-LazySplit`, default-ON, generation time, `--native`
only) makes `for p in t.split(c)` build and walk its `vector<text>` again — with it off, a
loop over the standard library's `split` with a CONSTANT separator takes each piece
straight from the text (a parameter nothing writes is borrowed; any other source is
iterated as a copy taken where the call stood, so a write in the body cannot reach it),
the vector and its per-piece records are never built, and the call's hidden buffer is
never minted (the drawing bench's `parse` row −9 %: its outer loop is
`for raw in src.split('\n')`) — and is the first bisect step for a wrong, missing or
extra piece out of a loop over a `split` on native.  A variable separator, a `rev`, a
split bound to a name first and a generator keep the vector.  `LOFT_TRACE_LAZY_SPLIT=1`
names each loop admitted and each declined with its reason.
**`LOFT_NO_TEXT_BORROW=1`** (`@FR-R-TextBorrow`, default-ON, generation time, `--native`
only) makes the loop variable of `for p in vector<text>` — and of a lazily split text —
copy its element into a `String` again — with it off, `p` is bound as the `&str` the
element read answers and read bare, where the body reads it only as a text VALUE (an
operand at a `text` position, the source of a bind into another slot), never writes,
links or captures it, and (for a vector) writes no store; a text VALUE op — scalar and
text operands, a scalar or text result, or a write to a text VARIABLE — is no store write
to that condition, and `OpGetText` is a reader, so the walk holds its header, length and
base like any other (the stdlib `join` 5.40× → 1.96× of Rust, `split_walk` 1.80× →
1.01×, `parse_num` 1.46× → 0.98×) — and is the first bisect step for a wrong, stale or
crashing text read through such a loop variable on native.  `LOFT_HOIST_VERIFY=1`
re-reads the element at the walk's release and panics when the borrow no longer names it
(address first, so a dangling borrow is never read); `LOFT_TRACE_TEXT_BORROW=1` names each
walk admitted and each declined with its reason.  A `&p` link, `p += …`, `p` handed to a
`&text` parameter, tupled or returned, a store written in the body and a generator keep
the copy.
**`LOFT_NO_CHAR_WALK=1`** (`@FR-R-CharWalk`, default-ON, generation time, `--native` only)
makes `for c in text` take every character through the step as written again — with it
off, an ASCII byte other than NUL is one move (the byte is `c`, `c#next` steps by one;
every other byte, a NUL — whose read notes a fault — and the end take the written step in
the `else` arm), and where nothing in the loop writes the text its null test — a content
compare of the whole text LLVM does not hoist — is asked ONCE before the loop
(`char_walk` 4.62× → 2.48× of Rust; the stdlib `split`, built on such a walk, 12.5× →
7.8×) — and is the first bisect step for a wrong character, a wrong `c#index` or a missed
fault out of a character walk on native.  `LOFT_HOIST_VERIFY=1` runs the written step
beside the fast arm and panics when they disagree, and asserts the hoisted null test
inside the loop; `LOFT_TRACE_CHAR_WALK=1` names each walk and whether its null test moved.
A call or literal source keeps the written step (the lowering evaluates such a source per
iteration).

## Element bases and record addresses

**`LOFT_NO_VECTOR_BASE=1`** (@PLN157 § V-ak, `@FR-R-Base`, default-ON) makes a
growth-free loop's fused element reads and writes resolve the store per element again —
with it off, a loop that grows no store (no push and no mint, whether or not the mint
emits through a push header — since 2026-09-21: one left on its templates grows its store
all the same, and a base bound beside it answered `null` on native; a null-discharge
buffer's mint is a fresh store and does not count, since 2026-09-18)
binds the address of each hoisted vector's element 0 beside its header and every read
or write is one bounds test and one load or store through it (the resample probe −14 %)
— and is the first bisect step for a wrong element read or write inside a growth-free
loop; `LOFT_HOIST_VERIFY=1` re-derives every base at every use and panics when a store
grew under one, and `LOFT_TRACE_BASE=1` prints each loop's growth-free verdict.
`LOFT_NO_NN_FAST=1` (P3c) also restores the nullable-aware counter step and the guarded
literal division that § V-aj (`@FR-R-Counter`, `@FR-R-LitDiv`) replaced.
**`LOFT_NO_TWIN_BASE=1`** (`@FR-R-Base`'s twin clause, default-ON, generation time) makes
a callee twin (`__inv`) take its header inputs alone again, every element read or write
inside it resolving the store — with it off, the twin takes each header's element BASE
beside it (`__ib_k`; the caller's held `__vb_N`, or one derived from the held header at the
call), a view of the path inside the twin shares it, and the fused reads and writes are one
bounds test and one load or store (`composite`'s pixel accessors) — and is the first bisect
step for a wrong element read or write inside a function a hoisting loop calls;
`LOFT_HOIST_VERIFY=1` re-derives the base at every use.
**`LOFT_NO_RECORD_PTR=1`** (`@FR-R-RecPtr`, default-ON, generation time, `--native` only)
makes every field read and in-place field write of a record VIEW resolve the store again —
with it off, a plain-record local bound by a statement (`e = tbl[i]?`, `s = o.inner`)
carries the address of its record for the rest of its block (`__pa_N`), every scalar
field read is one load and every in-place field write one store through it, and a twin the
view is handed to takes its scalar inputs read through the address at the call
(`wide_line`'s crossing loop −35 %) — and is the first bisect step for a wrong field read
through a record view, or a wrong argument inside a callee handed one, on native.  Admitted
where the block's remainder grows no store, frees no record before a later use, and never
rebinds the view; a nullable view — `e = v[i]` without `?`, a `for e in v` loop variable —
is admitted as its record, the null address answering the sentinel.  `LOFT_HOIST_VERIFY=1` re-derives the
address and re-reads the store at every use; `LOFT_TRACE_RECPTR=1` names each view bound
and each declined with its reason.  `LOFT_NO_VECTOR_BASE=1` switches it off too (one rule).
**`LOFT_NO_BASE_RECPTR=1`** (`@FR-R-RecPtr`'s base clause, default-ON, generation time,
`--native` only) makes the loop variable of `for e in v` resolve the store for its address
again — with it off, the address is the loop's held element base plus index times size
under the in-range test, no `DbRef` consulted, and the iteration's `#index` (never the
sentinel, `@FR-R-Counter`'s iteration clause, `LOFT_NO_NN_FAST`) steps through the non-null
add (`record_walk` −49 %, 2.98× → 1.50× of Rust; `tuple_kernel` −20 %; `entity_tick`
−30 %) — and is the first bisect step for a wrong field read through a `for e in v` loop
variable on native; `LOFT_HOIST_VERIFY=1` compares the address with a fresh `rec_ptr` at
every use.  An address derived from the element's `DbRef` instead of the index measured
+46 % SLOWER and is not what ships.
**`LOFT_NO_NESTED_FIELD=1`** (`@FR-R-RecPtr`'s path clause, default-ON, generation time,
`--native` only) makes a scalar field reached through INLINE sub-records (`v.pos.x`) rebuild
its `DbRef` and resolve the store again — with it off, such a field of a record view is read
and written through the view's address at the summed offset, and a view whose only accesses
are nested binds an address at all (`mesh_aabb` −47 %, 9.2× → 4.7× of Rust) — and is the
first bisect step for a wrong value read or written through a nested field path on native;
`LOFT_HOIST_VERIFY=1` compares every such read with the path walked the unrewritten way.
Emitter-local on purpose: folded in the parser, the read would become an `(R-Scalar)`
candidate typed by the PARENT record, which a write through a sub-record view never evicts.
**`LOFT_NO_MINT_WINDOW=1`** (`@FR-R-RecPtr`'s mint clause, default-ON, generation time,
`--native` only) makes every field write of an appended record resolve the store again —
with it off, a minted plain-record element holds its slot's address from the mint up to its
own finish, and its `integer` / `float` / `single` fields are one store each through it
(`record_append` −45 %, 3.55× → 1.95× of Rust; `mesh_emit` −43 %, 2.83×); a window that
holds a text set, a nested mint, a builder that appends or a delivery copy declines — and is
the first bisect step for a wrong or lost field in an appended record on native;
`LOFT_HOIST_VERIFY=1` compares the address with a fresh derivation at every write.  Its
cells could not fail until one reallocated the store INSIDE a window (`m4b`): small cells
never move the memory a stale address would miss.
**`LOFT_NO_COPY_IN_PLACE=1`** (`@FR-R-InPlace`'s copy clause, default-ON, generation time,
`--native` only) makes a whole-record copy a store writer again, so a loop holding one
hoists nothing — with it off, a flag-free `OpCopyRecord` over a record type that owns no
heap is an in-place write at a larger width: the write-back idiom `e = v[i]?; …; v[i] = e`
(a copy of a view onto the place it views, which the runtime makes a no-op) keeps its
loop's header, base and the address of `e` (`entity_tick` −23 %, 3.19× → 2.45× of Rust) —
and is the first bisect step for a wrong value in a loop that assigns a whole record to an
element or a field; `LOFT_HOIST_VERIFY=1` is the falsifier (the copy writes its type WHOLE
for the scalar hoist).  A heap-owning type and a call's freed result keep the store-writer
verdict.  The copy is not elided: on the absent-element path it is an out-of-range store
with a fault note of its own.
**`LOFT_NO_ENUM_RECORD=1`** (`@FR-R-PushRec`'s and `@FR-R-RecPtr`'s enum clauses, default-ON,
generation time, `--native` only) makes a USER struct-enum value no record to the hoist
family again — with it off, a `vector<Edit>` appends through a push header (the slot zeroed,
the literal writing its own tag), a minted element of any record type holds its window
address, a struct-enum view holds its address and its TAG is one byte read through it, and
`for e in edits` is seen through the `OpGetField` its element is bound behind (`enum_match`
−64 %, 12.4× → 4.5× of Rust) — and is the first bisect step for a wrong variant, a wrong
payload field or a lost element out of a vector of struct-enum values on native.  The
exclusion of enum payloads belongs to `(R-Scalar)`'s (type, offset) key and stays there; the
synthetic `__nullable<S>` stays out of all of it.

## Loop-local buffers and records

**`LOFT_NO_LOOP_BUFFER_REUSE=1`** (@PLN157 § V-al, `@FR-R-LoopBuffer`, default-ON,
generation time) makes a vector local declared `[]` INSIDE a loop re-mint its per-site
buffer every iteration again — with it off, the buffer's store and its vector survive the
iteration, the mint after the first is a length reset that keeps the capacity, and the
literal's zero of the vector field is not emitted (the resample's two per-pixel vectors:
−10 % on the row) — and is the first bisect step for a stale or wrong element read out of
a vector declared inside a loop on native.  Only for elements that own no heap (a `text`
vector keeps the re-mint: a length reset would strand what its elements own), a buffer no
other call reaches, and a declaration outside the loop.  `LOFT_TRACE_LOOP_BUFFER=1` names
each buffer kept and each declined.
**`LOFT_NO_LOOP_RECORD=1`** (`@FR-R-LoopRecord`, default-ON, generation time, `--native`
only) makes a plain no-heap record local declared and minted by a literal INSIDE a loop
(`sub = Spec {…}` per pass) free its store at the iteration's end and take a fresh one next
pass again — with it off, the local is declared at the loop's prelude, keeps its store and
its record across passes (a complete literal re-mints nothing, a partial one re-establishes
its declared defaults) and is freed once after the loop — and is the first bisect step for a
stale field, a leak or a double free out of a record literal in a loop on native.  Declined
where the local is copied, returned, captured, appended, handed to a callee whose return
borrows it, or owns heap.  `LOFT_TRACE_LOOP_RECORD=1` names each kept local and each decline.

## Counted push loops

**`LOFT_NO_PUSH_FILL=1`** (@PLN157 § V-am, `@FR-R-PushFill`, default-ON, generation time)
makes a counted push loop grow per push again — with it off, `for i in a..b { v += [x, y] }`
reserves two elements times its trip count once before it runs, and `for _ in a..b
{ v += [c] }` with `c` invariant is ONE fill of the vector's tail with the per-element
loop as its fallback (the resample's plane prefill: −2 % on the row) — and is the first
bisect step for a wrong element or length out of a counted push loop on native.  A body
that can `break`, `return` or loop again, a push under a branch, another write to the
path, or a range end that is not a simple invariant declines the loop.
`LOFT_TRACE_PUSH_FILL=1` names each decline; `LOFT_HOIST_VERIFY=1` re-derives the push
header at the fill.  The loop is read in either spelling — the statement and the
comprehension `[for i in a..b { e }]` — and the reservation stands in front of every
guarded copy of the loop (one written after a guard lands in the arm that does not run).
**`LOFT_NO_PUSH_WINDOW=1`** (`@FR-R-PushFill`'s window clause, default-ON, generation time,
`--native` only) makes a reserved counted push loop push through its push header again, the
length written back to the record per push — with it off, a loop whose body reaches its
vector through the pushes alone (an exclusive root, never named outside the pushes, no
variable that may view it, no other store write) pushes through a held element address and
a local length, and writes the record's length once when the loop ends (`push` 48.8 → 8.0 µs,
0.47× its Rust twin; `comprehension` 9.53× → 1.70×; `grid` 8.10× → 2.81×; `f32_build`
−80 %) — and is the first bisect step for a wrong element or length out of a counted push
loop or a comprehension on native.  `LOFT_TRACE_PUSH_FILL=1` names the clause that declined
a window.  ⚠ `LOFT_HOIST_VERIFY=1` checks the window's base and its frozen header at every
push, but CANNOT see what the admission exists to prevent — a runtime reader meeting the
lagging length changes no held fact — so there the interpreter is the falsifier.  The form
is load-bearing: the window is three scalars whose address never reaches a call, because a
counter whose address escaped to the growth arm lived on the stack and cost half the gain.
Since 2026-09-22 the window also takes RECORD mint groups, at the top level or under `if`
arms (the arms are exclusive, so the trip count bounds the appends): the slot is the
window's next, the field sets and the mint address are that pointer, the finish is the
window's bump (`enum_match` 35.9 → 23.4 µs, ~3.98× → ~2.6×); a group whose literal fills
the element's own vector field, a read or a view of the vector in the body, a `break` and a
second pushed vector keep the header mints.

## Guarded reads and integer arithmetic

**`LOFT_NO_JOIN_READ=1`** (`@FR-R-Base`'s join clause, default-ON, generation time, `--native`
only) makes `v[i]?.f` run its join on every pass and read the result through the store again
— with it off, a scalar field of a `?`-discharged element (`i` a variable) in a loop that
holds the vector's header and element base is one range test and one load through the base,
the join written once in the fallback arm for every index that test refuses (`record_update`
34.9 → 12 µs, 4.62× → 1.60× of Rust, pinned) — and is the first bisect step for a wrong field out of
`v[i]?.f` inside a loop on native.  The fallback is never a constant: a negative index
addresses from the end there and an absent element answers its default RECORD's field, which
a declared field default makes non-zero — `LOFT_HOIST_VERIFY=1` cannot see a mistake in that
arm, the interpreter can.  ⚠ Built at TWO sites through one recogniser
(`hoist::fused_join_read`): the pre-eval collector lifts every `Block` argument, so an
emitter arm it does not know about never fires — and only a pin shows that, no value does.
**`LOFT_NO_INVARIANT_HOIST=1`** (@PLN157 § V-ao, `@FR-R-Invariant`, default-ON, generation
time) makes every invariant integer chain evaluate at every use again — with it off, a
chain of `+ - * neg & | ^` over literals and variables a loop neither rebinds nor lets
escape is evaluated at its FIRST use and answered from a memo after (the same value on
every path, the overflow note fired where the first evaluation stands; declared at the
innermost loop that spells it so the test peels out — declared one loop out it stayed in
every tap), the resample tap's `yy * iw + xmin` −5 % on the row — and is the first bisect
step for a wrong index or arithmetic value inside a loop on native.  `LOFT_HOIST_VERIFY=1`
re-evaluates the chain at every use and panics when the memo disagrees; that form is what
found loft#1534, a by-reference argument the non-sentinel proof's escape collector could
not see.  `LOFT_TRACE_INVARIANT=1` names each memo.
**`LOFT_NO_BOUNDED_NEST=1`** (@PLN157 `R-BoundedNest`, `@FR-R-BoundedNest`, default-ON,
generation time, `--native` only) keeps every tap nest checked — with it off, an innermost
counted loop that is one accumulate over `?`-discharged element products
(`acc += a[(yy*w + x)*4 + ch]? * k[base + x]?`, a resample's tap) runs with PLAIN operators
behind a guard evaluated once at the loop's entry: no range end or invariant is the sentinel,
every vector read has a known element bound (taken ONCE at the outermost loop whose body
leaves the vector alone, never at the nest's own prelude), and the magnitude bound of every
chain and of `|acc| + trips × bound(term)` fits, so no operation can fault and the plain
answer is the checked one; the checked loop is the `else` arm.  It is C120's admissible
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
**`LOFT_NO_BOUNDED_SUM=1`** (`@FR-R-BoundedNest`'s reduction clause, default-ON, generation
time, `--native` only) makes `acc = acc + v[i]` over an integer vector pay the checked add on
every element again — with it off, a counted loop that is one such accumulate over a held
header and base sums what it can PLAIN first, a block of 1024 at a time, admitted when every
element lies in `[−2^40, 2^40)` and the running total is more than `2^50` from the i64 edge
(the nest's magnitude-bound proof, taken from the data per block in the same pass, so no
prefix in the block can overflow and the plain sum IS the checked answer), and the checked
loop resumes where the first block declines (the stdlib `sum` 11.9 → 3.7 µs, 6.45× → 2.06× of
Rust, pinned; no answer changes on any input, a null element and a large element take the checked
loop) — and is the first bisect step for a wrong sum out of such a loop on native.
`LOFT_HOIST_VERIFY=1` re-runs every admitted block through the checked add and panics on a
disagreement — the proof itself, so it has no blind spot.  The bound test is spelled in
add / shift / or on purpose: baseline x86-64 is SSE2, with a packed 64-bit add and no packed
64-bit signed compare, and written as two compares the loop stays scalar and gains nothing.
**`LOFT_NO_RANGE_ARITH=1`** (`@FR-R-Range`, default-ON, generation time, `--native` only)
makes every integer operator keep its checked template again — with it off, an operator
whose RESULT provably fits the type (a literal, a mask of a non-sentinel value, `len`/`size`
in `0..=u32::MAX`, a byte, a counted range's counter, a one-expression callee such as
`color_a`, and interval arithmetic over those in i128) emits the processor's operator, since
no fault can occur — and is the first bisect step for a wrong integer value on native where
the range proof admitted a plain operator; `LOFT_HOIST_VERIFY=1` compares the plain answer
with the checked one at every such operator.  It is C120's admissible successor for
straight-line arithmetic; a parameter or a record/element read is never ranged.  ⚠ Its
counted-range-counter clause was INERT until 2026-09-21 (loft#1558): the seeding looked for
the counter's seed INSIDE the loop and the parser emits it as the statement before, so no
counted loop was ever ranged.  Nothing showed it, because the pin that should have
(`range_arith` a4) was recording a plain form the CHAIN GUARD supplied — a pin can borrow
another rewrite's evidence and read as proof of its own clause.
Since 2026-09-22 the proof also reads the STATIC TYPE (`range::type_range`): a non-nullable
`u8`/`i8`/`u16`/`i16` or user `limit(lo, hi)` parameter, local or compiler-typed join (the
cbor decoder's `(bytes[p] ?? 0) * 256`) carries its type's range — a fact since loft#1593 made
every store into such a slot refuse an unprovable value; `i32`/`u32` (the templates) and a
signed alias with a spare bottom code (`limit(-100, 100) size(1)`, whose overflow writes the
sentinel) stay unranged, and a boxed capture is read through its box and stays checked.
**`LOFT_NO_GUARDED_CHAIN=1`** (`@FR-R-GuardedChain`, default-ON, generation time, `--native`
only) makes a counted loop's index chains keep their checked operators — with it off, chains
of `+ - *` and negation over literals, the loop's and nested loops' counters, integer locals
the loop never writes and hoisted record scalars run PLAIN behind a guard evaluated once at
the loop's entry (every leaf not the sentinel, every chain's magnitude bound fits), the
checked loop being the `else` arm (`composite` 103 → 68 µs: `j * lw + i`, `x0 + i`, `y0 + j`
over record scalars no static proof can bound) — and is the first bisect step for a wrong
index or accumulate in a counted loop on native.  A loop with ONE admitted operator
declines on PROFITABILITY (the guard is a fixed cost per loop ENTRY against a saving of one
null test per operator per ITERATION, and the doubled body costs a small hot function its
inline): measured against the guard off, the one-operator loops in `pil_hline` and
`matches_at` were costing `fill_circle` and `fill_star` ~50 %, `wide_line` 22 % and `parse`
18 %, while six-operator `composite_layer` gains 30 %.  `LOFT_HOIST_VERIFY=1` is the falsifier,
`LOFT_TRACE_CHAIN=1` names every loop admitted and declined with its operator count, and
**`LOFT_GUARDED_CHAIN_ONLY=<fn>[,<fn>…]`** admits the guard in the named functions ALONE — the
bisect step for a row that moved under `LOFT_NO_GUARDED_CHAIN`, because a whole-program A/B
cannot say which admitted loop paid (the drawing `parse` row lost 18 % to ONE of six: a 3–8
trip loop in a per-byte helper, whose doubled body stopped inlining into its caller).

## The release-pass probe (a measurement instrument)

**`LOFT_RELEASE_PASS_PROBE=1`** (generation time) is a MEASUREMENT INSTRUMENT, never a
build anyone ships: every integer `+`, `-`, `*`, negation, bit op and non-literal
division emits the processor's wrapping operator and every float comparison the plain
one — what the eventual release build pass for games would emit (DESIGN_DECISIONS.md
C120, NATIVE.md § Optimisation tiers).  The values after a fault are NOT the language's
(`b = MAX + 1; d = (b + 5) * 2` reads `10` for null), so a row is comparable only while
its hash still agrees; its time is the CEILING the checked build is measured against,
which is what says whether a row is bound by the checks or by something else — the
guide for what to optimise next.  The null test itself and the `*Nullable` twins keep
their templates: they are the language's null semantics, not its fault protection.

## Records returned and passed as values

**`LOFT_NO_VALUE_RECORD=1`** (@PLN157 § V-aa, default-ON since 2026-09-14) makes a
function whose result is a plain no-heap record of ≤6 scalar fields return it through
the buffer again — with it off, such a function whose every call site reads fields off
it and whose body builds it with `Object` blocks returns those fields in REGISTERS with
the call site reading tuple elements (measured: 1.65× on the call, `smooth` −25 %
standalone, `lock_curved` −3 %) — and is the bisect step for a wrong field out of a
record-returning call on native.  It was opt-in for two days because its call-site gate
did not hold over the script corpus (376 compile errors, 218 of them the fn-ref
DISPATCH: every arm of the `match` a `CallRef` emits shares one return type); the gate
now declines every arm by reading the arm set from `fnref::dispatch_arms`, the
emitter's own home for that question, and a mixed-arm branch is declined by shape (a
`__lift_` temp bound from a CALL was too, until `(R-ValueLocal)` made it a value local).
A `__lift_` temp bound from a
VIEW — a selecting tail's arm over a by-value parameter — is a value local bound to the
view's field tuple and never a view leaf (read as one it leaked the store its copy
mints, one record per call; found 2026-09-15 with the generic instance's statement join,
whose arms bind the join local the same way and did not compile).
`LOFT_TRACE_VALUEREC=1` names each admission and decline.
**`LOFT_NO_VALUE_LOCAL=1`** (`@FR-R-ValueLocal`, default-ON since 2026-09-25, generation
time, `--native` only) keeps every by-value record PARAMETER a `DbRef` again — with it
off, a parameter of a plain no-heap record of ≤6 scalars is received as the TUPLE of its
fields where the callee reads it only field-wise, hands it to another such parameter,
copies from it or answers it, AND the callee's body (with everything it calls) writes no
record of that type through any route a caller could view — a by-value record parameter
is a VIEW of its argument (`bump(p, w) { w.x = 5.0; p.x }` called `bump(a, a)` answers 5),
so the write set keyed by record type decides, a callee's return buffer and the frame's
own records set apart from it (`WriteSet::reaches_record`); a call site hands a value
local or an admitted call as it is and reads the fields off any other record at the site
(mesh3d `mat4_transform` 9.9 → 2.64 ms, 5.0× → 1.32× of Rust; `sphere` −38 %) — and is
the first bisect step for a wrong field read through a small-record parameter on native.
The interpreter is the values oracle; `LOFT_TRACE_VALUEREC=1` names each parameter
carried and each declined with the reason, and the aliasing cells are
`tests/scripts/a-small-record-parameter-is-carried-as-a-tuple.loft`.

**`LOFT_NO_VIEW_FIELD=1`** (@PLN164 C5 and E-1, `@FR-O-ViewField`, `@FR-R-ValueRecord`,
**default-ON since 2026-09-17**, generation time, `--native` only) restores the record form —
with it off, a function returns a record of two scalars and a vector as a TUPLE
whose vector element is a REFERENCE to the place the value already lives in — so a
`Mark { matched, bad, pts }` built from a local the function appended into a parameter's
collection costs no buffer store and no deep copy, and an exit with an empty literal delivers
a null view, which reads as the empty vector it replaces.  The interpreter keeps the record
form and is the values oracle.  Admitted only where the leaf's place outlives the frame and
every call site READS it: a `?`-discharged element declines (its ownership is a join — the
absent arm mints in the frame's store), an element read or an iteration at the site declines
(an element read answers a PLACE the site can write through — measured: the write landed in the
container, reading 109 where the copy holds 9), a disturbance between the bind and the last read
declines (measured: a removal made the result read the NEXT element's points), and a `pub`
function declines whole (`(R-Escape)`).  The exit's own answer is a PROOF rather than a
fallback: every mention of the return buffer must be one the tuple accounts for, because a
field filled without an append — a returned vector literal, a record literal's vector field —
read as "empty" delivered a null view for a vector of eight (`723-ncc-loop-element-bind`).  The
callee's half is a per-PATH PROOF (`hoist::fresh_leaf`, a forward walk that joins `if` arms
and runs loops to a fixpoint): on every path to the exit the last change to the container was
the append of the local's copy, and neither the local, a store it views, nor that element
changed after it.  An append in each arm of an `if` views the container's LAST element; an
append under a condition, or a local grown, rebound or written after its copy, declines
(measured wrong under the per-exit test this replaced: 0 points for 3, and 3,9 for 4,109).  A
naming that reaches a SIBLING field or claims a record in the store moves nothing in the one
the leaf views.  The gate stores each exit's leaf and the emitter writes the stored one.  It
is the first bisect step for a wrong or stale vector read out of a record-returning call on
native, and `LOFT_TRACE_VALUEREC=1` names every admission and decline, with the reason and —
for a site — the caller that consumed it.
**`LOFT_NO_FORWARD_TUPLE=1`** (@PLN164 E-1, `@FR-R-ValueRecord`, **default-ON since
2026-09-17**, generation time, `--native` only) makes a forward decline its callee again —
with it off, a function that keeps its record FORWARDS an admitted callee's answer —
`return nm()`, or the call arm of a value branch whose other arm is a record, both lowered as
the callee filling a buffer bound back from the call — by writing the tuple into that buffer
at the site: minted
where it is absent, every scalar set, a view part's vector copied; the call evaluates to the
buffer.  Without it such a forward is a site that consumes the record and declines the callee
everywhere — which is what kept the drawing library's `parse_circle` and `parse_line_cmd` on
buffers (their `no_mark()` tail was forwarded by `parse_fronds`).  It is the first bisect
step for a wrong field, a leak or a null-store panic at a `return g(…)` of a record-returning
function on native.

## Element-first builds, complete writes and return buffers

**`LOFT_NO_ELEMENT_FIRST=1`** (@PLN157 § V-z) makes a record-literal's vector field
keep its temp-store build and deep copy again — with it off, a local vector consumed
exactly once by one append is built INSIDE the appended element (minted at the temp's
declaration, invisible until the finish's length bump) — and is the bisect step for a
wrong vector field of an appended record on native, or for a wrong value read through an
element view between such a local's declaration and its append (loft#1553).  The declaration must be the local's only
binding (loft#1552: a local declared `[]` and rebound under an `if` left the element empty).
**`LOFT_NO_ELEMENT_PLACE=1`** (@PLN164 E-2, `@FR-R-ElemFirst`, default-ON, generation time,
`--native` only) keeps that build to local vectors and literal-built locals again — with it
off, it also reaches an append into a record's COLLECTION FIELD (`sc.ops += [Op { pts: p }]`)
and a local a CALL fills (`p = smooth(raw)`): the call is handed the early element's field as
its return buffer, provided it fills its buffer and never mints into it, no other argument
names the container, and the buffer serves that call alone.  An append in EACH arm of an `if`
(`parse_circle`'s stroked/filled pair) shares one early element, the second arm's mint an alias
of the first's.  No statement between the declaration and the append may name the container
(a naming that reaches only a sibling field is fine), jump out (a stranded early element keeps
the call's vector inside the container's store, where only a record census sees it), or read a
VIEW the early mint may have moved — the mint is the append's growth brought forward, so a
local viewing the container's element, or any heap parameter where the container is a
caller's, declines (loft#1553; a view of a sibling collection is spared).  It is the first
bisect step for a wrong, empty or leaked vector field of an element a parser appended;
`LOFT_TRACE_ELEMFIRST=1` names every admission and decline, and the variable a declined
window reads.
**`LOFT_NO_COMPLETE_WRITE=1`** (@PLN157 § V-y) makes every record keep its default
prefill again — with it off, a literal group the emitter PROVES writes every field
(declared defaults, sentinels, the variant tag included: the parser's lowering is
complete by construction) calls a no-prefill `OpDatabaseNP`/`OpNewRecordNP` twin —
and is the bisect step for a wrong default or sentinel in a literal-built record
on native.
**`LOFT_NO_LITERAL_HOIST=1`** (@PLN157 § V-x) makes an invariant loop-body vector
literal rebuild per iteration again — with it off, `v: vector<float> = [1.0, 2.0]`
(or an if-of-literals on a const-param field) under a loop builds ONCE per activation,
guarded on the local being unbound — and is the bisect step for a wrong constant
vector inside a loop on native.
**`LOFT_NO_RETBUF_ADOPT=1`** (@PLN157 § V-u) makes a vector-returning function keep its
delivery copies — with it on, a shape-A result local ADOPTS the hidden return buffer
(built where it must end up; the exits deliver nothing; the buffer's backing reused across
calls) — and is the bisect step for a wrong vector return, a leak at a vector-returning
call, or values accumulating across calls.  `LOFT_TRACE_ADOPT=1` names the declining gate.
The family's rules and their citations: `doc/claude/formal/rewrites.md` (`@FR-R-…`);
**`scripts/emission_audit.py <emitted.rs>`** validates a `--native-emit` output against
them (one holder per path per frame, no mover on a held path, a twin handed only live
holders) — run it on any emission that looks wrong before running the program.
PERFORMANCE.md § Design: P2, NATIVE.md.

## Null-discharge buffers and filling loops

**A null-discharge buffer does not block the hoist (@PLN157 § V-ad, `--native`, generation
time):** `e = tbl[i]?` on a vector of all-scalar records mints an absent element into a
hidden per-site buffer, and that allocation used to decline every header in the loop
around it — the drawing bench's polygon crossing loop hoisted nothing (`wide_line` −31 %
when it did); since 2026-09-18 it does not block the loop's BASES either (a fresh store,
or a clear of the buffer's own, moves no element a base addresses).
**`LOFT_NO_NULL_BUFFER_HOIST=1`** restores the blocking form and is the
first bisect step for a wrong element read in a loop that discharges a record element
with `?`; `LOFT_HOIST_VERIFY=1` is the falsifier.

**A filling loop is one slice fill (@PLN157 § V-ae, `--native`, generation time):**
`for i in lo..hi { v[base + i] = c }` with `base` and `c` invariant, over a vector a
header is held for, emits ONE range test and a slice fill, the per-element loop kept as
the fallback for every range the fill declines (a negative or partial index, an empty
range, an overflow) — `wide_line` −23 %, the two fills −48 %.  **`LOFT_NO_FILL_HOIST=1`**
emits the per-element form again and is the first bisect step for a wrong element or a
missed write out of a filling loop; `LOFT_TRACE_FILL=1` names the check that declined a
loop the idiom should have taken.

## The lean tier: frameless call trees

**A frameless call tree carries no prelude (@PLN157 row 11, `@FR-R-LeafChain`, lean tier
only, generation time):** N4's leaf rule made transitive — a function whose every user
callee is loft-bodied, off any cycle and free of fn-refs drops its depth entry and buffer
guard in `--native-release`, since there the frame is only a depth count (parse row −12–14 %).
**`LOFT_NO_LEAF_CHAIN=1`** keeps those frames (one step finer than `LOFT_NO_LEAF_PRELUDE`);
the named tiers never elide a non-leaf.
