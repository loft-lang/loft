<!-- size-exempt: a formal rule doc, read by rule tag and anchor (DOC_QUALITY § Maintainer docs 2) -->
<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Rewrites — the cheaper forms, which phase applies each, and what each one assumes

**Scope.** The forms a compiler phase substitutes for the plain one when a side condition
holds: a vector header derived once for a loop, a record scalar read once, a view's header
taken at its binding, a stdlib wrapper emitted as its op, a callee handed the values its caller
already holds, a loop tested at its bottom.  Each is a REWRITE — the program's observable
behaviour is unchanged.  ONE rule states one fact, whichever phase applies it: the IR phase
(parser and scope pass — both backends run the result), the NATIVE generator
(`src/generation/`), or the INTERPRETER's bytecode generator and runtime (`src/state/`).
`(R-Phase)` names the three and the oracle each has; a rule says which phase applies it when
it is not the native generator, the default for the rules that name no phase.
The runtime-side units of the same arc (the per-allocation and per-record
bookkeeping, the append path) are not rewrites and are not here; the IR lowerings
both backends share (the fused scalar append, append in place) belong to the
chapters of the constructs they lower.

**Why a chapter.** Every rewrite rests on ONE invariant, and a code site needs a name to cite it.  A rewrite that
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
                 applies no GENERATOR rewrite and is the oracle of every cell of one.
                 BOTH-BACKEND CLAUSE: a rewrite the parser or the scope pass applies —
                 so the interpreter runs it too — has no interpreter oracle, and where
                 it also has no checking form (the ownership / buffer family:
                 O-LazyBuffer, O-Move's adopt, O-Buffer's reuse, R-Place,
                 R-InPlaceLiteral, R-ValueRecord) its falsifier is the SWITCH A/B: the
                 script corpus and the consumer suites run with the switch off and
                 on, and any output difference is a defect.  Such a rewrite lands
                 only with that A/B green (`.github/workflows/switch-ab.yml`).  The A/B
                 compares SEMANTICS, per (R-Escape): an `advice[avoidable-copy]` and a
                 store dump's `bc:` offset report a representation and are not compared.
                 CURE CLAUSE: a switch whose off form is a DEFECT a cure removed (kept as
                 the first bisect step for it — LOFT_NO_OWNER_WITNESS, LOFT_NO_CALLEE_DISTURB,
                 LOFT_NO_APPEND_STAGING) cannot be A/B-clean.  Its falsifier is the
                 reverse: each guard written to catch that defect declares it
                 (`// @catches: LOFT_NO_<CURE>`) and MUST move with the switch, and a
                 declared guard that stops moving is a defect of its own.  Every other
                 program is compared under the cure's switch as under a rewrite's.
                 CENSUS CLAUSE: a rewrite counts each site it ADMITS
                 (`rewrite_census::fired("R-…", n)`, never at a decline), and the
                 counts over the benches are a committed baseline
                 (`bench/portal/rewrite_census.tsv`): a later fix that tightens a
                 condition shows as a DROP, named by rule and bench, in `make
                 rewrite-census` and on the PR — a loss no timing resolves.  A
                 deliberate decline lands with `make rewrite-census-bless` in its
                 own commit.  Counted: every rewrite but two — (R-Prefill) admits per
                 TYPE at its first mint and (R-Cold) is the runtime's own shape,
                 neither an emission-time fact.  A parser or scope-pass rewrite
                 counts at its admission (the standard library's own admissions
                 included, one constant per program); a generator rewrite counts
                 where it emits.
                 INTERPRETER CLAUSE: a rewrite the interpreter's bytecode generator or
                 runtime applies — native never sees it — has NATIVE as its oracle, the
                 mirror of the generator rewrites: the guard's cells run on both
                 backends and agree, the switch off and on, and an emission pin on the
                 bytecode (`loft introspect`) is the census, since the generator's
                 admissions are not counted.  A TIMING of such a rewrite is read on
                 two builds that differ only in it with the layout pinned
                 (INTERPRETER_PERFORMANCE.md § Measuring an interpreter change):
                 two ordinary builds differ by 15 % at identical instruction counts.
  (R-Phase)      every rule is APPLIED BY one phase, and the phase decides its oracle:
                 the IR phase (both backends run the rewritten IR — the switch A/B is
                 the oracle, BOTH-BACKEND CLAUSE), the native generator (the interpreter
                 is the oracle), or the interpreter's generator / runtime (native is
                 the oracle, INTERPRETER CLAUSE).  One FACT may be applied by two
                 phases — native's generator and the interpreter's generator each
                 dropping a reservation the append repeats — and is then one rule with
                 a clause per phase, never two rules.  A rule moved from a generator
                 into the IR phase keeps its name and changes its applied-by line.
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
                 conditions allow.  (C122.)
```

**`(R-Escape)` in words.** Every rule below asks for permission from nobody: it states
the conditions under which a program cannot tell the rewritten form from the written one,
and the compiler validates those conditions or declines.  What it may never do is change
a value, a fault or an effect (C120 is the same principle from the other side), or hand a
representation of its own choosing across a library's API, where the callers that would
have to validate the conditions are not in the compilation.  The freedom rests on loft having
no construct that observes representation — no address, no raw buffer, no `unsafe` (@C135);
the two places representation IS observable, a package's native code and a store bound to a
file, are boundaries every rule here respects.  `(R-ValueRecord)`'s bridge
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
rewrite from a lucky one.  `PERFORMANCE-history.md § Design: P2` and `NATIVE.md` list the
switches; `hoist_verify` in `Output` is the one flag every checking form reads.

**The both-backend clause in words.**  `LOFT_STRICT_STORES` and `LOFT_POISON` catch a leak or
a double free; they do not catch a stale VALUE, and a rewrite the interpreter shares gives
the two backends the same wrong answer, so their agreement proves nothing.  For this family the switch is the
only second answer there is, so it is run over everything, every night, rather than only when
someone already suspects the rewrite.

## The hoist state — what the rewrites compose through

A loop's rewrites do not compose pairwise.  They compose through ONE object, the loop's
HOIST STATE: the map from a vector path to the holder the loop keeps for it, plus the record
scalars it read once.  Each rewrite below is a claim on that state — what it may add, what it
may read, what it must keep current — and the three rules here are what make a combination of
rewrites decidable by a lookup instead of by the order their collectors happened to run in.
([rewrites-history.md](rewrites-history.md) records the first defect this state closed.)

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
                 self-slice pop (OpKeepRange, KEEP-RANGE CLAUSE) sets its header's
                 length to what the runtime wrote and re-derives the header when the
                 runtime moved the record instead; a scalar hoist is evicted at
                 analysis time by the (type, offset) its writes reach (R-Scalar).  An
                 op that could change a held fact and does not refresh it blocks the
                 loop.
```

**In words.** This is why a push may be admitted and an `OpAppendVector`, a remove or a
resize may not: not because they grow, but because only the push's emission refreshes what
it changes.  The refresh includes the RECORD, not only the local: the length is written
back per push so a runtime reader inside the loop (an admitted callee's `len(v)`) sees every
push.  Sites: `Stores::push_hoisted` (bump, write-back, re-derive), `hoist::WriteSet::evicts`
(the scalar half).

**The keep-range clause.**  `OpKeepRange(P, lo, hi, tp)` — `v = v[lo..hi]` over a scalar
element kind — on a pure path P is admitted under the push tier: the runtime keeps the
vector's record (the kept span copied within it, the length written) and answers whether it
did, so the emitter sets the held length at the op's site — the runtime's two clamps over
the length the holder carries — and, where the runtime took the copy form instead (a
foreign or read-only store released and re-appended, or `LOFT_NO_KEEP_RANGE`), re-derives
the whole push header as a push's growth step does.  The popped path holds a PUSH header
(the mutable holder) and joins the pushes' aliasing decision (`R-Alias`): an owned root or
the return buffer, else the loop declines; its root is a mover, so no element base is bound
over its store in that extent (`R-Base`'s growth clause).  A push window excludes it on its
own: the op names the pushed root, which the window's parts may not.  Without the clause
the op declined the WHOLE loop, and three rewrites with it — the stacks' headers, the push
header's appends and the callee's invariant inputs.  Sites: `hoist::keep_range_path` (the
one shape, asked by the gate, the collector and the emitter), `KeepRangeEmitter`,
`Stores::vector_keep_range` (the answer).  Switch `LOFT_NO_KEEP_RANGE_REFRESH` (the loop
declines as before); under `LOFT_NO_KEEP_RANGE` the runtime's copy form takes the
re-derive branch.  Falsifier `LOFT_HOIST_VERIFY=1` (`vector::verify_kept_header`: the
refreshed header against one derived from the record after every pop).  Guard
`tests/scripts/1014-keep-range-refresh.loft` (a stack popped while read, a `hi` past the
length, a pop to empty then pushes, two stacks in one body, a pop from both ends under
`len(v)`, a negative bound).

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
                 admitted whatever its root.  A held path whose store is proven
                 apart from every mover's (R-Base's growth clause — a `for` walk's
                 hidden vector over an owned local, a `&` view of one) cannot name a
                 pushed vector either, and is kept.
                 VERSIONED: a loop whose movers grow the function's return buffer while
                 it reads a PARAMETER runs a second copy under a run-time test that the
                 buffer's store is not the parameter's; inside that copy the pair is
                 apart, and so is every view rooted in the parameter (its element views
                 included), so the parameter's headers and those views' record addresses
                 (R-RecPtr) are kept.  The original loop runs when the test fails.  The
                 copy is emitted only when it gains a header no enclosing frame holds or
                 a record address the plain loop declines.
```

**In words.** The ownership facts are the ONE spelling (`@FR-O-Proxy`, the `__vdb`
witness, `is_argument`, `is_captured`, the hidden return-buffer attribute); this rule reads
them and no rewrite re-derives them.  A rewrite that fails it declines the WHOLE loop — a
mover left to its template would move a record a kept holder still describes.  Today only a
push moves; the rule is written for the next mover too.  Sites: `hoist::owned_local`,
`hoist::retbuf_var`, `hoist::StoreFacts`, the admission block in `hoist::hoistable`.  The
versioned clause: `Output::distinct_version` and `Output::record_ptr_gains` decide the copy,
`StoreFacts::distinct` reads its assumed pairs at the roots of both sides; switch
`LOFT_NO_DISTINCT_VERSION`; guards
`tests/scripts/a-parameter-element-view-keeps-its-address-beside-a-growing-result.loft`
(graphics' `polygon_crossings`: 8.1× → 5.4× of its twin).

### A vector reached by a pure path has one header for a loop that cannot move it

```
  (R-Base)       in a loop that GROWS no store — no push and no mint, WHETHER OR NOT
                 the mint emits through a push header (one left on its templates,
                 an element that is no struct or a tier switched off, appends
                 through the runtime and grows its store all the same); in-place
                 sets, store-free ops, store-free or in-place-only callees, frees
                 and a null-discharge buffer's mint (a FRESH store, or a clear of
                 the buffer's own — neither moves an element any header names) may run — a
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
                 same condition the loop's base rests on.  THE JOIN CLAUSE: a scalar
                 field of a `?`-DISCHARGED element, `v[i]?.f` with `i` a variable, is
                 served the same way.  Its getter's operand is not the element
                 address but the JOIN the `?` lowers to — the element where it is
                 present, a default record minted into a discharge buffer where it
                 is not — and where the loop holds P's header and base the field is
                 one range test and one load through the base, the join run ONLY for
                 an index that test refuses, written once, in that fallback arm, its
                 result read by the runtime's one general typed read.  The join's
                 temp is assigned in range as well — the very element address the
                 join would have given it — so the rewrite drops no effect of the
                 join.  The fallback is never a constant: a negative index addresses
                 from the end there, and an absent element answers its default
                 RECORD's field, which a declared field default makes non-zero.
                 THE BOUND CLAUSE: a range end its prelude binds ONCE from the plain
                 length of a vector path P (`for i in 0..len(P)`, `@FR-I-Range`)
                 reads, inside a loop that holds P's element base, as the held
                 header's length.  The header is the length at loop entry, and a
                 loop holding a header admits no op that could shrink P and, holding
                 the base, none that grows it — so it is the length the prelude took,
                 on every round, and the bound the element accesses are tested
                 against, which the compiler may then drop.  An end that is anything
                 but the plain length keeps its local (`LOFT_NO_RANGE_END_HEADER`).
                 THE GROWTH CLAUSE: a loop that grows a store keeps the base of every
                 held path whose store is proven APART from each store it grows — a
                 store's buffer is reallocated only by a growth of a vector living in
                 it.  A variable's store is the END of its dep chain (a view borrows
                 from its source, a local vector from its `__vdb_N` witness, a
                 parameter from nothing), and two ends are apart when they differ and
                 one is FRESH: a `__vdb_N` witness, or a user-named local record
                 owning its store — minted by this activation, shared with nothing
                 but its borrowers.  Never fresh: the witness a return-buffer adoption
                 left unallocated (the local IS the caller's buffer), a buffer
                 (R-Place) claimed in another record's store, an emitter-owned buffer
                 or witness.  Two parameters, or a parameter beside the return buffer,
                 are never proven apart — a caller can hand two fields of one record
                 — and a chain that forks or leaves the table answers nothing.
                 `LOFT_NO_DISTINCT_GROWTH` keeps every base off under any growth; the
                 verify form re-derives the base at every use as before.

  (R-Counter)    a counted range's counters — its `#index`, the `next` counter of a
                 computed start, and the loop variable — are never the sentinel: an
                 exclusive range steps its counter to at most its end, so the step
                 cannot overflow, and an inclusive range is admitted when its end is
                 a literal below the type's maximum.  The counters are seeded into
                 the non-sentinel proof from the one parser that reads a range's
                 shape, so the step emits as the non-null checked add and every
                 index built from the counter alone loses its operand pre-tests.
                 That parser reads the loop's ITERATOR — its first statement — and
                 nothing after it, because the counters are a fact about the
                 iterator: a `for` statement, one with a filter or a `#count`
                 step, and a comprehension `[for i in a..b { e }]` all step the
                 same counters under the same test, and all are counted ranges.
                 A rewrite that also reads the BODY as one block (R-Fill,
                 R-BoundedNest) asks for the plain `[iterator, body]` shape
                 itself.  The
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
                 scalars to ONE pure path at its top level every iteration — or to
                 SEVERAL paths with distinct roots, each judged on its own with the
                 others' pushes read as plain statements, which reserves every path
                 first and re-derives every header after (no fill, no window) — nothing
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
                 1` — plus one for an inclusive range, taken at loop entry.  The loop
                 is read in EITHER spelling — the statement `for i in a..b { v += [e] }`
                 and the comprehension `[for i in a..b { e }]` build one vector — and
                 the reservation is emitted IN FRONT of every guarded copy of the loop
                 (R-BoundedNest, R-GuardedChain): each emits a whole copy ahead of its
                 `else`, so a reservation written after the guard stands in the arm
                 that does not run.  THE WINDOW CLAUSE: once the trip count is
                 reserved nothing in the loop can grow that vector, so where NOTHING
                 BUT THESE PUSHES REACHES IT the pushes go through a WINDOW — the
                 address of element 0, the length reached and the capacity in
                 elements, three scalars whose address never reaches a call — and a
                 push that fits is one comparison, one store through the held address
                 and a bump of the window's length.  The record's own length is
                 written when the window CLOSES, after every copy of the loop, and
                 before the runtime's append on the growth arm, which answers a fresh
                 window; between those the push header is frozen and still describes
                 the record exactly.  (R-Refresh) asks that a push's refresh reach the
                 RECORD so that a runtime reader inside the loop sees every push; the
                 window is admitted exactly where there is no such reader: the pushed
                 root is exclusive (R-Alias), the body never names it outside a counted
                 push's own vector operand, every other variable the body names cannot
                 name its store either by the ownership test (R-Alias) keeps a read
                 candidate under — a scalar or a text, a local that owns its own
                 store, a parameter while the root is no return buffer, a view of a
                 plain NO-HEAP record (it reads fields and never a length, and the
                 window's slots lie past every element it can name); a vector view
                 taken before the loop fails it — and every other statement, and
                 every pushed value, GROWS no store (the header hoist's own
                 allowances: an in-place scalar set, a discharge buffer's mint and
                 defaults, a record free), so no buffer is reallocated under the
                 held address.  The single exit is the counted loop's own
                 (no `break`, `return`, `continue` or inner loop), so the close runs
                 on every path out.  A body that fails any of these keeps the header
                 push, which costs the window and never a value.  THE RECORD CLAUSE:
                 the pushed values may be RECORD MINT GROUPS — the parser's
                 reservation, mint, field sets on the fresh element and finish —
                 standing at the body's top level or under `if` arms, each over ONE
                 plain vector whose element qualifies for the record push
                 (R-PushRec); the arms of a branch are exclusive, so the most groups
                 any one pass runs, times the trip count, bounds the appends and the
                 reservation holds.  The window's admission is the scalar clause's,
                 read over the groups' operands: a field set whose root is the fresh
                 element is a write through the window's slot (it can name no other
                 element), so is a whole-record copy INTO the fresh element (a
                 builder's tuple delivered), the finish's element operand is that
                 slot too, and any other mention of the vector — a read, a view of it or of an element
                 named in the body, a second pushed path, a group whose element's
                 own heap the literal fills (a claim in the store whose base the
                 window holds) — keeps the header mints.  A windowed mint takes its
                 slot from the window (`base + len·size`, zeroed through the held
                 address, the record's length untouched), the group's field sets and
                 the address (R-RecPtr)'s mint clause holds are that pointer, its
                 finish is the window's length bump, and the close writes the
                 record's length once after every copy of the loop.

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
                 outer one already holds.  The FUNCTION clause: a body that reads a
                 vector PARAMETER twice or more through an element address, and
                 leaves that parameter as it found it — never rebinds it, never
                 hands it to a native that is not a reader, never frees it, and
                 hands it (or a heap value that may name its store: a parameter, a
                 view, a link — never a local's own fresh store or a work buffer)
                 only to callees of which the same holds at that position, a
                 recursive call answering for itself — binds its header once at
                 entry, and every read in the body, loops included, serves from it.
                 Growth of OTHER stores is no concern of a header (its record and
                 length are the vector's own), so a body that builds its result
                 beside the reads keeps them; only a BASE would need the store's
                 memory still, and the clause binds none — a loop inside that is
                 growth-free derives its base off the held header.
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

**(R-PushFill)'s window clause, in words.**
(`bench/portal/analysis/vector-build.md`).  A push through the header cost a capacity test in
bytes, a store resolved through `allocations[store_nr]`, the element, the length bumped in
the header AND written back to the record — 2.3–3.1 ns an element against a Rust `Vec`'s
0.3–0.75 — in a loop that, once reserved, cannot grow.  Three things were true and unused:
the comprehension was no counted range to any rewrite (its loop has three statements and the
range parser asked for two, so its step stayed the null-aware add, its arithmetic fully
checked, and it reserved nothing); the reservation stood in the guard's `else` arm; and the
push resolved a store it could have held.
THE FORM IS LOAD-BEARING: the header's own `len` is not the counter and the header is not handed to the growth arm by reference — its address would escape, LLVM would keep it on the stack, and every push would load and store the length through memory.  The window is three scalars, the growth arm takes the length BY VALUE and answers a fresh window by value.  Switch `LOFT_NO_PUSH_WINDOW` (and `LOFT_NO_PUSH_FILL`, one rule); trace
`LOFT_TRACE_PUSH_FILL=1` names the clause that declined a window; falsifiers
`LOFT_HOIST_VERIFY=1` (every windowed push checks the frozen header against a fresh
derivation and the window's base and capacity against that header, and the close checks the
header it leaves) and `LOFT_STRICT_STORES` / `LOFT_POISON` / `LOFT_POISON_CLAIM` /
`LOFT_NATIVE_LEAK_CHECK`.  ⚠ `LOFT_HOIST_VERIFY` CANNOT see the defect this clause exists to
prevent: a runtime READER meeting the lagging length changes nothing, so there is no stale
fact to compare — the interpreter is the falsifier, through the cells.  The looser
reading of `owned_local` is (R-Alias)'s own and is recorded here, not narrowed on suspicion.
Sites: `hoist::range_counters`, `hoist::plain_for_body`, `hoist::push_loop`,
`hoist::push_window_ok`, `Output::push_reserve` / `push_window_close` /
`active_push_window`, the `Value::Loop` emission in `emit.rs` (the order), the registry's
`HoistedPushEmitter`, `vector::PushWindow` / `push_window`, `Stores::push_windowed` /
`push_window_grow` / `push_window_close`.  Cells `tests/scripts/158-push-window.loft`
(w1–w12 a window; d1–d8 the declines; a1–b1 the aliasing cases; r1 the growth condition),
pins `tests/push_window.rs`.

**(R-PushFill)'s record clause, in words.**
(`bench/portal/analysis/round-3.md` § Built).  The consumer bench's `enum_match` builds
its edits as `for i in 0..3000 { if i % 3 == 0 { edits += [SetHeight { … }] } else if … {
edits += [Paint { … }] } else { edits += [Wall { … }] } }`, and every record went through
the header push: a capacity test in bytes, a store resolved through
`allocations[store_nr]`, the slot zeroed, the length bumped in the header AND written back
to the record.  The window clause did not reach it for two reasons that are one — a mint
group is four statements the scalar push's recogniser does not read, and it stands under
an `if` arm, which the scalar clause declines — and neither matters to the window: the
arms are exclusive, so at most one group runs a pass, the trip count times that bounds the
appends, and the window never grows inside the loop.  The first
hand placement closed only the CHECKED copy of the loop and read hash `0`: a window whose
close does not run leaves the record's length at 0, and the consuming loop reads an empty
vector — which is why the close stands after every guarded copy, and why the VALUE channel
is the falsifier here (`LOFT_HOIST_VERIFY=1` compares the frozen header against a fresh
derivation at every mint and cannot see a length nothing wrote).  Two admissions the first build got
wrong, each a cell: a NESTED record element's field set (`pos.x` on the fresh element)
read as a view of the vector because its path had a field on the root — the root being
the fresh element decides, not the path's length (r5); and the finish's own element
operand counted as a mention of the vector (every group would have declined).  Switch
`LOFT_NO_PUSH_WINDOW` (and `LOFT_NO_PUSH_FILL`, one rule); trace `LOFT_TRACE_PUSH_FILL=1`
names the clause that declined; falsifiers `LOFT_HOIST_VERIFY=1`, `LOFT_POISON` /
`LOFT_POISON_CLAIM` (the slot's zero is the window's own, so a mint that skipped it reads
the poison), `LOFT_STRICT_STORES`, `LOFT_NATIVE_LEAK_CHECK`, and the interpreter through
the cells.  Sites: `hoist::mint_loop` / `MintLoop`, `hoist::mint_window_ok` (sharing
`window_parts_ok` with `push_window_ok`), `Output::mint_reserve` / `windowed_mint_of`, the
windowed-mint address in `Output::bind_record_ptr`, the registry's `NewRecordEmitter` and
`FinishRecordEmitter` windowed arms, `Stores::push_record_windowed` /
`push_record_window_grow`.  Cells `tests/scripts/158-record-window.loft` (r1–r3, r5, r9
and `build` a window; r4, r7, r8, r10, r11 the declines), pins `tests/record_window.rs`.

**(R-Base)'s join clause, in words.**
(`bench/portal/analysis/vector-build.md`).  `v[i].f` was already one load — the scalar
getter sits on the element address and the two fuse — but with the `?` the getter's operand
is a `Block`, so the fused read answered `None`, the join ran on every pass and its result was
read through the store again, in a loop holding the base.  The
result-identity form looked better on paper (it re-evaluates nothing and drops no effect) and
loses 40 % of the gain: the join stays in the loop, and with it a cold `OpDatabaseNP(cell, …)`
call that holds the whole body back.  Assigning the temp in range costs NOTHING, so the form
owes no proof that the temp is unread — a walker that did not have to be written.
⚠ TWO SITES, ONE RECOGNISER.  An emitter arm alone compiles, passes every value test and never
fires: the pre-eval collector lifts every `Block` argument into a `let _pre_N`, so the getter's
operand is a local by the time its emitter runs.  `hoist::fused_join_read` is asked by both
(as `fused_element_read` is), the collector leaving the join where it stands and the emitter
folding it; only a PIN can show they agree (`a_folded_join_is_written_once…`), no value can.
The index is a VARIABLE in this build: a computed one is evaluated for the range test and
again inside the fallback's join, and the second evaluation of a CHECKED operator can note
one overflow twice.  Switch `LOFT_NO_JOIN_READ` (and `LOFT_NO_ELEM_FUSE` / `LOFT_NO_VECTOR_BASE`,
one rule); falsifier `LOFT_HOIST_VERIFY=1` re-derives the header and the base at every such
read — and CANNOT see the defect this clause's design avoids: sabotaged to answer a constant
off the fast path, j3 reads `null null null` for `80 57.5 52.5` with no panic, because the
absent arm holds no fact the verifier compares.  The interpreter is the falsifier there.
Sites: `hoist::fused_join_read`, `Output::fused_join_read`, `pre_eval::collect_pre_evals_inner`
(the arm that leaves the join in place), `ops::vector_ops::emit_join_read` inside
`FusedElementReadEmitter`, `vector::elem_field_at`, `vector::field_of`.  Cells
`tests/scripts/158-join-read.loft` j1–j11, pins `tests/join_read.rs`.

**(R-Base)'s growth condition, in words.**
`growth_free` counted a loop's pushes and its MINT PUSHES — the mints that earned a push
header — so a mint left on its templates read as no growth, and a base was bound for a
sibling field of the record whose store that mint was growing.  Reachable in the default
configuration: `o.rows += [seed]` with `rows: vector<vector<integer>>` (an element that is no
struct never earns a header) beside `o.vals[i % 4]?` answered `null` on --native for
1 425 000, and `LOFT_HOIST_VERIFY=1` panicked "hoisted vector base is stale"; under
`LOFT_NO_RECORD_PUSH=1` a plain struct element answered 3 200 425 for it.  Every mint counts
now.  Cell r1, pin `a_mint_left_on_its_templates_is_still_a_growth`.

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

**In words.** The `(R-LoopBuffer)` shape for a record.  A record literal
bound inside a loop cost the free of its store at the body's end and a fresh store next
pass — `free_named`, the free-slot search, the re-init, the claim, the zero, the tag: 39 ns
per pass against 10 ns for a vector loop buffer's reset — where a local declared outside
the loop already kept its store through `OpDatabase`'s clear arm.  Switch `LOFT_NO_LOOP_RECORD`
(and `LOFT_NO_LOOP_BUFFER_REUSE`, one family); trace `LOFT_TRACE_LOOP_RECORD=1`; falsifiers
`LOFT_STRICT_STORES` / `LOFT_POISON` / `LOFT_POISON_CLAIM` / `LOFT_NATIVE_LEAK_CHECK` (a kept
store must still be freed exactly once).  Sites: `hoist::loop_records`, the `Value::Loop`
emission in `emit.rs` (prelude and postlude), `output_block` (the two dropped statements),
`ops::misc_ops` (the kept-record mint).  Cells `tests/scripts/157-loop-record.loft` l1–l15,
pins `tests/loop_record.rs`.

### A return buffer refills the store a rebind released

```
  (R-RefillBuffer) on `--native`, the store `(H-SwapRebind)` releases — holding the
                 rebound variable's previous value, a root of its own type that
                 nothing references — is KEPT instead of freed (one at a time; a
                 second release frees the first).  A function's hidden return buffer
                 whose every mint is the buffer's null-check prologue or a bare
                 `OpDatabase`, each heading a literal group that writes every field
                 (`(R-CompleteWrite)`'s coverage) of a REFILLABLE type — every field
                 owns no heap, or is a vector whose elements own none — mints with
                 `OpDatabaseRefill`: a null buffer takes the kept store of the same
                 type as it stands, and the group's zero of each vector field empties
                 that vector in place (`clear_vector`) for the fill that follows.
                 Every other mint is fresh and the empty is a no-op on it.  A text, a
                 keyed collection, a nested record that owns heap, a mint the walk
                 cannot see the group of, and a `par` worker's stores decline.
                 THE IN-PLACE CLAUSE: an empty followed by a repeat literal of the
                 same field (`n` copies of one scalar, `(R-PushFill)`'s form) is one
                 overwrite when the kept vector holds exactly `n` elements of that
                 width — every element written, the length already `n`, so the
                 answer is the empty-and-fill's; any other length, a foreign or
                 locked vector, empties and fills.
```

**In words.** The `(R-LoopRecord)` shape across a call.  A builder called in a loop,
`c = mul(a, c)`, minted a store for its result every call, claimed its vector and filled
it; the rebind then exchanged it into `c` and freed the store holding `c`'s old value.  The
two stores now take turns: the one released is the next call's buffer.  It is exact because
the literal is complete — every scalar is written again — and a refillable type's only heap
is vectors the group empties and refills, so nothing of the old value survives and nothing
old is left unreachable in the store.  A kept store is the runtime's, not a leak, and is
skipped by the exit census.  Effect on mesh3d's `mat4_mul` loop: priced by hand at −52 %
of the call.  Switch `LOFT_NO_REFILL_BUFFER=1`, read where the buffer is emitted and at run
time.  The in-place clause skips the reset, the reserve and the two header derivations of a
fill whose length the kept vector already has: `mat4_mul` −17 % (priced −24 %), switch
`LOFT_NO_REFILL_IN_PLACE=1`, site `Stores::fill_exact`, guard
`tests/scripts/a-refilled-repeat-literal-is-overwritten-in-place.loft`.  Sites: `hoist::refill_buffers`, `ops::misc_ops` (the mint), the block loop in
`emit.rs` (the vector-field empty), `Stores::take_spare` / `park_spare`.  Guard
`tests/scripts/a-return-buffer-refills-the-store-a-rebind-released.loft`, pin
`tests/refill_buffer.rs`.

### A pooled buffer's texts are refilled in their slots

```
  (R-RefillText) at a call site whose result buffer b is a POOLED call buffer —
                 minted once per activation by a zero-filling OpDatabase and
                 released before every later call by OpClear(b, tp) (the
                 (R-Reuse) pool, @FR-H-ClearRelease) — the release is not
                 emitted, and the call goes to the callee's REFILL TWIN, whose
                 text sets into its own return buffer refill the slot: the
                 slot's block is written over when the new text fits its claim
                 (len + 8 ≤ 8 × the block's header words), released and claimed
                 anew when it does not, claimed when the slot is 0; a null text
                 releases the slot's block.  Admitted where (a) tp's heap is
                 TEXT ONLY — every heap slot of the type, nested inline records
                 flattened, is a text (no collection, no reference, no enum
                 payload that owns heap); (b) the callee builds its result as a
                 record LITERAL into its own buffer on EVERY exit — `return b`,
                 or a block whose value is b — the literal writing every text
                 slot of tp exactly once, and nothing else in the callee names
                 the buffer; and (c) b is named by nothing but its pool
                 statement, this call's buffer argument and its releases, and
                 the result bound from the call is only READ (a reading OpGet*,
                 the source of a copy that keeps its source).  Where the site
                 would call an `__inv` or `__rg` twin it releases first instead,
                 in the pool statement's order.
```

**In words.**  A pooled buffer's release walk frees every text the previous result held, and
the callee then claims a fresh block for every text of the new one: per call, one walk and
one claim-and-free per text field, for blocks that are the same size each time more often
than not.  Writing over the old block when the new text fits keeps the bytes where they are
and drops both halves.  What makes it sound is ONE invariant, enforced at three points:
**a text slot read by the refill holds 0 or a text block that slot owns.**

- *The buffer's history* — (c): its mint zero-fills it (a slot is 0) and every later write
  is the refill twin's literal (a slot owns its block).  A no-prefill mint
  (`OpDatabaseNP`, `(R-CompleteWrite)`) or an element slot handed in as the buffer
  (`(R-Place)`, `(R-ElemFirst)`) leaves BYTES there, which a refill would read as a block
  number; neither is a pooled buffer, and neither can reach a refill twin, because the
  decision is the SITE's, not the callee's.  A twin that mints its own buffer keeps the
  zero-filling mint for the same reason.
- *The callee* — (b): every exit is a literal writing every text slot, so after the call no
  slot still holds the previous result's text (no stale value is visible, none leaks); a
  forwarded exit (`return g(…)`), a copy into the buffer (`OpCopyRecord`, which claims fresh
  blocks over the old ones) or an early exit that writes nothing declines.  The parser
  writes every field of a literal, a defaulted text included, so what (b) protects is the
  live-record count, not a value.
- *The type* — (a): a refilled text slot is the only heap the walk would have freed; a
  collection or a heap-owning enum payload would be left for the walk and is out of scope
  until a clause of its own (vector-build.md's text-bearing elements).

Why a TWIN and not the callee itself: the same callee is also called with buffers that do
not keep the invariant (a fresh element slot, a no-prefill mint), and one emitted body cannot
tell them apart; a twin, chosen where the buffer's history is known, can.  It is the shape
`(R-Inputs)` (`__inv`) and `(R-RangedCall)` (`__rg`) emit.  A previous result read through a
borrow after the next call is no new hazard: the release walk freed its block at the same
point, and the scope pass never pools a site whose result a later text still depends on (it
mints per call there instead).

Native only; the interpreter keeps the walk and the fresh claims and is the oracle.  An
armed release no call takes is an emission error, never a silent leak.  Effect on
game_protocol `msg_ping` (x86-64): 18.0 → 4.89 ms, 7.55× → 2.05× Rust, hash unchanged.
`flow_layout_full`, pluginabi `check_request` and cbor `decode` need the collection clause
after it (vectors of text-bearing records).  Switch `LOFT_NO_REFILL_TEXT=1`; trace
`LOFT_TRACE_REFILL_TEXT=1` names each pooled site's verdict.  Sites: `hoist::refill_text_sites`
and `hoist::refill_text_callee` (the admission), `Stores::text_slots` (a), the block loop in
`emit.rs` (the pool statement), `user_fn_call_body` (the call), `substitute_template_body`
(the twin's text set), `Store::refill_str` (the write).  Guard
`tests/scripts/a-pooled-buffers-texts-are-refilled-in-their-slots.loft` — value cells and a
live-record census (`m1`), with three planted defects under `tests/falsified/` — and pin
`tests/refill_text.rs`.

```
  (R-RefillText), the collection clause: a function whose hidden return buffer b is
                 a (R-RefillBuffer) heap-element buffer — a one-field wrapper of
                 vector<E>, E a record of scalars and texts — KEEPS b's elements
                 where the mint keeps b (refill_keeps, the test OpDatabaseRefill
                 asks): the entry sets the length to 0 and keeps K, the old length,
                 in place of the releasing clear; a text set into an element the
                 build appended refills the block its slot owns when the slot is
                 under K and claims past it; and every exit releases the texts of
                 the kept slots [len, K) the call did not reuse.  Admitted where the
                 buffer is cleared once, at entry; every append mints its element
                 with OpNewRecord, emitted without the prefill (R-CompleteWrite);
                 and every other mention of b, its field views and its appended
                 elements is a reservation, a length, an element read, an
                 append's finish, an element's own set or read, the field's zero,
                 a self-replace, a release or a return.
```

**In words.**  The same invariant, one level down: a text slot read by the refill holds 0 or
a block that slot owns.  Slots under the length at entry are live elements, which own their
texts; past the length a removal can leave a stale copy of a live element's handle, and fresh
capacity holds whatever the store left there, so neither is trusted and K is the length, not
the capacity.  The length stays true during the build — a read of the vector mid-build sees
only what this call appended — which is why each kept slot still goes through the append (a
form that overwrote kept elements in place under the old length priced −20 % against this
form's −16.5 %, and answers a mid-build read with stale elements).  The vector grows only
when its length reaches its capacity, which is at least K, so a slot past K is never one a
growth copied.  A second clear inside the build would reset the store under the kept slots;
a prefilling mint would write 0 over a kept text and strand its block; a removal, an insert
or any use the whitelist does not name declines.  Effect on zttext `flow_layout_full`
(x86-64): 26.0 → 22.7 ms, 14.15× → 12.35× Rust, hash unchanged.  Switch
`LOFT_NO_REFILL_ELEMENTS=1`; `LOFT_TRACE_REFILL_TEXT=1` names each heap-element buffer's
verdict.  Sites: `hoist::keep_elements` (the admission), `Stores::refill_keep_open` /
`refill_keep_close` (entry and exit), `codegen_runtime::RefillKeepGuard` (the exit on every
return), `clear_vector` in `text.rs` (the entry), `substitute_template_body` (the element's
text set).  Guard `tests/scripts/a-kept-buffers-elements-are-refilled-in-their-slots.loft`,
with two planted defects under `tests/falsified/`; pin `tests/refill_text.rs`.

```
  (R-RefillText), the vector clause: at a pooled call site whose buffer b's type has
                 for heap only vector fields of plain elements (no text, no
                 collection, no linked group in an element; no other heap field —
                 Stores::vector_slots), and whose callee (R-RefillBuffer) refills its
                 buffer with elements that own no heap, the release is not emitted
                 and the plain callee is called.  Admitted under (c) without its
                 "only READ": the buffer named by nothing but its pool statement,
                 the call's buffer argument and its releases, the result bound once.
```

**In words.**  The refilling callee keeps a live buffer (`refill_keeps`) and empties each
vector field where it stands; the release before the call freed exactly the vectors it would
have kept, and the callee's appends then claimed new ones.  The invariant the emptying reads
is that a vector slot holds 0, the absent mark, or a vector that slot owns — and every write
the language makes to a vector field keeps that, which is why the result may be written,
grown or passed on between calls, unlike a text clause's refilled slot.  No twin is needed:
an emission of the callee that does not refill mints over the live buffer, and that mint
clears its store.  Native only; the interpreter keeps the release and is the oracle.  Effect
on hex_recover `forms_upto` (x86-64): 2.00 → 1.02 ms, 12.96× → 6.62× Rust, hash unchanged.
Switch `LOFT_NO_REFILL_VECTORS=1`; `LOFT_TRACE_REFILL_TEXT=1` prints `admitted (vectors)`.
Sites: `hoist::refill_vector_callee`, the clause choice in `refill_text_site_declines`,
`RefillTextSites::vectors`, `refill_text_call` in `calls.rs`.  Guard
`tests/scripts/a-pooled-buffers-vectors-are-emptied-in-place.loft`; pin
`tests/refill_text.rs` (the values pass either way, so the pin is the falsifier).

### A rebind hands the displaced store to the call

```
  (R-RebindBuffer) on `--native`, the rebind `x = f(…)` of an owned local from a
                 direct call that answers a FRESH store (`Own::Owned`) through a
                 hidden buffer local `b` — a compiler local nothing but this call and
                 its scope-exit free names — ADOPTS the result and hands `x`'s
                 previous store to `b`, the buffer of the call's next execution,
                 instead of exchanging the result into `x`'s store
                 (`(H-SwapRebind)`).  Run-time conditions, each falling back to the
                 exchange: the result has a store of its own (not null, not `x`'s),
                 `x` held one, `b` is null or IS the result's store (so no live
                 buffer is dropped), and `x`'s store is not the caller's entry buffer
                 (a return-buffer local).  Static conditions: `x` is not an argument
                 (the return-buffer attribute with its entry witness excepted), not
                 captured, not never-free, not witnessed, not a coroutine field; the
                 rebind sits inside a loop (the handed store is a buffer only if the
                 call runs again); and no local borrowing `x`'s or `b`'s store is named
                 at or after the top-level statement holding the rebind.  At its
                 release `b` PARKS the store it holds where the exchange would have
                 parked its released one (`(R-RefillBuffer)`'s spare), so the stores a
                 function leaves behind for the next mint are the exchange's.
```

**In words.**  `c = mul(a, c)` in a loop built each result in a buffer, then exchanged the
two stores' contents so that `c` kept its slot — a check of both stores' flags, the swap,
the park of the released store and a take of it at the next mint, every iteration.  Nothing
needs `c`'s SLOT to stay put when nothing else names it: the result is fresh, so `c` can
simply take it, and the store `c` held — whose value the rebind ends — is exactly a buffer
of the right type for the next call, which the refilling callee (`(R-RefillBuffer)`) writes
in place.  The two stores take turns by name instead of by content.  The scope-exit free of
`b` releases whichever one `c` does not hold.  Effect: mesh3d's `mat4_mul` loop 54 → 39 ns
per product (−29 %, priced −29 %).  Switch `LOFT_NO_REBIND_BUFFER=1`, read where the rebind
is emitted; trace `LOFT_TRACE_REBIND_BUFFER=1` names each decline.  Sites:
`Output::rebind_buffer_for`, the call-return arm in `generation/dispatch.rs`, the body
pre-pass that marks the buffers (`Output::rebind_handed`, so a release emitted before the
rebind — an early `return` inside the loop — parks too), and `Stores::park_or_free`.  Guard
`tests/scripts/a-rebind-hands-the-displaced-store-to-the-call.loft`, pin
`tests/rebind_buffer.rs`.

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
                 or marks a fresh destination: both stay store writers.  THE READ
                 CLAUSE: f#read(n) as a SCALAR writes that local and the File
                 record's cursor fields in place, and is on the list with the
                 address of its local; the scalar tier still reads it as a write
                 it cannot type.  A text, vector or record read stays a writer.
```

**In words.** @PLN157 P4a.  Aliasing is free for headers under this rule — an
in-place write moves nothing, so no header, aliased or not, can go stale; that is
why the header needs no write set while a scalar (R-Scalar) does.  Switch
`LOFT_NO_WRITE_HOIST`.  Sites: `hoist::IN_PLACE_SET_OPS`,
`hoist::blocks_header_hoist`.
**The text clause of (R-Base)**: a TEXT element read —
`OpGetText(OpGetVector*(P, 4, i), 0)`, a walk's element or an indexed `v[i]` — in a loop
that holds P's header and element base reads the element's record number through the
base (one bounds test, one `u32` load) and slices the text off the vector's store's DATA
SPAN, derived once beside the base (`vector::text_span_of`: the buffer's address and size,
not a pointer to the `Store` struct, which may move when the slot table grows while the
buffer cannot).  A text element is a record in the same store as its vector, so every
element's bytes lie in that span.  Past the end — where a walk's last read lands before
its length test breaks — is the null text without a call, as `get_vector` answers any
non-negative index at or beyond the length (a null vector's length is 0); a NEGATIVE index
counts from the end and takes the unfused path (`text_elem_cold`, outlined: folded in, the
hot half lost its inline and the walk its registers — the exception (R-Cold)'s fold clause
names).
`LOFT_HOIST_VERIFY=1` re-derives header, base and span at every read.  A twin's input
base carries no span, and such a read keeps the store-resolving form.  Switch
`LOFT_NO_TEXT_BASE`; cells `tests/scripts/158-text-base.loft` b1–b8 (falsified by the
past-the-end arm answering the element before it); pins `tests/text_borrow.rs`.  Sites:
`Output::fused_text_read`, the span beside each base in `Output::bind_loop_headers` and
`Output::bind_view_header`, the arm in `LazySplitNextEmitter`, the pre-eval exemption in
`Output::collect_pre_evals_inner`, `vector::text_elem_at`, `vector::text_at`,
`vector::text_span_of`, `Store::text_span`.
**The link clause**: a hoist's ROOT is loop-invariant only while nothing the
root LINKS to is rebound — `g = &e` reads whatever `e` holds now, so a rebind of `e` in
the body repoints every path rooted at `g`, and `g` counts as rebound (`hoist::rebound_vars`
closes the set over links, `hoist::rebinds_root` answers the single-root form for a view's
header, a record's address and a mint group's path).  Measured before it: `g.tags`' header
hoisted across `e = h[i]`, native read `h[0]`'s tags on every pass (15 for 16, every
switch on or off); cells `tests/scripts/158-link-rebind.loft` l1–l7.
**The hidden-buffer allowance** (@PLN157 § V-ad): the allow-list also admits
`OpDatabase`/`OpDatabaseNP` into a null-discharge buffer — the hidden `__ref_p2_N` that
`e = tbl[i]?` mints an ABSENT record element into — whatever the record holds.  The
allocation takes a store of its own from a null slot or clears the buffer's OWN store,
and that store is reachable only through the site's `__ncc_N` temp, which the site
rebinds on every run: no loop-invariant path names anything in it, so no header and no
base can go stale — and the buffer's own re-mint in a loop body counts as a REBIND of
the buffer (`hoist::rebound_vars`), so a path rooted at a pass-2 buffer the body re-mints
takes no holder either: a literal handed to a call (`s = me(Bx { v: [i], n: 7 })`) is
built in such a buffer and re-minted per pass, and a push header hoisted off its vector
field would push into the record the previous pass claimed (measured: `1575-…`'s c3
read `null(oob)` for `1` the moment the allowance took a heap-holding record); the field sets that follow are (R-InPlace) sets and walk on their own,
and the scalar tier reads the allocation as the record's type whole (R-Scalar).  A growth
of the buffer's own store from the body — an append to the absent element's vector
field — is a push through a non-pure path, which the gate declines on its own.  Only the pass-2
discharge buffers qualify: a `__ref_N` work-ref may be a return buffer, and a return
buffer may be a record the caller offered (R-Callee's second half carries that case).
Switch `LOFT_NO_NULL_BUFFER_HOIST`; falsifier `LOFT_HOIST_VERIFY=1`; cells
`tests/scripts/158-heap-discharge-buffer.loft`.  Site: `hoist::null_buffer_alloc`.
**The copy clause**.  A consumer writes the copy-out / mutate /
write-back a Rust or C author writes — `e = ents[i]?; e.energy += e.speed; …; ents[i] = e`.
In loft `e` is a VIEW of `ents[i]` (`(B-View)`), so the last statement copies the element
onto itself, which both backends already make a no-op (`data == to`).  But the whole-record
copy was an unclassified store writer: the loop held no header, `e` no address (*"the
remainder may grow a store"*), and each of the ~20 field accesses per pass resolved the
store.  The copy itself STAYS.  Eliding it
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

**The read clause**.  `OpReadFile(f, OpCreateStack(t), n, tp)` with a
scalar `t` sets `t` and the File record's `#next` and `#pos` (and its handle number on the
first read), each in place; nothing is claimed, grown or freed, so no header, base or record
address can go stale across it.  `OpCreateStack(t)` of a scalar local is on the list with it:
taking the address writes nothing, and whatever writes through it is judged on its own.  The
scalar tier gets no exemption — `body_writes` still answers "untyped" for the read, because
it advances `f#next`, which is a field a loop may hoist: typing it as writing nothing reads
`[2,2,2,2]` for `[2,4,6,8]` in cell r3 (the patch receipt).  A text read builds a text and a
vector or record read fills a value in a store; each stays a writer.  Switch `LOFT_NO_WRITE_HOIST` (the tier); cells
`tests/scripts/158-scalar-file-read.loft`; pin `tests/scalar_file_read.rs`.  Sites:
`hoist::scalar_file_read`, `hoist::scalar_stack_ref`, `hoist::blocks_header_hoist`,
`hoist::in_place_only_writer`.

### A callee is admitted by what its body writes, one call deep

```
  (R-Callee)     a user callee whose store writes are all (R-InPlace) sets — through
                 any address, transitively through callees it admits the same way —
                 or whose only writes land fixed-width scalars in its own hidden
                 return buffer, is admitted under (R-InPlace) as a direct set is.
                 Its body is read with the header hoist's own allowances — a
                 null-discharge buffer's mint, its defaults and the SETS of its
                 fallback (a text field's included: the buffer's store is the
                 site's own, reached by nothing a caller holds), a lazy buffer's mint, a
                 record free, and its DEAD buffers (R-ValueRecord: a tuple answer's
                 buffer is never minted, so its mint, clear and frees are nothing;
                 the fact is per FUNCTION, read off one program-wide table) — since
                 none reallocates a store a caller's header describes.  A CallRef,
                 a Parallel, a Yield and a recursion keep the WRITER verdict.
```

**In words.** @PLN157 § V-l (`in_place_only_writer`) and § V-c
(`retbuf_only_writer`): the P4a argument applied to a whole body.  The arguments
still walk below the call node, so a growing op inside one blocks on its own.
Switches `LOFT_NO_INPLACE_CALLEE_HOIST`, `LOFT_NO_RETBUF_HOIST`.  Sites:
`hoist::in_place_only_writer`, `hoist::retbuf_only_writer`,
`hoist::call_writes_store`.
*The set half of the discharge allowance (`hoist::discharge_buffer_set`, under
`LOFT_NO_NULL_BUFFER_HOIST` with the mint):* a callee whose `??` fallback carries a text
field (`Sequence { …, q_name: "" }`) wrote it with `OpSetText`, the one op outside the
twelve scalar setters, and was a WRITING callee for it — stage's `pack_instances` called
three such and held no header at all.  Guard
`tests/scripts/a-discharge-buffers-text-set-keeps-the-callers-headers.loft` (c3 is the write
the allowance must not reach: a text set into an element of the walked vector, which grows
that store); pin `tests/discharge_set.rs`.  `pack_instances` 920 → 309 µs per op (24.3× →
8.4× of Rust) here, hash unchanged.

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
                 growth) or grows only stores proven apart from r's (R-Base's
                 growth clause: r's store is the end of its dep chain, `pr` for
                 `e = pr.items[i]?`), frees no record before a later use of r, never
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
                 not plain records and are never bound.  THE PARAMETER CLAUSE: a
                 plain-record PARAMETER is a view the caller fixed before the first
                 statement, so the whole body is its remainder and takes the same
                 verdict; a generator's parameters, which outlive a resumption, never
                 do.  THE BASE CLAUSE: where
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
                 window meets the conditions above in the remainder's place; a copy
                 INTO e from a value local (R-ValueLocal) is a fusable write of e,
                 since it lowers to one typed setter per field, and grows nothing.
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

**In words.** The drawing library's crossing loop (`pg_cur = pg_table[i]?`,
then six field reads of `pg_cur` and three more inside `edge_x` per edge per row): each
read resolved the store — `stores.store(&db).get_int(db.rec, db.pos + off)` — where the
Rust reference's `let cur = table[i]` reads registers.  (R-View) hoists the HEADER of a
vector view; this is the same promise for a RECORD view, whose derivation is an address
rather than a header.  Sound for the reason (R-Base) is: nothing in the remainder can move
the record's bytes, and a freed record's read is excluded by order (the releases a block
ends with follow the last use).  Switch
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

*The base clause.*  A `for e in v` loop built a `DbRef` for its
element and then resolved the store a second time to turn that `DbRef` back into an address
— per element, in a loop that already held the address of element 0.  An explicit index binding (`e = v[i]?`)
is a JOIN — the element, or a discharge buffer in another store — and keeps `rec_ptr`.
Inside the in-range test the address is NOT NULL, and the emitted code says so
(`vector::held_elem_ptr`): a held base is null only for an absent vector, whose length is 0,
so `index < len` proves it real.  Every field read through the address tests it for the null
record; with the fact stated, those tests and the absent value they guard fold away for the
loop body — and with them the cost the null-aware float compare showed (`mesh_aabb` 28.3 →
17.0 µs, `entity_tick` −19 %).  Under `LOFT_HOIST_VERIFY=1` the fact is checked, not assumed:
an address built without its range test panics at the first empty or absent vector (cell w16).
Switch `LOFT_NO_BASE_RECPTR`; falsifier `LOFT_HOIST_VERIFY=1` (`rec_get`/`rec_set` compare
the address with a fresh `rec_ptr` at every use — a sabotaged `index + 1` panics there, and
answers `0 0 162 135 243` for `0 -7 155 135 250` without it).  Cells
`tests/scripts/158-iteration-base.loft`, pins `tests/iteration_base.rs`.  Sites:
`hoist::iteration_head`, `Output::held_iteration_base`, `Output::bind_record_ptr`, the
`BIND_RECPTR_BASE` rule in `scripts/emission_audit.py`.

*The path clause.*  Only a DIRECT field of a view counted as a
fusable access, so a loop over records of records — `for v in m.verts { … v.pos.x … }` —
bound no address at all, and each of its reads rebuilt a `DbRef` with two offset additions
and resolved the store (`mesh_aabb`: twelve per vertex, 9.2× the Rust reference).  **Emitter-local by design:** the fold
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

*The mint clause.*  An address is promised for the whole
remainder of a block, and a minted element never got one: its remainder holds the NEXT
append, which grows a store (`_elm_N`: *"the remainder may grow a store"*).  So every field
of an appended record resolved the store again — a store lookup, a record-validity read and
two bounds checks per field, 143 instructions for a three-field record against the Rust
reference's 18, even through a push header that had just produced the slot.  The element
needs the address only while it is filled.  The window is judged
by the verdict a remainder gets (`hoist::view_extent_verdict`, one definition for both
extents): a text set, a nested mint, a builder that appends and a delivery copy each
decline it; a field value that names the container runs BEFORE the mint (loft#1548) and so
leaves the window clean; an element handed to its builder as the return buffer has no write
left for the caller to serve.  Any mint form qualifies — a keyed collection's claimed
record is one `DbRef` too, and its finish files it by the key just written.  The shared
verdict also stopped reading an in-place set of a kind the address does not serve
(`OpSetBoolean`, the narrow integers) as a REBIND of the view: it is a fixed-width write to
the same bytes, and counting it had declined every record with such a field.  Switch
`LOFT_NO_MINT_WINDOW`.  Cells `tests/scripts/158-mint-window.loft`, pins
`tests/mint_window.rs`.  Sites: `hoist::mint_window`, `hoist::view_extent_verdict`,
`Output::bind_record_ptr`, `Output::close_ptr_windows_before`.

*The order clause.*  The free-before-use question was asked per
top-level STATEMENT of the extent, and a lookup loop's whole body is one statement — so
`for c in m.chunks { if c.cx == cx && … { return c.hexes[i]?.h } }`, where the `return`
releases the `?` discharge buffer after the last read of `c`, read as *"frees and uses in
one statement"* and `c` held no address (*"the remainder frees a record before a use of the
view"*).  It is the shape of every find-and-answer helper.  Asked in execution order it is
no free before a use at all.  A `break` or `continue` keeps running this function, so a block holding
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

*The enum clause.*  `hoist::plain_record_type` answers `None` for
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
head is seen through the `OpGetField(element, 0, _)` a struct-enum element is bound behind.  The tag's answer
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
                 a literal) or a PURE VECTOR PATH (constant field projections over a
                 variable) the pre-evaluation holds no binding for, and no text
                 operand or result, is emitted as that op with the arguments in the
                 operand positions.  A USER function's call is observable — the live
                 tier may flip it to the interpreter, and its frame is on the shadow
                 call stack — and stays a call.
```

**In words.** @PLN157 § V-o.  The wrapper's compiled Rust body IS the op's template,
so for a frameless, unflippable stdlib method the op is the call; `len(d)` after a
view binding then reads the header.  Leaf arguments, because the op's operands are
emitted from a fresh list the pre-evaluation map (keyed on node addresses) cannot see —
and a pure vector path too: side-effect free and
never bound by the pre-evaluation, its clone emits what the original would, and as the
op's operand it is what a held header serves.  Switch `LOFT_NO_WRAPPER_INLINE`; pin `tests/wrapper_op.rs`.
Sites: `hoist::one_op_wrapper`, `Output::wrapper_op`.

### A callee's invariant inputs cross the call

```
  (R-Inputs)     a callee admitted by (R-Callee), of a parameter p it never
                 rebinds, has INVARIANT INPUTS: for a plain-struct p, the scalar
                 fields p.f its own write set does not reach and the vector paths
                 p.g it views (R-View) or indexes (R-Header); for a vector p, its
                 own header when it indexes p (g empty).  A caller loop that holds,
                 for the argument a it passes for p — a leaf variable c, or a path of
                 INLINE sub-records over c, for a scalar input; a pure path (R-Header)
                 for a header input — the value of (c, f) under (R-Scalar), keyed on
                 the root and the field's SUMMED offset for a path and stale after a
                 write at the root's type and that offset or at any sub-record's type
                 and the offset within it, and the header of a.g under (R-Header) —
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
the callee's own write set proves it does not write them either.  Switch
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
**The address half** ((R-RecPtr)): a scalar input the caller holds no VALUE
for — the record changes per iteration — but whose record ADDRESS the caller's block holds
is read through that address at the call (`Output::rec_ptr_read`), so `edge_x(pg_cur, y)`
takes its twin with three loads for arguments where the plain call read three store
resolutions inside; and handing `r` to such a twin counts as a use of `r`'s address for
(R-RecPtr)'s admission.
**The base half** ((R-Base)'s twin clause): a header input alone left the
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
and `PreAllocEmitter`, `Stores::push_hoisted`.  The fusable kinds are `i64`, `f32`, `f64`, the
`i32` (`OpPushInt4`, the integer null kept as `i32::MIN`), the CHARACTER (`OpPushCharacter`,
the code point as `u32`) and the BYTE (`vector<u8>` / `vector<i8>`, `OpPushByte`), whose element is the encoded byte
`Store::byte_raw(min, val)` — the one encoding `OpSetByte` writes, so the header's raw store,
its growth step and the unfused path agree; `hoist::push_operands` is where every site reads a
push's value, so none can take the unencoded one.  A byte loop's one-value slice fill is not
admitted (it would write the value unencoded); its reserve and window are.  The BOOLEAN and
ENUM pushes (`OpPushBoolean`, `OpPushEnum`) are the byte kind at bias 0 — their templates are
`append_byte_min(r, 0, v)` — and `Store::byte_raw(0, v)` is `v` for every value a native `u8`
holds, the enum null 255 included, so their element is the value itself, spelled `as u8`
because a boolean's native value is a `bool` (`hoist::push_value_cast`); being unbiased, their
one-value slice fill IS admitted, and a `true`, `false` or enum literal is a simple invariant
for it.  Cells: `tests/scripts/a-boolean-and-enum-push-hold-a-header.loft`.
Applied by: native generator.  Its clause on the parser's `OpPreAllocVector` is one fact with
`(R-FirstClaim)`, which the interpreter's generator applies where no header is held.

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
*The rebound clause:* the drawing library's `fronds` declares its point and width
vectors per pass of the side loop, and every rebound mover declined the WHOLE loop — so
neither the side loop nor the `for i` around it hoisted anything (the `for i` was declined a
second way, by its per-pass `fd_sides` loop buffer's `OpDatabase`).  A rebind from the
emitter's own buffer is the one rebind that cannot alias a held vector, so it costs the mover
its holder and nothing else.  Falsified on the alias the clause refuses: a mover rebound to a
VIEW of a held vector (`w = &big; w += [...]` beside `len(big)`) must still decline the loop,
or the hoisted length answers the pre-push count (`tests/scripts/157-group-push.loft` g7).
Switch `LOFT_NO_REBOUND_MOVER`; trace `LOFT_TRACE_HOIST_DECLINE=1` names the first statement
that declines a loop.  *The own-buffer mint:* the guarded mint of the frame's own
hidden return buffer (`OpDatabase(__retbuf)`, which runs only where the caller offered no
record and then claims a fresh store) moves nothing a header describes, so a lookup loop
that returns `v[i]?` keeps its holders (`map_get_hex`, the `own_retbuf_mint` arm).  Sites: `hoist::HoistOwned`, `hoist::rebinds_alias_free`, the
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
*The field clause (@PLN158 R1):* every builder a consumer writes appends to a
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
`Stores::push_record_finish` returns on an absent owner, as `vector_finish` does.  A field-form group
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
`Stores::push_record_hoisted`, `Stores::push_record_finish`.
*The heap clause:* `fronds`' element is `Frond { fpts, fwid }`, two vector
handles, so every one of its 1 272 mints per call paid the dispatch and a `set_default_value`
walk whose whole effect was two zero words.  Falsified by sabotage: `push_record_hoisted_zero`
made to skip its `zero_range` turns the reused-buffer cell red on `--native` under
`LOFT_POISON_CLAIM=1` — the slot's poisoned handle is read as the element's `xs` vector, a
store guard panic naming the poisoned record — while a plain run can stay green on stale
bytes that happen to read as an empty handle, which is why that falsifier and not the plain
run guards the clause.  Switch
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

**In words.** @PLN157.  `(R-PushRec)` gave the header to a mint INSIDE a loop
that holds one; every other append — a literal group at function level, a group inside a
loop that declined, a group on a per-pass local — paid the `record_new` dispatch, the prefill
and the `record_finish` dispatch per element: the drawing library's `fronds` built its 1 296
points per call that way (`fd_pts += [pt(…), pt(…)]`, `fd_pts` declared per pass), 15 µs of a
102 µs call.  The group's own reservation already guarantees the slots, so the header derived
after it is exact for the whole group, and the group is straight-line code the parser emits —
the same producer `(R-Mint)` trusts.
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
                 a parameter or a record/element read is never ranged BY SHAPE
                 (C80).  THE TYPE CLAUSE: a value whose STATIC TYPE is a fact carries
                 that type's range as a leaf — a non-nullable `Integer[lo, hi]` whose
                 range fills its width (`u8`, `i8`, `u16`, `i16`, every user-written
                 `limit(lo, hi)`), whether it is a parameter, a local the compiler
                 typed so, or a block the compiler typed so (the join `v[i] ?? 0`
                 over a `vector<u8>` is `integer(0, 255)`).  It is a fact because,
                 since loft#1593, every store, call and literal into such a slot is
                 refused unless provably in range, a `?? d` fallback lands in
                 range, and the one run-time arrival — the slot's own arithmetic
                 stepping past the range — answers the type's default, in range.
                 Three specs are NOT facts and stay unranged: the two full-integer
                 templates (the plain `integer` reports a range it does not hold),
                 and a spec that keeps a code back for null which an overflow WRITES
                 — a signed narrow alias whose range leaves a spare bottom code
                 (`limit(-100, 100) size(1)`; `i32` is the same shape and is the
                 template) — because the slot then holds the sentinel and a plain
                 add over it answers a number for null.  A nullable `τ?` and a `&`
                 link are not facts either.  The fallback is the checked template,
                 always right.
```

**The type clause, in words.** The C129/C120 discussion with the `cbor` library as
the consumer (`bench/portal/analysis/vector-build.md` § The cbor library).  The proof ranged by
SHAPE and never by TYPE, so the cbor decoder's `(bytes[p] ?? 0) * 256 + (bytes[p + 1] ?? 0)`
was emitted checked although the compiler itself typed both joins `integer(0, 255)`, and a
library that declared `major: integer limit(0, 7)` — C120's own stated route, *"a library opts
in by declaring the bounded element types it already knows"* — got nothing for it.  Probed on
four shapes: every op checked.  The clause could not be built until the declaration WAS a fact
(loft#1593: a user `limit` accepted an out-of-range store and read a wrong in-range number —
a proof trusting it would have run plain on a value the type says cannot exist), so that fix
came first, and this clause reads `Parser::is_narrowing_int`'s complement: what the narrowing
rules make true, the range proof may assume.  One helper (`range::type_range`) at the two
homes a fact is read from — `range_vars` seeds every variable of such a type before its
fixpoint, and `range()`'s block leaf reads `Block::result` before the tail.  Switch `LOFT_NO_RANGE_ARITH` (one rule); falsifier `LOFT_HOIST_VERIFY=1`.  A boxed capture is read through its box and stays
checked (c7, measured, a refinement not a defect); a bare-variable `??` over a nullable
parameter lowers to an `if` whose then arm is the parameter and stays checked (c5).  Cells
`tests/scripts/157-range-arith.loft` c1–c8, pins `tests/range_arith.rs`.

**In words.**  C120 declined the CLOSURE of the non-sentinel proof over
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
makes it a byte.  Operators with record-scalar or parameter operands (`j * lw + i`, `x0 + i`, `y0 + j`, the accessors' `by * width + bx`) have no static proof, and that is what `(R-GuardedChain)` is for.  Switch `LOFT_NO_RANGE_ARITH`; falsifier `LOFT_HOIST_VERIFY=1`
(`ops::range_verify` compares the plain answer with the checked one at every admitted
operator).  Cells `tests/scripts/157-range-arith.loft` a1–a9, pins `tests/range_arith.rs`.
Applied by: native generator; using the same facts in the IR to REMOVE a no-op mask or a
dead `??` is `(R-MaskRange)` (proposed).

**The counted-counter clause** (loft#1558).  The parser emits `index = <start>` as the statement BEFORE the loop, so the seed is looked for in the whole FUNCTION, which is what makes
it sound rather than merely wider — `v_seed` counts every non-step `Set` to that counter
anywhere, so a counter written from a second place declines on `seeds != 1` instead of taking
the first value it meets.  Cells b1–b6, of which b3/b4 are the BOUNDARY PAIR: they differ only in the range's
end (`0..=10` against `0..=9`, `i * 1e18` crossing i64::MAX between them), so the multiply's
verdict is a claim about the counter's top bound and nothing else — a top taken one too LOW
is the only unsound direction, and it turns b3 red and makes `LOFT_HOIST_VERIFY=1` panic
naming the operator.
**The accumulator clause** (@PLN158 round 4): a second self-stepping shape
read off a loop, after the counters.  A local seeded ONCE by a ranged value (`n = 0`) and
stepped only by LITERALS (`n += 1`, `n -= 2`), every step straight-line after the seed in the
seed's own block or under loops there whose TRIPS are bounded, is ranged.  A character walk
`(R-CharWalk)` over a text the body never writes makes at most `size(T)` trips (a `u32`
word); a counted loop makes at most `hi - lo + 1`, over the range `(R-Counter)`'s clause gave
its stepped counter.  A step under nested bounded loops moves `n` by `|c|` times the PRODUCT
of their bounds, so per run of the block `n` moves by at most the sum of those, and the seed
plus that bound is `n`'s range whenever it fits the type — the checked step cannot fault,
so the processor's operator answers what the template would.  An inner loop whose end is
an outer counter (`for _ in 0..i`, the insertion sort's `j -= 1`) is bounded once that
counter is ranged, so the clause is re-read where the fixpoint re-seeds counters.  The
seed's block may itself sit in a loop (each pass re-seeds); a step under a loop with no
bound (a `while`, a counted loop whose seed or end is unranged, a walk over a text the body
appends to), a step by a non-literal, a second seed, a write to `n` anywhere else, a
parameter seed or `n` handed out by reference declines.  `LOFT_HOIST_VERIFY=1` compares every
plain step with its template.  Cells `tests/scripts/158-walk-accumulator.loft` a1–a18 (a16/a17
the boundary pair across i64::MAX, a18 a step that really overflows), pins
`tests/walk_accumulator.rs`.  Site: `range::seed_accumulators`.
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

**In words.**  `(R-BoundedNest)`'s method — a fact ESTABLISHED before the
arithmetic runs, in the same magnitude-bound arithmetic the rule states — for the index
chains of any counted loop, where the static `(R-Range)` proof stops: `composite_layer`'s
`j * lw + i`, `x0 + i`, `y0 + j` read record scalars (`lay.lw`, `lay.x0`) that a program
cannot bound at generation time and a guard can bound at the loop's entry in six checked
operations.  A leaf may be
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
`3 409 0`).  Composes with `(R-PushRec)`: a no-heap element's slot comes from the
push header and the move lands in it (the c16 cell).  Switch `LOFT_NO_MOVE_APPEND`; the value
cells run under `LOFT_POISON=1` and both leak checks.  Sites: `hoist::move_appends`,
`hoist::pair_for_block`, `Output::move_pair_for_block`, `Output::active_move_pair`, the
placement in `Output::output_block`, the move arm in `OpCopyRecordEmitter`, the record
free in `OpFreeRefEmitter`, the null decl in `Output::emit_null_dbref`,
`Stores::place_record_in`, `Stores::move_record_shallow`, `Stores::free_record_in`.

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
text is the edge that has to agree three ways: the iterator, `split` and its switch-off form
all answer it no pieces, as for the empty text — `(L-Null-Text)` gives an absent text no
characters, so `split`'s trailing-piece test `len > 0` does not fire.  The
cells that pin it pass a real null (s2, s21), not `nothing ?? ""`, which is an empty text
and cannot tell the two apart.  The source
needs no gate because the copy is always sound; the borrow is the one case where the
copy is provably unnecessary.  Two spellings of the `For` block reach it —
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

### A split bound to a name is a table of its pieces

```
  (R-SplitTable) `V = T.split(c)` — the standard library's `split`, `c` a character
                 CONSTANT other than the null character — where V is a plain LOCAL
                 `vector<text>`, that bind is its only binding, and every other
                 mention of V is a READ the table can answer: `len(V)`, the element
                 read `V[i]` (bare, under `?`, under `?? d`, bound to a `text?`), or
                 the source of the walk `for p in V` (the hidden vector the parser
                 binds from V, itself read by nothing but its element read and its
                 length test), all of them inside the block that binds V and after
                 the bind — the vector is never built: at the bind ONE pass over T
                 records its pieces as a TABLE of slices, the pieces (R-LazySplit)
                 names in the order it names them; `len(V)` is the table's length,
                 `V[i]` is the slice at `i` (a negative `i` from the end, a null or
                 out-of-range `i` the null text — what the element read answers past
                 the end), and the walk iterates the table.  T is evaluated ONCE,
                 where the call stood: a plain text PARAMETER the function never
                 writes is borrowed for the block; every other T is copied there.
                 Every other mention — V returned, appended to, written, linked,
                 captured, tupled, handed to a call, rebound, a second binding, a
                 mention outside the binding block, a generator — DECLINES and
                 keeps the vector.  The call's hidden buffer, when it serves that
                 call alone, is never minted.
```

**In words.**  `parts = src.split('\n'); n = len(parts); for i in 0..len(parts) { … parts[i]? … }`
is how a library reads a line-oriented text when it needs the count or the i-th line, and it
is the ONE shape `(R-LazySplit)` declines — a vector bound to a name first.  Every such bind
in the repo and the library checkouts is a local that never leaves its function (14 binds,
walked, indexed or `len`'d: `round-3.md` § `split`), so `(R-Escape)` licenses the
representation and the API stays what it is.  The table is what the Rust reference does
(`split(c).collect::<Vec<&str>>()`), one pass over T with no record and no text copy per
piece; the readers are the three the analysis admits and the emitter answers, so a shape
the rule has not met declines rather than compiles to a read of a vector nobody filled.
The walk through the name is the parser's `_vector_N = V` followed by the two readers of
`(R-LazySplit)`'s loop, and is read as an ALIAS of the table — so `p` in `for p in parts`
borrows its slice under `(R-TextBorrow)` with no store condition, as a lazy split's piece
does; and because the table is random access, a `rev` and a comprehension over V are walks
too, where the lazy split's forward iterator has to decline them.  The element read comes
in the parser's two twins — the nullable read under `?`, `??` and a null-tested bind, and
the RAISING read of a bare `v[i]` — and the table answers each as its op does: the null
text and the out-of-bounds note for the first, the recoverable `IndexOutOfBounds` /
`NegativeIndex` fault for the second (`LOFT_DEV_SOFT_HALT` halts both forms alike).  The binding-block condition is the Rust scope of the `let` the table becomes; a
mention outside it would not compile.  `len` reaches the emitter through `(R-Wrapper)`,
so the switch that keeps the wrapper a call (`LOFT_NO_WRAPPER_INLINE`) keeps every table a
vector too.
Switch `LOFT_NO_SPLIT_TABLE`; trace `LOFT_TRACE_SPLIT_TABLE` (each table admitted, each
declined with its reason, and whether the buffer is minted).  The interpreter keeps the
vector and is the oracle; `LOFT_NO_SPLIT_TABLE=1` is the same-build oracle on native.
Cells `tests/scripts/158-split-table.loft`; pins `tests/split_table.rs`.  Sites:
`hoist::split_tables`, `Output::split_table_of`, the bind and the alias bind in
`Output::output_set`, the element read in `LazySplitNextEmitter`, the length in
`HoistedLengthEmitter`, the pre-eval exemption in `Output::collect_pre_evals_inner`, the
null decl in `Output::emit_null_dbref`, the header exclusion in
`Output::bind_loop_headers`, `codegen_runtime::split_table_get`.

### A walk of texts borrows each element

```
  (R-TextBorrow) in `for p in W { … }` where W is a `vector<text>` — the plain forward
                 walk the parser gives a vector, `p` bound to the text of the element
                 at `p#index` — or a split the loop takes lazily (R-LazySplit), and
                 the body reads `p` only as a text VALUE (an operand of an op or a
                 call at a `text` position; the whole source of a bind or a tuple
                 write into ANOTHER slot; the walk's own release), never writes,
                 links (`&p`) or captures it, and — for a vector — writes no store
                 (R-Header's condition), `p` is a BORROW of the element for the
                 iteration: no copy is taken, and every read of `p` reads the text as
                 W holds it, which is what the copy held.  A text VALUE op — one
                 whose operands are scalars and texts and whose result is one or
                 nothing, or one that writes a text VARIABLE — writes no store: a
                 text value on native is a `&str` or a `String`, never a store.
                 Every other walk — a body that writes a store or `p`, a generator,
                 a body that runs arms in parallel — keeps the copy.
                 DISCHARGE CLAUSE: the temp of a `?` / `?? d` over a split TABLE's
                 element read (R-SplitTable) — `parts[i]?`, `parts[i] ?? d` — whose
                 every mention is a text-value read, its own null test and its
                 own value arm included, is likewise a BORROW of the slice, and
                 the discharge's value is that slice: the slice points into the
                 table's source, which outlives the temp's block and the
                 statement that consumes the value.  A rebind, a link, a capture
                 or a return declines it, as for `p`.
                 STORE-READ CLAUSE: a text read `OpGetText(R, f)` where R is a
                 PARAMETER the function never rebinds, reached through field and
                 element reads (`words[i]?`, `b.names[i] ?? d`), is BORROWED by the
                 `?` / `??` temp bound to it and by every text LOCAL whose each
                 binding is a text literal, such a read, a text parameter or
                 another variable this clause admits — each variable's every
                 other mention a text-value read (a trailing key of a keyed
                 lookup is one) — provided that over the innermost statement
                 block holding all of the variable's mentions, no step can grow,
                 free or rewrite a store the frame did not MINT: every native
                 store writer names only frame-minted stores (a store operand, or
                 a slot operand spelled as a variable, each rooted at a local
                 whose every binding is null, `OpDatabase`, or a field, element
                 or record read through another such local) or is a scalar set in
                 place; every user callee writes no store, or writes only in
                 place; and no fn-ref call, parallel arm or yield runs.  A writer
                 that names no store, or takes an operand of any other kind,
                 declines.
```

**In words.**  `for p in words` bound `p` as `{ … get_str(…) }.to_string()` — an
allocation and a copy per element — and the loop hoisted NOTHING, because its element read
`OpGetText` and every text op in its body (`size(p)`, `result += p`, `p == "x"`) read as
store writers to `(R-Header)`'s condition.  A text local's Rust slot is decided at every site
that spells it by ONE predicate (`Output::text_borrowed`: a text parameter, or a local this
rule admits), so the borrowed loop variable reads bare where an owned local reads `&var`,
converts with `.to_string()` where an owned local clones, and needs no site of its own.
The vector's condition is the one `(R-Header)` already states, sharpened by the text-value
clause: an op on text values touches no store, so `OpAppendText(result, p)` and
`OpEqText(p, "x")` are no longer writers, and the walk's header, length and base follow
from the rules that already hold them.  A lazy split's piece borrows the iterator's source
— a borrowed parameter or a loop-long copy — and needs no store condition at all.  The
escape conditions are what keeps the borrow a text VALUE: a `&p` link, a rebind or a
text-building op with `p` as its destination would need a `String` slot, and a capture or a
generator would carry it across the iteration.  The interpreter takes the copy the IR
spells and is the oracle.  Switch `LOFT_NO_TEXT_BORROW`; trace `LOFT_TRACE_TEXT_BORROW`
(each walk admitted and each declined with its reason); falsifier `LOFT_HOIST_VERIFY=1`,
whose checking form re-reads the element after the body and panics when the borrow no
longer names it.  The discharge clause is what takes the `split` row from 6.5 to 3.4 µs
(the twin 2.7): `parts[i]?` was one copy into the temp and a second at the block's tail,
where `_ret.to_string()` materialised a value that might borrow a block-local — the
temp is now the block-local it might have borrowed, and points outside the block.  It
has no runtime check to arm: the pins say what is emitted, and the cells of
`158-split-table.loft` that discharge (s1, s4, s6, s10, s13, s14, s16) are the values.
Sites: `hoist::borrowed_text_walks`, `hoist::text_walk_head`,
`hoist::native_op_is_store_free` (the text-value clause), `hoist::borrowed_discharge_temps`
and the discharge arm of `hoist::text_escapes_with`, `Output::text_borrowed`, the bind and
the discharge bind in `Output::output_set`, the `#ncc` tail in `Output::output_block`.

**The store-read clause.**  `w = words[i]?` over a `const vector<text>` parameter bound `w`
through two copies (the temp's `.to_string()` and the bind's `.clone()`) and a third where
it became a hash key, while the Rust twin borrows `&str` throughout; `word_count` is that
loop 300 000 times.  A text read out of a store is a slice of that store's buffer, valid
until the buffer grows, is freed, or the text's own block is released — so the question is
which stores the borrow's scope can disturb.  The answer is read off the operands, because
of `(H-Alloc)`: a store the frame MINTS is distinct from every store live when it is
minted, so it is never a parameter's.  A stored reference names a record of its own store
(`layout.md` § DbRef — the one 12-byte exception is a fn-ref's closure half, reachable only
through the `CallRef` this clause refuses), so a place read through a frame-minted local is
in a frame-minted store.  A scalar set in place moves no block, so it may land anywhere.
The scope is the innermost statement block holding every mention of the variable: a value a
statement computes does not outlive its block unless it is bound, and a bound borrow is a
variable this clause judges on its own.  Cells
`tests/scripts/158-text-borrow-store.loft` (a1–a7 admitted, d1–d7 declined, d1 the one that
reads differently without the store condition); pins `tests/text_borrow.rs`.  Sites:
`hoist::borrowed_store_texts`, `hoist::param_rooted`, `hoist::frame_minted_locals`,
`hoist::mention_scope`, `hoist::foreign_store_writer`, the key arm of
`hoist::text_escapes_with`, the local bind in `Output::output_set`.

### A character walk steps an ASCII byte in one move

```
  (R-CharWalk)   in `for c in T { … }` where T is a text VARIABLE, the walk's step — bind
                 `c` to the character at `c#next` and advance `c#next` by its width —
                 takes an ASCII byte other than NUL in ONE move: `c` is that byte and
                 `c#next` steps by one, which is exactly what the character read, the
                 width and the checked add answer for such a byte; every other byte (a
                 multi-byte lead, a continuation, a NUL — whose read notes a fault — or
                 an index at or past the end) takes the step as written.  And where no
                 statement of the loop writes T, the walk's null test — is T the null
                 text — is asked ONCE before the loop instead of on every iteration,
                 since nothing in the loop can change its answer; the size test stays
                 per iteration, because a body may grow T.  A generator declines
                 whole: its loop variables live on the state machine.
```

**In words.**  `for c in src` paid, per character, `text_character` (a bounds test, a
byte load, its own ASCII arm), a fault note, a CALL for the width the byte already told it,
a checked add, a `next <= index` guard, a content compare of the whole text against the
null sentinel, and a length test — 2.7 ns a character against the Rust twin's 0.6, with the
profile FLAT (`round-3.md` § W1: inlining the width or making the body's compares plain
moved nothing on its own; LLVM folded those already).  The fast arm is a re-spelling with
no condition: for a byte in `1..=0x7F` at an in-range index the step as written answers
that byte, width 1 and no fault, so the arm answers the same and the slow arm is the step
verbatim.  NUL is excluded on purpose — `text_character` answers it as the null character
and the walk's fault note fires, which the fast arm would silence.  The null test is a
CONTENT compare (`T != "\0"`) LLVM does not hoist past the step's calls; asked once it
costs one compare per walk, and the condition — T not written in the loop — is
`(R-LazySplit)`'s borrow condition over the loop's statements.  Switch `LOFT_NO_CHAR_WALK`; falsifier `LOFT_HOIST_VERIFY=1`,
whose checking form runs the step as written beside the fast arm and panics when the two
disagree, and keeps the per-iteration null test as an assertion.  Sites:
`hoist::char_walks`, `hoist::text_written` (shared with the lazy split's borrow), the bind
in `Output::output_set`, the guard in the `Loop` arm of `Output::output_code_inner`.

### A format appended to a text is written into it

```
  (R-FormatAppend) `x += "…{e₁}…{eₙ}…"` where x is a text VARIABLE (a local, a `&text`
                 parameter, a promoted return buffer) — lowered as a work text set to
                 the literal prefix, every literal and every hole appended to it, and
                 the work text appended to x — is the same appends made to x directly,
                 in order, PROVIDED no hole reads x (`x += "{x}!"` reads the text as it
                 stood before the statement, which only the work text gives it) and
                 every part is a write of the format family on the work text (an
                 append of a text or a character, a formatted number, text or record).
                 A `for` hole, a part naming the work text anywhere but as its
                 target, and a FIELD or element destination keep the work text.  A
                 `&text` destination takes each write's STACK twin, the spelling
                 every write on such a target carries (the store-text instance of
                 the function reads it to write a linked field or element).
```

**In words.**  The work text existed to make the format a VALUE; appended and never read
again, it is a `String` cleared, grown and copied per statement — 13 ns a character in an
escape loop on native, against 3.4 for the push itself.  The value is unchanged by
construction (the same parts, in the same order, onto the same text) except where a part
reads the destination, which is the one decline the rule needs.

**BUILT** (`Parser::format_append_in_place`, `LOFT_NO_FORMAT_APPEND`; the
guard `tests/scripts/a-format-appended-to-a-text-is-written-into-it.loft` and the pin
`tests/format_append.rs`).
The stdlib's own `char_slice` takes it too.

### A text copied byte by byte is one append

```
  (R-ByteCopy)   `for i in lo..hi { buf += [t.byte_at(i) as u8] }` — a counted range over
                 `i` whose body is the one push of `t`'s byte at `i` into a byte vector
                 `buf` stored raw, the byte masked with 255 or not, `t` a text variable
                 and the bounds pure (a literal, a variable, `size(t)`) — is ONE append
                 of the bytes `[lo, hi)` of `t` behind the guard
                 `0 <= lo && lo <= hi && hi <= size(t)`, the loop as written running for
                 every range the guard refuses (an index past the text reads its null,
                 and the loop pushes what it always pushed).  A second statement in the
                 body, a text that is a field or an element, a destination whose element
                 is not a raw byte, and a bound that is any other call keep the loop.
                 THE VECTOR CLAUSE: `for i in lo..hi { buf += [v[off + i] ?? d] }` (or
                 `v[i]`, `i + off`) over a byte vector VARIABLE `v` whose element is
                 `buf`'s, `off` a literal or a variable, `d` a literal, `buf` EXCLUSIVE
                 as `(R-VecCopy)` defines it (a record `(R-Place)` claimed for it counts
                 as minted here) — is ONE slice append of `v[off + lo .. off + hi]`
                 behind `0 <= off && 0 <= lo && lo <= hi && hi <= len(v) - off`.  A raw
                 byte read in range is never null, so `?? d` never fires inside the
                 guard; outside it the loop runs as written (a negative index counts
                 from the end, one past the end reads null and pushes `d`).  An element
                 of any other kind — `integer`, `float`, a narrow bias — can hold the
                 null its `?? d` replaces, and keeps the loop.
```

**In words.**  A binary encoder copies a text payload into its byte buffer one byte at a
time because that is the only spelling the language offers; the per-byte read, mask and
push cost 4.3 ns a byte against a block copy.  The append grows the vector the way the push
grows it (`vector::append_bytes`, one home with `vector_append`'s doubling), because a
reservation to the exact length reallocated on every chunk.

The vector clause is the decoder's half of the same copy: `bs += [bytes[pos + k] ?? 0]` is
how a reader takes a byte string out of its frame, and the guard makes the slice exactly the
bytes the loop pushes.

**BUILT** (`src/byte_copy.rs` after the compaction pass, `LOFT_NO_BYTE_COPY`;
guards `tests/scripts/a-byte-wise-text-copy-is-one-append.loft` and
`tests/scripts/a-byte-range-of-a-vector-copied-one-at-a-time-is-one-append.loft`, pin
`tests/byte_copy.rs`).

### A run of one constant is one repeat fill

```
  (R-RepeatRun)  four or more consecutive pushes of the SAME literal into the SAME target —
                 `[c, c, c, c, …]` spelled out, the target a local or a field path over one
                 with literal operands, the element at least four bytes (`float`,
                 `integer`, `single`, `i32`), the literal compared by its bits — are the
                 repeat literal `[c; n]`: the reservation `OpPreAllocVector(target, n,
                 size)` (which claims n elements for an ABSENT vector and leaves one that
                 exists alone), one push of the template, and `OpAppendCopy(target, n, tp)`,
                 which copies the vector's LAST element — the template — `n - 1` more
                 times.  Elements appended before the run stay where they were.  A shorter
                 run, a narrower element, a computed value or target keep the pushes.
```

**In words.**  A matrix or a buffer zeroed by its literal is spelled with every element, and
each push paid a capacity test and a length bump, and a fresh vector grew partway.  The
repeat literal already fills by doubling block copies; this gives the spelled-out form the
same lowering.  **BUILT** (`src/repeat_run.rs`, scope pass, `LOFT_NO_REPEAT_RUN`; guard
`tests/scripts/a-run-of-one-constant-is-one-repeat-fill.loft`, pin `tests/repeat_run.rs`).

### A byte run read once as text is never built

```
  (R-TextRun)    `t: vector<u8> = []; for i in lo..hi { t += [v[off + i] ?? d] }; …;
                 x = text_from_bytes(t)` — a byte vector declared empty immediately
                 before a copy `(R-ByteCopy)`'s vector clause admits, and read once as
                 text as the direct value of an assignment — is never built on the
                 guard `g` of that copy: the declaration and the copy run only under
                 `!g`, and the read is `if g { x = text_from_byte_range(v, off + lo,
                 off + hi) } else { x = text_from_bytes(t) }`.  `g` is evaluated twice,
                 so every statement between the copy and the read may only read, mint
                 the frame's hidden return buffer or write a scalar field of it, and
                 assign no variable `g` reads.  A second mention of `t`, a mention of its
                 wrapper other than its declaration and its frees, a vector that is not
                 empty before the copy, a read that is not an assignment's value, and a
                 read under a loop keep the vector.
```

**In words.**  A decoder takes each text out of its frame by copying the bytes into a vector
and converting it, so every text cost two objects — the vector's wrapper and its element
record — claimed, filled and released only to be read once.  The range read decodes the same
bytes with the same conversion (`String::from_utf8`, invalid bytes answering `""`), so on the
guard's path the text is identical and neither object exists; the frees of the wrapper the
path never claimed pass a null by.  The choice sits above the assignment, not inside it,
because a text native assigned to its destination is lowered destination-passing and must be
the assignment's direct value.

**BUILT** (`src/text_run.rs`, called by `src/byte_copy.rs` on the copies it just guarded;
`LOFT_NO_BYTE_COPY` turns both off; guard
`tests/scripts/a-byte-run-read-once-as-text-is-read-in-place.loft`, pin `tests/text_run.rs`).

### A vector copied element by element is one append

```
  (R-VecCopy)    `for e in V { t += [e] }` — the forward walk of a vector PLACE V (a
                 local, or a field chain over one) whose body is the one push of the
                 loop value into t, the element a kind whose read and push are exact
                 inverses on the stored bytes (`integer`, `i32`, `float`, `single`,
                 `boolean`, an enum, `character`, a raw byte at the push's own bias),
                 V's element t's element — is `t += V`, where t is EXCLUSIVE: a local,
                 or the frame's hidden return or work buffer, every binding of which is
                 the field of a store minted in this frame.  A parameter destination —
                 a `&` link, a by-value vector — is not exclusive: a caller can hand V
                 in as t, and the walk then grows what it walks where the append takes
                 it once.  A second statement, a transformed value, an element that owns
                 heap or reads through a null sentinel, a source that is a call or a
                 slice, and a destination of another element width keep the loop.
```

**In words.**  "A copy of this buffer, then more" has no one-statement spelling a library
author reaches for first, so it is written as the walk — and the walk paid an element read,
a length re-read and a push per element.  Nothing in the body can move V's length, because
no push into an exclusive t reaches V, so the walk visits every element in index order
exactly as the append copies them.  No guard is needed: the conditions are all static.

**BUILT** (`src/vec_copy.rs` after the byte-copy pass, `LOFT_NO_VEC_COPY`; guard
`tests/scripts/a-vector-copied-element-by-element-is-one-append.loft`, pin
`tests/vec_copy.rs`).

### A fixed-width integer read is typed at its site

```
  (R-TypedRead)  `OpReadFile(f, OpCreateStack(t), n, τ)` into an `integer` local t, where
                 τ is a fixed-width integer and n its width — `integer` / `long` 8 signed,
                 a byte or short type 1 or 2 with the sign its range gives, a 4-byte type
                 signed — reads through the same format test, `#next` and handle as the
                 generic read, decoding W = n bytes at sign S fixed at the site, the bytes
                 taken by value.  A short read leaves t unchanged and advances `#next` by
                 what arrived, as the generic read does.  `boolean`, `character`, text, a
                 float, a collection, an n that is not τ's width and a slot that is not an
                 `i64` local keep the generic read.  Native only; the interpreter's read is
                 the reference the guard compares against.
```

**In words.**  The generic read asked, per call, whether τ is text and which width and sign
it decodes, and copied the bytes through a slice on the stack — facts the site has at
compile time, and a reload that stalled on the bytes just written.  The decode is the
generic one's, arm for arm, so the answer cannot differ; what moved is when it is decided.

**BUILT** (`src/generation/ops/file_ops.rs`, runtime `OpReadFileInt`, `LOFT_NO_TYPED_READ`;
guard `tests/scripts/a-fixed-width-integer-read-is-typed-at-its-site.loft`).

### A walk of a scalar literal builds no vector

```
  (R-LiteralWalk) `for x in [e₀, …, eₙ₋₁] { … }` — a `for` whose iterable is a vector
                 LITERAL of 1..=16 items of one scalar element type (`integer` of any
                 width, `float`, `single`, `boolean`, `character`; an item a later item
                 widened counts at the widened type) evaluates each item once, in source
                 order, into its own scalar temp before the first iteration, and walks
                 the temps by a counted select: the loop variable at step i is temp i,
                 `x#index` is i, and the walk ends after step n−1.  No vector is built,
                 on either backend.  A `rev(…)` around the literal, a `par` walk, a
                 `[v; n]` repeat, a text, record or tuple item, and seventeen or more
                 items keep the vector walk.
```

**In words.**  "For each of these three" is written as a walk over a literal —
`for i in [tri.a, tri.b, tri.c]` — because that is the spelling the language offers, and
the literal cost what a vector costs: a buffer reset (or a mint), a reservation, one append
per item, and a length and an element read per step through the store, per execution of the
loop.  The values are fixed before the loop starts on the vector form too (the literal is
built, then walked), so holding them in scalar temps changes no value, no order and no count;
the select is what the vector's element read was, on a bound the compiler knows.  A body
that writes the loop variable writes its own copy either way, and a body that writes an
item's SOURCE after the loop began never reached the built vector either.  Both backends
take the lowering at parse time, which makes their agreement no evidence — the guard's cells
are hand-computed and the switch is the A/B.

**BUILT** (`Parser::literal_walk` at parse time, `LOFT_NO_LITERAL_WALK`; guard
`tests/scripts/a-walk-of-a-scalar-literal-builds-no-vector.loft` — 21 cells, falsified by a
one-off in the select, which the order-sensitive cells catch and the plain sums do not — pin
`tests/literal_walk.rs`).  mesh3d's `mesh_to_floats` (`for i in [t.a, t.b, t.c]` per
triangle): **2.85 → 1.84 ms per op, 15.8× → 10.5× of Rust** on this box, same hash; the
census loses the literal's own loop buffer, header and complete write in every program that
carries the two mesh3d walks, and gains the walk.  Building it surfaced a defect in the
literal itself: `[1, 2.5, 4]` in an iterable position converted the WIDENING item through
an integer conversion and left the earlier `1` an integer under a float vector (an
assignment's variable type repaired it on the second pass, so `v = [1, 2.5, 4]` was right
and `for f in [1, 2.5, 4]` read 2.5's bits as an integer) — fixed at the coercion site, cell
c21 keeps the vector form of it.

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
are about 250.  The typed entry is the table's header reads, the digest inline, and the walk compiled for the key's kind.  The condition is read off the schema the emitter already
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
§ V-j hook.

### A returned local bound from a call is built in the return buffer

```
  (R-ForwardResult) a local L of a function value-returning a vector through its
                 hidden return buffer rb, bound in a block from a call that delivers
                 into its hidden buffer argument (a one-buffer return, never a borrowed
                 view) — `buf = head(…, __ref_N)` — and delivered later in the SAME
                 block by `OpClearVector(rb); OpAppendVector(rb, L)`, with nothing
                 between the bind and the delivery naming rb and nothing after it
                 naming L, hands the call rb instead: L is then rb, the delivery pair
                 is the identity and goes, and `__ref_N` — named only at its null
                 inits, its null guard, this call and its frees — is never minted.
                 L is judged across all its sites (the arms of one `match` bind one
                 `buf`): outside them it is named only at its null inits.  rb shares
                 no store with a parameter (every road that hands a buffer in refuses
                 one that does), and the callee's entry clear is the buffer's reuse
                 contract (R-RetAdopt), which is why the delivery's clear can go.
```

**In words.** cbor's `encode` arms (`buf = head(2, len(value)); buf += value; buf`)
minted a store per call, built the header and the payload in it, copied the whole vector
into the caller's buffer and freed the store; the other arms of the same `match` already
handed `head` the return buffer.  Switch `LOFT_NO_FORWARD_RESULT`;
`LOFT_TRACE_FORWARD_RESULT=1` names each admission and decline.  Site:
`forward_result::rewrite`, in the scope pass after `(R-ExitVector)`, so both backends
read it.  Guard
`tests/scripts/a-returned-local-bound-from-a-call-is-built-in-the-return-buffer.loft`.

### A helper's arithmetic is plain where its caller bounded the arguments

```
  (R-RangedCall) a counted `For` block whose loop hoists integer record fields — each
                 invariant over the loop by the hoist's write set, or read only in the
                 block's prelude, which calls no user function — is emitted twice: under
                 a run-time guard `|x| <= G` for each such field (G = 2^20) with those
                 field reads RANGED `[-G, G]`, and as before.  Inside the guarded copy a
                 local's derived range is kept only when every assignment of it in the
                 whole function lies in the block and it never escapes by reference.  A
                 call there — or inside a ranged variant — whose every integer argument
                 and every twin scalar input is proven within `[-L, L]` (L = 2^30) calls
                 the callee's RANGED VARIANT: the same body with its integer parameters
                 (each one the body never assigns) and its scalar inputs ranged `[-L, L]`,
                 where (R-Range) then decides every operator by interval arithmetic.  A
                 longer callee's RESULT is ranged in context: its body analysed with the
                 parameters carrying the arguments' ranges, the union of every exit's.  The
                 copy runs only when it calls a variant of a callee doing checked integer
                 arithmetic; a `yield`, a `par` or a fn-ref call declines it.
```

**In words.** The checked operators' null and overflow branches are what keep a chain of
small grid helpers (`nb_q`, `eg_index`, `eg_dir_from`) from folding: hex_field's
`edgeset_count` was 18× its Rust twin.  `(R-Range)` cannot range a parameter or a field by
shape (C80), so the bound comes from the caller, tested once per loop entry, and flows
into the helpers through variants.  With the byte read of `(R-Base)` the chain is free of
store calls and LLVM folds it: `edgeset_count` 18.2× → 1.9×.  Switch
`LOFT_NO_RANGED_CALLS`; `LOFT_TRACE_RANGED_CALL=1` names each copy, variant and decline;
`LOFT_HOIST_VERIFY=1` checks every plain operator.  Sites: `ranged_call.rs`
(`range_guard_for`, `ranged_call`, `variant_ranges`), `range.rs` (`Ranges`, the seeded
`range_vars`, `call_range`), the `For`-block arm in `emit.rs`, the variant loop in
`output_functions`.  Guard
`tests/scripts/a-helper-runs-plain-only-where-the-callers-guard-bounds-its-arguments.loft`.

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
                 range at either end declines the nest whole.  THE REDUCTION
                 CLAUSE: a counted, exclusive, literal-start loop that is ONE
                 integer accumulate of a scalar vector's elements — `acc = acc +
                 v[i]` (or the operands the other way round), an 8-byte `OpGetInt`
                 at field 0 over a pure path with `i` the loop variable, the
                 stdlib `sum`'s loop — in a loop that holds the path's header and
                 base, sums what it can PLAIN before the loop: a block of 1024
                 elements at a time, admitted when every element lies in
                 `[−2^40, 2^40)` and the running total is more than `2^50` from
                 the i64 edge — the magnitude-bound proof above, taken from the
                 DATA per block in the same pass — so no prefix inside the block
                 can leave the type and the plain block sum is the checked
                 answer; the first block that fails ends the plain part, the
                 `#index` is advanced to one below the first element not summed,
                 and the checked loop, emitted unchanged, resumes there (a null
                 element fails the bound and is left to it, which propagates
                 it).  Every answer is the loop's own on every input.  The bound
                 test is spelled `(x + B) as u64 >> 41`, OR-accumulated, and the
                 sum `wrapping_add`, because the baseline target has a packed
                 64-bit add and no packed 64-bit signed compare.  A float
                 accumulate, a `?`-discharged read, a second statement, a computed
                 index or a computed start keeps the checked loop.
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

**The reduction clause, in words.**  @PLN158 (`bench/portal/analysis/vector-build.md`
§ `sum` re-priced).  The stdlib `sum` was 6.45× its Rust twin: every element paid the checked,
null-propagating add — a sentinel test on each operand and a `checked_add` — which cannot
vectorise, against a twin's wrapping add that runs 2-wide on SSE2.  The correct problem is the per-element check, and the answer is this rule's
own: prove from a bound that the checked answer is the plain one, then run plain — per block,
at run time, in the same pass.  The remaining 2× is the op count (add, shift, or, add per element against
the twin's add), and the owner has said 2× is fine for a routine an optimised program caches
the result of.  Switch `LOFT_NO_BOUNDED_SUM` (and `LOFT_NO_VECTOR_BASE` /
`LOFT_NO_VECTOR_HOIST`, one rule); trace `LOFT_TRACE_NEST=1` names each reduction admitted
and each counted loop declined with why; falsifier `LOFT_HOIST_VERIFY=1` re-runs every
admitted block through the checked add and panics on a disagreement — the proof itself,
re-run, so unlike the join clause's verifier it has no blind spot here: sabotaged on the ROOM
test, s5 answers `−9223372036854773861` for null (wrapped garbage, exactly what C85 forbids)
and the verifier panics; sabotaged on the element BOUND, s2 answers `MAX` for null (the
`MAX + 1` prefix overflow summed plain and cancelled) and the verifier panics.  Three of the
cells' expectations were first written by hand and were wrong; every number stands as
computed, and the interpreter agreed with the computation each time.  Sites:
`hoist::bounded_sum` (the shape), `Output::sum_fast_path` (the prelude, emitted before the
guards so both copies of a guarded loop resume from the advanced counter),
`vector::sum_blocks_i64` with `SUM_BLOCK` / `SUM_BOUND`.  Cells
`tests/scripts/158-bounded-sum.loft` s1–s9, pins `tests/bounded_sum.rs`.  Next of the same
clause, not built: a dot product (`acc += a[i] * b[i]`, bound `2^20`); `min_of` / `max_of`
need no proof, and `product` is multiplicative and is not this.

**Both backends (@PLN180 § Kernels).**  The reduction is now decided in the scope pass
(`loop_kernels::rewrite`), where it replaces the whole loop by ONE call of the loop kernel
`vector_sum_int(v, acc)` — an ordinary stdlib function, reached by the interpreter through its
single native-call operator and by `--native` through `codegen_runtime`, whose body is this
clause (the plain blocks through `vector::sum_blocks_i64`, then the checked add).  The
interpreter stops dispatching every element: bench 14's `sum` went from 1,174,380 ns/op to
4,830 against native's 4,300 (273× → 1.1×).  Admitted narrower than the native shape: the loop
is exactly `end = len(v); idx = -1; loop { i = step; { acc = acc + v[i] } }` over 8-byte
integer elements, the summed vector the ranged one, the loop's own variables mentioned nowhere
else.  Switch `LOFT_NO_LOOP_KERNELS` (the loop stays, native's own clause then takes it);
cells `tests/scripts/a-reduction-loop-is-one-kernel-call.loft`.

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

### A local that never leaves the frame is the caller's buffer

```
  (R-WorkBuffer) a vector local of a callee — `v: vector<τ> = []`, τ a scalar — whose
                 every mention, its own and its aliases' (`for x in v` binds one), is an
                 operand of a vector operator that reads or writes the vector IN PLACE
                 (a push, an append in either role, a reservation, the length, a clear,
                 a removal, an element read or written through a scalar getter or
                 setter) is STORAGE OF THE CALLER: a hidden `vector<τ>` parameter,
                 named as the local and marked a work buffer, that every call site
                 supplies from its frame's POOL — the k-th work buffer of element type τ
                 in ONE call is the frame's buffer (τ, k), shared by every site, because
                 calls in a frame never overlap and the callee clears it on entry; the
                 pool is disjoint from the frame's own promoted locals, the only buffers
                 that hold data across a call — as the work-ref a hidden return buffer
                 takes (O-LazyBuffer: minted once per activation, on a path that makes
                 the call, freed at the caller's exit), and that the callee CLEARS where
                 the declaration stood.  A callee handed the null sentinel — an entry the
                 runtime enters, a host's call — takes the rebound-parameter road at
                 that same site: a store of its own, minted there and released at exit
                 against the entry witness (O-Buffer's callee half), so no route owes
                 the callee a buffer and a null is never a wrong answer.  The mention
                 test IS the escape proof: with a scalar element every admitted operand
                 position yields a scalar or nothing, so no view, copy or link of the
                 store can leave the frame.  A loft-bodied callee may take the local BY
                 VALUE where its answer cannot hold it — the return type, through every
                 nested type, carries no dep naming that parameter and is no function
                 value — because a by-value heap parameter is a view for the call's
                 duration (F-ParamHeap) that no store of the callee's retains by
                 identity: a bind or a field store copies (B-Copy), a link cannot be
                 stored, and a rebind (F-ParamRebind) is the callee's own store,
                 released at its exit.  Declines, keeping the mint: a hand-off to a `&`
                 parameter (a rebind through it repoints the caller's variable), to a
                 callee answering a view or a function value, to a native or a parallel
                 builtin, or through a function value; a local copied WHOLE into any
                 other place — a record's field (`out += [Rec { pts: v }]`, which the
                 native emitter builds inside the appended element with no store at all,
                 R-ElemFirst; `a.items = v`, which the move elision builds straight into
                 the field through its staging local), the return buffer, which adopts
                 the local (R-RetAdopt), or another local — and a local bound PURELY as
                 a copy of another vector (`v = s.v`: one copy in, no push or insert),
                 which the copy elision's borrow tiers and the transparent link remove
                 outright — every one of those rewrites asks for a local, and a buffer
                 would turn a saved copy into a paid one; a local declared INSIDE a loop, whose
                 store the native emitter already keeps across the passes with a length
                 reset (R-LoopBuffer) cheaper than a clear, and whose invariant literal the
                 literal hoist builds once (R-LitHoist); a local written and never read, the
                 dead-store lint's subject; an ENTRY POINT, which no caller hands a buffer;
                 a local numbered before an argument in a function whose RETURN TYPE
                 carries deps — the swap that puts it after the arguments renumbers frame
                 numbers, and those deps are attribute-space (the debug-assertions gate
                 caught the swap rewriting `text[0]` to `text[3]`); a mention in a return or a tail,
                 in a literal, a tuple, a link or a capture, a second assignment, a copy
                 into a local that is then not so used; an element type that is a
                 record or a text; a body that suspends or forks; `main`; a generic, a
                 synthetic, a lambda, a function whose address is taken; a `par` worker.
                 TRANSITIVELY: a caller's work-ref minted for such a buffer, whose only
                 mentions are its entry null-init and work-buffer argument positions, is
                 itself a local that never leaves the frame and becomes a hidden
                 work-buffer parameter of the caller — no mint, no free there; the
                 POOL climbs round by round to the outermost frame that is not so
                 promoted, bounded by the widest single call rather than by the number
                 of sites, and a null handed down reaches the innermost callee's null
                 road unchanged.  Declined where the callee reaches back to the caller
                 through direct calls: a recursion wants a buffer per activation, which
                 the ref already is, and each round would only mint the next.  A call
                 through a function value needs no such edge — its target's address is
                 taken, so that frame mints its own buffers and the chain stops there.
```

**In words.**  The overview's largest language-side bucket is a value Rust keeps on the
stack and loft mints as a STORE: a vector made fresh per call and freed at its end, 67 ns on
the smallest such function against ~20 ns for a `Vec` created and dropped, and the whole cbor
bench binary spending ~45 % of its time in that bookkeeping.  What removes the mint is not a
cheaper mint but no mint: the store lives across calls, in the caller's frame, exactly as the
text work buffer and the hidden return buffer already do — the rule is those two read once
more for a LOCAL.  Decided after pass 2 on the settled IR (`Parser::promote_work_buffers`,
beside the targeted `__tret` promotion, whose caller-patching it shares), so a forward- or
backward-referenced caller is patched alike; the callee's shape is the one the parser already
emits for `if c { v = [] } else { v.clear() }` on a by-value vector parameter (entry witness,
guarded frees), with `OpRefIsNull(v)` as the condition.  The attribute carries an explicit
`work_buffer` mark because every route that builds a frame by hand — the argv entry, the par
worker, the shared-cdylib bridge, the engine host, placement — reads the hidden compound
attribute as THE return buffer, and a second one would have been pushed as a result or
refused.

**The pool** (loft#1697; `Parser::pooled_work_buffer`, opened by
`patch_work_buffer_callers`; `LOFT_NO_WORK_BUFFER_POOL`; guard
`tests/scripts/a-frame-pools-its-work-buffers-across-call-sites.loft`).  Per site, the
transitive clause multiplied buffers up the call tree: a library of 100 functions of 150 calls
each gave every function 150 hidden parameters and `main` 15 002 locals, and the promotion
fixpoint re-ran the scope pass on each function up to twelve times — 96 s where the build
before the rule took 1.85 s in all.  Pooled, the same file takes 0.78 s and each function one
forwarded buffer.  A self-call now takes the forwarded buffer from the pool where it minted one
per activation (`tests/work_buffer.rs` `n_c18`): a forwarded buffer is only ever an argument,
so it holds nothing across the call; a frame's OWN promoted local, which does, is never in the
pool, and a self-call that must supply one still mints a buffer of its own.

**BUILT** (`src/parser/work_buffer.rs`, run from `after_pass2` beside the
targeted `__tret` promotion; `LOFT_NO_WORK_BUFFER`, `LOFT_TRACE_WORK_BUFFER`; guard
`tests/scripts/a-non-escaping-vector-local-is-a-caller-buffer.loft`, pin
`tests/work_buffer.rs`, and `LOFT_WORK_BUFFER_NULL` the positive control that runs every
promoted callee down its null road on both backends).  The attribute mark is
`Attribute::work_buffer` (serialised; cache format 13), read by the engine host (a null
where it offered its result record), placement (a function with one runs in-process) and
the shared-cdylib bridge (a fresh scratch, released after the call whatever was returned);
the argv entry, the native `main` and the par worker's queue already allocate one store per
hidden vector attribute and free it after.  A `par(…)` worker is declined by name of the
builtin's `func` operand: its scalar route builds the frame from the element alone.  A lazy mint for a callee's buffer lands in the arm
that makes the call, not before the whole value `if` (`scopes::place_in_if`) — cbor's
`encode` minted its map arm's four buffers on every call.  A promotion is only a gain
where the emitter still sees an owner and the form it replaces was a store at all.  The cure is the
TRANSITIVE clause above — the wrapper's own `__ref_N` work-refs are locals that never leave
its frame and are candidates in their turn, so the buffers climb to the outermost frame
that loops — Phase A once, then promotion of the work-refs and Phase B alternating until a
round promotes nothing (capped at sixteen rounds; stopping after any Phase B leaves every
buffer minted by its caller).  Without the cycle decline the fixpoint does not end rather
than answer wrong: each round hands the recursive callee one more parameter, so every
depth still has a buffer of its own (the pin's falsifier: `c2` grows seventeen).  A buffer lives as
long as its caller's
activation, so a promoted call site in `main` keeps its buffer for the run exactly as a
return buffer does; `LOFT_STORES=warn`'s high-water heuristic (more than 30 live stores)
reads such a `main` as a possible leak, and it is a working set.

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
`Output::output_function`.  Applied by: native generator; its IR form, inlining the leaf for
both backends, is `(R-InlineLeaf)` (proposed).

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

### A frame no registrant can reach carries no buffer guard

```
  (R-GuardFree)  a function from which NOTHING that registers a fn-ref return buffer
                 is reachable — over the closure of its call graph, cycles INCLUDED:
                 no fn-ref call, no `parallel`, no `yield`, no `OpFreeRefOrHandUp`,
                 no callee without a loft body that is a user's native function or
                 takes a fn-ref — constructs no `FnRefBufGuard`, in every tier.  The
                 depth count stays: it is the recursion cap both backends share.
```

**In words.**  @PLN158 round 3, C1.  A `--native` frame entered with a `FnRefBufGuard`
whose drop releases the fn-ref return buffers registered above its mark; three sites
register — the fn-ref dispatch (`cr_fnref_buf`, `cr_fnref_minted`, both under a `CallRef`)
and the op `OpFreeRefOrHandUp` (a capturing lambda's join tail) — and a `parallel` body, a
`yield` and a native callee are opaque.  Where none of those is reachable no entry can ever
stand above the mark, so the drop is empty every time and the guard is two `Cell` reads paid
for nothing: `fib`'s frame was 27 % of its row.  `(R-LeafChain)` already elides it — with the
whole prelude — for an ACYCLIC chain in the lean tier; the guard never depended on
acyclicity (only the depth count does), so this rule reaches the recursive function in every
tier, and keeps its depth push.  The predicate is a breadth-first closure with the
per-body half cached (`Output::registers_buffers`); a `#rust`-bodied standard function is
the body's own work, as `(R-Leaf)` reads an op, and a stdlib function taking a fn-ref would
be opaque (none does today).  The stack-pointer overflow check the round priced beside it
(a further −15 %) is DECLINED: `MAX_CALL_DEPTH` is a cap both backends share and report
identically (`runtime_errors.rs`, `native.rs` pin "exceeded 10000 stack frames"), and a check
against the machine's stack would fault at a machine-dependent depth — the disagreement
`formal/operational.md` D-op-1 forbids.  Switch `LOFT_NO_GUARD_FREE`; trace
`LOFT_TRACE_GUARD_FREE` (each frame that keeps its guard, naming the registrant); scored on
the LEAK channel — cells `tests/scripts/158-guard-free.loft` g1–g9 under
`LOFT_NATIVE_LEAK_CHECK=1` / `LOFT_STRICT_STORES=1` (falsified by answering `true` for every
frame: g5's three delivered buffers leak), pins `tests/guard_free.rs`.  Sites: `Output::registers_buffers`, `Output::is_guard_free`, the guard
line in `Output::output_function`'s prelude.

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
                 `set_default_value`'s walk (or its all-zero `zero_range`) writes
                 nothing that survives, and
                 the site calls the no-prefill twin (`OpDatabaseNP` /
                 `OpNewRecordNP`).  A WHOLE-record `OpCopyRecord` into the element
                 itself (`self.items += [p]`, `p` a value of the element type)
                 covers every field when the element owns no heap; a heap-owning
                 element's copy trusts the zeroed handles the prefill wrote and
                 keeps it.  An uncovered field — a nested struct arriving by
                 `OpCopyRecord` into a FIELD, a vector field bound by an append, a
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
                 THE COMPREHENSION CLAUSE: a row built by a comprehension and appended
                 as a WHOLE element of a local vector of vectors (`[for y { [for x {
                 … }] }]`) is the same build with a third declaration shape and a
                 whole-element destination — the row's vector, declared through its
                 `__vdb` buffer INSIDE the comprehension's block, is bound to the
                 element (the element of a vector of vectors IS the inner vector's
                 handle) and the `OpCopyRecord` into the element vanishes.  Gates: the
                 block's value is read by the copy alone, the row's vector and its
                 buffer are named only inside the block (the buffer also by its
                 declaration and frees), the block names neither `out` nor a view of
                 its elements and jumps nowhere outside itself, and the block LOOPS —
                 a body that loops again holds no push window on `out` (R-PushFill),
                 the one raw address into `out`'s store the row's growth could leave
                 stale; everything else held on that store is refreshed or declined per
                 pass already, because the element's own mint grows it.  The append
                 group stands in the body of the LOOP that builds the outer vector, so
                 a loop body's statements consult the same overrides a block's do.

  (R-ValueRecord) a function whose result is a PLAIN NO-HEAP RECORD of at most eight
                 scalars (an `integer` only at its 8-byte width), an INLINE sub-record
                 contributing its own scalars at the summed offset, returns those
                 fields BY VALUE — a Rust tuple, in registers — instead of writing them
                 into a return buffer the caller then reads back.  Admission is a
                 FIXPOINT over two gates, because a tail may forward another admitted
                 function's result and a site may bind a branch of admitted calls:
                 the BODY gate asks that every result position be a VALUE LEAF — an
                 `Object` build of the function's own record (the tuple of its writes),
                 a call to an admitted function (its tuple, forwarded), a value local,
                 or a borrowed VIEW of the record (the tuple of its field reads; a view
                 is never freed, so reading it is all the value form owes), where a
                 DISCHARGED view returned through the buffer (`return v[i]?`: the copy
                 of the view-or-default into the buffer and the buffer's return) is the
                 leaf its copy SOURCE is, the mint, the copy and that return dropped
                 with it — a heap
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
                 join local from a parameter's view).  The TUPLE lists its elements in
                 field order, but a literal's field values run in the order the literal
                 WRITES them: an out-of-order fill binds each value first, in that
                 order, so a field expression with an effect runs where the program put
                 it.  BOTH BACKENDS (@PLN180): where the whole shape is a flat record
                 returned from an object literal to callers that only read its fields,
                 the decision is made in the IR (`value_record::rewrite_program`, at the
                 top of `byte_code_from`, so the closed program only): the callee
                 returns `(a, b, …)` in fill order, the local is a tuple read by
                 `TupleGet`, and the buffer's null init, mint guard and frees leave the
                 IR, so the interpreter claims no store for the call.  Its candidates
                 are this rule's admissions, and native finds only what the IR
                 declined.  Switch `LOFT_NO_IR_VALUE_RECORD=1`; guard
                 `a-small-record-returned-to-a-reader-is-a-tuple.loft`.

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
                 halves back together.  So is `#[cold]`: it is the branch weight
                 that keeps the fast path's test a plain branch — without it LLVM
                 may turn that test into flag arithmetic paid on every element,
                 and which form it picks then moves with unrelated code nearby.
                 FOLD CLAUSE: an off-fast-path answer computable from what the
                 fast path already holds — no store resolution, no write, no
                 report — is FOLDED IN, never outlined: an element read resolves
                 every index with `vector::elem_index` (a negative index counts
                 from the end, anything else outside answers the sentinel), the
                 ONE definition `get_vector` uses too.  A call is opaque to LLVM
                 even when it is cold and only reads, so two reads of one element
                 stay two reads and the loop around them is too large to unswitch;
                 folded, the read is pure and both merge.  `LOFT_HOIST_VERIFY=1`
                 compares every folded answer with a fresh `get_vector` read
                 (`verify_folded_read`).  A fold is a layout choice like the
                 split, so it is kept only where the lanes show it pays: the
                 TEXT element read keeps its negative-index half outlined,
                 because folded in, in any arrangement, it slowed the text walk
                 of the `join` row and a reader of distinct elements has no
                 merge to gain.
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
symbol and self time is the candidate).  The fold clause's cells are
`tests/scripts/fused-element-read-resolves-every-index.loft` e1–e7 (falsified by a fold
that drops the from-the-end resolution: `--native` fails e1, the verifier names index −2),
and `vector::elem_index`'s own unit tests.  Fold sites: `vector::get_elem_at`,
`vector::get_elem_hoisted`.  Outlined sites: `vector::text_elem_cold`,
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
                 the destination is a relocation within S (R-MoveLast).  The callee
                 writes that buffer on every exit: a fresh literal built into it, or a
                 CHAIN exit that hands the same buffer to a callee of which this holds
                 (asked recursively; a cycle declines) — a chain function's buffer is
                 the `__ref_N` the chain renamed it to.  When the
                 destination place EXISTS at the call and no argument of the call
                 reaches it, the buffer IS the place and nothing moves — a field
                 assigned (`h.v = f(…)`) and a field of the record a struct literal
                 is building (`H { v: f(…) }`, which exists once the literal has
                 written its empty handle; a `?` field keeps the replace that leaves
                 it absent).  Declines: a
                 path that reads the result after a RELOCATING store (the destination
                 owns it then, and B-Copy would show — where the buffer IS the place, a
                 read of the result reads what was written and needs nothing); an
                 argument that reaches the destination's store (source and destination
                 alias); a callee that may hand back a store it did not mint (O-Opaque:
                 empty deps license nothing), and, where the buffer IS the place, a
                 callee that MINTS into its buffer on some exit (a projection chain
                 `return g().inner.v` does), which would mint over the place it was
                 handed — a returned collection LITERAL is not one: its wrapper mint
                 on a place answers the place, the collection it held released, so it
                 builds where it lives; a callee
                 that on some exit answers a store other than the buffer it was handed
                 (the destination would hold the writes of the exit not taken); a
                 `?`/`??` discharge on the result; a destination that does not
                 unconditionally EXIST at the call — a field of a struct-enum VARIANT
                 exists only while the value holds that variant.  A member of a linked
                 collection group is NOT a decline: the group's maintenance brackets
                 the fill (Col-Group), and a fill that arrives through the buffer
                 instead of an append is one it must see.  A bind INSIDE a loop is
                 admitted through the same argument when the destination is the ONE
                 heap-owning field of the result — its payload — stored into an
                 element appended to a host that is a parameter or a local vector
                 bound before the loop, with the result's scalars read after the
                 store: the buffer is claimed in the host's store on the first turn
                 and cleared on the next (the lazy mint keeps its shape), the field
                 moves by relocation with its source ZEROED — the turn's clear and
                 the callee's refill then find an empty record, and a later read of a
                 scalar reads what the callee wrote — and the exit releases the placed
                 record as a block, so a host that outlives the frame keeps nothing of
                 it.  Declines keep the copy: a read of the payload after the move, a
                 second destination, a nested loop, the local or the buffer named
                 outside the turn, a host bound after the loop or rebound.  Every
                 other call keeps its own buffer.
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
  (R-ExitVector) a LOCAL VECTOR whose wrapper is minted once and that reaches the
                 function's exit only inside the literal built into the return
                 buffer — `xs: vector<T> = []; …; return Out { xs: xs, … }` — has its
                 wrapper CLAIMED IN the return buffer's store (the buffer ensured
                 first, exactly as the exit ensures it), the literal takes the vector
                 by a HANDLE MOVE — the slot's record number moves and the source slot
                 is zeroed, so no element and no heap claim moves — and every exit
                 releases the wrapper's block, which finds nothing on the path that
                 moved and the whole vector on one that did not.  Between its init
                 and the exit the local is only ever a RECEIVER (the first argument of
                 a native op: a push, a read, a placement host) or a borrowed argument
                 of a loft-defined call, and the wrapper is named only at its null
                 init, its mint, its length reset and its exit frees.  Declines keep
                 the wrapper store and the copy: a rebind or an alias of the local, a
                 bare `return xs`, a copy of it anywhere but the buffer's literal, a
                 mention of it in that literal AFTER the field that took it (the move
                 would be read as empty), a free outside an exit, a wrapper freed
                 nowhere, and a wrapper whose type is not `main_vector<T>` for the
                 local's `vector<T>` (a tuple-destructured local's is typed a level
                 too deep, loft#1757, and a release that walked it by that type would
                 read every element as a handle).  It composes with R-Place's loop clause: a loop whose host is
                 this local places its buffers in the return buffer's store too, so a
                 recursive decoder builds its whole tree in the store of the outermost
                 buffer and copies nothing on the way up.  The move is same-store and
                 into an empty slot by construction; the runtime keeps the copy for any
                 other pair, and `LOFT_HOIST_VERIFY=1` makes that pair fatal.
  (R-ReturnField) the returned FIELD of an OWNED local — `p = mk(…); return p.a`, its
                 view-local spelling `v = p.a; return v`, or the natural spelling
                 `mk(…).a`, whose call the parser lifts into a local — is answered as the local's own
                 store at the field's position, instead of a store minted for the return
                 and the field deep-copied into it.  The parser's copy stands where a view
                 of a frame local would dangle once the frame's free ran; here the local is
                 the frame's own store (a dep-empty record local, never a parameter or a
                 hidden buffer) and the exit is the last thing that names it, so the store
                 goes to the caller as it stands and the caller frees it whole as it frees
                 any adopted store — a store's release is keyed on the store and not on the
                 handle's type or position, and the wrapper's other fields go with it.  The
                 exit's other store frees are RE-WITNESSED against the root: a buffer that
                 aliases the handed store (a pooled `__ref_N` the root adopted, an alias
                 `q = p`) is skipped, one that does not is freed as before, and the root's
                 own free is dropped.  Admitted only where the returned record OWNS HEAP (a
                 scalar-only record is the native value form's to answer as a tuple, and its
                 copy is a few words), the exit block is the parser's materialised copy over
                 the function's own buffer, every statement between its mint and the copy
                 is a bind or the mint-or-release guard of a hidden buffer the lifted call is
                 handed (`@FR-O-LazyBuffer` — it names neither the function's buffer nor the
                 root), and every statement between the copy and the return is a store or
                 text free.  Declines keep the copy: a parameter's or a
                 view's field, an ELEMENT (`p.items[i]` — a slot inside a claimed block, not
                 a field of the root record), any other statement in the exit, a buffer that
                 is not the function's own.  A caller is not consulted: it binds what comes
                 back exactly as it binds any buffer-carrying return, adopting with the
                 witnessed free or copying with the source freed, so a callee whose other exit
                 writes the buffer is fine — the witness settles which store came back per
                 execution.
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
*The chain form of the callee clause (`place_result::chain_writes_buffer`):*
`return mk(n)` hands the buffer through, and the exit test read it as an exit answering
another store, so every chain-built result cost a store of its own — the shape a decoder's
`decode` (`return d`) and every `return build(…)` wrapper has.  Guard
`tests/scripts/a-chain-exit-hands-the-placed-buffer-through.loft`, the placement MATRIX for a
decoder's shapes (the chains, the wrapper-field shapes the next clause must admit, the
placement sites and their three declines); pin `tests/place_result.rs`.  The matrix also
records what the decoder still needs and this rule does not yet admit: a bind INSIDE a loop
(the pass reads top-level binds only), a destination in a LOCAL's store, and the placed
thing being a heap-owning FIELD of a wrapper result whose scalars are read after the move.

The rules are written BEFORE the phases so that a question met while building one is
answered here rather than decided in the code.

### A value already home is not copied to a second one

```
  (R-Rebind)     `x = f(x, …)` — a whole-value bind to a plain OWNED local x (O-Owner:
                 never a parameter, a view, a `&` link or a witnessed local, nor a `τ?`
                 that may be absent) — or to the frame's OWN return buffer promoted onto
                 a named local (`fn build() -> Doc { d = …; d = step(d, …); d }`), the
                 record the caller handed this frame to fill, which shares no store with
                 a sibling parameter because every road that hands a buffer in refuses
                 one that does — from a call that receives x BY VALUE as an
                 argument, where no OTHER argument reaches x's store (R-Place's alias
                 condition) and no view of x is live across the call (B-Disturb: the
                 CALL, not the bind, is now where x's store changes, so a view the
                 rebind would have materialised is materialised before the call) —
                 hands the callee x's OWN record as its return buffer.  The parameter
                 already IS that record (a by-value record parameter writes through,
                 B-Ref-Uniform), so the callee's reads and writes of it during the call
                 land where they always did; only its EXITS change.  An exit literal is
                 staged (R-InPlaceLiteral: every field expression evaluated before the
                 first write) and then written field by field into x's record: a field
                 whose staged value is a view of that same field (`Doc { buf: d.buf,
                 … }`) is H-CopySelf and costs nothing; a field whose value is an OWNED
                 DEAD local built in a store of its own (`pieces: np`) is delivered as
                 R-MoveLast delivers it — a relocation where the stores are one, the
                 deep copy where they are not — after the old field's owned heap is
                 released (H-ClearRelease); an exit `return d` answers the buffer
                 itself and moves nothing.  The call site binds nothing and frees
                 nothing: x's store stays x's, freed at x's scope exit as before, and
                 the call's own hidden buffer is never minted (O-LazyBuffer);
                 O-Buffer's destination clause is what makes this buffer no store of
                 the callee's to release.  Every write to the buffer must FOLLOW the
                 callee's last read of the parameter on that path — an exit literal
                 does; a local promoted onto the buffer and filled BEFORE the parameter
                 is read (`mat4_mul` builds its result at entry and reads `mb` after)
                 declines, because the fill would overwrite the argument it is about
                 to read.  Declines also, keeping the call's own buffer and the copy: a
                 callee that on some exit answers a store other than the buffer
                 (R-Place); a callee that MINTS into its buffer (a returned vector
                 literal); a callee that stores or returns a view of a field of d into
                 a place that outlives the call while an exit overwrites that field; a
                 recursive call; a `?`/`??` discharge on the result; and the field form
                 `o.f = f(o.f, …)` exactly as R-Place admits or declines the place.
  (R-Compact)    `V = t` — a whole-value bind of a plain vector place V (a local, a
                 field, an element) from a local t that was declared `[]`, filled ONLY
                 by appends of V's OWN elements taken in NON-DECREASING index order
                 (`for i in a..b { t += [V[i]] }`, `for e in V { if p { t += [e] } }`,
                 `t += V[a..b]`), interleaved with at most values that do not reach V,
                 such that at every append the count of elements already in t is at
                 most the index being read (no slot is written before its old value is
                 read), V neither written nor viewed between t's declaration and the
                 bind, and t dead after it — is performed IN V's OWN record: each kept
                 element is relocated to its new slot (H-CopySelf where the slot is its
                 own, a block move where it is lower), the length is set, and every
                 element not kept is released where it stands (H-ClearRelease, per
                 element, its owned heap included).  t is never allocated.  The IDENTITY
                 form — every element kept on its own slot, then more appended — is
                 `V += more`.  The PREPEND form `t += g; t += V; V = t` (g not reaching
                 V) is the same rule read backwards: V's elements move UP by len(g) as
                 one block, back to front, and g's are written in front.  Declines: an
                 append order the walk cannot prove monotone; an element of V appended
                 twice; V read after the bind through a view taken before it; an
                 element type that owns a droppable (H-Drop: a release here is a
                 resource release, which keeps the written order of frees).
  (R-Const)      a function whose body is ONE literal over literals — `fn codes() ->
                 vector<integer> { [ … ] }`, or a record literal of such — reading no
                 parameter, is a CONSTANT: its value is built ONCE, in the const store
                 (read-only, H-WriteLocked), and every call answers a VIEW of it
                 (O-Borrow: deps → the constant), never a fresh store.  A plain bind of
                 that view (`c = codes()`) is a view too (skip-free) where c is only
                 READ in its frame — indexed, iterated, handed to a `const` parameter or
                 to a callee that does not write its parameter (the callee's own fact,
                 one call deep, as R-Callee reads it) — and is B-Copy's copy the moment
                 c is written, rebound to a non-constant, returned, stored into a field
                 or element, captured, or handed to a parameter the callee writes: the
                 copy is placed at the bind, and the program cannot tell the two forms
                 apart (R-Escape).  A top-level `const` vector already lives in that
                 store; this rule gives a literal-bodied function the same home.
  (R-CopyView)   a record local t bound ONCE by B-Copy's copy of a record VIEW — `t = a`,
                 or a join `t = if c { a } else { b }` whose every arm is such a view — is
                 a VIEW of the chosen record (O-Borrow: t's deps name its sources; no
                 store minted, none freed) where no program can tell the two apart:
                 (1) t is only READ in its frame, in R-Const's sense (a field read, a
                 `const` argument, a copy into another such local) — never written,
                 rebound, returned, stored, captured or handed to a writing parameter;
                 (2) every source's view closes over PARAMETERS only, so the record
                 outlives the frame; and (3) nothing in t's live range — the statements
                 from the bind to t's last mention — can change the record's bytes, move
                 it or free it: every operation there whose heap operand's TYPE can
                 reach the source's record type (through fields and elements) only
                 reads, and every call to a loft-bodied function handed such an operand
                 hands it to a `const` parameter.  An operation reaches only the records
                 its operands' types can hold, so a push into a `vector<integer>` in
                 range cannot disturb a `Coord`, whatever store it lives in — which is
                 why (3) is asked of types and not of stores (`store_viewers` is an upper
                 bound on stores, and a parameter may share the caller's return buffer's
                 store).  Declines, keeping the copy: a nullable t or source; a `&`
                 source (C-Ref's copy is its own question); a source that is an owned
                 local (its free is scoped, the frame's is not); a source that IS a
                 parameter — R-ValueRecord may carry a small record parameter as a
                 tuple, where its copy already costs nothing and a view would need the
                 store the tuple removed; a struct-enum.
  (R-ValueLocal) a plain local bound from a call R-ValueRecord admits — the result a
                 no-heap record of at most eight scalars, nested or flat — carries the
                 record's fields as its TUPLE in the binding frame too, where every use
                 of the local is a field read (`v.f`, or `v.pos.x` through inline
                 sub-records at the summed offset), a scalar field WRITE at a constant
                 offset (the tuple element's assignment), a hand-off of the local or of a
                 SUB-RECORD of it as a by-value or `const` argument to another admitted
                 function (the fields cross the call as scalars), a copy FROM such a
                 sub-record, or a
                 rebind from such a call (`p = mat4_transform(m, p)`); and an exit that
                 answers a VIEW of such a record (`for c in self.cells { if … { return
                 c } }`, `return self.inner`) answers the tuple by one load per field —
                 no store minted, no copy at the caller.  The record is materialised at
                 the one place a representation is owed — a store into a field or an
                 element, a return from a function R-ValueRecord declines, a library's
                 exported API (R-Escape) — and nowhere else.  Declines: a `&` link to
                 the local; the local handed to a fn-ref; a nullable binding.
```

**The nested clause** (cells `tests/scripts/158-nested-value-record.loft` n1–n14,
the moved pin `tests/nested_field.rs` n12).  A record whose fields are scalars and inline
sub-records of scalars — `Vertex { pos: Vec3, normal: Vec3, uv: Vec2 }`, eight floats two
structs deep — rides the tuple like a flat one: `hoist::type_layout` recurses into an inline
sub-record and keys its scalars by the PARENT type at the summed offset, the ONE home a
result, a local and a parameter read.  A site's nested read folds its `OpGetField` chain to
that key (`hoist::view_field`, the same fold `(R-RecPtr)`'s path clause uses); a builder's
literal takes a sub-record COPY from a tuple source as that source's elements and a nested
in-place write at its summed offset (`Output::value_record_parts`); a sub-record of a value
local handed to a tuple parameter, or copied into a literal, is a RANGE of the tuple
(`hoist::sub_record`).  A view leaf stays a top-level field: a sub-record's heap field would
be a place two records deep no site gate proves.  What the nesting changed in the SOUNDNESS
half: a write through a sub-record is a write into every enclosing record, so
`hoist::setter_target` answers the chain — `s.a.pos.x = …` is `(Vec3, 0)`, `(Vertex, 0)` and
`(Seg, 0)` — and a target whose parent is out of sight (a `&` link, a parameter's view)
reaches every type that holds its record inline (`inline_parents`), which only ever declines
more.  Sabotaged (the chain struck), cell n12 reads 1 where the write through the parent
made it 7.  A bind FROM a sub-record (`q = w.pos`) is a VIEW by the oracle and keeps the
local a record (n10).  Trade to know: a callee's nested tuple parameter is filled at the site by one read
per scalar, so a store record handed to one costs eight loads where the callee may read two
(`nested_field` n12's row moved 2 → 9 reads through the address) — the flat rule's trade at
the new width.

**The body clause** (loft#1697; cells
`tests/scripts/a-literal-table-is-built-from-its-constant.loft`, switch `LOFT_NO_CONST_VIEW`).
A call `(R-Const)` must decline — the result stored into a field, returned, written — still
ran the function, and the function built its vector one push per literal: a 115 000-element
terrain table was a 115 000-line function.  The function's own body now builds its fresh
vector as ONE copy of its constant twin (`OpAppendVector(v, OpConstRef(k), elem)`, what
`v += K` lowers to, deep-copying a text element), because the twin is built from that very
literal.  Admitted where the body's vector block is exactly the parser's literal — its
declaration, an optional reservation, and statements that build its elements from constant
arguments (no variable, no call but a built-in operator: what the twin's extractor folds);
an element kind the constant store does not pre-build (an enum, a narrow integer) has no twin
and keeps its pushes.  Beside it, native emits every constant table whose elements share one
field layout as one static array per field and one loop (`write_const_columns`), the
per-element form kept for a table of mixed shapes.

**The own-buffer clause** (`LOFT_NO_REBIND_OWN_BUFFER`; cells
`tests/scripts/a-builder-rebinds-into-its-own-return-buffer.loft`, pin
`tests/rebind_own_buffer.rs`).  A builder promotes the local it returns onto its hidden
buffer, which made it an argument, and every argument was refused — so each `d = step(d, …)`
filled a pooled buffer and the bind copied it whole into `d`.  Built beside it, the live-view
half of this rule as the code had not enforced it: a view of x read after the call — a loop
variable over one of x's fields, read after the body rebinds x — now declines (it had
answered 3004 where the rule says 4; `tests/fixtures/rebind-view-across-call.loft`), which
cost no library admission.

**In words.**  The rules above this section build NEW bytes where they will live:
`(R-Place)` hands a call the place its result is going, `(R-ElemFirst)` builds a vector
inside the element that consumes it, `(R-InPlaceLiteral)` writes a literal into its slot,
`(R-RetAdopt)` builds a result in the buffer it is returned through, `(R-MoveAppend)`
relocates a dying temporary.  The wide pass (`bench/portal/analysis/
libraries-wide.md`) measured a second class those cannot reach, and it holds the rows
furthest from Rust: bytes that ALREADY EXIST in a store the program still owns, copied
whole into a second store because the language's plain bind copies (`(B-Copy)`).  The
four rules here each name one shape in which the value's home is already there, so the
copy has no work to do — C125's direction, *remove the temporary*, applied to a temporary
that is a copy rather than a mint.  Their common floor is `(H-CopySelf)`: a whole value
delivered onto the place that holds it is a no-op the runtime decides by identity, which
is what lets a rule hand a value's own home as its destination without a second answer
to bisect to.

- **`(R-Rebind)`** — the immutable-update idiom, `doc = delete_range(doc, a, b)` with the
  callee ending in `Doc { buf: d.buf, pieces: np }`: zttext `delete_range` **138×**,
  `set_style`, and `invert`'s `cur = apply_op(cur, op)` (118× with `(R-Compact)`); the
  `buf` field (100 000 characters) is copied per edit today — TWICE: the exit mints a fresh
  `Doc` and `vector_add`s `buf` into it, and the call site deep-copies that result into `doc`
  again — and is H-CopySelf under the rule, so an edit costs O(pieces) as the Rust twin's
  does.  `mat4_mul` (37×) DECLINES —
  it fills its result before reading `mb` — and stays where it is: its cost is the mint
  of a 16-float vector per call, which no copy rule removes.
  **BUILT** (`LOFT_NO_REBIND_PLACE`, `LOFT_TRACE_REBIND`), in three pieces.
  The parser (`Parser::rebind_safe_literal`, on every exit literal built into an offered
  buffer): a vector field's `OpAppendVector` into the buffer becomes `OpReplaceVector`,
  whose runtime self-test is `(H-CopySelf)` and whose clear releases (`(H-ClearRelease)`,
  `Stores::vector_replace`), and its zeroing default is dropped; a SCALAR field expression
  that reads a by-value parameter of the buffer's type is staged into a temp ahead of the
  first write (native's value-record tuple emits those temps ahead of the tuple); and a body whose LAST statement is an explicit `return` takes the buffer road as well as a bare tail.  `return <by-value
  parameter>` is admitted as an exit beside the literals.  The scope pass
  (`src/rebind_place.rs`, after `place_result`): at `x = f(x, …)` the hidden buffer
  argument becomes `x` — the one IR change; the `Set` stays, since the bind's identity
  test already frees nothing when the result is its argument.  Admission reads the
  callee's IR (the parser lowered it first): every exit answers the parameter or a literal
  into `__retbuf` whose writes consult the parameter only through the staged temps or the
  same-field self-view replace, and no vector field keeps a zeroing default.  DECLINES,
  measured on the cells: a text or reference field read off the parameter (not staged),
  a vector read at another of the parameter's fields (the two-vector swap), a vector
  field built from a literal list or omitted, a chain exit, a promoted buffer, the local
  handed in twice, a view local, a witnessed local, recursion.  21 sites admitted across 6 corpus files; the cells
  `a-rebind-from-a-call-taking-the-same-local-keeps-its-values` c1–c20 pass on both
  backends under the falsifiers, and a build with the staging and the consults-the-
  parameter decline struck fails them.  **Found on the way, and fixed:** the interpreter's
  pre-Set free of a local rebound from a CALL released the store the local was displacing
  before the call ran — and once a callee builds into a pooled buffer (`OpClear` on
  re-entry), that store IS the buffer, so the second pass of `x = a ?? mk(i)` wrote its
  record into a freed store (`a-join-bound-local-owns-what-it-was-handed` jo2 read a stale
  id; garbage under `LOFT_POISON`).  Native had always freed by identity after the call.
  The Set now takes the guarded post-free (`state/codegen.rs`, `@FR-O-Buffer`); the
  bare-tail road had carried the defect since @PLN157 § V, pinned by
  `a-pooled-buffer-outlives-the-reassign-of-the-local-it-fills`.
- **`(R-Compact)`** — a vector rebuilt from its own elements, `te_new += [h.entries[i]]
  … h.entries = te_new`: dryopea `truncate_to` / `drop_oldest` **178×** (a prefix, a
  suffix; every entry owns two vectors, copied per push today, untouched in place),
  zttext `invert`'s prepend (118×) and `insert_text`'s identity copy (95×; its other
  half, a `character` push holding no push header on native, is closed — `OpPushCharacter`
  and `OpPushInt4` joined `hoist::FUSABLE_PUSHES` ).  The runtime lacks the primitive
  (a range compaction: `remove_vector` shifts one element and releases nothing,
  `clear_vector_release` releases all) and its parts all exist; the 22-line composition is
  what the build adds.  Found on the way: `clear_vector_release` derives the element type
  only under a one-field `main_vector<T>` root, so a rebind of a vector FIELD of a
  multi-field record released nothing and stranded every old element's owned heap in the
  store — fixed beside this price (see heap.md's register).
  **BUILT, the CONTIGUOUS-RANGE form** (`LOFT_NO_COMPACT`, `LOFT_TRACE_COMPACT`;
  scope pass, BOTH backends): `t: vector<S> = []; for i in a..b { t += [V[i]?]; } V = t;`
  becomes ONE guarded op — `if 0 <= lo && lo <= hi && hi <= len(V) { OpKeepVectorRange(V,
  tp, lo, hi) } else { the statements as written }` — where the op (`Stores::keep_vector_range`,
  one home for both backends through its `#rust` template, beside `remove_vector_at`)
  releases every element outside `[lo, hi)` where it stands, moves the run to the front as one
  block and sets the length.  The fallback arm is the program as the parser lowered it, so a
  range the guard refuses — a negative start, an end past the length (whose `?`-discharged
  reads pad with default records), an end below the start, a null bound — answers exactly
  what it always answered; the guard reads the range, not the walk, so the rule's "at every
  append the count already in t is at most the index being read" is a fact of the range
  itself here.  `src/compact.rs` matches, on the settled IR, the declaration `t = []` (a
  mint, the field read, the length word), a counted `for` over `a..b` in either range
  prelude (a literal or a variable end), a body appending exactly the `?`-discharged RECORD
  element `V[i]` of the loop variable, and the rebind `V = t` in its field form (snapshot,
  clear, copy back; the snapshot's free beside it or deferred to the block's end, in which
  case it joins the fallback arm) or its local form — with `t` named nowhere else in the
  function and the bounds a literal, a variable the loop does not write, or `len(V)`.
  A SCALAR element keeps the rebuild: an in-range scalar can hold the null its `??`
  replaces, where an in-range record element is never absent — the one difference between
  the two forms the guard cannot see.  Not yet built: the FILTER form (`for e in V { if p {
  t += [e] } }`, a loop rewrite with a write index), the PREPEND form (`invert`) and the
  IDENTITY form (`insert_text`), which the rule states and this unit's recogniser does not
  reach.
  Cells: `tests/scripts/a-vector-rebuilt-from-a-run-of-its-own-elements-is-compacted-in-place.loft`
  (c1–c18, hand-computed, both backends under `LOFT_STRICT_STORES` / `LOFT_POISON` /
  `LOFT_POISON_CLAIM` / the native leak check, the switch A/B answering the same); the
  admission is pinned on the emitted Rust in `tests/compact.rs`.
- **`(R-Const)`** — text2d `write_text` **86×**: `face_codes()` / `face_rows()` return
  vector literals of 56 and 392 elements, rebuilt for every glyph drawn and read only;
  moros `panel_build` (19×) copies a constant `vector<text>` into a list box per frame,
  which is a STORE into a field and keeps its copy under the rule — the rule removes the
  rebuild, not the author's choice to store it.  The caller's bind had NOT
  copied — the emission binds the callee's buffer directly — so the second half of this
  rule strikes a BUFFER, never a copy, and its landing note must say so.  What remains is
  the per-pixel `set_pixel` (the call class) and one emitter waste found on the way: the
  fused element write derives `text_span_of` on every call for an INTEGER element and
  never uses it (9–13 % of the row).
  **BUILT** (`LOFT_NO_CONST_VIEW`, `LOFT_TRACE_CONST`; parse time + scope pass,
  BOTH backends), on the machinery a top-level constant already has rather than on a
  `OnceLock`: the parser gives a zero-parameter function whose whole body is ONE vector
  literal over literals a synthetic `DefType::Constant` twin holding that literal
  (`Parser::literal_body_constant`, linked through `Definition::literal_const`; the twin is
  added on pass 1 and its literal re-stored on pass 2 like `parse_constant`'s), which
  `compile::build_const_vectors` and the native `emit_const_vectors` pre-build ONCE in
  `CONST_STORE` from the same extractor — so an element the constant store cannot hold
  (`const_elem_unsupported`, `const_vector_blocker`) simply leaves the function a function.
  The scope pass (`const_fn::rewrite`, beside `(R-Place)` and `(R-Rebind)` on the settled
  IR) then answers each call whose result lands only in READ positions with `OpConstRef`,
  the node a top-level constant's use site emits, and removes the call's lazy mint guard
  where the buffer has no other user; the bind, the index, the `len` and the iteration that
  follow are the forms both backends already emit for `NAMES[i]`.  The admission is
  `use_analysis::read_only_uses`, the walk behind `(R-ValueRecord)`'s read-only record
  locals generalised to classify a marked CALL node by the same positions — arg 0 of a
  scalar getter or of a projection chain ending in one, or the value of the single bind of
  a local that is read-only — with two extensions the plain form keeps off so its own
  answers never move: a block in argument position hands its tail on (the `__ref_p2_N`
  inline container of `codes()[i]`), and a bare copy `w = v` is an ALIAS of `v` rather than
  a reach, which is how `for x in c` (lowered to `_vector_N = c`) reads `c`.  Every other
  position keeps the call — a write or append through the local, a hand-off to ANY user
  call (a by-value vector parameter is written through), a store into a field or element, a
  return, a `?`-discharged record element (its absent arm mints), a `&` link — because the
  constant store is write-locked and a program's copy must be its own; a wrong decline is
  the build the program already paid.  The second half of the rule as written — the bind
  of the view is B-Copy's copy the moment the local is written — lands as that decline:
  the copy is the call's own store, so no copy road was added.  Cells: `tests/scripts/a-literal-bodied-function-is-a-constant.loft`
  (c1–c24, hand-computed, both backends under `LOFT_STRICT_STORES` / `LOFT_POISON` /
  `LOFT_POISON_CLAIM` / the native leak check, the switch A/B answering the same); the
  admission is pinned on the emitted Rust in `tests/const_fn.rs` (which calls read the
  constant store, which keep their call), since the values pass on either form.
- **`(R-CopyView)`** — graphics `fill_polygon` **22.6×**: `polygon_crossings` picks each
  crossing edge's top and bottom vertex with `pc_ty = if pc_a.py < pc_b.py { pc_a } else
  { pc_b }`, and each of the two binds minted a store, copied a `Coord` into it and freed it,
  per crossing per scanline — not the crossings vector the overview first blamed, which the
  return buffer already reuses.  **BUILT** (`src/copy_view.rs`, after `(R-Const)` on the settled IR; `LOFT_NO_COPY_VIEW`, `LOFT_TRACE_COPY_VIEW`).  Over the benches it admits exactly
  those two locals: the 303 record copies there are mostly a PARAMETER copied (declined —
  `(R-ValueRecord)`'s tuple already makes that copy free, and the rewrite census caught the
  first cut taking eight of `server`'s tuple parameters away, R-ValueLocal 15 → 7) or a
  source the frame owns.  Cells: `tests/scripts/a-read-only-record-copy-is-a-view.loft`
  (k1–k11, hand-computed, both backends, switch A/B, `LOFT_POISON` + `LOFT_STRICT_STORES`),
  the admission pinned by `tests/copy_view.rs`.
- **`(R-ValueLocal)`** — `(R-ValueRecord)` returns a small record in registers only where
  EVERY call site reads fields off it; a site that binds the whole record to a local, or
  rebinds its own argument, declined it and paid a store per call: mesh3d
  `mat4_transform` **22×** (`p = mat4_transform(m, p)`) and `sphere` (23×: `vec3`,
  `normalize3`, `vec2`, `vertex` per vertex), moros `resolve_move` (7.5×: `vec3`, a `Hex`
  answered as a VIEW of an element, a `HexAddress` from `world_to_hex`) and
  `emit_to_material` (14×), hex_body `bone_shape_has` (69×, three records per query).  The decline
  `(R-ValueRecord)` makes today is its SITE gate — *a site consumes its record*: the local is
  handed on as an argument — and what the site pays is not a deep copy but a store MINTED
  and the previous one FREED per call (the buffer handed in is the null sentinel every
  iteration), the same class.  What remains is twelve loop-invariant element reads of the
  unwritten matrix per call: an instrument that hoists them reads ~1.2–1.5×, so the rule
  that would own it — a loop-invariant ELEMENT read of a held, unwritten vector, where
  `(R-Invariant)` covers integer chains only — is the next one to write for this class.
  **BUILT** (`LOFT_NO_VALUE_LOCAL`, `LOFT_TRACE_VALUEREC`; generation time,
  `--native` only, the interpreter the values oracle), as the TUPLE PARAMETER: a by-value
  parameter of a plain no-heap record of at most eight scalars is received as the tuple of
  its fields (`hoist::tuple_param_candidates`, decided in `value_records`' fixpoint beside
  the value locals, the layout per TYPE in `ValueRecords::types` — one home for a result,
  a local and a parameter of the same record).  The callee reads it as a value local (a
  field read is a tuple index, a copy FROM it materialises, `return p` is the tuple), a
  call site hands a value local or an admitted call as it is and reads the fields off any
  other record expression at the site (`Output::emit_call_arg`, the same question the site
  gate asked through `hoist::tuple_arg_ready`), the `__inv` twin takes no input for it,
  the live-reload arm materialises it into a record for the parked call, and the cdylib
  bridge reads the tuple off the record the C boundary hands (`hoist::tuple_reads`, one
  spelling).  Its soundness is the parameter's ALIASING: a by-value record parameter is a
  VIEW of the argument's place — `fn bump(p: V3, w: V3) { w.x = 5.0; p.x }` called
  `bump(a, a)` answers 5, the `slow-reference-parameter` advice says as much — so the
  callee's body, with every function it calls, must write no record of that type through
  any route a caller could hold a view of: the write set keyed by record type
  (`hoist::body_writes`), with a callee's return buffer and this frame's own records
  (`WriteSet::retbufs`, `WriteSet::own`: a buffer minted from its sentinel, a discharge
  buffer, a loop record, a released record) set apart from the `whole` writes that reach
  an existing place — the parameter gate asks `reaches_record`, the loop hoist's `evicts`
  still counts all of them.  The frame's own return buffer is seeded FRESH for that walk,
  so a literal exit and its sub-record copies contribute nothing; `(R-Rebind)` is what
  makes that exemption hold, since it hands a callee the caller's record as its buffer
  only where every read of the parameter is staged ahead of the first write.  The walk a
  body `body_writes` cannot type falls back to (`hoist::may_write_existing`) carries the
  same exemptions: a CALLEE's own return buffer is fresh in the callee's
  walk too, the frame's buffer stays fresh through a rebind of the local it was promoted
  onto (`to = vec3(…)` where `to` is the buffer), and a mint into a local that OWNS its
  store (`hoist::owning_local`) makes that local fresh for what is written into it — so
  `resolve_move(from: Vec3, …)`, whose body only builds `Vec3`s through `vec3`, carries its
  parameters, and `vec3` stops being consumed as a record at those sites (the moros/dryopea
  lane: `vec3` and `hex_to_world` tuples, four mints and frees fewer per `resolve_move`).
  Cells `tests/scripts/158-a-callees-own-buffer-is-a-fresh-record.loft` (c4b and c5b are
  the aliasing negatives: a parameter that names the record written reads the write back,
  which a tuple copy could not).  Declines:
  a parameter the body rebinds, a nullable or `&` one, a record with a view leaf, a
  function reached through a fn-ref dispatch, and any body the walk cannot type
  (recursion, a `CallRef`, a `par`, a local record literal written after its mint).  With
  it, a `__lift_` temp bound from a CALL is a value local (until then declined by shape),
  because a nested call argument — `vertex(vec3(…), …)` — is exactly such a lift and a
  tuple parameter is served only by a lift that is a tuple.  Two emitter facts the corpus
  walk found: the per-function value-local setup ran only for a program with an admitted
  FUNCTION, so a program with tuple parameters alone took the tuple in its signature and
  read the parameter through the store (E0609 over 75 scripts); and the three sites that
  spell a call themselves (`output_call_inner`, the adopt-or-copy bind, the hoisted-argument
  bind) each name the callee for the argument question, restored per argument because a
  nested call argument had left the outer call's callee naming the inner one.  A `(R-RecPtr)`
  view handed to a tuple parameter keeps its address for the hand-off, exactly as it did for
  the twin's inputs, and the site reads the fields through it.  The cells:
  `tests/scripts/a-small-record-parameter-is-carried-as-a-tuple.loft` (18, every alias
  route that must decline beside the shapes that admit; receipt: a sabotage of the
  write-set test fails c3 on native, `1` for `5`); the value-record pins
  (`tests/value_record.rs`) carry the three predictions the rule flips (a result passed to
  a read-only parameter, a chain to it, a lift into a field).  Falsified further by
  `LOFT_HOIST_VERIFY=1` on the bench (the twin clause: the tuple beside a held header),
  `LOFT_STRICT_STORES` / `LOFT_POISON` / the native leak check on every cell corpus, and
  the switch A/B.  Not yet: a parameter of a record with a view leaf, a copy of a
  parameter into a fresh local (`q = p` declines as an untyped mint), and the
  loop-invariant element reads the price named.

**The width, the discharged return, the field write and the terminal copy (@PLN158; cells `tests/scripts/158-a-seven-field-record-returns-by-value.loft`).**  Six
fields became eight — moros's `Hex` has seven, and a tuple past the register file is
returned through the caller's stack slot, still nothing beside a store record.  Three
admissions beside it, each the one thing that kept `map_get_hex` off the value path:
a `return v[i]?` exit (`materialized_view_return`) is the leaf its copy source is
(`hoist::mv_return_source`; the emitter returns that tuple and keeps the discharge
buffer's frees); a scalar field write on a value local is the tuple element's assignment
(`hoist::VALUE_RECORD_SETTERS`, the arm in `FusedElementWriteEmitter`); and the tuple
PARAMETER gate leaves out a whole-record copy that is the last statement before a bare
`return` (`hoist::without_terminal_copies`: nothing runs after it that could read the
parameter), and where `body_writes` cannot type the body — a callee that grows a
collection — asks the narrower question the tuple form needs, whether an EXISTING record
of the type may be written (`hoist::may_write_existing`: growth writes fresh slots only, a
set and a copy are asked their target's type, an unknown native answers yes).  The copy
FROM a value local derives its destination once (seven element lookups for one record,
`OpCopyRecordEmitter`).

**Landing (R-Switch).**  `(R-Rebind)`, `(R-Compact)` and `(R-Const)` are parse- or
scope-pass rewrites the interpreter shares, so their falsifier is the switch A/B over
the corpus and the consumer suites — `LOFT_NO_REBIND_PLACE`, `LOFT_NO_COMPACT`,
`LOFT_NO_CONST_VIEW` — with `LOFT_POISON` / `LOFT_STRICT_STORES` / the native leak check
armed on every cell, since three of the four move a RELEASE (the old field's heap, the
dropped elements, the copy that is no longer made).  `(R-ValueLocal)` is generation-time
(`LOFT_NO_VALUE_LOCAL`), and the interpreter is its oracle.  Each rule carries a cell
that only its gate can fail, built before the rule: for `(R-Rebind)` a callee that reads
its parameter AFTER filling the buffer (admitting it answers the fill, not the argument),
and a view of x live across the call; for `(R-Compact)` an append order that is not
monotone (an every-second filter read backwards, whose in-place form overwrites an
element before it is read); for `(R-Const)` a write through the bound local, which must
copy — and H-WriteLocked is the runtime backstop, a fault rather than a silent write into
the constant; for `(R-ValueLocal)` a local whose address escapes.  Hand-price each on the
emitted Rust of its headline row before any emitter code: `delete_range` with `var_d`
handed as the buffer and the `buf` copy struck, `truncate_to` as a length set plus the
per-element release, `write_text` with the two tables hoisted to statics,
`mat4_transform` with `p` carried as three floats — the price the twin sets is the
ceiling each is measured against.

## The interpreter's emission — fewer ops for the same program

Applied by the interpreter's bytecode generator or runtime (`(R-Phase)`); native never sees
them and is the oracle of every cell (`(R-Switch)`'s interpreter clause).  Each REMOVES ops
or work; none merges ops into a new one (INTERPRETER_PERFORMANCE.md § Why the interpreter is
optimised at all): an op merged per shape is fast for that shape only and grows the
instruction set.

### An integer operator over locals and literals is one op

```
  (R-Fuse)       an integer operator whose operands are frame locals or a literal —
                 `a op b`, `a op 7`, a store `x = a op 7`, an `if`/loop test and its
                 jump, a text walk's step and end tests, an integer element of a local
                 vector at a local index, `for x in v`'s end test — is emitted as ONE op
                 that reads its operands in place and calls the unfused operator's own
                 function: what is computed is unchanged, only where the operands come
                 from.  Every position is taken at the stack height the op STARTS at.
                 THE MIRROR CLAUSE: a COMPARISON whose literal stands on the LEFT is
                 mirrored so the local reads first — `c < v` is `v > c`, `c <= v` is
                 `v >= c`, `==`/`!=` swap — exact on `i64`, the null sentinel included,
                 because the unfused comparisons are plain `i64` compares.  An
                 ARITHMETIC operator is never mirrored: its overflow report names its
                 operands in the order written.
```

**In words.** Applied by: interpreter generator (`fusable_int`, `gen_if_test`,
`emit_fused_vec`, `hoist::char_walks`).  The mirror clause is what makes the end test of a
counted range over a literal end (`if 100000 <= i break`) one compare-and-jump: it was four
ops a round in every `for … in 0..LITERAL`.  It extends the fused compare's KINDS (`GT`,
`GE`), not the op set.  Effect: 3 ops a round on a literal-ended loop; the `12_drawing` hash
loop −5 to −8 % (pinned layout).  Switch `LOFT_NO_FUSE`.  Guard
`tests/scripts/an-integer-operator-over-locals-runs-as-one-op.loft` (`test_literal_on_the_left`).

### A reservation the append repeats is not emitted

```
  (R-FirstClaim) an `OpPreAllocVector(v, n, size)` before a literal append of n <= 11
                 elements to a plain local `v` is emitted as nothing: it claims a record
                 only for an ABSENT vector, `max(n, 11)` elements wide, and does nothing
                 to a vector that has one; the append's own first claim (`vector_append`,
                 reached by `record_new` for every vector element kind) is the same 11
                 elements.  Capacity is not observable — `len` reads the length, `size`
                 multiplies it by the stride.  n > 11 keeps the op (it widens the
                 first claim), and the IR keeps it in every case: it is the head and the
                 stride native's append-group recognisers read.
```

**In words.** Applied by: interpreter generator (`generate_call`).  The same FACT native's
generator applies under a held push header (`(R-Push)`: "emitted as nothing under a held
push header") — one rule per fact, a clause per phase (`(R-Phase)`).  Effect: two ops per
push, every iteration of a push loop; `push` −16 %, a record append −7 %.  Switch
`LOFT_NO_PREALLOC_ELIDE`.  Guard `tests/scripts/a-literal-append-claims-its-vector-once.loft`,
pin `tests/prealloc_elide.rs`.  What it would break: an append path that does NOT claim for
itself on an absent vector — the planted defect the guard catches.

### A counted loop's variable lives in its index's slot

```
  (R-LoopSlot)   `for i in a..b` steps a hidden index and copies it into `i` every
                 round.  Where nothing but that copy writes `i` — exactly one `Set`, its
                 iterator's, and no `OpCreateStack(i)` (a `&integer` argument or a `&`
                 link) — and nothing in the body writes the index or takes its
                 address, `i` and the index are ONE value under two names: `i` takes
                 the index's slot and the copy is not emitted.  The index is bound
                 before the loop and read by every round's test, so it straddles the
                 loop and I6 keeps every other local off its slot.  Every reader of the
                 layout reads the pair as one value: the slot validator (I1, I6), the
                 debugger's frame view (neither is `<reused by>` the other), an undo
                 entry across a step.
```

**In words.** Applied by: interpreter slot allocator and generator
(`slot_alias::range_slot_aliases`, the one home, read by `assign_slots_v2`,
`validate_slots`, `frame_view`; the copy is dropped where the two POSITIONS agree, so a
table without the decision keeps it).  The one visible difference is in the live debugger:
editing `i` edits the loop's counter, as a C `for` would.  Effect: two ops a round; a tight
`for` −20 %, `push` −17 %.  Switch `LOFT_NO_LOOP_VAR_ALIAS`, trace
`LOFT_TRACE_LOOP_VAR_ALIAS`.  Guard
`tests/scripts/a-counted-loop-variable-shares-its-index-slot.loft`, pin
`tests/loop_layout.rs`.

### A loop whose first statement is its exit runs it at its bottom

```
  (R-Rotate)     a loop whose FIRST statement carries its exit test — a counted range's
                 iterator `v = {step; if c break; …; index}`, or a `while`'s
                 `if !c { break }` — is laid out with the test after the body: one jump
                 enters at the test, the test jumps back to the body while the loop goes
                 on, and falls through where the `break` would have jumped.  Earlier
                 `if … break`s of the iterator (an inclusive range's stop) are emitted as
                 they were, at the test; the statements after the test (a two-counter
                 iterator's steps, the loop variable's store unless `(R-LoopSlot)`
                 holds) open the body.  A `continue` jumps FORWARD to the test, patched
                 when it is emitted.  The test keeps the line the loop started on.
```

**In words.** Applied by: interpreter generator (`gen_rotated_loop`, `rotation`).  The round
count is what it could get wrong, and every guard cell counts rounds: a loop that must not
run at all, a `continue` that would skip the step, a `while` condition evaluated n + 1
times.  Effect: one jump a round; −5 to −7.5 % on tight loops.  Switch `LOFT_NO_LOOP_ROTATE`.
Guard `tests/scripts/a-loop-tests-at-its-bottom.loft`, pin `tests/loop_layout.rs`.

### A frame records where it was called from, not the line

```
  (R-CallLine)   an interpreted call records its call POSITION; the source line of a
                 frame is derived from it when a stack is rendered (`stack_trace()`, a
                 fault's frame chain, the debugger), by the same lookup the call made:
                 the nearest line entry strictly before the position (loft#1753).  A
                 frame with no call site (`call_pos` 0) answers line 0, as before.
```

**In words.** Applied by: interpreter runtime (`State::call_line`).  The line is a
representation of the position (`(R-Escape)`), so deriving it late changes nothing a
program or a report can observe.  Effect: one BTreeMap search per call; recursive
fibonacci −24 % (pinned layout).  No switch: there is no second form to keep.  Guards: the
stack-trace cells (`1753-…`, `55-stack-trace`, `117-deep-stack`, `1806-…`) and
`runtime_errors`, `frame_readers`.

### The dispatch loop tests one flag, set where each rare event happens

```
  (R-DispatchStop) the lean dispatch loop tests ONE flag after each op (`Stores::
                 dispatch_stop`) instead of each event that ends or diverts it, and
                 each event sets the flag where it happens: a runtime error in its one
                 store (`Stores::raise_runtime_error` — no other assignment of
                 `runtime_error` exists); a frame yield, a native's runtime error and a
                 `par` worker's fatal after a native returns (`State::invoke_native`, the
                 one path of every native call); a worker's fatal after the `parallel`
                 block's join; a debugger or profiler arming (`enable_debug`,
                 `arm_profiler`).  On the flag, a cold path does what the per-op tests
                 did, in their order — a frame yield returns, a worker's fatal is raised,
                 a runtime error halts with its frames, an attached debugger hands the run
                 to the full loop — and RE-DERIVES the flag from the events, so a nested
                 loop never clears one its caller still needs.  The end of the run needs
                 no test: a halt and the entry function's return set `code_pos` to
                 `u32::MAX`, which the loop condition ends.
```

**In words.** Applied by: interpreter runtime (`execute_argv`'s lean loop, `State::lean_stop`).
The five events cannot start between two plain ops — each needs a native call, a raise, a join
or a `&mut State` method — so testing each after every op paid for nothing, and it was 15–24 %
of a loop's time.  What it could break is an event that does not reach the loop: a fault that
does not halt.  Effect (pinned layout): the `12_drawing` hash loop −21 %, `record_walk` −20 %,
`push` −17 %, `index_read` / `index_write` −13 %, recursive fibonacci −5 %.  No switch: the
per-op tests are not a form to keep.  Guards `tests/dispatch_stop.rs` (a native and an op raise
inside a loop halt at once; `runtime_error` is stored only by its setter), `dispatch_reentry`
(the frame yield), `runtime_errors`' worker-fault cases (a `par` worker's fault, a `parallel`
block's).  Not taken beside it: reading the op byte unchecked (−1 to −3 %), which drops the
operand bound's guarantee.  The flag is an `AtomicBool` (a relaxed load is the same instruction as a plain
one): the timeout's watchdog sets it at the deadline through a pointer the running loop
publishes (`timeout::publish_stop_flag`), and `lean_stop` exits gracefully, naming the frame
that runs — so a loop stops at the deadline whether or not it calls.  Guard
`tests/timeout_breadcrumb.rs` (a loop without calls stops gracefully; a native hang is still
hard-killed), falsified by the watchdog never setting the flag and by the loop never
publishing it.

### The stack is addressed through one cached base

```
  (R-StackBase)  the interpreter's fast stack path addresses a slot as ONE add to a base
                 pointer cached in `State` (`stack_base`), instead of re-deriving it
                 through the store table, the store and its buffer on every access.  The
                 base is re-derived wherever the stack store's buffer can move — at
                 construction, by `grow_stack`, by `claim_in_stack` (a `par` worker's text
                 work buffers), by a checkpoint restore (which replaces every store) — and
                 every OTHER buffer move refuses the stack store (`Store::stack_buffer`:
                 `claim_grow`, `adopt_image`, `reclaim_tail`, `take_slot`, `take_store`
                 panic on it), so a missed re-derivation is a refusal at the move, never a
                 read of a freed buffer.  A snapshot copy keeps the mark; a locked or
                 borrowed copy for a worker does not (the worker has its own stack).
```

**In words.** Applied by: interpreter runtime (`State::stack_slot`, `stack_base_of`).  A simple
op is latency-bound: the three dependent loads per stack access were on its critical path, and
removing them took −10 % cycles with −16 % instructions where removing a third of the
re-derivations per op (−8 % instructions) took nothing.  The refusal found the convention false
on its first run: a `par` worker claims its text work buffers IN the stack store, which can grow
it — that path is `claim_in_stack` now.  A stale base is invisible while only the fast path
uses it (the freed buffer still holds what it wrote), so the guard reaches one slot both ways.
Effect (pinned layout): −8 to −17 % per routine.  Not available in a debug-assertions build,
which runs the checked stack path.  Guards `tests/scripts/a-stack-that-grows-mid-call-keeps-every-frame.loft`,
`rpc.rs`'s continue-after-step-back cell, `par_nested`, the `store::tests` refusal pair.

### A crash context is derived, not written per op

```
  (R-DispatchPublish)  the lean dispatch loop publishes ONE position per op — `alloc_pc`,
                       which the allocator reads anyway — and registers that field and its
                       bytecode with the crash report once per loop (`LeanSource`).  Every
                       reader of "the op this thread is on" (`last_context`, `last_op_name`,
                       the signal handler, the panic hook) derives the opcode from those two;
                       the registration's drop publishes the last op as a written context and
                       restores the registration it replaced, so a reader after the loop, and
                       a caller's loop after a nested one, still name their op.  The full loop
                       keeps writing its context per op (it tracks the function).
```

**In words.** Applied by: interpreter runtime (`crash_report::current_ctx`).  Allowed because
the position is already written per op for the allocator, and the bytecode the loop runs is a
field of the `State` that does not move while it runs.  Effect (pinned layout):
−4.6 to −6.4 % instructions and −2.4 to +1.2 % cycles — the write was retired by the store
buffer beside the op chain, not on it, the same latency-bound picture as removing a third of
the stack derivations.  Kept for the instructions it frees, not for time it bought.  Guard
`tests/dispatch_publish.rs`: a native asks mid-run which op it is called from (two calls, two
positions), and the last op is named after the run.

### The bytecode is read through one cached base

```
  (R-CodeBase)  every read of the bytecode on the run path — the opcode fetch, each operand
                (`State::code`), an inline text (`code_str`, `string_from_code`) — goes
                through a base pointer and a length cached in `State` (`code_base`,
                `code_len`) instead of `State::bytecode`'s `Arc` and `Vec`.  The cache is set
                at construction and re-derived by `State::edit_code`, the ONE writer of the
                bytecode: a write that grows the buffer, or copies it because a `par` worker
                still shares the `Arc`, moves it.  The length keeps its check on every read.
```

**In words.** Applied by: interpreter runtime.  Allowed because nothing else writes the bytecode
(`tests/code_base.rs` checks it over the source) and the cache lives in `State`, not in a loop
local, so code written while a loop runs (a REPL input, a live reload, a debugger `eval`) is read
as written.  Effect (pinned layout, against the same tree without it): the opcode
fetch went from four dependent loads to two and instructions fell 1–2 %; time moved by +0.9 %
at the geomean, inside the noise — the fetch chain was not the serial path, because the
indirect call is predicted and the core runs ahead of it.  Kept for the shorter code.  Guard
`tests/code_base.rs`: a REPL session defines 48 long functions after earlier runs (the buffer
has to grow) and reads old and new results back.

### The position and the stack top travel between ops in registers

```
  (R-RegisterTable)  the lean loop passes each operator the bytecode position and the stack
                     top as arguments and takes them back as its result (`fill::OPERATORS_REG`,
                     an entry `<op>_r` per operator).  The entry writes both into `State`
                     (`State::regs_in`), runs the operator's direct-path body inlined, and
                     returns what `State` then holds (`State::regs_out`), so `State` is in step
                     after every operator and nothing that reads it between ops changes.
```

**In words.** Applied by: interpreter runtime, the lean loop with the direct stack path
(`(R-FastTable)`); every other loop keeps the plain tables.  Allowed because the entry is the
plain operator with two stores in front: inlined beside them (`s: &mut State` is unaliased), the
body's reads of `code_pos` and `stack_pos` fold to the incoming registers, and the stores remain.
What it removes is the store-then-load of the two fields from one op to the next, which put every
op behind the previous op's write on the critical path.  Effect, against the plain table in the
same binary: the dispatch-bound bench rows −11 to −20 %, a call-bound row unchanged.
`LOFT_NO_REGISTER_TABLE=1` dispatches the plain table.  Guarded by every interpreted test, and
by `tests/scripts/a-hot-operator-answers-the-same-inline-as-through-the-table.loft` with
`LOFT_NO_HOT=1`, which runs every operator through these entries.

### An op's operands are checked once

```
  (R-OperandSpan)  an operator whose operands all have a fixed width bounds-checks them
                   ONCE against the bytecode's length and advances the position past them in
                   one step (`State::operands`); each operand is read at its constant offset
                   (`Operands::get`).  An operator with an operand of no fixed width reads each
                   operand on its own (`State::code`), as before.
```

**In words.** Applied by the `fill.rs` generator (`create::generate_code_into`), so it holds for
every table.  Allowed because the operands are contiguous and their widths are known when the
operator is generated: one check over their sum fails exactly when one of the per-operand checks
would have, and the position the body sees is the same.  What it removes is a compare, a branch
and a `code_pos` store per operand — four of each in `OpIntVCPut` — and the stores mattered
most: each one has to happen before a check that can panic, because an unwinding `State` is
observable.  Effect: sum_loop's kernel 542 → 483 instructions an iteration, a call 656 → 604.
Guarded by `tests/issues.rs::fill_rs_up_to_date` (the generator's output is the committed
table) and by every interpreted test.

### A hot operator runs inline, on the loop's own registers

```
  (R-HotInline)  an operator marked `#hot` in `default/` is generated a second time against
                 `State::Hot`, whose position and stack top are LOCALS, and the lean loop runs
                 it inline (`fill::dispatch_lean`, a `match` on the opcode).  `Hot` offers only
                 the operands, the four stack accessors, the jump target and
                 `raise_recoverable`, so a template that needs more does not compile as hot.
                 `State` is brought up to the registers before anything else reads it: the
                 next non-hot operator's entry (`(R-RegisterTable)`), the stop path, the
                 loop's end, and inside `Hot::raise_recoverable`.
```

**In words.** Applied by the `fill.rs` generator and the lean loop (`State::lean_register_loop`,
its own function so its register allocation is not shared with the rest of `execute_argv`).
Allowed because a hot body is the same template as the table's body, written against an
interface that has nothing else in it, and because the only readers of `State` between ops are
the four named above.  What it removes is what `(R-RegisterTable)` cannot: `State`'s fields are
reloaded after every write through the stack's raw pointer, which for all the compiler can prove
may point into `State`, and a local is not; plus the call, its prologue and its return.  A
stack-underflow or operand report is out of line (`stack_underflow`, `code_out_of_range`): a
formatted `assert!` takes its argument's address and puts the whole view back in memory.
Effect, against `LOFT_NO_HOT=1` in the same binary: the dispatch-bound bench rows −15 to −22 %,
a call-bound row unchanged.  `LOFT_NO_HOT=1` runs every operator through the register table.
Guard `tests/scripts/a-hot-operator-answers-the-same-inline-as-through-the-table.loft`, with its
planted defects named in its header; the fused hot operators' own guard is
`tests/scripts/an-integer-operator-over-locals-runs-as-one-op.loft`.

### A `par` worker runs on the lean loop

```
  (R-WorkerLean)  a `par` worker's frame (`State::run_to_return`) runs on the lean register
                  loop whenever the main run's would (`State::worker_lean_ok`: no debugger,
                  no per-op instrument armed), with `#hot` operators inline as there.  A
                  worker's stop flag ends ITS loop only; a fault a worker raised is reported
                  by the main loop, not taken over by another worker.  The definition a
                  worker enters is found once per function position (`State::worker_d_nr`),
                  not by scanning every definition for each element.
```

**In words.** Applied by the interpreter's worker entry.  Allowed because a worker frame
executes the same bytecode as the main run, and the lean loop differs from the checked one only
in the per-op instruments it leaves out — which `worker_lean_ok` requires to be off.  The stop
path is the one place they differ: the main loop also takes over a fault some worker set, so the
worker instantiation leaves that test out (`lean_stop(true)`).  The remembered definition is
reset wherever `fn_positions` is rebuilt.  Effect on a one-thread `par` over a 1000-element
vector: 306 → 175 ms, the same as the loop without `par`; four threads 92 → 49 ms.
`LOFT_NO_WORKER_LEAN=1` runs a worker on the checked loop.  Guard
`tests/scripts/a-par-worker-runs-on-the-lean-loop.loft`; the stop path is falsified by
`runtime_errors::i1056_a_par_worker_fault_names_the_workers_own_frames` and the placement-parity
library-fault cells (the receipt is in the guard's header).

### An operator's priority decides its opcode's width

```
  (R-OpPriority)  opcodes are numbered by priority: the `#hot` operators take the first
                  slots, the unmarked ones the next, the `#cold` ones the last, each class in
                  declaration order.  The generator lays every table out in that order and
                  records the class sizes (`fill::OP_HOT`, `fill::OP_NORMAL`); `Data::op_code`
                  numbers a declaration from its class and those sizes.  A slot below 255 is a
                  one-byte opcode, so every hot operator has one; the generator refuses more
                  than 255.
```

**In words.** Applied by the parser (`Data::op_code`, at the declaration, after its annotations)
and the `fill.rs` generator (`create::operator_slots`).  Allowed because a number is only a slot
in the tables, and both sides compute the same order from the same annotations: every route that
parses `default/` (a directory, the embedded sources, the browser build) numbers it the same way.
`#cold` is for an operator whose own work dwarfs one byte — file I/O, whole-collection passes,
parsing, set-up — so that every unmarked operator still fits the one-byte range.  Effect:
sum_loop's kernel 424 → 408 instructions an iteration, a call 582 → 555 (its fused operators
lost their escape byte).  Guarded at every load by `stdlib_ops::verify`, which compares each
slot's declared name with the binary's table and refuses the run on the first that differs, and
by `tests/issues.rs::fill_rs_up_to_date`.

### An element in range is addressed in one straight path

```
  (R-ElementPath)  the interpreter's element read `v[i]` on a live vector with
                   `0 <= i < len` reads the collection record and the length ONCE each,
                   compares once (unsigned, so a negative index fails it too) and builds the
                   element's reference itself; an append whose element FITS reads the record,
                   the length and the capacity once each, writes the element in place and
                   derives the new length from the slot (`8 + len * size`) instead of reading
                   it again.  Everything else — null, absent, empty, counted from the end, out
                   of range, a new or full vector — takes the unchanged full path, so each
                   refusal keeps one home (`append_capacity`, `vec_get_or_raise_slow`).  And a
                   raw store read reads the record's size header only in a build that asserts
                   on it: the field read's own bound catches every failure that read did.
```

**In words.** Applied by: interpreter runtime (`State::vec_get_or_raise`,
`vector::append_slot_in_capacity`, `Stores::append_with`, `Store::valid`).  Allowed because
the fast paths answer exactly what the full paths answer on the cases they take, and decline
the rest.  The short path is inlined whole: a helper that returns a `DbRef` (or an
`Option<DbRef>`) through memory writes it as narrow fields, and the caller reads `rec` and
`pos` back as one 8-byte load, which the store buffer cannot forward — a stall of a dozen
cycles on every element (measured: 22 M blocked loads in a 20 M-push loop, +8 %
cycles, until both helpers were `#[inline(always)]` and the two append branches stopped
meeting in one `Option`).  Effect (pinned layout, against R-CodeBase): matrix_mul −16 %,
sort −16 %, index_write −13 %, a single-precision append loop −18 %.  Guard
`tests/scripts/an-element-read-in-range-answers-what-the-full-path-answers.loft`.

## A leaf's body in place of its call — four rules over one IR pass

Built in `src/leaf_inline.rs`, after `(R-ValueRecord)` at the top of
`byte_code_from`, so both backends see the result.  Found in the `12_drawing` hash loop, the
last routine above 100× its native time; each removes bytecode from the IR rather than cycles
from an op.  The loop's body went from 45 ops an element to 25 (+2 for the loop): the
inlined body, its literals folded, the second mask and the fallback gone, the last temporary
in its read, the scale in the divisor.  `hash` 146× → 79× native (pinned layout, the pass
switched off on the same binary as the baseline), `lock` −10 %, `fov_rays` −5 %; no other
routine's instruction count moved.  Hand-written forms had priced it at 72×: the difference is
two operands a correct fold may not regroup (below).

### A scalar leaf's call is its body

```
  (R-InlineLeaf) A call of a SCALAR LEAF — parameters, locals and result all integer,
                 float, single, boolean or character (never `τ?`); a body of assignments
                 and a result built only from operators, variables, literals, `if` and
                 nested blocks of assignments; a final `return` read as the result; at
                 most 120 nodes — is replaced in the IR by that body over fresh caller
                 locals.  An argument for a parameter the body never assigns is read IN
                 PLACE when it is a literal, or a plain variable when no argument holds a
                 call (none can then write it before the body reads it); any other
                 argument is bound to its local before the body, in argument order.
                 Literal operations the substitution makes adjacent fold — checked: an
                 overflow or a result equal to the sentinel is left to the operator — and
                 operands are never regrouped.  The body's line markers go.  Not in an
                 `open_world` program, not in one compiled for a run that observes
                 function entries (`Data::observes_entries`: `loft test`'s coverage), and
                 not into a generator or a `parallel` body.
```

**In words.** Applies in: the IR phase, both backends (native already inlined through LLVM).
**Regrouping is not exact**: every integer operator tests its result for the sentinel, so
`(a ^ x) ^ b` and `(a ^ b) ^ x` disagree when an intermediate is `i64::MIN` — the fold takes
`1 * 73856093` and `7 * 83492791`, not their xor across `i * 19349663`.  Coverage counts a function <!-- doc-lint: ok -->
entered by a call, so the test runner compiles with the calls kept — the value guards run the
inlined form as programs, and `tests/function_coverage.rs` failed with the pass on before the
flag existed.  The body's `Span` wrappers stay, and a fault's position comes from them, so a
fault inside the body keeps its exact position and loses only the leaf's frame, as
`(R-Leaf)` settled; with the line markers gone a frame's line is the call's; the debugger, which would want the
body's lines, runs `open_world` and keeps the call.  It reaches the stdlib too (`clamp`,
`approx` inline their `min`/`max`).  `LOFT_NO_INLINE_LEAF=1`; `LOFT_TRACE_INLINE_LEAF=1` names
each inlined call, each decline and each reduction below.

### A value's range removes the operations it makes redundant

```
  (R-MaskRange)  Inside an inlined body: `e & m` with `m = 2^k - 1` is `e` when `e`
                 provably lies in `0 ..= m`; and `{t = x / c (nullable); if t is not
                 null then t else d}` is `x / c` when `x` converts a ranged — hence
                 non-sentinel — integer and `c` is a finite non-zero literal.  The facts
                 are `(R-Range)`'s, carried statement by statement through the body: each
                 assignment ranges its local from the facts before it, so a local that
                 reads itself (`hx = (hx ^ (hx >> 13)) & m`) is ranged at each step; an
                 `if` keeps only what both arms agree on.  It extends `(R-Range)` by one
                 operator, for both backends: `a ^ b` over two non-negative ranges lies in
                 `0 ..= 2^k - 1`, `2^k` the least power of two above both maxima.
```

**In words.** Applies in: the IR phase; the xor clause also widens native's plain operators.
**A range inside `0 ..= m` is not enough for any `m`**: `5 & 6` is `4`, so only an all-ones
mask is the identity on its range.  The fallback needs the NON-NULL proof, which a value built
from a parameter never has (C80) — inside `seed_hash` alone it stays; after the inline, with
literal or ranged arguments, it goes.  `LOFT_NO_MASK_RANGE=1`.

### A value read once, right after it is written, is not stored

```
  (R-SingleUse)  An inlined body's last assignment `x = e`, to one of its fresh locals,
                 followed by the body's result reading `x` exactly once and as the first
                 thing that result evaluates: the result reads `e` in place of `x`, and
                 the store and the load go.
```

**In words.** Applies in: the IR phase.  "First" means no operator completes before the
read, so moving `e` crosses no effect; the fresh local is read nowhere outside the body.
A result that reads the local twice keeps it (substituting one read would leave the other
reading the value before).  `LOFT_NO_SINGLE_USE=1`.

**The statement clause** (`single_use::rewrite_program`): outside an inlined
body, a comprehension's element temporary `_comp_N` — assigned once, read once in the whole
function, never captured — whose value is pure (operators over locals and literals,
`same_read::pure`) is moved into the next statement's read when everything that statement
evaluates before the read is pure too.  A store and a load per element go.  The purity clause is
load-bearing: dropped, an element computed by a function that appends to a log moved as well —
the pin caught it.  `LOFT_TRACE_SINGLE_USE=1`.  Guards
`tests/scripts/a-comprehension-element-goes-straight-into-its-push.loft`,
`tests/single_use.rs`.

### A power-of-two scale folds into a literal divisor

```
  (R-ScaleFold)  Inside an inlined body: `x / c * m` (either operand order of the product)
                 and `x * m / c`, with `x` an integer conversion, `c` a finite non-zero
                 float literal with `|c| < 2^1021` and `m` a power of two of at least 2,
                 are `x / (c / m)` when `c / m` is exact and normal.  Then neither form
                 passes through a subnormal, and both round the one quotient `x·m / c`
                 once: scaling by a power of two is exact and commutes with rounding.
```

**In words.** Applies in: the IR phase; native gains too, because LLVM folds `fmul (fdiv x,
C1), C2` only under `reassoc`, which loft never grants.  It is not reassociation: `acc + (y -
1.0)` into `(acc + y) - 1.0` rounds differently and stays.  Checked bit-identical over the
hash's integer range; and the subnormal exclusion is real — an `x` whose quotient lies a
quarter grid step past a subnormal point answers `0x0.8000000000000p-1022` as `x / c * 2.0`
and `…0001p-1022` as `x / (c / 2)` (random samples miss it; a constructed one finds it).
`LOFT_NO_SCALE_FOLD=1`.

**Guards.** `tests/scripts/a-leaf-call-inlined-answers-what-the-call-answers.loft` (values,
both backends, the cells where each rule must NOT fire beside those where it does);
`tests/leaf_inline.rs` (which rule fires on which cell, read off the trace; every switch; the
hash loop calls no leaf); `leaf_inline::tests` (the fold never answers an overflow or the
sentinel; the scale fold's conditions).

## Proposed — the next eliminations, by reach

Found measuring what still keeps six routines near 100× native; ordered by how
many programs they reach, not by those six (the owner: *improve all loft scripts as
much as possible*).  Each carries **PROPOSED** until a site implements it; the two runtime
rules are priced before they are built.

### The fast-or-checked choice is made once per run

```
  (R-FastTable)  The stack access mode (`State::fast_stack`: the direct path,
                 or the checked path every debug instrument needs) is fixed for a run, yet
                 every push, pop and local read tests it.  Each operator body is compiled for
                 both modes (generic over a `const bool`) into two operator tables, and the
                 dispatch loop takes the one matching the run's mode.  The same operators,
                 never a new one; the mode never changes while a table is in use.
```

**In words.** Applied by: the fill generator (`create::generate_code_into` writes each op as
`fn op<const F: bool>` and respells its stack accessors `get_stack_m::<F, _>` and siblings, and
emits `OPERATORS` and `OPERATORS_FAST`, both `static`) and `State::op_table`, which the lean
dispatch loop asks once.  Other loops dispatch `OPERATORS`, whose accessors decide at run time
and are valid in every mode; ops that are `State` methods keep the runtime test.  Built
Measured: −4.7 % instructions, −5.2 % cycles at the geomean over 17 benches (pinned layout);
the bound measured by deleting the test was −8.2 %, the rest is the doubled operator code —
`07_string_build` moved +4.9 % in cycles with −0.3 % in instructions.  Guards
`tests/fast_table.rs` (under the stack census the operators still write through `put_stack`:
92.6 % of the writes, 6.6 % with the direct path planted into the checked table; the direct
and checked paths answer alike) and `state::fast_table_tests` (the table follows the mode;
caught the fast table planted into every run).

### A frame reserves its stack once

```
  (R-FrameHeadroom) Codegen records each function's highest stack position (the frame-
                 relative position `remember_stack` sees at every op), and the room a frame
                 needs is that plus a 256-byte margin (`State::frame_headroom`, shared with
                 `par` workers).  Every frame enters through `State::push_frame`, which grows
                 the stack store, once, to the frame's base plus that room; the direct-path
                 push (`put_stack_m::<true, _>`) tests no capacity.  Every other push — the
                 checked path, and the `State` methods (a call's return address, a native's
                 result) — keeps its test.
```

**In words.** Applied by: `State::record_frame_headroom` (at the end of `def_code`),
`State::push_frame` (the one site a `CallFrame` reaches `call_stack`), `put_stack_m`.  The
lean loop never runs under live reload, whose redirected function would carry another's
height.  The capacity compare was 18 % of the samples inside `get_float`.  Falsifier
**`LOFT_HEADROOM_VERIFY=1`**: the lean loop's stop flag is held set, so its cold path runs
after every op and checks the stack top against the running frame's room and the room
against the buffer — free when off.  The value cells cannot see a missing room on their
own: the store grows by 7/3 when it grows, so a push past the room usually lands in slack.
Guards `tests/scripts/a-frame-never-pushes-past-the-room-its-entry-made.loft` (a 100-deep
right-nested sum, position 608, atop a recursion 1500 deep and in a `par` worker) and
`tests/frame_headroom.rs` (only `push_frame` pushes a frame; the guard program under the
falsifier).

### A forward walk over a vector the body cannot resize is a counted loop

```
  (R-ForwardWalk) `for p in v`, walking forward, over a vector the body cannot resize,
                 runs as the counted range's loop: the length is read once before it, the
                 iterator is an `Iter range` block over the same `p#index`, the element
                 read — the reference, or a scalar's value read over it — opens the body,
                 and the `index < 0` test, which only the reverse step can make true, goes.
                 "Cannot resize", by shape: the walked vector (the source local and the
                 walk's `_vector_N` handle) appears in the body only as a direct argument of
                 an element read or of the length; neither they nor `p#index` is assigned;
                 no closure captures the source and the body calls nothing through a
                 reference; and when the source is a parameter, nothing in the body
                 receives a `&` parameter, which the caller may have passed the same vector
                 as.  Appending to the walked vector is refused by the compiler already.
```

**In words.** Applied by: `forward_walk::rewrite_program`, in the IR phase after the scope
pass (both backends), re-laying out the frame of each function it changes.  The length is
re-read today because `x#remove` shrinks the vector and steps the index back; the conditions
are exactly that no such write can reach the vector.  The `&` clause is load-bearing: without
it `both(w, w)` — a `const` parameter walked while the body appends to a `&` one — ran two
rounds instead of five and answered wrong with no diagnostic.  A result buffer the scope pass
promoted to a parameter is not a `&` parameter and does not decline (the caller fills it from
its own work buffer).  Native's held-base record address (`@FR-R-RecPtr`'s base clause) read
only the walk's own iterator, so it learned the counted form too (`hoist::element_binding`:
the element binding at any index local — the clause checks the index against the held length
itself); without it `entity_tick` ran +52 % and `record_walk` +28 % on native.  Built
Measured: the record walk −11 % instructions and −7.5 % cycles interpreted; on native
`record_walk` −12.7 %, −2.0 % at the geomean over benches 14 and 16 (the routines that moved
up hold no rewritten walk — emitted code placement).  `LOFT_NO_FORWARD_WALK=1`, `LOFT_TRACE_FORWARD_WALK=1`.  Guards
`tests/scripts/a-walk-the-body-cannot-resize-is-a-counted-loop.loft` (counted walks and the
walks that must keep re-reading, both backends) and `tests/forward_walk.rs` (which walk is
rewritten, read off the trace; the cells with the rewrite off).

### A record element its loop proves in range is read plainly

```
  (R-InRange)    Inside `for i in 0..len(v)` whose body can neither resize `v` nor assign
                 `i` or `i#index` (`(R-ForwardWalk)`'s body check), a discharge block over
                 a RECORD element of `v` at `i` — `v[i]?`, `v[i] ?? d` — is its element read
                 alone: an in-range element of a vector of records is a record, so the read
                 never answers null and the fallback is never minted.  Not for a scalar
                 element: a scalar's `?` also discharges an element whose VALUE is null
                 (C80), which no loop bound rules out.
```

**In words.** Applied by: `in_range::rewrite_program`, in the IR phase (both backends).  Both
conditions are load-bearing, each falsified with a silent `null`: reading another vector at the
same index (`another_vector` read null for 15), and dropping the body check (`v.clear()` part-way
read null for 10).  The record-only clause is enforced twice: by the read's shape (a
scalar's read is a value read over the element, never the bare element read) and by the
conversion test — the second is defensive, its plant passes.  Measured on `record_update` by
the hand-written form: −20 % cycles.  `LOFT_NO_IN_RANGE=1`, `LOFT_TRACE_IN_RANGE=1`.  Guards
`tests/scripts/a-record-element-its-loop-proves-in-range-reads-plainly.loft` and
`tests/in_range.rs`.

### A computed range start is one counter, stepped at the test

```
  (R-StartStep)  A range `s..e` whose start is not a literal (bound once to
                 `_range_start`, `@FR-I-Range`) iterates with two counters, `next` one
                 ahead of `i#index`: `{if e <= next break; i#index = next; next += 1;
                 i#index}`.  Where the loop rotates (`(R-Rotate)`) and nothing after the
                 iterator names `next` — nor `i#index`, unless `i` shares its slot
                 (`(R-LoopSlot)`) — the interpreter seeds `i#index = next` before the
                 loop, steps `i#index` at the test, and enters PAST the step: the first
                 round tests the start itself.  A `continue` jumps to the step.
```

**In words.** Applies in: the interpreter's bytecode generator (`start_step`,
`gen_rotated_loop`); the IR keeps both counters, so `--native` and every IR rewrite that reads
a range's shape see the form they always saw.  Allowed because after every round `next` equals
`i#index + 1`, and before the first it equals the start, so the test reads the same value it
read through `next`.  The step cannot overflow, since the round ran because `i#index < e`, and
a null start stays null under both steps.  An inclusive, reverse or filtered range keeps the two
counters.  What it removes is the copy from `next` into the index each round: `dot_product` 16
→ 14 operators a round, −15 % on the interpreter, with the same result.  The seeding is what an
IR form could not do: there, `start - 1` is the null sentinel for a start at `i64::MIN + 1`, and
a step at the bottom costs the jump it saves.  `LOFT_NO_START_STEP=1` keeps the copy;
`LOFT_TRACE_START_STEP=1` names each loop taken or declined.  Guard
`tests/scripts/a-computed-range-start-steps-one-counter.loft` (its planted defects named in its
header), pin `tests/start_step.rs`.

### An effect-free call made twice is made once

```
  (R-PureReuse)  A function is EFFECT-FREE when its result is its only effect: every
                 operator in its body reads, computes, or writes into a store the
                 function itself made (a write whose target is rooted at a parameter,
                 the function's own return buffer excepted, declines it); every function
                 it calls is effect-free (a fixed point over the call graph); and it
                 calls no function value, yields nothing and names no native outside a
                 short list of known value-only ones.  A PROJECTION WRAPPER is a function
                 whose body is one call of an effect-free function on its parameters
                 followed by a read of the result: `d = f(…); return d.k`,
                 `d = f(…); v = d.k; return v`, or the natural spelling, the read
                 written on the call (`f(…).k`, returned or as the body's value — a
                 record field through its materialised copy).  In one block, two calls of projection
                 wrappers of the same `f` on the same argument variables — the first in a
                 position its statement always evaluates (the statement, an `if`'s test,
                 an assignment's value, a call's arguments), nothing between them
                 writing a store, calling a function with effects or rebinding an
                 argument — are `f` called once into a fresh local before the first
                 statement, and each wrapper call is its read of that local.
```

**In words.** Applies in: the IR phase, at the start of the scope pass, both backends, so the
local it binds is owned and freed as any local the author wrote.  Allowed because an
effect-free call on unchanged arguments answers the same value each time and its only effect
is that value: the second call is the first call's value, read the second way.  The
wrapper's own copy of the field it handed back becomes a read of the local in place, which is
what the copy was of.  What it removes is the second call and everything it builds: a request
check that asked `pa_decode_ok(frame)` and then `pa_decode(frame)` decoded the frame twice.
The list of value-only natives is short on purpose — the `#impure` annotations do not mark every
effect (`print`, `file` and `env_variable` carry none) — so an unknown operator declines, which
costs the reuse and never an effect.  Effect, per `check_request` on the library as written,
native: 4 stores, 30 claims and 10 deletes → 3, 17 and 5, and −43 % time.  Switch
`LOFT_NO_PURE_REUSE=1`, trace `LOFT_TRACE_PURE_REUSE=1`.  Site: `pure_reuse.rs`.  Guard
`tests/scripts/a-pure-call-made-twice-is-computed-once.loft`, pin `tests/pure_reuse.rs`.

### Two equal reads in one statement are one read

```
  (R-SameRead)   Within one statement — an assignment's value, a `return`'s, or a block's
                 value — two discharge blocks (`v[i]?`, `v[i] ?? d`: `{t = <read>; if t
                 is not null then t else d}`) with the same read, the same conversion test
                 and the same default are bound once: the first block to a fresh local
                 before the statement, every equal one reads that local.  The read is built
                 only from read operators, locals and literals (compared with source
                 positions ignored); every call in the statement is a pure operator
                 (arithmetic, comparison, conversion, a read), so nothing assigns a local or
                 writes a store between the reads; a read inside an `if` arm is not
                 collected.  A discharged read raises no fault, so the faults reported are
                 the faults reported today.
```

**In words.** Applied by: `same_read::rewrite_program`, in the IR phase (both backends; native
gains nothing measurable, LLVM already merged the reads).  Each `v[i]?` is the read plus a
five-op discharge.  Both clauses are load-bearing, each falsified with a silent wrong answer:
the default ignored made `(v[i] ?? 1) * 10 + (v[i] ?? 2)` read 11 for 12, and a user call
taken as pure made `v[0]? + set_first(v) + v[0]?` read 198 for 103.  The first
version compared reads exactly and bound nothing: the two spellings carry different source
positions.  Measured on `index_read` by the hand-written form: −31 % interpreted.
`LOFT_NO_SAME_READ=1`, `LOFT_TRACE_SAME_READ=1`.  Guards
`tests/scripts/a-read-a-statement-spells-twice-is-read-once.loft` and `tests/same_read.rs`
(which statements bind; the cells with the rewrite off).

### `(R-InlineLeaf)` takes a tuple result

```
  (R-InlineLeaf) + A leaf whose result type is a tuple of scalars, built by tuple literals (an
                 `if` chain choosing among them included), is admitted; its call is replaced
                 by its body as a scalar leaf's is.
```

**In words.** Applied by: `leaf_inline::admit` (the result clause) and `pure`/`remap` (a tuple
literal).  A tuple holding text or a record is not a tuple type to this check: the language
makes it a record with a result buffer (`__tuple<integer,text>`), which the scalar-result check
declines — so the all-scalars condition on a `Type::Tuple` is defensive, and the plant that
admits any tuple passes.  Built: `bfs_flow`'s `nbr` inlined, −12.8 %
instructions and −10.6 % cycles interpreted.  Guard cells in the leaf-inline file: an `if`-chain
tuple read in both elements and in a loop, and a text-holding tuple that stays a call; the pin
counts four inlined calls and the decline.

### A text discharge that assigns a local is read straight into it

```
  (R-DischargeInto)  A statement `w = r ?? d` over a TEXT read, lowered to
                     `{t = r; if t is not null then w = t else w = d}` and `free t`, is
                     `{w = r; if w is null then w = d}` when `w` appears in neither `r` nor
                     `d`, and `t` is named nowhere but its assignment, the test, the moving
                     arm and its free.
```

**In words.** Applied by: `discharge_into::rewrite_program`, in the IR phase (both backends).
A text local owns its bytes, so the temporary cost the interpreter a second allocation and copy
per bind; native already held both as `&str`, and its locals stay borrowed (`tests/text_borrow.rs`
asks the locals of both emissions).  Why the three conditions: assigning a text CLEARS the
target before its value is evaluated, so a read naming `w` would see it emptied; the default now
runs after `w` holds the read rather than its old value; and a temporary named anywhere else
would lose its value.  All three are DEFENSIVE against today's parser, measured: an assignment
whose right-hand side names its target is lowered through a work buffer (`__work_p2_N`) and
never reaches this shape, so the two cells that name `w` in the read and in the default stay
green with each clause planted away, as does the single-use clause.  The rewrite itself is
caught: the null test inverted reads `[]` for `[yy]`, the default dropped reads `[null]` for
`[]` — both silent.  Measured (pinned layout, one tree): `word_count` 88.5 →
64.3 ms together with `(R-KeyList)`, of which this rule −19 %; hashes unchanged.
`LOFT_NO_DISCHARGE_INTO=1`, `LOFT_TRACE_DISCHARGE_INTO=1`.  Guards
`tests/scripts/a-text-discharge-reads-straight-into-its-local.loft`, `tests/discharge_into.rs`.

### A keyed type's key list is a fact of its schema

```
  (R-KeyList)    The key content types a keyed operation pops — `Stores::get_keys(tp)` — are
                 derived once per type, on the first lookup that asks, and read from the type
                 afterwards.  Deriving the type's key descriptors again (`determine_keys_for`)
                 resets the list, so it never outlives the descriptors it is derived beside.
                 A lookup copies the kinds onto its own stack (eight inline) before it pops
                 the key.
```

**In words.** Applied by: `Stores::get_keys`, `Stores::compute_key_contents`,
`Stores::determine_keys_for`, `State::stack_keys` (the interpreter; native passes its key as a
`Content` slice and never asked).  LAZY on purpose: derived eagerly in `determine_keys_for`, it
cost the front end 171 allocations on the tiny program (`tests/frontend_counts.rs` caught it),
for types most programs never look up.  A `OnceLock` per type, like `TypeFacts`, and ignored by
the schema's equality for the same reason: a reloaded schema has not derived it yet.  Before it, every `OpGetRecord` / `OpSetKeyed` re-walked the
type's key fields through `key_field` / `key_contents_for_field` and allocated the list, about
7 % of `word_count`'s profile.  Measured (pinned layout, one tree, with `(R-DischargeInto)`):
`hash_find` −20 %, `hash_update` −22 %, `hash_text_keys` −15 %, `hash_remove` −10 %, every hash
unchanged.  The falsifier is **`LOFT_KEY_LIST_VERIFY=1`**: each cached read is re-derived from
the type's parts and a disagreement stops the run naming the type.  Swept over all 2020
`tests/scripts` and bench programs, eager and lazy: none.  Planted a stale cache (every key cached
as text): without the verify the keyed bench panics deep in `allocation.rs`; with it, at the
first lookup, `type 97 (hash<E[id]>) caches keys [5], its parts derive [0]`.

### A text predicate over a case fold is answered without building the fold

```
  (R-FoldCompare) a text predicate P ∈ { ==, !=, starts_with, ends_with, contains }
                 one of whose operands is a case FOLD F(t), F ∈ { to_lowercase,
                 to_uppercase }, where the fold's value has that ONE mention (a
                 synthesised temporary, or a local whose every binding is such a
                 fold or `fold + literal` and whose every other mention is a text
                 VALUE read as an operand of such a P), is answered WITHOUT
                 building the folded text: over the span P inspects, F is applied
                 byte by byte while every byte of that span and of the other
                 operand is ASCII; the FIRST non-ASCII byte met in either operand
                 takes the written form (the fold built, P applied) for that
                 evaluation.  A `+ literal` on the folded operand is compared as
                 the fold followed by the literal.  The other operand is read as
                 written.  Any other mention of the fold — a bind read elsewhere,
                 a return, a link, a capture, a non-text-value op — declines.
```

**In words.** `h.to_lowercase().starts_with(prefix)` is a question, not a text: the fold
exists only to be compared.  Unicode case is not byte-local (U+212A KELVIN SIGN lowercases
to ASCII `k`, `İ` lowercases to two scalars, `ß` uppercases to `SS`), so the byte-wise form
is sound only while both operands' inspected bytes are ASCII — one `is_ascii()` over the
span — and the fallback is the program as written, so no answer can change.  Reach: every
case-insensitive lookup in the text-scan class.  Priced on server `header`
(`bench/portal/analysis/over-9x.md`): with the walked line borrowed, 9.43 → 3.52 M ns/op.
Generation time, `--native` first (`src/generation/ops/text_ops.rs`, beside the
`starts_with` emitter; the folding helper beside `lazy_split` in `src/codegen_runtime.rs`);
the IR phase only on broad evidence (`(Perf-Order)`).  Switch `LOFT_NO_FOLD_COMPARE`; trace
`LOFT_TRACE_FOLD_COMPARE` names each admitted predicate and each declined fold.  Falsifier:
hand-computed cells on both backends — ASCII hit and miss, a prefix longer than the line,
the Kelvin sign in the line against an ASCII prefix (must match), `İ` within the span (two-
scalar fold, the written form), `ß` under `to_uppercase` against `SS`, an empty operand, a
fold bound to a local and read twice (declines); the switch A/B over `server`'s bench hash;
`(R-TextBorrow)`'s pins with this rule off.

### A call whose result is moved whole into an element builds it there

```
  (R-Destination) a call whose result record r the caller consumes ONLY by moving
                 one heap-free field r.v (OpMoveField) into a fresh element e of a
                 container the caller owns, and by reading r's remaining fields as
                 scalars at most once each, is emitted as the callee's DESTINATION
                 TWIN: the twin takes e's address for r.v — every write the body
                 makes to r.v lands at e, every sub-record it places in r's store
                 is placed in e's store — and answers the remaining fields as a
                 tuple of scalars; the caller mints e before the call without its
                 prefill (the twin's writes cover r.v whole, R-CompleteWrite) and
                 makes it visible (OpFinishRecord) only on the path where it moved
                 r.v.  Admission asks the CALLEE: on every return path r.v is
                 written whole before r is returned, r.v's writes name no address
                 but r's, and r is not read back by the body after a write to r.v.
                 The twin receives e's address and no path through which the
                 container's record can be reached, so the container's header
                 stays valid across the call.
```

**In words.** `sub = read_value(…); if sub.ok { items += [sub.value]; p = sub.next }` builds
`value` in a pooled buffer, reads `ok`, mints an element, prefills it, moves `value` in and
reads `next`, where the twin does one `push`: this is `(R-Place)` one call deeper — the
child built where it will live.  The ok-false path is the ownership question, and the
answer is the one `(R-PushRec)` already gives a minted element: an element minted but not
finished is NOT a member of the container (`vector_finish` is the one visibility step), its
slot is capacity the next append reuses, and whatever heap the failed child placed is
released by the CALLEE on its failing path exactly as it releases its own buffer today — the
caller frees nothing.  A callee that answers `ok=false` after placing heap into r.v without
releasing it is the deviation to guard.  Priced on cbor `decode` (over-9x.md): with D2, D3,
D5 and D6 it takes the row 12.04 → 6.49 M ns/op, and it is the one step without which that
ladder stops at 8.3×.  Sites: `(R-Callee)`'s admission (a second emission like
`(R-Inputs)`' `__inv`), `Output::user_fn_call_body` for the tuple answer, the append
lowering's mint, `src/exit_vector.rs` for the return-path analysis.  Switch
`LOFT_NO_DESTINATION`; falsifier `LOFT_HOIST_VERIFY=1` (the twin re-reads e after each write
and compares with the plain form's buffer).  Guard: a nested array and a map, each truncated
at every byte position (ok=false at every depth), hand-computed on both backends, with the
live-record count equal to the plain form's; a finish planted on the failing path must go red.

### A fn-ref whose every target answers a value record answers the tuple

```
  (R-FnRefValue) a call through a fn-ref of type `fn(…) -> R`, R a record that
                 (R-ValueRecord) admits, answers R's tuple when EVERY function of
                 that signature in the program — the arms of the dispatch `match`
                 the emitter writes — is (R-ValueRecord)-admitted; the `_ =>` arm
                 answers the null tuple.  Decided per fn-ref TYPE over the whole
                 program in a pre-pass: one unadmitted function of the signature
                 keeps the buffer road for every call of that type, and a capture
                 (a closure fn-ref) declines.  The interpreter is untouched: the
                 tuple is native's representation of the same record (C122).
```

**In words.** `resolve(r.style)` minted a store, wrote four fields, adopted and freed it per
call, to hand back a record `default_style` had already answered in registers (the fn-ref
result ABI stops at the buffer road, `@FR-O-Unknown`).  The target set of a fn-ref is complete
in the emitted binary (`src/generation/emit.rs:1766` writes the `match`), so the admission
`(R-ValueRecord)` proves per function lifts to the signature.  Sound where every arm is
admitted — the pre-pass reads the fixpoint `(R-ValueRecord)` already computes; a null fn-ref
answers the null tuple exactly as the buffer road answers a null record.  Priced on zttext
`flow_layout_full` (over-9x.md): −27 % alone.  Switch `LOFT_NO_FNREF_VALUE`.  Falsifier: a
program with two functions of one signature, one admitted and one answering a heap record —
the call must take the buffer road; the switch A/B on the flow row with hand-computed boxes;
`make rewrite-census` with `(R-ValueRecord)`'s admissions unchanged.

### A lookup's found entry is answered as a view the caller reads in place

```
  (R-ViewReturn) a function whose result at some exit is a VIEW V of a sub-record
                 reached by a pure path rooted at a `const` (or never-written)
                 PARAMETER p — `return e.value` for `e = p.entries[i]?`, or a match
                 binding of such a view — answers that exit as V's address (the
                 parser's materialised copy of the view is not emitted) when EVERY
                 caller consumes the result INSIDE the statement that binds it: the
                 result is the subject of a `match` (or a field read) whose arms
                 read scalars, read texts as VALUES (R-TextBorrow's sense) or copy
                 a field into their own slot, and no arm writes, grows, frees or
                 rebinds any store reachable from p before its last read.  Every
                 other exit keeps its form; a function with one caller that cannot
                 be so proven is emitted as today.
```

**In words.** `(R-ReturnField)` answers a field of an OWNED local as its store at a position
and declines a parameter, because a view of a parameter's record dangles the moment the
caller frees or grows that record; here the referent is the caller's own argument and the
whole use of the result is the one statement around the call, so the dangling window is
empty by construction — the check is on the caller's consuming statement.  Conditions: p is
reached only through pure paths in the callee and never written there; the consuming
statement neither writes nor releases p's store before the bindings' last read (an
`OpDatabase`, an `OpFreeRef`, an append, a rebind of the argument inside an arm each
decline); the absent arm hands up `DbRef::NULL`, which the tag read answers as "no variant",
so a caller that matches the null variant BY NAME declines.  Profitable where the found value
holds heap.  Priced on pluginabi `check_request` (over-9x.md): −10 % — the mint, the deep copy
and its text claim per lookup; `pa_get` is the shape of every `match`-based finder.  An
ownership decision for the owner before it is built.  Switch `LOFT_NO_VIEW_RETURN`; trace
`LOFT_TRACE_VIEW_RETURN` names the declining condition per call site.  Falsifier: the hash
and `LOFT_NATIVE_LEAK_CHECK=1` on both backends (a view freed as if owned shows as a double
release); a planted arm that appends to `p.entries` before reading the binding must decline;
`LOFT_POISON=1` catches a read through a view whose record moved.

### Proposed clauses on existing rules

Each extends a rule above with one admission the measured rows need; the rule's own switch,
trace and falsifiers apply, plus the cells named here.  Priced in
`bench/portal/analysis/over-9x.md`.

- **The split-table verdict, for `(R-TextBorrow)`.**  A `Set(t, split(…))` whose `t` is a
  split table `(R-SplitTable)` lowered is not a store write: `borrowed_text_walks`
  (`src/generation/hoist.rs`) takes the body's split tables, not only the walked vector's,
  and `may_write_store` skips such a bind.  `(R-Header)`'s loop clause admits by the same
  verdict.  Cell: a walk whose body splits the walked text and reads the pieces (borrows);
  one that appends to the walked vector (declines).  server `header`: 9.43 → 7.30 M.
- **The match-binding clause, for `(R-TextBorrow)`.**  A text bound by a match pattern that
  the arm reads only as a text VALUE — an operand, a `text` argument the callee does not
  store, the source of a format into another slot, `size`, a comparison, a `Str` result —
  never writes, links, captures or stores it, and the arm writes no store of the subject's,
  is a borrow of the field (`&str` from `get_str`): no `String` is built.  The walk clause
  borrows an element for an iteration; this borrows a field for an arm under the same
  value-only reading.  Cells: an arm that appends to the subject's vector after reading the
  binding (declines); one that stores the binding into another record (copies).
- **The pooled clause, for `(R-RefillText)`'s collection clause.**  A return buffer whose
  elements the callee keeps is only kept across calls the CALLER's buffer survives: a
  caller that mints it per activation and is itself called per token (zttext's
  `token_width`) hands a fresh store each time.  Lifting that work-ref into the caller's
  caller, as `(R-WorkBuffer)`'s transitive clause lifts a work buffer, is the next unit:
  priced −8 % alone and −31 % with the collection clause on `flow_layout_full`.
  `check_request` and `decode` build their vectors by other operations than appends and are
  not admitted.
- **Text-bearing elements and the work text, for `(R-WorkBuffer)`.**  `vector<τ>` with τ a
  record of scalars and texts is admitted under the mention test unchanged PROVIDED the
  callee refills it under the text clause (otherwise the per-call release is what the pool
  was meant to remove); and a function's `__work_*` text local is the same buffer one level
  down, pooled per frame, with an assignment of the empty literal to it emitted as `clear()`.
- **The function clause takes a base, for `(R-Base)`.**  Beside the function-clause header
  `(R-Header)` emits (`src/generation/mod.rs`), `vec_base` is emitted under the base's own
  condition — no growth of that vector anywhere in the function — and the `?? default`
  element read outside a loop takes it as the in-loop read does.  Cells: a read past the end
  (the default answers); a function that appends to the vector it reads (declines).  cbor
  `decode`: 36 byte-read sites, 6.49 → 5.15 M on the ladder.
- **A field MOVE covers, for `(R-CompleteWrite)`.**  `group_covers_type` counts an
  `OpMoveField` whose destination is the whole element (offset 0, the element's own type) as
  covering every field, so the mint is the no-prefill form.  Cell: `LOFT_POISON_CLAIM=1` over
  the `(R-MoveLast)` corpus — a prefill skipped wrongly shows as poison.
- **A push window across an admitted recursive call, for `(R-Push)`.**  `(R-Callee)` answers
  `None` to every question on a recursive edge (`hoist.rs`: a callee met while its own
  verdict is open), which denies a recursive builder every call-crossing hoist at once; one
  fixpoint over the callee's own write set admits them together, and `items += [sub.value]`
  keeps its push header across `read_value`'s call to itself.  cbor `decode`: 5.15 → 4.06 M.

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

**OPEN: 0**.
