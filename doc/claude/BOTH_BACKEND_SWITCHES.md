<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Both-backend optimisation switches

Which switch turns off an optimisation that the interpreter AND `--native` both take, and
what checks it.  Two families: LOWERING rewrites, made at parse time or in the scope pass, so
both backends run the rewritten IR — here; and RUNTIME policies inside the store, keyed
collections and the free tree — [BOTH_BACKEND_RUNTIME_SWITCHES.md](BOTH_BACKEND_RUNTIME_SWITCHES.md).  A wrong answer on `--native` only belongs to
[NATIVE_SWITCHES.md](NATIVE_SWITCHES.md) instead.

Each entry names its switch (the first bisect step), the falsifiers that apply
(`LOFT_STRICT_STORES=1`, `LOFT_POISON=1`, `LOFT_POISON_CLAIM=1`, `LOFT_KEYED_VERIFY=1`, the
native leak check, `LOFT_POISON_HOST=windows` —
[WINDOWS.md § Falsifier](WINDOWS.md#falsifier-the-emulated-windows-host)) and its `LOFT_TRACE_*`
switch where one exists.  Because both backends
share the rewrite, agreement between them proves nothing here: hand-computed cell values and
the switch A/B are the evidence.  The store's design is [DATABASE.md](DATABASE.md); the rules
are in [formal/rewrites.md](formal/rewrites.md), [formal/heap.md](formal/heap.md) and
[formal/ownership.md](formal/ownership.md).  Past measurements:
[BOTH_BACKEND_SWITCHES-history.md](BOTH_BACKEND_SWITCHES-history.md).

## Lowering: format appends, record placement and buffers

**`LOFT_NO_FORMAT_APPEND=1`** (`@FR-R-FormatAppend`, default-ON, parse time, BOTH backends)
makes `out += "…{e}…"` build its work text again — with it off, every literal and hole of
the format is appended to `out` directly, in order, where no hole reads `out` and `out` is
a text variable (a field keeps the work text) — and is the first bisect step for a wrong,
missing or doubled part of a text built by appending format strings.
`LOFT_TRACE_FORMAT_APPEND=1` names each append written through and each kept, with the reason.

**`LOFT_NO_LITERAL_EXIT_BUFFER=1`** (@PLN164 B2, `@FR-R-Place`'s callee clause, parse
time, BOTH backends) makes a mid-body `return S { … }` build its record in a store of its
own again — with it off, every literal exit of a record function writes the `__retbuf`
the caller handed (a record enum's variants too, a unit variant included), through the same null-guarded mint the TAIL literal has used since
@PLN157 § V, so a callee with several literal exits answers ONE store; that includes a
buffer a CHAIN renamed (`return no_mk()` beside `return Mk { … }`, `parse_circle`'s
shape), and a literal tail beside chain exits — and is the first bisect step for a wrong
record out of a callee with more than one exit.
**`LOFT_NO_PLACE_RESULT=1`** (@PLN164 B2 units 2–3, `@FR-R-Place` + `@FR-R-MoveLast`, decided
after the scope pass, BOTH backends) keeps a call result minting its own store and deep-copying
into its destination again — with it off, a plain local bound from a callee whose every exit
writes the handed buffer, and whose ONE owning destination on every path is a record-literal
field of an element appended to a PARAMETER's collection (`sc.ops += [Op { paint: pp, … }]`),
gets that buffer CLAIMED IN the parameter's store (`OpPlaceRecord`), the field takes it by
RELOCATION (`OpMoveRecord`: the bytes move, the heap handles keep their claims, the source's
block is released on the spot), and the exit releases the record alone on the one path that
still holds it (`OpFreeRecordIn`) and nothing after a move — the path state is a compile-time
fact the pass writes into the IR, never a runtime stand-in (no zeroed source, no `v = null`,
which the interpreter lowers as a store-level free of what the local held).  It is the first
bisect step for a wrong field, a leak or a double free out of a record built by a call and
stored in an appended element; `LOFT_TRACE_PLACE=1` names each admission and each decline
(a read after the store, a hand-off to a call, a rebind, a second destination or host, a
destination in a local record, a loop, a rejoin of a stored and a held path all keep the copy).
The same switch covers `(R-Place)`'s LOOP clause: a bind inside a loop whose
result's ONE heap-owning field is stored into an element appended to a parameter or to a
local vector bound before the loop (`sub = read(…); items += [sub.value]; p = sub.next`)
gets its per-turn buffer claimed in the host's store (`OpPlaceRecord` in the lazy mint's
`then`, the turn's `OpClear` kept), the field moves by `OpMoveField` — the bytes relocate,
the source is ZEROED so the next turn's clear and the callee's refill find an empty record
— and each exit releases the placed record as a block (`OpFreeRecordIn`).
**`LOFT_NO_TAKE_LOCAL=1`** (`(R-MoveLast)`'s local clause, the scope pass, BOTH backends) keeps
`x = y` for a vector local `y` built afresh each pass (the double buffer `tmp = []; …; result =
tmp`) a deep copy — with it off, `y` is given up to `x`, the stores exchanged.  It is the first
bisect step for a wrong or emptied vector after such a rebind; `LOFT_TRACE_PLACE=1` names each
decision (`[take-local]`).
**`LOFT_NO_ENDED_CAPTURE_SCOPE=1`** (`@FR-B-Scope`, parse time, BOTH backends) offers a lambda
the locals whose block ended before it, so a closure written after a loop shares the loop body's
local — with it off (`Parser::enter_capture_scope`), such a name does not exist in the lambda:
an assignment there binds a local of the lambda's own and a read is refused.  The bisect step
for a closure after a block that reads or writes that block's local.
**`LOFT_NO_TEXT_TAIL_THROUGH_IF=1`** (`@FR-H-Free`, the scope pass, BOTH backends) homes a text
value block's tail local at the function scope only where the block's last operator is that
local — with it off (`scopes::scan`'s `tail_vars_through_arms`), every local an `if` arm of the
tail hands out is homed too, so its free runs at the function's end rather than right after
the block's value.  The bisect step for an interpreter text that reads back as garbage.
**`LOFT_TAIL_READS_WHOLE_STATEMENT=1`** (`@FR-F-Ret`, parse time, BOTH backends) declines a
record tail's copy into the return buffer whenever the function's whole last STATEMENT reads
the buffer local — with it off (`Parser::delivered_leaf_reads`), only a delivered leaf that
reads it declines, so a tail after an early `return` of that local is copied into the buffer.
The bisect step for a nullable-record function that answers an empty record from its tail.
**`LOFT_LOOP_LOCAL_CAPTURE_UNCONFINES=1`** (`@FR-L-CapOwn`, the scope pass, BOTH backends) reads a
scalar capture, or a local the loop body binds, as a capture the loop REBINDS, so a closure
built as an argument in that loop is not confined to the pass — with it off
(`closure_keep::capture_set_in`), only a store-holding capture set both in the loop and outside
it unconfines the record, and a confined record borrows the function-scope stores it captures.
The bisect step for a closure built in a loop that frees an outer collection it captured.
**`LOFT_NO_REBOUND_CAPTURES=1`** (`@FR-L-CapKeep`, after the scope pass, BOTH backends) leaves
a closure built in a loop over a local the loop re-assigns with the releases the scope pass
placed alone — with it off (`scopes/rebound_captures.rs`), the re-mint of a backing a record
holds mints fresh, a rebuild frees the capture no other name holds, and the frame's end leaves a
returned record its capture, each by store identity.  Not an optimisation: it is the bisect step
for a stale or freed capture in that shape, and the A/B its emission diff is taken with.
**`LOFT_NO_OWNER_HANDOVER=1`** (`@FR-L-CapOne`, after the scope pass, BOTH backends) leaves a
closure record that owns a capture through its owner slot (`<capture>__own`) releasing it at its
cascade — with it off (`scopes/owner_handover.rs`), a record released while another live record
captures the same store hands the ownership over.  The bisect step for a closure that reads a
store an earlier sibling closure released.
**`LOFT_NO_EXIT_RECORD=1`** (`(R-ExitVector)`'s record clause, the scope pass right after it,
BOTH backends) keeps a local RECORD returned inside the exit literal (`lb = ListBox { … };
Panel { list: lb, … }`) in a store of its own, deep-copied into the return buffer's field —
with it off, the local is a view of that field, built there.  It is the first bisect step for
a wrong field or a stale default out of a callee that assembled its result from locals, and
`LOFT_TRACE_PLACE=1` names each admission and decline (`[exit-record]`).
**`LOFT_NO_EXIT_VECTOR=1`** (`(R-ExitVector)`, decided in the scope pass after the placements,
BOTH backends) keeps a local vector that is returned inside the exit literal (`items:
vector<T> = []; …; return Out { items: items, … }`) minting a wrapper store of its own and
the literal deep-copying it into the return buffer — with it off, the wrapper is claimed in
the return buffer's store (`OpPlaceRecord(__retbuf, …)` behind the exit's own ensure), the
literal takes the vector by a handle move (`OpMoveVector`: the slot's record number moves,
the source slot is zeroed) and every exit releases the wrapper's block (`OpFreeRecordIn`).
It is the first bisect step for a wrong or EMPTY vector field out of a callee that built it
in a local, and for a leak into a caller's store; `LOFT_TRACE_PLACE=1` names each admission
(`[exit-vector]`) and each decline (a rebind, an alias, a bare return, a copy elsewhere, a
mention after the move, a free outside an exit, a wrapper typed a level too deep — the
tuple-destructured local `tests/branch_join.rs::a_vector_local_is_backed_by_its_own_element_type`
guards).  A vector placed this way is not a fresh store root, so the native header and base hoists (`(R-LoopBuffer)`, `(R-CompleteWrite)`,
`(R-Base)`) decline the loops that fill it.
**`LOFT_NO_ZERO_COUNT=1`** (`@FR-R-ZeroCount`, on the closed program, BOTH backends) keeps
`len(t) > 0` and every other comparison of a text's character count with the zero boundary
counting characters — with it off, each reads the byte count (`size(t)`), which answers the
same: a text has no characters exactly when it has no bytes.  The first bisect step for a
wrong emptiness answer; `len(t) == 1` and `len(t) > 1` are never folded.
**`LOFT_NO_LITERAL_HOST=1`** (`(R-MoveLast)`'s host clause, `@FR-R-MoveLast`, decided in the
scope pass after the forwarded results, BOTH backends) keeps a record literal bound to a local
(`cur = Cur { src: toks, pos: 0 }`) minting a store of its own and deep-copying the vector
local its field takes, though that local is dead after it.  With it off, the literal is claimed
in the vector's store (`OpPlaceRecord(toks, …)` where `OpDatabase` stood), the field takes the
vector by a handle move (`OpMoveVector`) and the literal's exit free becomes
`OpFreeRefIfDistinct(cur, owner)` before the owner's own.  It is the first bisect step for an
EMPTY vector field of a local literal, or a vector local that reads empty after one;
`LOFT_TRACE_PLACE=1` names each admission and decline (`[literal-host]`).
**`LOFT_NO_RETURN_FIELD=1`** (`(R-ReturnField)`, `@FR-R-ReturnField`, decided in the scope
pass after the exit vector, BOTH backends) keeps the parser's copy at an exit that returns a
FIELD of an owned record local (`p = mk(n); return p.a`, or `v = p.a; return v`): the return
buffer minted, the field deep-copied into it, the local freed.  With it off, the exit hands
the local's store over at the field's position (`return OpGetField(p, …)` in a
`return_field_handover` block), the root's own free goes, and every other store free in the
exit is witnessed against the root — a pooled `__ref_N` the root adopted or an alias `q = p`
is then skipped, a distinct buffer freed as before.  A returned VIEW local is admitted only when
every bind of it is a field of that one root: a `??` bind's dep names its subject and never its
default, so `pv = maybe(a) ?? T {…}; pv` keeps the copy.  It is the first bisect step for a wrong
or freed record out of a callee that returns a field of a local, and for a caller reading a
store that was released under it; `LOFT_TRACE_PLACE=1` names each admission
(`[return-field]`) and each decline (a parameter's or a view's field, an element, a returned
record that owns no heap — the native value form's to answer — a statement in the exit that
is not a free, a buffer that is not the function's own).

**`LOFT_NO_REBIND_PLACE=1`** (`@FR-R-Rebind`, default-ON, parse time +
scope pass, BOTH backends) keeps `x = f(x, …)` minting the callee's exit record in a store
of its own and copying it over `x` again — with it off, the call's hidden buffer argument IS
`x`, so the callee's exit literal writes x's record in place (a vector field that views the
same field, `Doc { buf: d.buf, … }`, is `(H-CopySelf)` and costs nothing; a scalar the
literal reads off the parameter is STAGED ahead of the first write; `return d` answers the
buffer itself), and a body whose last statement is an explicit `return` takes the buffer
road at all (zttext `delete_range` 138× → 9.6×) — and is the first bisect step for a wrong
field, a leak or a double free out of a local rebound from a call that takes it.
`LOFT_TRACE_REBIND=1` names each admission and each decline (a text field or a vector read
at ANOTHER field of the parameter, a literal-list vector field, a chain exit, a promoted
buffer, the local handed in twice, a view or witnessed local each decline); the falsifiers
are `LOFT_STRICT_STORES` / `LOFT_POISON` / the native leak check on the cells.
**`LOFT_NO_CONST_VIEW=1`** (`@FR-R-Const`, default-ON, parse time + scope
pass, BOTH backends) makes a literal-bodied function build its vector on every call again —
with it off, a zero-parameter function whose whole body is ONE vector literal over literals
(`fn face_rows() -> vector<integer> { [0, 0, 0, 4, …] }`, the idiom the `const` diagnostics
themselves prescribe for what the constant store cannot paste) is given a synthetic constant
twin, pre-built ONCE in `CONST_STORE` like a top-level `NAMES = [ … ]`, and a call whose
result only lands in READ positions — the single bind of a local that is only indexed,
measured, iterated or copied into another such local, or the call itself under an index,
a `len` or an iteration — answers `OpConstRef`, the node a top-level constant's use site
emits; the call's lazy buffer is never minted.  Every other position keeps the call (a
write or append through the local, a hand-off to ANY user call, a store into a field or
element, a return, a `&` link), because the constant store is write-locked and the
program's copy must be its own — and is the first bisect step for a wrong element, a
"write to read-only store" panic or a stale table out of a call of such a function.
`LOFT_TRACE_CONST=1` names each function made a constant (and each that is not, with its
body's shape) and each call admitted or declined; the interpreter and native share the
rewrite, so the switch A/B and the cells' hand-computed values are the falsifier.
**`LOFT_NO_COPY_VIEW=1`** (`@FR-R-CopyView`, default-ON, scope pass, BOTH
backends) makes a read-only record copy copy again — with it off, `t = a` of a record, and a
join `t = if c { a } else { b }`, where t is only READ (a field read), every source is a VIEW
into a parameter's records (not the parameter itself, which `@FR-R-ValueRecord` may carry as
a tuple), and no operation in t's live range can write a record of that type (asked of the
operands' TYPES: a push into a `vector<integer>` cannot reach a record), binds t as a view —
`t = OpGetField(a, 0)`, or the join's arms answering their sources — so no store is minted,
copied or freed.  It is the first bisect step for a record local that reads a value its
source took after the bind, or a store released under it.  `LOFT_TRACE_COPY_VIEW=1` names
each local made a view and each candidate declined with the reason; both backends share the
rewrite, so the switch A/B and the cells' hand-computed values
(`tests/scripts/a-read-only-record-copy-is-a-view.loft`, pinned by `tests/copy_view.rs`) are
the falsifier.
**`LOFT_NO_COMPACT=1`** (`@FR-R-Compact`, default-ON, scope pass, BOTH
backends) makes a vector rebuilt from a contiguous run of its own elements copy again —
with it off, `t: vector<S> = []; for i in a..b { t += [V[i]?]; } V = t;` (a history
truncated to its cursor, its oldest entries dropped) is ONE guarded op: `if 0 <= lo &&
lo <= hi && hi <= len(V) { OpKeepVectorRange(V, tp, lo, hi) } else { the statements as
written }`, the op releasing every element outside the run where it stands, moving the
run to the front as one block and setting the length, and the fallback arm answering
exactly what the copy answered for any range the guard refuses (a negative start, an end
past the length that pads with default records, a null bound).  RECORD elements read
through `?` only — a scalar in range can hold the null its `??` replaces — and `t` named
nowhere else; the filter, prepend and identity forms of the rule keep their rebuild
(dryopea `truncate_to` 259× → 14×).  It is the first bisect step for a wrong, missing or
stale element after a vector is rebuilt from its own elements; `LOFT_TRACE_COMPACT=1`
names each rebuild admitted and each declined with the reason.
**`LOFT_NO_ELEMENT_IN_PLACE=1`** (@PLN164 C1, `@FR-R-InPlaceLiteral`, parse time, BOTH
backends) makes `v[i] = S { … }` build its record in a temp store and deep-copy it into the
slot again — with it off, the literal writes the slot's fields, as the FIELD destination
`o.f = S { … }` always has, with every field expression STAGED first (the language evaluates
a literal's fields before the assignment stores, so `El { a: o.f.b, b: o.f.a }` reads the
place it is about to overwrite) and an omitted field taking its declared default — and is the
first bisect step for a wrong element, a leak or a double free at an element assigned from a
literal.  A computed index keeps the copy (the receiver is re-derived per field write), as do
a collection-TYPED field and a linked-group member.
**`LOFT_NO_BUFFER_IS_PLACE=1`** (@PLN164 C2, `@FR-R-Place`, `@FR-O-Buffer`, parse time, BOTH
backends) makes `h.v = mkints(n)` and the literal `H { v: mkints(n) }` mint a buffer store,
fill it and copy every element into the field again — with it off, the call is handed the destination AS its return buffer: the
buffer stays the variable the call site mints and only what it holds changes, so the result's
deps and the free sweep are untouched, and `(O-Buffer)` gains the clause that such a buffer is
never freed — and is the first bisect step for a wrong, empty or stale vector field after an
assignment or a literal field from a call.  Declined where an argument reaches the destination,
where the callee MINTS a record into its buffer (a projection chain does; a returned
collection literal's wrapper mint answers the place, so it is placed), for a struct-enum
variant's field read through a variant check, and for a literal's `?` field (the replace is
what leaves it absent).  `loft-suite/rules/rewrites/R-Place/a-literal-vector-field-is-built-in-its-field.loft` is
the literal's guard.
**`LOFT_NO_PLACE_FOREIGN_EXIT=1`** (`@FR-R-Place`'s foreign-exit clause, parse time, BOTH
backends) gives a callee one of whose exits answers a global constant, a parameter or a field
of either its own buffer store again, filled and copied into the field — with it off, the call
is handed the field and the site copies the answer in only when it is not the field
(`palette_items_for_tool` answering `HEIGHT_STEP_LABELS`).  The first bisect step for a field
that holds the wrong one of a callee's answers.
`tests/scripts/a-placed-call-copies-an-answer-from-elsewhere.loft` is its guard.
The callee clause admits a CHAIN exit
(`return mk(n)` hands the buffer through; asked recursively, a cycle declines), so a result
that arrives through a chain is placed like a literal's; the switch covers it with the rest.

**`LOFT_NO_CALLEE_DISTURB=1`** (@PLN164 C3, `@FR-B-Disturb`, `@FR-B-Ref-Reshape`, BOTH
backends) makes the disturbance walk read THIS frame's ops only again — with it off, a
container a CALLEE grows or removes from disturbs the caller's live view of it, so
`e = sc.els[0]?; grow(sc, 200); e.a + e.b` materialises the view instead of reading an element
that moved (it answered 4294967401 for 3), the reach closed over the call graph
(`disturbed_params_map`) and the copy notice naming the callee — and is the bisect step for a
wrong field read through a view across a call that appends to or removes from a container.
`LOFT_TRACE_DISTURB=1` names each disturbed place; a hidden `__retbuf` is an argument slot and
is excluded, or every record-returning function that fills a vector field would report one.
It also names every view copied out of its container (`[materialise] fn \`view\` out of
\`container\``) — the `view-copied` advice names only one something writes through.

## Lowering: branches, locals and counted loops

**A value branch of calls witnesses every arm's buffer (@PLN157 § V-af, BOTH backends,
parse time):** `v = if c { mk(i) } else { mk2(i) }` in a loop binds `v` to whichever arm's
hidden return buffer ran, so every arm's buffer is `v`'s witness — the buffers are
allocated once per activation and `v`'s per-iteration free declines against each.
**`LOFT_NO_JOIN_BUFFER_WITNESS=1`** restores the unpaired form and is the first bisect step
for a leak, a double free or a stale record out of a loop that binds a record from a branch
of calls; `LOFT_STRICT_STORES=1` and `LOFT_POISON=1` are the falsifiers.  Where an arm's
answer needs a temp of its own (a call answering a borrow, which the bind copies), `v`
borrows that temp and the temp releases it (`@FR-O-Complete`, `Scopes::lift_join_arm_tails`);
**`LOFT_NO_CALL_ARM_HOLDERS=1`** leaves `v` owning it too — a re-free per evaluation.
A literal built in a buffer the local adopts is freed only where the local moved off it
(`OpFreeRefIfDistinct(buffer, local)`, ahead of the local's own free, `@FR-O-Buffer`);
**`LOFT_NO_LITERAL_BUFFER_PAIR=1`** keeps both plain frees — a re-free per evaluation.

**A match lowered once per arm (`@FR-M-Once`, default-ON, both backends, parse time):** a
guarded arm folds its guard into its own condition (`parser::control::guarded_arm`), so a
failed guard reaches the next arm by control flow and no arm is lowered twice;
**`LOFT_NO_GUARD_ARM=1`** builds the copying form, whose size grows exponentially with the
guards (`tests/match_once.rs` counts both).  It is the first bisect step for a wrong answer, a
leak or a crash in a `match` with guarded arms: a binding first written inside a folded
condition is the shape to suspect.  Guards `loft-suite/rules/matching/M-Once/`.  (An enum arm's
test as one op is the interpreter's alone, `LOFT_NO_TAG_JUMP`: `formal/rewrites.md` `(R-Fuse)`.)

**Store confinement across sibling blocks (default-ON, both backends):** a local reassigned
across sibling `if`/`else if`/`match` arms keeps only the store of the arm that ran:
`recover_backer` confines each block's store to its block, so the watermark grows with the
arms that run, not with the reassignment sites. **`LOFT_NO_CONF_RECOVER=1`** emits the pre-confinement form and is the first bisect
step for a wrong answer in a function that reassigns a local across sibling blocks. ⚠ The
soundness condition is `store_dead_after_block`, NOT the flag: a local READ after the blocks
does not confine, because freeing a confined store while the local still holds it returns the
wrong element on the branch NOT taken. QUALITY-history.md § Cluster III Route 2.

**Owner witness for a mixed-ownership local (loft#1336, default-ON, both backends):** a
heap-record local that OWNS after one assignment (a copy, a minting call) and VIEWS after
another carries a hidden `__own_<name>` naming the store it minted while it still holds it;
the IR releases it by store identity at the rebind or at scope exit, and the local itself is
never-free (`formal/ownership.md` @FR-O-Witness).  A bind of a store a compiler temp holds (an
element of a call's result, a `??` result) is a view to the witness, the lift temp of a call
whose field the local reads hands its store to the witness, and a call whose result the local
adopts never takes a pooled buffer, so every store the witness adopts has one owner.
**`LOFT_NO_OWNER_WITNESS=1`** emits the
pre-witness form: the first bisect step for a leak or a wrong answer in a local that is both
copy-bound and view-bound (a walker `cur: Node? = a; cur = cur.next`), and what the
`LOFT_NO_JOIN_OWN` positive controls set beside their own switch.

**The counted loop's second counter (@PLN157 § V-ab, default-ON, both backends):** a
`for i in a..b` whose start is not a literal runs a hidden `next` counter seeded AT `a`
(tested, yielded into `i#index`, then stepped) instead of a null-encoded counter that asked
`if !i#index { a } else { i#index + 1 }` on every iteration — a null test and a select LLVM
cannot fold, half the cost of a tight loop; a literal start already took P3b's single
counter seeded at `a - 1`, which a computed start cannot use because `a - 1` is the null
sentinel when `a` is the type's minimum.  **`LOFT_NO_NEXT_COUNTER=1`** emits the
null-encoded form again on both backends: the first bisect step for a wrong value out of a
counted loop whose start is a variable or an expression.  Every value the compare and the
step see is unchanged, so the edges are too — including loft#1525, an inclusive range to
the type MAXIMUM that never terminates on either form.

**`LOFT_NO_BYTE_COPY=1`** (`@FR-R-ByteCopy`, default-ON, scope pass, BOTH backends) makes
`for i in lo..hi { buf += [t.byte_at(i) as u8] }` push byte by byte again — with it off, the
loop is one append of the bytes `[lo, hi)` behind `0 <= lo && lo <= hi && hi <= size(t)`,
the loop as written running for a range the guard refuses — and is the first bisect step for
a wrong, missing or extra byte out of such a copy.  It turns off the rule's vector clause with it
(`for i in lo..hi { buf += [v[off + i] ?? d] }` as one slice append behind its in-range
guard) and `@FR-R-TextRun`, which builds on that clause: a byte vector filled only by such a
copy and read once by `text_from_bytes` is read in place by `text_from_byte_range`, so the
switch is also the first bisect step for a wrong text out of a decoder.
`LOFT_TRACE_BYTE_COPY=1` names each site admitted and each kept (`[byte-copy]`, `[text-run]`).

**`LOFT_NO_REPEAT_RUN=1`** (`@FR-R-RepeatRun`, default-ON, scope pass, BOTH backends) keeps a
literal that spells one constant four or more times (`[0.0, 0.0, 0.0, 0.0, …]`) as one push per
element — with it off, the run is a reservation, one template push and one doubling fill, as
`[0.0; n]` lowers — and is the first bisect step for a wrong element out of such a literal.
`LOFT_TRACE_REPEAT_RUN=1` names each run admitted.

**`LOFT_NO_STORE_SWAP=1`** (`@FR-H-SwapIn`, default-ON, runtime, BOTH backends) keeps the deep
copy of a rebind `x = f(…, x, …)` from a callee that minted its result — with it off, the two
stores' contents are exchanged and the released slot freed, where both records are their
store's root of the copied type and the type keeps its pointers inside its store — and is the
first bisect step for a wrong record after such a rebind.  Read at run time, so one binary
serves both arms.  `LOFT_TRACE_STORE_SWAP=1` names each exchange.

**`LOFT_NO_REBIND_OWN_BUFFER=1`** (`@FR-R-Rebind`'s own-buffer clause, default-ON, scope
pass, BOTH backends) keeps `(R-Rebind)` to plain locals: a builder's `d = step(d, …)`, where
`d` is the frame's own promoted return buffer, copies the step's result into `d` again — the
first bisect step for a wrong or stale record out of such a loop.  `LOFT_TRACE_REBIND=1` names
each site.

**`LOFT_NO_VEC_COPY=1`** (`@FR-R-VecCopy`, default-ON, scope pass, BOTH backends) makes
`for e in V { t += [e] }` push element by element again — with it off, the walk is
`t += V` where t is a local or the frame's hidden return or work buffer — and is the first
bisect step for a wrong, missing or extra element out of such a copy.  `LOFT_TRACE_VEC_COPY=1`
names each site admitted and each kept.

**`LOFT_NO_LITERAL_WALK=1`** (`@FR-R-LiteralWalk`, default-ON, parse time,
BOTH backends) makes `for x in [a, b, c]` build and walk the vector again — with it off, the
items are evaluated once into scalar temps and the loop is a counted select over them, no
vector on either backend — and is the first bisect step for a wrong value, order or count out of a loop over a literal of scalars.
`LOFT_REWRITE_CENSUS` counts its admissions as `R-LiteralWalk`.

**`LOFT_NO_FILL_LOOP=1`** (`@FR-R-PushFill`'s fill form, default-ON, IR phase, BOTH backends)
keeps `for _ in a..b { v += [c] }` a loop instead of one push and an `OpAppendCopy` — the
writers' reservation and window clauses then take it; `LOFT_NO_PUSH_FILL=1` turns the whole
rule off.  `LOFT_TRACE_IR_FILL=1` names each loop taken.

**`LOFT_NO_WRAPPER_INLINE=1`** (`@FR-R-Wrapper`, default-ON, scope pass, BOTH backends) keeps
a call to a stdlib one-op wrapper (`len(v)`, `sqrt(x)`, `v.clear()`) a CALL instead of its op —
the bisect step for a wrong length or libm value; it also keeps `(R-SplitTable)` from reading
`len` as its count.  `LOFT_TRACE_WRAPPER=1` names each call taken and each kept with why.

**`LOFT_NO_SLICE_TEXT=1`** (`@FR-R-SliceArg`'s range clause, default-ON, scope pass, BOTH
backends) keeps `text_from_bytes(v[lo..hi])` as a slice built in a store of its own and then
converted — with it off, the bytes are read where they lie, `text_from_byte_range(v, lo, hi)`
after the slice's bound statements — and is the first bisect step for a wrong text out of a
slice of bytes.  `LOFT_TRACE_SLICE_TEXT=1` names each site taken and each kept with why.

**`LOFT_NO_LITERAL_APPEND=1`** (`@FR-R-LiteralAppend`, default-ON, scope pass, BOTH backends)
keeps `t = T { … }; v += [t]` as a record minted for `t`, copied into the element and freed —
with it off, the literal is built in the element, as `v += [T { … }]` is — and is the first
bisect step for a wrong field in an element appended from a bound literal.
`LOFT_TRACE_LITERAL_APPEND=1` names each rewrite.  **`LOFT_NO_RET_ARM_COPY=1`** (`@FR-O-Complete`, default-ON, parse time, BOTH backends) copies a tail branch that can `return` from inside an arm WHOLE through a `_ret_src_N` local again, where each arm is otherwise copied into the return buffer where it ends: the first bisect step for a record freed twice, or left unfreed, at the exit of a function whose tail `match` has an early `return`.  **`LOFT_NO_LIFT_CALL_VIEW=1`** (`@FR-B-View`, default-ON, scope pass, BOTH backends) leaves a local bound from a declared view of a call's result (`json_parse(t).field("a")`, a `-> T[self]` native) typed an owner, beside the lifted temporary that holds the call's answer: the first bisect step for a store freed twice at such a bind.  **`LOFT_NO_ARGUMENT_CLOSURE_CONFINED=1`** (`@FR-L-CapOwn`, default-ON, scope pass, BOTH backends) lets a closure built as a call's argument inside a loop adopt what it captured, where it otherwise borrows (the callee only calls it, so its record ends with the statement): the first bisect step for a captured vector or record freed twice in a loop that passes a lambda.  **`LOFT_NO_RELAY_CAPTURE_BORROW=1`** (`@FR-L-CapOwn`, default-ON, scope pass, BOTH backends) lets a record that a nested lambda builds adopt a capture its enclosing lambda relays out of its own closure (asking the variable a write through the capture leaves behind, which the enclosing lambda never binds), where it otherwise borrows: the first bisect step for a captured store freed twice when a lambda that writes its capture builds an inner lambda over it.  **`LOFT_NO_ARGUMENT_BUILD_SHARE=1`** (`@FR-L-CapKeep`, default-ON, scope pass, BOTH backends) leaves a fn-ref set from a call whose argument builds a closure (`g = pass(fn(x) {…})`) out of the store-identity release that `g = pass(f)` takes, so the record the argument built and the fn-ref the callee hands back both release it: the first bisect step for a closure record freed twice where a callee returns the lambda it was given.  **`LOFT_NO_RETURNED_CLOSURE_CONFINED=1`** (`@FR-L-CapOwn`, default-ON, scope pass, BOTH backends) reads a fn-ref, or a closure built in the argument, that a loop pass hands to a callee returning it to a holder of the pass (`g = pass(f)`) as a value read, so its record adopts what it captured from outside the loop where it otherwise borrows: the first bisect step for a captured store freed twice in a loop that passes a lambda through a callee that hands it back.

## Lowering: adopting and minting call buffers

**`LOFT_NO_NULLABLE_RETBUF=1`** (`@FR-N-Road`, default-ON, parse time,
BOTH backends) takes the caller-allocated `__retbuf` away from a `-> τ?` return again — with
it on, a nullable collection, record or record enum takes the buffer its dense twin takes, and
its caller binds the result through the dense arm (`use_analysis::first_bind_shape`).  It is the
first bisect step for a wrong value, leak or use-after-free that appears only in the `τ?`
spelling of a function.  `LOFT_TRACE_RETPROMO=1` names each return promotion and its verdict;
`LOFT_STRICT_STORES=1` is the falsifier, since every value is right whether or not the delivery
was.  The pairs `scripts/nullable_road.py` measures are the coverage.

**`LOFT_NO_NULLABLE_INPLACE=1`** (`@FR-N-Road`, default-ON, parse time, BOTH backends) builds
a struct literal bound to a NULLABLE local (`x: S? = S { … }`, or an inferred local a later
`= null` widened) on a work reference again — with it on, the literal is built in the local
itself, as its dense twin's is, and the local owns the store.  It is the first bisect step
for a leak or a wrong value that appears only where a `τ?` local is bound from a literal.
`LOFT_STRICT_STORES=1` is the falsifier.

**`LOFT_NO_DEMOTE=1`** (`@FR-R-Demote`, default-ON, parse time, BOTH backends) copies the old
record into the field again where `h = T { …, f: h }` or `o.g = T { …, f: o.g }` moves it there
— the first bisect step for a wrong value or a leak in a list or tree built one node at a time.
**`LOFT_NO_VARIANT_INPLACE=1`** (`@FR-R-InPlaceLiteral`'s variant clause) builds a variant
literal rebinding an enum local or a return buffer on a work reference again, and so also
turns off `(R-Demote)`'s variant clause: the next bisect step for an enum list or tree.

**`LOFT_NO_CAPREBIND_MINT=1`** (`@FR-L-CapRebind`, default-ON, parse time and scope pass, BOTH
backends) lets a rebind of a captured vector local that reads the local itself (`a = a + [3]`,
`a = a[0..1]`) work in the local's store again, and drops the detach that keeps a loop pass's
closure record on the store it was built with (`scopes::rebound_captures`, records confined to
the pass).  The first bisect step for a closure that sees its local's later rebind;
`LOFT_STRICT_STORES=1` and the leak gate are the falsifiers, since an escaping closure then reads
a freed store.  **`LOFT_NO_CAPREBIND_KEEP=1`** lets the last-use pass free a store a captured
local was bound to at its last read here (`scopes::last_use`), while the closure record still
reads it: a use after free the next allocation exposes (`--native-c` under
`LOFT_STRICT_STORES=1` names it; the interpreter under `LOFT_POISON=1` reads a corrupt
reference).  `LOFT_OWN_ORACLE=check` reports it as `capture-freed`.  Three switches bring back
a re-free at a scope's end (`@FR-H-FreeTwice`, scope pass; `LOFT_DOUBLE_FREE` counts it): a fn-ref
freeing the local record it was built over (**`LOFT_NO_FNREF_STAND_DOWN=1`**), a record a
capturing record's cascade adopts (**`LOFT_NO_CAPTURED_RECORD_STAND_DOWN=1`**), and a captured
vector local beside its `__vdb_N` backing (**`LOFT_NO_CAPTURED_VIEW_STAND_DOWN=1`**).

**Adopt at first bind (@PLN164 B1, `@FR-O-Move`, default-ON, both backends, parse time):**
a plain local first-bound from a callee that returns the local it promoted onto its buffer
(`fn mk() -> P { o = P { … }; …; o }`) adopts the store the callee minted — one mint and one
free per call where there were two mints, a deep copy and two frees — paired with the call's
buffer for an identity-guarded free exactly as a literal-returning callee's result is.  The
buffer stays null on purpose: pooled at function entry it is freed by the interpreter's
rebind of the promoted local (plan 51 cluster 3's shape; native guards that free with
`_rb_w_`), which is the plan's B1b.  **`LOFT_NO_ADOPT_FIRST_BIND=1`** restores the copy and
is the first bisect step for a leak, a double free or a wrong field out of a local bound
from such a callee; `LOFT_STRICT_STORES=1` and `LOFT_POISON=1` are the falsifiers.

**Reuse the adopting call's buffer (@PLN164 B1b, `@FR-O-Buffer`, default-ON, both backends,
scopes pass):** the record-buffer pool now takes those buffers too, so such a callee is handed
the CALLER's store after the first call and fills it — one store per call site per activation
instead of one per call (the parse row's `Mark` class).  A callee's promoted buffer local then
holds either the caller's store or one it minted, so the scope pass snapshots the store handed
in (`__rbw_<buf> = OpRefAlias(buf)`) and every free of that local declines on it; and a local
first bound inside an `if` (whose pre-init makes the bind a rebind) adopts at that bind
(`Variable::deferred_first_bind`).  **`LOFT_NO_ADOPT_BUFFER_REUSE=1`** keeps those buffers
null again (its snapshot stays: a rebind frees what it displaces through it) — the first bisect step for a use-after-free or a wrong
field out of a callee that rebinds or returns past a local it promoted onto its buffer.
`LOFT_TRACE_POOL=1` names the gate that keeps each witnessed buffer out of the pool.

**Detach a literal buffer its owner took (`@FR-O-Owner`, default-ON, both backends, after
the scope pass):** a collection literal or copy is built in a `__vdb_N` buffer and handed to
its local through the buffer's field.  Where that local is an OWNER that releases the store
itself (a value branch whose other arm mints, `r = if c { m(0) } else { cp }`), the buffer is
set to the null sentinel after its last use in the block, so its next mint is fresh and its
exit free a no-op.  **`LOFT_NO_BUFFER_DETACH=1`** keeps the buffer naming the store; it is the
first bisect step for a store released twice, or cleared under another owner, out of a
branch's literal arm.  The store timeline (`LOFT_STORES=timeline`, more frees than
allocations) and `LOFT_STRICT_STORES=1` are the falsifiers.

**A reduction loop as one kernel call (@PLN180 § Kernels, `@FR-R-BoundedNest`'s reduction
clause, default-ON, both backends, scope pass):** `acc = acc + v[i]` over `0..v.len()` of an
integer vector — the stdlib `sum` over integers — is one call of the loop kernel
`vector_sum_int`, so the interpreter runs the whole sum in Rust instead of dispatching every
element.  **`LOFT_NO_LOOP_KERNELS=1`** keeps every loop a loop; it is the first bisect step
for a wrong sum on either backend.  `LOFT_HOIST_VERIFY=1` re-checks each plain block the
kernel admits against the checked add.

**A small record returned as a tuple (@PLN180, `@FR-R-ValueRecord`, default-ON, both
backends, at the top of `byte_code_from`, `src/value_record.rs`):** a function returning a
small record of scalars (flat, or with inline sub-records) from object literals returns a tuple
in schema order, and its callers' locals carry it (`@FR-R-ValueLocal`): a field read is a
`TupleGet`, a field write a `TuplePut`, and the local may be handed to a tuple parameter,
copied into a place field by field, or forwarded.  A site that owes a record — an argument a
record parameter takes among them — calls a TWIN that keeps it and runs what it ran before; it
declines the function nowhere else.  A return buffer followed by hidden work buffers is dropped
at its index.  The interpreter claims no store per call.  A program parsed against again (the
REPL, the debugger, live reload, a host calling by name, the live tier — plain `--native`)
keeps records (`Data::open_world`); `--ship` and `--interpret` run the pass.
**`LOFT_NO_IR_VALUE_RECORD=1`** restores the record form, leaving the case to `--native`'s own
rewrite (`LOFT_NO_VALUE_RECORD`); it is the first bisect step for a wrong field out of a
small-record call.  **`LOFT_NO_VR_TWIN_SITES=1`** makes a record-consuming site decline the
function again (the writers' gate); the second step.  **`LOFT_TRACE_IR_VALUEREC=1`** names each
carrier the pass declines and why.  The analysis (`opt::value_records`) skips, each round, a
caller with nothing to decline, and remembers a caller's value locals while the verdicts its
body reads are unchanged.  **`LOFT_NO_VR_SKIP=1`** walks every caller each round; the compiled
program is the same, so it is the control half of an A/B on compile time, never a bisect step.

**A scalar leaf's call is its body (`@FR-R-InlineLeaf`, default-ON, both backends, at the
top of `byte_code_from` after the value records):** a call of a function whose parameters,
locals and result are scalars and whose body is operators over them is replaced by that body
over fresh locals, literal and plain-variable arguments read in place.  Inside the inlined
body three reductions follow: a redundant all-ones mask and a provably non-null `??` go
(`@FR-R-MaskRange`), the last temporary moves into its one read (`@FR-R-SingleUse`), and a
power-of-two scale folds into a literal divisor (`@FR-R-ScaleFold`).  A program parsed against
again keeps the calls (`Data::open_world`).  **`LOFT_NO_INLINE_LEAF=1`** keeps every call and
is the first bisect step for a wrong answer out of a small scalar function on either backend;
**`LOFT_NO_MASK_RANGE=1`**, **`LOFT_NO_SINGLE_USE=1`** and **`LOFT_NO_SCALE_FOLD=1`** keep one
reduction each; **`LOFT_NO_INLINE_APPENDER=1`** keeps the calls of a void leaf that appends to a
record parameter's vector fields (the appender clause); **`LOFT_NO_INLINE_READER=1`** keeps
the calls of a leaf that reads scalars from a `File` parameter (the reader clause — the first
bisect step for a wrong value or cursor out of a small function that reads a file);
**`LOFT_NO_TAIL_SELF=1`** keeps a scalar function's self-call in tail position a call instead
of the jump `(R-TailSelf)` makes of it — the first bisect step for a wrong answer out of a
recursion written as a loop, and the way to see the `MAX_CALL_DEPTH` cliff it removes;
`LOFT_TRACE_TAIL_SELF=1` names each rewrite and each decline;
**`LOFT_TRACE_INLINE_LEAF=1`** names every inlined call, every declined leaf
and every reduction, per function.

**A constructor's call is its literal (`@FR-R-CtorLiteral`, default-ON, both backends, after
the leaf inliner):** `x = ctor(args)`, where `ctor`'s body is one record literal over its
parameters, is replaced by that literal written into `x`, a vector-literal argument built in
its field instead of in a buffer the callee copied from.  An open-world program keeps the
calls.  **`LOFT_NO_CTOR_LITERAL=1`** keeps every call and is the first bisect step for a wrong
field, a leak or a double free out of a constructor call on either backend;
**`LOFT_TRACE_CTOR_LITERAL=1`** names every rewritten call and every declined site.

**The IR-phase eliminations (default-ON, both backends, at the top of `byte_code_from` after
the leaf inliner):** each removes work the IR shows is redundant, and each has its switch and
its trace.  **`LOFT_NO_FORWARD_WALK=1`** keeps `for x in v` an iterator where the body cannot
resize `v` (`@FR-R-ForwardWalk`); **`LOFT_NO_SAME_READ=1`** keeps both of two equal discharged
reads in one statement (`@FR-R-SameRead`), and **`LOFT_NO_MATCH_READ=1`** alone its match
clause, so each arm test of a slice `match` reads the subject again, and
**`LOFT_NO_SEQUENCE_READ=1`** alone its sequence clause, so a store read the next statements
repeat across a fresh record's fill is read again each time; **`LOFT_NO_SELF_REPLACE=1`**
keeps the delivery of a buffer's own view into that buffer at every exit
(`@FR-R-SelfReplace`, trace `LOFT_TRACE_SELF_REPLACE=1`); **`LOFT_NO_IN_RANGE=1`** keeps the discharge of a
record element its loop proves in range (`@FR-R-InRange`), and **`LOFT_NO_LENGTH_LOCAL=1`** alone
its length-local form — a range end `n` bound by `n = len(v)` earlier than the statement before
the loop — together with `@FR-R-Base`'s counter clause reading the same local in the C writer;
**`LOFT_NO_SINGLE_USE=1`** also keeps
a comprehension's element temporary (`@FR-R-SingleUse`'s statement clause);
**`LOFT_NO_DISCHARGE_INTO=1`** keeps the temporary of a text discharge that assigns a local
(`@FR-R-DischargeInto`).  The `LOFT_TRACE_*` of the same name lists what each rewrote, per
function.  For a wrong answer out of a loop or a `??`, these are the bisect steps, one at a
time.

**The key list a keyed lookup pops is cached per type (`@FR-R-KeyList`, interpreter):**
**`LOFT_KEY_LIST_VERIFY=1`** re-derives every cached list from its type on each read and stops
the run on a disagreement, naming the type — the falsifier for a wrong-width key pop.

**Append in place (@PLN157 § V-d, default-ON, both backends, parse time):** a vector-literal
element that is a call to a loft-defined builder — `v += [mk(…)]`, the builder writing a
fresh literal into its hidden buffer on every exit and its return adopted raw — is handed
the element `OpNewRecord` just claimed as that buffer, so the record is written where it
lives: no temporary store, no copy, no free.  The record may carry text,
plain-vector and inline-record fields (the element is fresh, so nothing leaks under the
write); a keyed collection field, a struct-enum field and a synthetic-nullable field keep the
copy.  **`LOFT_NO_APPEND_IN_PLACE=1`** restores the lift + copy — the first bisect step for
a wrong field, a leak or a double free out of an appended call result; `LOFT_STRICT_STORES=1`,
`LOFT_POISON=1` and `LOFT_NATIVE_LEAK_CHECK=1` are the falsifiers.

**Mint at first use (@PLN164 A0, `@FR-O-LazyBuffer`, default-ON, both backends, scopes
pass):** a hidden return buffer (`__ref_N`) is minted in front of the statement that hands it
to a callee, behind `OpRefIsNull`, instead of at function entry — a scanner tried on every
line and matching one paid its buffers' mint and free on all the others (the drawing parse
row −7 %).  A vector buffer carries `Variable::lazy_buffer`, so its entry init is the null
sentinel on both backends and its guarded `Set(b, Null)` is the mint.
**`LOFT_NO_LAZY_BUFFER=1`** mints at entry again and is the first bisect step for a leak, a
double free or a wrong value at a call that takes a hidden buffer.

**`LOFT_NO_BOUND_APPEND=1`** (`@FR-R-BoundAppend`, default-ON, the scope pass's IR phase, BOTH
backends) keeps `k = f(…); …; dst += k` as written — with it off, the append is made where the
call is (`dst += f(…)`, so native builds the result in `dst` through the append twin) and the
statements between read the two lengths from before the growth.  The first bisect step for a
wrong length or element of a vector built by appending results that were bound first;
`LOFT_TRACE_BOUND_APPEND=1` names each append moved.
**`LOFT_NO_TEXT_LEN=1`** (`@FR-R-TextLen`, default-ON, the last of the compile phase's IR
passes, BOTH backends) counts `len(t)` at every use — with it on, a loop counts the
length of a text variable that borrows from nothing and does not escape once, keeps it in a
memo, and counts again only after a statement that may write the text.  The first bisect step
for a wrong length inside a loop that writes the text it measures;
`LOFT_TRACE_TEXT_LEN=1` names each memoised text.
**`LOFT_NO_WORK_BUFFER=1`** (`@FR-R-WorkBuffer`, default-ON, after pass 2, BOTH backends)
makes a vector local that never leaves its frame mint its store at the declaration and free
it at the exit again — with it off, `v: vector<τ> = []` (τ a scalar) whose every mention is
an operand of an in-place vector operator (push, append, length, clear, remove, an element
read or written as a scalar; `for x in v` included) or a by-value argument of a loft-bodied
callee whose answer carries no dep on that parameter is a hidden WORK-BUFFER parameter the
caller supplies as the per-site work-ref a return buffer already takes (minted once per
activation, freed at the caller's exit), and the callee CLEARS it where the declaration
stood; a callee handed the null sentinel takes the rebound-parameter road at that site (its
own store, released against the entry witness), so an entry the runtime builds by hand owes
it nothing (the probe 81 → 33 ns a call).  It is the first bisect step for a wrong, stale or
leaked vector inside a function that declares one, or for a frame-shape fault at a hand-built
entry.  `LOFT_TRACE_WORK_BUFFER=1` names each local promoted and each candidate declined with
the reason; `LOFT_WORK_BUFFER_NULL=1` is the positive control for the null road (every
caller-side buffer left null, both backends).  A hand-off to a `&` parameter or to a callee
answering a view, a local copied whole into any other place (a record's field, where the
emitter builds it in place or the move elision builds it straight into the field; the
return buffer, which adopts it; another local) or bound purely as a copy of another vector
(one copy in and no push, the borrow elision's), a local numbered before an argument in a
function whose return type carries deps (the swap would renumber them), a local declared
inside a loop (`@FR-R-LoopBuffer`'s length reset is cheaper than a clear), a local written
and never read (the dead-store lint's), a mention in a return, a literal, a link or a
capture, a record or text element, a body that suspends or forks, an entry point, a generic,
a lambda, a function a closure or a yielded function value names and a `par` worker keep the
mint (a function passed by name is promoted: its call through the function value hands a
buffer of the calling frame's, `LOFT_NO_FNREF_WORK_BUFFER` in NATIVE_SWITCHES.md).  A caller that
only hands such a buffer down is promoted in its turn (the trace's `ref=… PROMOTED onward`),
so the buffer is minted by the outermost frame that is not; a callee that reaches back to its
caller declines that (`stays a local`), and the same switch turns both halves off.

**`LOFT_NO_RESULT_BUFFER=1`** (`@FR-R-WorkBuffer`'s result clause, default-ON, after pass 2,
BOTH backends) keeps the buffer of a call's vector result in the frame that binds it.  With
it off, a function that binds `x = f(…)` — `f` answering a `vector<τ>` through its hidden
buffer — and only READS `x` (its length, its elements, their fields; `for e in x` included)
takes that buffer as a hidden work-buffer parameter, provided the function runs again (a
call of it stands in a loop, or in a function that does or that calls itself): the caller's
frame keeps the store across the calls, and a callee that refills a live buffer finds the
previous result's elements in it.  Handed no buffer — a call through a function value, a
`par` worker's frame — the function mints its own at entry and releases it at its exit.  A
result written, appended to, copied whole, stored, handed on or returned keeps the mint, as
do a function answering a text and a constant `split` (the native build reads that one
without a vector, `@FR-R-SplitTable`).  The bisect step between `LOFT_NO_WORK_BUFFER` (which turns
this off too) and the rest of that rule: a stale element or length in a vector a call
answered, in a function that only reads it.  `LOFT_TRACE_WORK_BUFFER=1` names each ref
(`result=… PROMOTED to a caller buffer`, `keeps its mint: <why>`).

**`LOFT_NO_WORK_BUFFER_POOL=1`** (loft#1697, default-ON, with `@FR-R-WorkBuffer`) gives every
call site a work buffer of its own again.  By default a frame POOLS them: the k-th work buffer
of element type τ in one call takes the frame's buffer `(τ, k)`, shared by every call site,
because calls in a frame never overlap and a callee clears its buffer on entry.  Per site,
the buffers climb one per call: a function of 150 calls takes 150 hidden parameters and its
caller 150 more per call, which the scope pass pays for.  The pool is disjoint from the frame's OWN promoted locals, the only buffers
that hold data across a call.  The bisect step between the rewrite and the pool: a wrong
value that goes away here and not under `LOFT_NO_WORK_BUFFER` is two buffers of one frame
sharing a store they must not.
## Lowering: slices

**`LOFT_NO_SLICE_COPY=1`** (@PLN174 F4a, `(Slice-Value)`'s block-copy form, default-ON, parse
time, BOTH backends) keeps the per-element loop for a SCALAR-element slice bound to a local
(`s = v[a..b]`, `s += v[a..b]`, and every vector-typed position the coercion serves) — with
it off the span is copied in ONE block (`OpSliceVector`, `Stores::vector_slice`) — the first bisect
step for a wrong slice of a scalar vector.  A heap, struct, type-variable or linked element
keeps the loop either way.  **`LOFT_TRACE_ITERVEC=1`** names an iterator that meets a
vector-typed position and is NOT materialised, with the value's shape — the instrument for
"expected vector, got iterator" surviving at a site the coercion should serve.

**`LOFT_NO_FOREIGN_VIEW=1`** (@PLN174 F4b, `(Slice-Value)`'s view clause, default-ON, RUNTIME,
both backends) copies a slice of a FOREIGN store (a mapped file, a
library's buffer) where a bind into the local's own store takes a VIEW of the bytes
(`OpSliceView` → `Stores::vector_slice_view`; under the switch the same op copies) — the
first bisect step for a wrong byte, a leak or a refused write out of a slice of `file_map`'s
result, and the A/B `tests/foreign_store.rs` runs both cell files under.  The one observable
difference between the forms is that the copy takes a write and the view refuses it.  The
same switch covers a `#native` bridge's whole vector answer bound to a local
(`OpAdoptVector` → `Stores::vector_adopt`, which views a foreign answer and copies under the
switch; `tests/foreign_bridge.rs` is its A/B).

**`LOFT_NO_SLICE_INTO=1`** (`@FR-R-SliceInto`, default-ON, scope pass, BOTH backends) keeps a
scalar slice appended to a PLACE — a literal's field `R { f: v[a..b] }`, `x.f += v[a..b]` — as a
slice built in a store of its own and then appended; with it on, the span is appended onto the
place in one block, `OpSliceVector(P, v, lo, hi, tp)` after the slice's bound statements, the
form a local place takes at parse time.  It also turns off the C's transient clause, which
reads that form.  The first bisect step for a wrong or missing span in a vector field.
`LOFT_TRACE_SLICE_INTO=1` names each site taken and each kept with why.

**`LOFT_NO_RETBUF_ADOPT=1`** (`@FR-R-RetAdopt`, default-ON, every backend: the interpreter's
generator and both compiled writers apply `opt::ret_adopt`'s one decision) makes a
vector-returning function keep its delivery copies — with it on, a shape-A result local ADOPTS
the hidden return buffer: built where it must end up, the exits deliver nothing, the backing
reused across calls.  The bisect step for a wrong vector return, a leak at a vector-returning
call, or values accumulating across calls.  `LOFT_TRACE_ADOPT=1` names the declining gate.

## Layout: out-of-line fields

**A field whose type would contain its host is a child record (@C159, `@FR-L-OutOfLine`,
default-ON, both backends, at layout):** `left: Tree?` in `Tree` holds the number of a record
in `Tree`'s store, written by `OpSetChildRec` and read through `OpRefFromChildRec`.
**`LOFT_NO_OUT_OF_LINE=1`** lays nothing out of line, so such a type is refused again as one
that contains itself; it is the first bisect step only for a program whose types do not
recurse, since a recursive one stops compiling.  For a wrong value, a leak or a double release
in a recursive type, the falsifiers are `LOFT_STRICT_STORES=1` and `LOFT_POISON=1` against
hand-computed values (`loft-suite/rules/binding/B-OutOfLine/`).

**A field write frees a call's answer once (both backends, scope pass):** `OpSetChildRec`, like
`OpCopyRecord` and `OpReplaceKeyed`, frees its source store under `COPY_FREE_SOURCE`, so the
call's `__lift_N` is read back as null after it and keeps no free (`OpSets::freed_source_arg`).
**`LOFT_NO_CHILD_REC_TAKES=1`** leaves `OpSetChildRec` out: the first bisect step for a
re-free (`LOFT_DOUBLE_FREE`) after `cur = f(…)` through a link (`B-Ref-Write/2433-…`).

## Lowering: `??` chains and nested literals

**A `??` chain is RIGHT-associated (loft#1612, `@FR-G-Assoc`, `@FR-B-Copy`, default-ON, both
backends, parse time):** `x ?? y ?? d` parses as `x ?? (y ?? d)`, so every operand is an ARM of a
plain `if` and an arm's bind is the copy `(B-Copy)` describes.  Left-associated, `(x ?? y) ?? d`
has a subject that is not a variable, which is hoisted into a `__ncc_N` temp that VIEWS the
operand it chose — so the destination shared that operand's store (a record on the interpreter, a
vector and a call-first chain on both).  Either grouping answers the same operand in the same
order, so no program's value moves with it.  **`LOFT_COALESCE_LEFT_ASSOC=1`** parses the old form
and is the first bisect step for a wrong value, a leak or a double release out of a chain of three
or more operands.  A chain the AUTHOR parenthesised still hands the parser a hoisted subject, and
is right-associated in the SCOPES pass instead (**`LOFT_NO_COALESCE_REASSOC=1`** keeps loft#1591's
destination gate, the bisect step for that spelling).  A hoist that holds a whole LOCAL rather
than a projection — `(e as B?) ?? d`, `(if c { s } else { null }) ?? d` — is read by the arm lift
as that local, so the destination copies it as `(B-Copy)` says
(`loft-suite/rules/binding/B-Copy/1752-a-coalesce-over-a-whole-local-copies.loft`); a projection subject
keeps the view.  Beside them,
**`LOFT_NO_CHAIN_ARM_SINK=1`** stops a chain BLOCK being a sunk ARM of a branch — with it off,
`x: H = if c { s.h ?? b } else { mk(3) }` writes the chain out as a statement of its own, but ONLY
where every arm of that chain is owned, since an arm that views a place the program can still
reach owes the destination a copy the written-out `Set` cannot make — and is the bisect step for a
double release, a leak or an aliased record out of a branch with a `??` chain in one of its arms.

**A nested literal is written into its field (@PLN164 C6, `@FR-R-InPlaceLiteral`, default-ON,
both backends, parse time):** `sc.ops += [Op { paint: Paint { … } }]` writes `Paint`'s fields
into the element's `paint` field instead of building a store of its own and copying it —
only where the outer record is fresh (an appended element, a construction temporary), so
nothing can read the place while it is written (parse row −6–7 %).
**`LOFT_NO_NESTED_IN_PLACE=1`** builds and copies again, and is the first bisect step for a
wrong field, a leak or a double free in a record built from a nested literal.
**`LOFT_NO_APPEND_STAGING=1`** (loft#1548, `(E-Asgn-Compound)`) lets an appended literal's
field expressions run between the element's mint and its finish again — with it off, a field
that reads or grows its own container (`s.qs += [Q { b: g(s) }]`) is evaluated first; it is
the bisect step for a lost or overwritten element out of such an append.

## Runtime policies

The runtime switches — repeat fills and self-appends, keyed collections, store claims and
clears, the free tree — are in [BOTH_BACKEND_RUNTIME_SWITCHES.md](BOTH_BACKEND_RUNTIME_SWITCHES.md).

## Instrument: the test ledger

**`LOFT_LEDGER=<file>`** makes a run record, per fault site, how often it ran and how often the
processor's answer would have differed from the checked one (an overflow, a null operand, a
zero divisor), merged into `<file>` at exit (@PLN201 step 2, `src/ledger.rs`).  The run is the
ordinary checked run on either backend and answers the same; it is slower, since every integer
operator is a call, and it neither reads nor writes the program cache.  Both backends write the
same file for the same runs.  `--ship` ignores it.  Guards: `tests/ledger.rs`.
