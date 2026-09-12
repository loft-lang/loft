# formal/calls-history.md — the deviation register for [calls.md](calls.md)

> **The rules are next door.**  [calls.md](calls.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0** — `D-call-20` opened 2026-09-09 and CLOSED 2026-09-12 (loft#1485, a lambda's heap return that reaches a CAPTURE is handed out as a view, below); `D-call-19` opened 2026-09-09 and CLOSED 2026-09-12 (loft#1484, a generic monomorph's `-> T` record return aliases the argument where its concrete twin copies, and D-call-13's guard reads green over it, below); `D-call-18` opened AND CLOSED 2026-09-09 (loft#1482, the DENSE half of `D-call-17`: a `-> S` return whose tail is a named binding viewing a PARAMETER aliases the caller too, and `D-call-17` closed on the belief that it does not, below); `D-call-17` opened and closed 2026-09-08 (loft#1468, a `-> τ?` return of a match-arm view of a PARAMETER aliased the caller, where its dense twin copies, below); `D-call-16` opened and closed 2026-09-08 (loft#1451, a generic's `-> T?` at a tuple was not boxed, because the promotion matched one spelling of its own shape, below); `D-call-15` opened and closed 2026-09-08 (loft#1432, a method's two receiver nullabilities resolved by declaration order and by call spelling, below); `D-call-14` opened and closed 2026-09-05 (a vector parameter reassigned from a variable refilled the caller's store, below); `D-call-13` opened and closed 2026-09-05 (a generic's instance returned the argument it was handed, below); `D-call-12` opened and closed 2026-09-04 (loft#1357, the residue of
`D-call-9` under the release valgrind sweep), the same day as `D-call-10` and `D-call-11`
(loft#1345, loft#1347), `D-call-9` (loft#1338) and `D-call-8` (loft#1337); before them
`D-call-7` closed 2026-09-02 and `D-call-6` was opened and closed the same day by the
reference review of chapter 31.

### D-call-20 — OPENED 2026-09-09, CLOSED 2026-09-12 (loft#1485): a lambda's heap return that reaches a CAPTURE is handed out as a view

`(F-Ret)` again, at the capture boundary.  Found by the peer stream while measuring loft#1482;
re-measured here on `e4c7db584` before recording, on both backends, with `(F-Ret)`'s own test
(`bump(g()); println("{g().a}")` — no binding anywhere in it, so a copy cannot swallow the answer):

| lambda tail, over a capture `q` | answer | wanted |
|---|---|---|
| `fn() -> S { q }` | **8** | 7 |
| `fn() -> S { e = q; e }` | **8** | 7 |
| `fn() -> vector<S> { q }`, appended through | **len 2** | len 1 |
| `fn() -> S { e = q[0]; e }` | **ICE** — H5 two-pass abort | 7 |
| `fn() -> S { q[0] }` — the CONTROL | 7 | 7 |

The control is what makes the rest readable: a DIRECT projection tail reaches
`return_projects_into_local`, whose base is not an argument, so it materialises and the shape is
already right.  What escapes is the tail that NAMES the capture, directly or through a local.

The ICE is verbatim: *"H5 two-pass contract: def `n___lambda_0` (#736) grew a pass-2-only attribute
`e` (pass1=1, pass2=3) that is not a documented lazy append"*.  It is the same rename that grows
the attribute, but a capture is not an argument, so loft#1482's `var_views_an_argument` does not
reach it and that fix leaves this exactly as it was.

⚠ **`(L-CapHeap)` does not license this.**  It shares the store in the direction the closure READS
and says nothing about what the closure RETURNS, so `(F-Ret)` is unopposed here — checked before
recording rather than assumed.

**CLOSED 2026-09-12.**  Re-measured on both backends in this entry's own five rows, in the `bump`
spelling above rather than the guard's `f(q).a = 99`, because a deviation closes on the cells it
was written from: `fn() -> S { q }` 7, `{ e = q; e }` 7, the appended-through `fn() -> vector<S>`
len 1, `{ e = q[0]; e }` 7 — the row that aborted on the H5 two-pass contract now runs — and the
projection control 7.  The harness is not vacuous: `bump` on a plain local reads 8 and through a
`v[0]` projection reads 8, so a mutation that reached the source would show.  The fix and its
guard (`1485-a-lambda-does-not-return-a-view-of-its-capture.loft`) landed in #1490, the same PR
that opened this entry.

### D-call-19 — OPENED 2026-09-09, CLOSED 2026-09-12 (loft#1484): a generic monomorph's `-> T` record return aliases the argument where its concrete twin copies

`(F-Ret)` says a returned whole heap value is fresh, and *"nothing in the rule distinguishes a
generic's instance"* — this doc's own sentence, two paragraphs above the deviation list.
Re-measured here on `e4c7db584`, both backends:

```loft
fn idg<T>(x: T) -> T { x }      bump(idg(q)); idg(q).a  →  8    ← aliases
fn idn(x: S)   -> S { x }       bump(idn(r)); idn(r).a  →  7    ← its concrete twin is fresh
```

`p[0]` and `e = p[0]; e` fail the same way.  The monomorph publishes `ref(S)` with an EMPTY dep
list: D-call-13's `Own::Borrowed` re-derivation in `try_generic_instantiation` is not firing.  This
is the residual D-call-13 itself named — *"a `-> T` that MIXES a mint and the argument … wanting
the return buffer at instantiation"*.

⚠ **And D-call-13's own guard reads GREEN over it**, which is the finding worth keeping.  Every
cell in `a-generic-instance-returns-what-its-concrete-twin-returns.loft` binds the result before
mutating it (`r = g_struct_whole(src); r.n = 99`) — and a record bind COPIES, so the cell measures
the CALLER'S COPY, not the callee's return.  `t_3Ctr_g_struct_whole` publishes an empty dep list
with every cell passing.

> **A guard whose cells all bind before observing is measuring the bind, not the return.**

The shape that discriminates is `(F-Ret)`'s own: mutate THROUGH one call and re-read through
another — `bump(f(q)); println("{f(q).a}")` — which never touches a binding.  The same blindness
cost two probes in @PLN160 before it was named; it applies to QUALITY.md § B7t's 48-cell matrix.

**CLOSED 2026-09-12.**  Re-measured on both backends in exactly that discriminating shape, over
all six halves — whole, element and element-bind, generic beside concrete — every one reading 7.
The same probe's positive control (`bump` on a plain local, and through a `v[0]` projection) reads
8, which is what says the 7s are independence rather than a probe that cannot see a write.  The
fix and its guard (`1484-a-generic-instance-return-is-as-independent-as-its-twin.loft`, whose
cells write through the call for this reason) landed in #1490, the same PR that opened this entry.

### D-call-18 — OPENED AND CLOSED (2026-09-09, loft#1482): the DENSE half of D-call-17 aliases too, and D-call-17 closed on the belief that it does not

`(F-Ret)`: *"a caller that mutates one call's result does not affect another call's result."*  Run
as written, on both backends:

```loft
fn f(p: vector<S>) -> S { match p { [a, ..] => a, _ => S { a: 0 } } }
fn main() { q: vector<S> = [S { a: 7 }]; bump(f(q)); println("{f(q).a}"); }   // 8, wants 7
```

**The dense return does not copy through its buffer.**  Pass 1's `Rename` leg moves the tail's own
local ONTO `__retbuf`, and the assignment then overwrites the buffer's `DbRef` with the view rather
than copying the record into it — `fn n_head(p, a) -> S["a", "p"]` with
`a = OpGetField(OpGetVector(…))`, and no `OpCopyRecord` anywhere.  `return_views_an_argument`
cannot see it either: it exempts any `v` with `is_argument(v)`, and after pass 1's rename the
tail's local answers that TRUE — the predicate reads a fact the delivery itself created.

**The boundary is wider than D-call-17's shape.**  It is any named local whose deps name a
parameter, returned as the bare tail — `e = p[0]; e` fails identically and is not a match arm.
`p[0]` direct, a `for` binding, a minted literal and a DECLARED parameter handed straight back all
stay fresh; that last one is the control that leaves loft#1368's exemption standing.  The cells are
`plans/160-nullable-road/fret-boundary/`.

**Why it is open rather than closed.**  Both pass-2-only repairs were built and measured: by pass 2
the buffer var IS the tail's local, so source and destination are one var — `MaterializeView`
answers an empty record and orphans a store (loft#848's collision), `ForwardCopy` answers an empty
record.  The decision has to move to PASS 1, which is what `!self.first_pass` exists to prevent,
because a `??`-join local's deps are not pass-stable (the H5 two-pass contract).  Closing this means
giving that guard a pass-stable predicate — a design call, not a leg to move.

**How it was found, which is the reusable part.**  Not from a failing test: `@PLN160`'s pair
instrument asked *"do `τ` and `τ?` reach the store machinery by the same road?"*, and the divergence
it reported was read by asking WHICH half was wrong instead of assuming the dense one was right.
A deviation register entry that names an oracle is a claim to re-measure — the same warning
`tuples.md` carries about its own `OPEN: 0`.

**CLOSED** by `3ee333f94` + `7f0f123ab` (branch `tuxedo-1481-addr-alignment`, not yet on `main`).
Measured by the peer stream; the commits were read here to confirm the change is where the entry
says, and this plan re-measures its own channel table on the join rather than carrying it.

**It did not land where either reading pointed.**  Not `classify_reference_delivery` — the fix is
in `classify_ret_promotion`'s `allow_rename`, whose ladder already refuses the rename for a
candidate viewing a LOCAL and stops at the frame boundary on purpose: a local's store dies with the
frame, so renaming it would dangle.  **A parameter's store does not die, which is exactly why this
read as sound** — the rename produces a perfectly live record, and it is the caller's.
`var_views_an_argument` is the argument twin, one hop deep because a record bind COPIES (`e = p[0];
e` still looks at the caller's record; `e2 = e` owns its own).  Refused, the candidate drops to the
`Bind` copy leg and materialises into `__retbuf`.

**The pass-stability requirement is met STRUCTURALLY, not by a better predicate.**  Refusing the
rename is what stops the tail's local from BECOMING an argument, so the fact
`return_views_an_argument` reads is no longer one the delivery itself created, and both passes
answer alike.  `return_views_an_argument` now delegates to the per-var predicate, so the two
spellings cannot drift.

**A second half was needed, and only a LEAK found it.**  A candidate copied into the buffer must
not have its deps walked into the return type — the copy severs the borrow.  Without that,
`find_join(table, …)` in `496-borrowed-view-return-dep-prune` published `-> Mon["__retbuf",
"table"]`; the caller read the stale `table`, bound the fresh copy as a view of its own vector and
freed nobody's.  One record per call, every value correct, `make ci` green.  `jo_arm_skip`'s walk
already says that sentence for the vector arm pre-pass; this is the record twin.

**The boundary was wider than the eight rows the issue recorded**, in both directions: a field
projection of a struct parameter, an explicit `return e`, a method receiver, a record ENUM, and a
vector ELEMENT of a vector parameter (`fn f(o: vector<vector<S>>) -> vector<S> { e = o[0]; e }`),
which appended to the argument.  Its sibling `e = b.v` — a vector FIELD of a struct parameter — was
already right through the vector selector's own `CopyBorrow` leg, which is why the former looked
settled.  The `if`-arm cell moved on `--native` ONLY: it answered a zeroed record where
`--interpret` was accidentally right.

⚠ **The methodological finding, and it is the reusable part.**  The first gate run was RED on six
`tests/use_analysis.rs` oracle cells: the new refusal answers the same question as the
`LOFT_JOIN_OWN` match-return pre-pass for a `_mv_` marked vector borrow.  With the pre-pass ON
those candidates sit in `jo_arm_skip` and the legs never meet; under `LOFT_NO_JOIN_OWN` the caller
has asked for the pre-pre-pass emission and the new leg handed back the post-pass answer anyway —
**so the switch could no longer show its own before-half.**  Nothing about VALUES moved; the
runtime was clean either way, and only the pinned dep in
`join_own_match_return_strips_the_borrow` could see it.  `7f0f123ab` stands the refusal down for a
binding `is_marked_vector_borrow` says the pre-pass owns — @PLN155's one-home predicate, reused
rather than respelled.

> **A new leg answering a question an A/B switch already answers makes that switch VACUOUS, and the
> switch's own test is the only thing that says so.**  A bisect control that cannot fire is worth
> nothing, and no value channel reports its loss.

Evidence at `7f0f123ab`: `make ci` ALL GATES PASSED · `LOFT_POISON=1` 2016/2016 ·
`LOFT_VERIFY_STACK=1` 2016/2016 · corpus emit diff DIFFERENT 18 of 1436, every one clean on both
backends with `strict=0` **and** `exhausted=0` (the second grep matters — `LOFT_STRICT_STORES`
implies `LOFT_NO_SLOT_REUSE`, so a store-churning program aborts with *"store table exhausted"* and
a sweep grepping only for `strict-store` scores that CLEAN).  Guard
`tests/scripts/1482-a-record-return-that-views-a-parameter-is-copied.loft`, 8 cells + 9 controls,
falsified at `14ed307de` on both backends.

### D-call-17 — OPENED AND CLOSED (2026-09-08, loft#1468): a `-> τ?` return of a match-arm view of a PARAMETER aliased the caller

`(F-Ret)` makes a returned value FRESH and independent — *"a function that returns a whole heap
value hands out an OWNED value … never a view of a local. EXCEPTION: a `&T` return"*.  A DENSE
heap return was believed to earn that through its hidden `__retbuf` — *"the caller allocates and
`ref_return`'s copy leg writes into it"*.  ⚠ **@PLN160 measured that premise and it is FALSE; see
`D-call-18` below, which is this deviation's dense half, still open.**  A NULLABLE one has no
buffer — loft#896's `__nullable<S>` is a
different representation, and `ret_promo_base` records that giving it one as well leaks a record
per call — so nothing materialised it and the view escaped:

```loft
fn head(p: vector<S>)   -> S? { match p { [a, ..] => a, _ => null } }        // aliased
fn head_d(p: vector<S>) -> S  { match p { [a, ..] => a, _ => S { a: 0 } } }  // NOT fresh — D-call-18
```

A write through `head`'s result landed on the caller's own vector, identically on both
backends, with no diagnostic.  That is also `(N-Shape)`'s complaint in one line — a shape
question answering differently for `τ` and `τ?`.  ⚠ **The dense twin was taken as the oracle that
settles which answer is right, and it is not one:** it aliases too (`D-call-18`).  `(N-Shape)`
says only that the two spellings must AGREE; it does not say which of them is correct, and
`(F-Ret)` is what settles that.  Reading agreement as endorsement is how a half-fix closed.

**Two sites, and the pair is the point.**  `classify_reference_delivery` never asked whether the
tail borrows an ARGUMENT (`return_views_an_argument`, the mirror of `return_views_local`), and
`return_leaf_is_owned_or_null` then called such a leaf OWNED, because `return_views_local` walks
PAST an argument dep — deliberately, since that store outlives the frame and nothing dangles.
Both sites were right about DANGLING and neither was asking about FRESHNESS, so a local bound to
`p`'s element passed both.  Fixing either alone leaves the defect: the first without the second
publishes an owned signature over an unchanged view, which is worse than the alias.

**Measured, not assumed.**  A third leg was written first — a tail-shape test that materialises a
projection rooted at an argument — and REMOVED once the matrix showed it changed no cell: the
filed shape's tail is a bare `Var` (the arm binds the element, then the tail names the binding),
so the fact is in the dep list rather than in the tail, and every field-projection spelling
(`o.inner`, `b.v[0]`, an `if`-arm of either) was already fresh through the caller's own first-bind
copy.  Fourteen cells, both backends, value + `LOFT_STRICT_STORES` + `LOFT_POISON`.

**What must NOT move, and does not:** `keep(s) -> S? { s }` and `first(p) -> S? { p[0] }` are
loft#1368's settled shapes and stay fresh; `-> &T` is `(F-Ret)`'s own exception and is excluded
by name; a callee whose subject is a LOCAL still aliases within its own frame, which is what a
local subject means.  Guard
`1468-a-nullable-call-result-binds-without-an-ownerless-copy.loft` plus the cross-backend driver
`tests/nullable_first_bind_copy.rs`, falsified at 33b72675 on both backends with
`the match arm's result is FRESH, so the source keeps 7 — got 93`.

⚠ **A guard cell that recorded the OPEN answer had to be tightened in the closing commit.**  The
file was written while this was open and asserted `93 || 7` on purpose, so that a known-wrong
value was not frozen into the contract; leaving that disjunction behind would have left a closed
deviation with a guard that cannot see it re-open.

### D-call-16 — OPENED AND CLOSED (2026-09-08, loft#1451): a generic's `-> T?` at a tuple was not boxed, because the promotion matched one spelling of its own shape

`(F-Ret)` makes a returned value fresh and independent, and a heap-carrying tuple return earns
that by being BOXED into the synthetic `__tuple<…>` record with a hidden `__retbuf` —
`tuple_return_rewrite`, the chokepoint D-call-13 already routed the generic instance through.

It matched `Type::Tuple` and nothing else.  A `-> T?` instantiated at a tuple is
`Optional(Tuple(…))`, which is not that, so it fell through unrewritten: the monomorph declared
`(integer, integer)?` — a type the language refuses at every declaration and which has no layout
— while its body still handed up the DbRef the template compiled `T` as.  One shape, two
different wrong answers: `--interpret` read the pointer's bits as member 0 and printed
`34359738371` (`(1 << 35) | 3`) with no diagnostic, and `--native` would not compile the function
(E0308).  The same blindness sat one line away in `from_tv`, which asked whether the template's
return IS the type variable by matching `Reference(tv)` — so every `-> T?` answered no and took
the branch written for a signature that does not mention `T`.

**Closed by peeling in both places**, and the second question now has one home rather than the
same `matches!` at two call sites that a comment already required to stay identical.  Removing
the wrong ANSWER then uncovered two REFUSALS, which is the ordinary shape of this: with the
return correctly boxed, `first(v) ?? (7, 7)` was refused (the value arrives boxed, the default
is written in the stack spelling — the coalesce now takes the stack spelling when
`unboxes_stored_tuple` says they are the same tuple), and `first(v)?` was refused naming a
default *"of type `boolean`"* in a program with no boolean (`Reference(__tuple<…>)` is a struct
to `def_type`, so the record default tried to sub-parse a name loft cannot spell; both tuple
spellings answer "no default" now).

⚠ **This does NOT settle whether `optional((τ₁, τ₂))` is `(τ₁?, τ₂?)`.**  That is the general
question about what a tuple's absence IS, and closing it is several pieces in the member read,
the store and the monomorph's return type.  What is closed here is narrower and independently
true: the generic spelling of a read answers what the NON-GENERIC spelling of the same read
answers.  The non-generic `v[i] ?? d` and `v[i]?` are right on both backends and were right
throughout, which is what makes "agree with the plain read" an oracle rather than a guess — and
it is what the guard asserts, cell by cell, instead of pinning numbers that would go stale the
day the general rule moves.

Guard: `tests/scripts/1451-a-generic-nullable-return-at-a-tuple-matches-the-plain-read.loft`.

### D-call-15 — OPENED AND CLOSED (2026-09-08, loft#1432): the same two declarations were one method or two, depending on which was written first

`(F-Recv)` makes `m(τ)` and `m(τ?)` two definitions — the def key carries the `?` — and says a
call on an absent receiver reaches the nullable one with `self == null`.  Neither half held, and
the rules doc had a closing paragraph disclaiming what its own gloss stated, which is the shape
that lets the next reader derive whichever half they find first.  That paragraph is deleted and
replaced by the sentence the rule lacked: the RESOLUTION ORDER.

**Two spellings of one call disagreed, in opposite directions, and each direction hid the other.**
With both overloads declared, `z.area()` on an absent receiver answered the DENSE body while
`area(z)` beside it answered the nullable one — so the nullable overload was not merely
unreachable but inert (deleting it changed nothing), and the author was warned that their
receiver was undischarged in a program declaring the overload written for exactly that.  With
ONLY the nullable overload declared it ran the other way: `d.area()` on a dense receiver answered
and `area(d)` was refused as an unknown function.

And the declaration ORDER decided which of two things the same two lines meant: dense-first was
accepted with a dead overload, nullable-first was refused as a redefinition.  No reading of the
rule defends that.

**Closed at three sites, and the third is the one the design did not predict.**  The definition
check keyed its collision on *"is the NEW one Optional"*, which is the order dependence itself; it
now compares the two nullabilities, admits the pair either way, and gives the shared attribute
slot to the DENSE definition, which is the one `one_implementation_per_variant` (loft#1427)
prefers.  The method call spelling resolved through that slot — one routine per name — and now
asks `find_fn`, the home the free spelling already used.  But `find_fn`'s own fallback list was
`[sig, base]`: the receiver's own spelling then the dense one, which is a list with only one
direction in it, so a DENSE receiver never tried `m(τ?)`.  Routing method calls into it without
making the list symmetric would have turned the working `d.area()` cell into a refusal — the
second direction above is only visible once the first is fixed, which is why the matrix was built
before the fix and not after.

Guard: `tests/scripts/1432-a-nullable-receiver-overload-resolves-in-either-order.loft` — four
declaration orders {dense-first, nullable-first, dense-only, nullable-only} × three receivers
{dense, present `τ?`, absent `τ?`} × both call spellings, with the `(N-Store)` warning count as
its own assertion, on both backends.

### D-call-14 — OPENED AND CLOSED (2026-09-05): a vector parameter reassigned from a variable refilled the caller's store

`(F-ParamRebind)` names two spellings — `p = [..]`, `p = other` — and says both rebind LOCALLY.
Its oracle (`1290-a-heap-parameter-rebind-is-local-in-every-spelling`) crosses {struct,
struct-enum} × {literal, call, another local}, and the keyed row (`1294`) carries the vector kind
only as a literal control; the vector × `other local` cell was in neither.  Measured (QUALITY.md
B7w): `fn f(x: vector<integer>) { va = [1, 2]; x = va; x[0] = 7 }` wrote `7` into the CALLER's
vector on both backends — alone, through a value branch, when the rebound value was returned,
and on every turn of a loop.  The copy lowering asks `(O-Proxy)` whether the local owns the
store it holds; a parameter's carve-out (no `__vdb_N` of its own, to avoid an allocation) read
as "owns", and the refill cleared the caller's store and appended into it.

**Closed** in the same lowering: a parameter's first rebind mints a store of its own
(`vec_copy_needs_db`); once rebound the local names it and the proxy answers again.  A PROMOTED
return buffer keeps the carve-out — the caller receives the value through it.  Closing it moved
one more thing: the Tier-0 elision then rewrote the parameter's reads onto the source, the
loop's first-turn read ahead of the rebind included (`885-vector-hoist` answered 24 for 27);
its "read-only local" verdict now asks that the destination be a local, since a parameter is
defined at entry.  Guard `a-vector-parameter-reassigned-from-a-variable-rebinds-locally.loft`
— single, returned, through a branch, twice, in a loop, and read-then-rebind — with the
mutate-through, a self arm and a `&` parameter as the controls; falsified at faa38979 on both
backends.  Found-via: the loft#1370 walk (D-own-35).

### D-call-13 — OPENED AND CLOSED (2026-09-05): a generic's instance returned the argument it was handed, and the tuple return leg wrote three members wrong

`(F-Ret)` says a returned whole heap value is FRESH — owned by the caller, never a view of
a local or of the argument.  A monomorph is an ordinary program and owes the same; it did not
deliver it.  The declaration DEFERS a generic's return promotion to instantiation
(`return_shape_depends_on_type_var`) and nothing at instantiation received it, so the instance
kept the RECORD lowering the template parsed `T` as: a `-> T { x }` carried no return deps
and the caller bound the argument's own store; a `-> (T, integer)` stayed a stack tuple whose
heap member was the argument; a vector `s = x` aliased and the frame then freed the caller's
vector.  Measured on the F-Ret independence matrix (QUALITY.md B7t): 13 generic cells failed
identically on both backends, every concrete twin passed.  Boxing the generic then routed it
through the concrete tuple-return leg, which had two pre-existing defects of its own — a
nullable record member written on the tag's discriminant (`4294967199` for `7`) and a nullable
vector member's `null` appended as an EMPTY vector — plus a keyed member written as a 4-byte
header (an interpreter panic and a native E0308, an accept/reject split).

Closed at instantiation, each half mirroring the concrete twin: the return deps from the
oracle where every leaf is the parameter itself (`(O-Oracle)`; a local bound from it copies,
so it is left owned); the deferred tuple boxed by the one function both passes share and its
body tails rewritten (`promote_monomorph_tuple_return`); the vector bind and the vector
return copied (`promote_monomorph_vector_return`); and the tuple leg's three members
written as a struct field writes them.  Guard
`a-generic-instance-returns-what-its-concrete-twin-returns.loft`, 52 cells, falsified at
`babf9e64` on both backends.  Residual, named: a `-> T` that MIXES a mint and the argument
adopts on the borrow arm, wanting the return buffer at instantiation.

### D-call-12 — OPENED AND CLOSED (2026-09-04, loft#1357): eight shapes still minted a text buffer nothing released

`(F-Call)` and `(F-Ret)` again, measured this time by the release's valgrind sweep
(`scripts/valgrind-sweep.sh`) after `D-call-9` closed: 11 corpus files still lost one
`String` per call on the interpreter, every value right on both backends, and the CI leak
gate blind to all of them by suppression.  Not one defect but eight sites that each
answered the question *"who releases this buffer?"* for the shapes they were written
against and not for the one in front of them:

- a LAMBDA that already held its one hidden `&text` buffer (`parse_return` took it) had
  its tail promotion declined on pass 2 — the gate keyed on an `__acc`/`__tret`
  attribute such a lambda never minted — so `fn(n) -> text { return cap ?? "x" }` returned
  its `??` temp as a view; and a lambda with two text locals returned the unpromoted one
  bare.  Closed: the gate reads the buffer the lambda holds, the accumulator / bind temp
  stays a local and is MOVED into that buffer, and a returned bare text local is delivered
  through the buffer and freed (`free_vars`; `free_copied_text_sources` now frees the
  returned local it copied — the sweep had suppressed it as handed up);
- a nullable text LOCAL returned (`t = text_src(i, s); return t`): the ownership oracle
  followed the binding to the argument `text_src` borrows and called the copy safe — right
  for the store, wrong for the `String` a text `Set` copies into.  Closed in the orphan
  predicate (`text_return_orphan_risk`), not the oracle, whose store answer the
  `ownership_oracle` suite pins;
- an early `return` inside a LOOP of a generic monomorph (`for v in it { return v; } d`):
  the rewriter that routes early returns into `__tret` stopped at `Loop`/`Iter`/`Drop`;
- a `-> S?` generic FORWARDER: promoted while its tail call still named the nested
  TEMPLATE (not a text call), and never asked again once `instantiate_nested_generics`
  retargeted it — `try_generic_instantiation` asks again, only where nothing was promoted;
- a tail that READS its own buffer (`rest[0..3]` of the promoted `rest`; a `match` arm
  yielding the work text) fell to the `__ret_N` residual — now staged into the temp,
  moved into the buffer, and the temp freed (`any_text_return_buffer`);
- a `??` temp consumed by a SCALAR tail (`len(s.name ?? "")`, `t.0 + len(t.1 ?? "")`) —
  case b's "the tail is the returned value" premise holds only when the block yields the
  TEXT; the scalar is hoisted first and the temp freed.  And one consumed by an `if`
  CONDITION whose arm returns took its free after the statement the return never
  reached — the condition is evaluated into a boolean, the temp freed, then the branch;
- a `par` loop's `_ = e` over `vector<text>` was marked never-free as a borrowed view of
  the element — a text binding copies, so it owns;
- a `parallel { … }` arm's formatted argument was built in the WORKER's copy of the frame,
  which nothing freed — each arm frees the `__work_N` texts it wrote, on the worker.

Two instrument findings closed with it: the text ledger was per-thread (a worker's orphan
read as "NO text leak") and silent under `--tests`; it is one ledger for the process now and
reports at the end of a suite.  Measured: 11 red files → 0 under the sweep; the guard
`tests/scripts/1357-every-text-buffer-a-frame-mints-is-released.loft` (24 cells, 7 controls)
99 orphaned buffers → 0 by hand under `LOFT_TEXT_TIMELINE=1`, every value byte-identical on
both backends before and after; `tests/text_buffer_ledger.rs` scores it.  The sweep's last
red file, `85-yield-resume`, was the TEST RUNNER rather than the program: a `main` that
`yield_frame`s hands control back after each frame, the CLI resumes it until it finishes,
and `run_tests` scored the first frame as the whole test and abandoned the frame with its
formatted texts unreleased — the sweep runs every script under `--tests`, which is why a
direct run never showed it.  The runner resumes now, as the CLI does.

### D-call-11 — OPENED AND CLOSED (2026-09-04, loft#1347): a lambda declared `-> vector<T>?` lost the `?` when its tail was delivered

`(F-Return)` returns the tail, and storing a non-null value where `τ?` is declared is the
widening every named function performs.  Two vector-delivery legs — the borrow-copy of a
projected tail (`copy_borrow_tail_into_retbuf`) and the forwarder copy (`emit_forward_copy_409`)
— re-set the function's returned type as a BARE vector, dropping the declared `?`, where
`ref_return` keeps it.  A named function survived that (nothing reads its signature against a
variable), but a lambda's Function type is built from the def's returned type, so a lambda
`fn(q: Bag) -> vector<integer>? { q.items }` published `-> vector<integer>` on the pass that
delivered and `-> vector<integer>?` on the pass that did not, and the variable holding it was
refused as a type change — while the named twin compiled.  A `-> S?` lambda with a non-null
record tail was refused the same way.

**Closed at the two legs, by the one rewrap `ref_return` already applies**
(`set_delivered_vector_return`: the deps belong to the storage and the `?` to the value).
Guard `tests/scripts/1347-a-lambda-declared-nullable-heap-accepts-a-non-null-tail.loft` —
the vector, record and whole-argument lambdas, the null-arm twin, the scalar and text
controls, the named twins; the control build refuses the file at parse time (exit 1 → 0 on
both backends).

### D-call-10 — OPENED AND CLOSED (2026-09-04, loft#1345): a `-> vector<T>?` function handed up a projection of its argument as the view

`(F-Ret)` says a returned whole heap value is owned, never a view.  The buffered non-null
vector return copies a projected tail (`Delivery::CopyBorrow`, row 104), but a nullable
return always branches on its null arm and reached the vector materialiser instead, whose
leaf cases were a local vector and a call carrying its own buffer — a projecting arm
(`if q.rec.y > 0 { q.items } else { null }`) matched neither and escaped as the view.  The
caller's bind then aliased the callee's argument field on both backends, whether the callee
was named or a fn-ref and whether the result was bound or chosen by an `if`; the non-null
twin copied throughout, which located the gap at the nullable delivery.

**Closed in the materialiser, with a projection leaf**: a field, element or keyed projection
(`use_analysis::is_projection_op`) is appended into the buffer — `OpClearVector(w);
OpAppendVector(w, <projection>); w` — and nothing is freed, because the source is the
argument's store.  Guard `tests/scripts/1345-a-nullable-vector-return-of-a-projection-is-copied.loft`
(named and fn-ref callees; plain bind, `if` join, a loop with a filler allocation; a field and
an element-of-field projection; the null arm; the non-null controls and the bind-then-rebind
workaround), falsified at `dd46146c` on both backends.  Held fixed and filed apart: a lambda's
lifetime TUPLE takes no synthetic-tuple rewrite at all (loft#1349), and a named function's
tuple result refuses to join a tuple literal (loft#1350).


### D-call-9 — OPENED AND CLOSED (2026-09-04, loft#1338): an early text return was delivered as a view of an orphaned local

`(F-Call)` says the frame frees every local it owns when it drops, and `(F-Ret)` says a
returned value is handed out through the return-buffer, never as a view of a local.  A text
function's block tail met both: `text_return` promotes the tail's accumulator, work text or
built local to a hidden `&text` parameter the CALLER owns, and the tail writes it.  An EARLY
`return` did not.  `return lo(n) ?? ""`, `return lo_n(n)`, `return t[0][0]`, `return s.name`
inside an `if` arm, a loop body, a `match` arm, a nested arm or a recursion base case reached
the scope pass with frees to run before the value could leave, and `free_vars` (and its
block-tail twin in `insert_free`) copied the value into a frame-local `__ret_N` String marked
`skip_free`, under a comment that called the orphan fine "because the caller copies
immediately".  The right characters came back, the String never did — one per call on the
interpreter, 600 blocks definitely lost in valgrind for the issue's 300-iteration loop, and a
second buffer per call where the value was a `??` (its `__ncc_N` temp is `skip_free` for the
same premise).  `--native` collapses the temp into a direct `return`, so only one backend
leaked, and nothing said so: the LSan gate's `append_text` suppression rests on the premise
that those frames leak only on a fault path, and 14 corpus scripts leaked with no fault in
them.

A tail that VIEWS a local — `t[0][0]`, `ts[0][0]` through a slice, a self-recursive call —
had the same orphan for a second reason: the loft#568 orphan predicate classified only the
block tail, so a function whose only owned return was an early one was never flagged, and
the targeted promotion (@PLN104 Phase A) deferred the `view-of-local` class outright with a
note that a bare rebind had crashed `553 textslice`.  And a third defect sat under both: the
predicate, and `--native`'s own return-buffer choice, read ANY `RefVar(Text)` attribute as the
hidden buffer — a user-written `&text` parameter is one too — so `fn f(s: &text, c) -> text {
if c { return mk() } … }` was left unbuffered on the interpreter and, on `--native`, had its
returned text written INTO `s`: the caller's variable changed, silently, with the return value
right.

**Closed at the chokepoint that minted the orphan, and at the one home of the buffer
question.**  Where a text return must be hoisted past frees and the function holds a hidden
`&text` buffer the value does not read, both hoist sites now write each arm of the value into
that buffer (`push_text_arms_into`, per arm so native's arm types stay uniform), free the
`__work_N` / `__ncc_N` temps the copy drained, run the frees, and return the buffer.  The
`__ret_N` copy remains only for a function with no buffer at all, and is documented as the
residual it is.  The orphan predicate classifies every `return` site (`early_return_ownerships`),
a null arm excluded because `OpConvTextFromNull()` is a sentinel with nothing behind it — the
first cut counted it, promoted `text_src(i, tag) { if i == 0 { return null } return tag }`, and
that made its direct-call caller leak the way its local-bind sibling already did; and the
targeted promotion promotes the view-of-local and join-of-local classes, which `Set(__tret,
view)` materialises through the interpreter's own `OpAppendText` copy — `553 textslice` is
green on both backends.  The buffer question reads `Definition::text_work_buffers` (hidden
only) at all six sites that had restated it ([IMPLEMENTATIONS.md](IMPLEMENTATIONS.md) § The
text return buffer).

Measured: the issue's program 600 → 0 blocks under valgrind, two probe matrices of 27 cells
27 + 23 → 0 orphans, nine of the issue's fourteen corpus scripts → 0, the other five (a
tuple-of-text return, a generic monomorph inside a `par` worker, an iterator `?` discharge)
unchanged at their baseline counts — other shapes, held fixed here and named in the LSan
suppression they still hide behind.  Every value byte-identical on both backends before and
after.  Guard `tests/scripts/1338-an-early-text-return-is-delivered-through-the-caller-buffer.loft`
(29 cells, six controls, the `&text` negative cell) with `tests/early_text_return.rs` scoring
the text ledger; falsified at `3d8f2b9e` — native exit 1 → 0 on the `&text` cell, interpret
106 orphaned buffers → 0 by hand under `LOFT_TEXT_TIMELINE=1`, inert on the six channels
`make falsify` scores.  Side finding filed: loft#1343 (a boolean `match` with both arms warns
nullable-into-non-null).

### D-call-8 — OPENED AND CLOSED (2026-09-04, loft#1337): a view of a local escaped through a nullable return

`(F-Ret)` says a whole heap value is handed out OWNED, never a view of a local.  A dense
return has a delivery buffer and `ref_return`'s copy leg lands every arm in it; a nullable
record return has none — a `-> S?` is delivered as the DbRef its tail yields — so what escapes
is exactly what `classify_reference_delivery` decides, and two shapes got past it on both
backends:

```loft
fn walked() -> Node? { …; cur: Node? = a; cur = cur.next; cur }        // views b, a local
fn arm(take: boolean) -> Leaf? { t = Tree{…}; if take { t.l } else { null } }
```

The first: `cur = cur.next` leaves the local a SELF-dep, and `return_views_local` walks the
deps of the return sources with the sources already in its `seen` set — so the one dep it
should have read as *a record reached through my own field, which may be any store this frame
frees* was skipped and the local read as an owner.  The caller received `b`'s record after
`b`'s exit free; `LOFT_STRICT_STORES` names it, a plain run answers the stale value or the
next allocation's.  The second: `return_projects_into_local` stopped at the `if`, the arm's
projection was never seen, the selector chose `Rename` on a function with nothing to rename
onto, and the tail was demoted to a discarded statement plus `return null` — `--native`
printed `null`, the interpreter a reused record — with a literal on the other arm as much as
with a `null`.  Every other arm kind beside a `null` was already right (a literal, an owned
local, a parameter, a view BOUND to a local first), which is what located the gap at the
direct projection.

**Closed at the selector, by its own `MaterializeView` cell.**  A self-dep on a user local
(never on a work-ref, whose self-dep is the ownership marker) reads as a view; an `if` arm is
a tail where there is no buffer; and the buffer-less materialise is made PER ARM
(`materialize_view_arms`): every arm that is not PROVABLY owned or null is copied into one
work-ref (`return_leaf_is_owned_or_null` — a `null`, an argument, an owned local, a struct
literal, a callee that mints its own store are handed up as they are; a projection, a keyed
lookup, a lifted temporary's element, a join are copied), and a nullable LOCAL source copies
only where present — both emitters' `OpCopyRecord` leaves an allocated EMPTY record for a
null source, presence standing in for absence.  The leaf rule is stated in that direction
on purpose: the first cut copied what LOOKED like a view (a field or vector projection, a
viewing local), which is narrower than the criterion that had selected `MaterializeView`,
and a keyed element of an inline call's temporary went out raw again — the `882` poison
cells caught it.  The dense route is untouched: its arm walk and copy leg already satisfy
the rule, and the `if` recursion is gated on the buffer's absence so the dense IR is
byte-identical.

Guard: `tests/scripts/1337-a-view-of-a-local-returned-through-a-nullable-return-is-copied.loft`
— the two shapes, the walk that ends at null, eight nullable controls and the two dense twins,
each read after a filler allocation so a handed-up view would read the filler; both backends;
falsified at `c0a09c95`.  Found while widening loft#1336's matrix, and held fixed there.

### D-call-6 — OPENED AND CLOSED (2026-09-01, loft#1286): the `&` lint could not see a forward

`advice[slow-reference-parameter]` asks whether the body REASSIGNS the parameter, because
`(F-ParamRef)` makes replacement the one thing `&` buys. A FORWARDER never reassigns — its
callee does — so `fn f(b: &B) { g(b); }` looked redundant while it was the only thing
carrying `g`'s write-back out to `f`'s caller. Taking the advice answered `[0]` where the
program had answered `[9,9]`, with nothing reporting the change, and the lint fired **only
on the correct spelling**.

`(F-ParamRef)` is transitive and the rule now says so; the lint asks the transitive question
too. Two implementations of that question were written independently in the two checkouts,
and the one that ships is `callee_param_reassigns` (memoised, per callee parameter): it asks
whether the callee REASSIGNS the argument, where the first version asked only whether the
callee's parameter was declared `&`. The difference is not cosmetic — a `&` parameter the
callee only writes a FIELD through is precisely the case this advice exists to flag, and the
declaration-shaped question suppressed it. Guard `tests/ref_forward_lint.rs` holds either
way: it counts notices on stderr, because `make falsify` has no channel for a diagnostic
that must NOT fire, and carries two true-positive controls so a deleted lint cannot pass it.
The corpus firing counts recorded here earlier are dropped rather than restated: they were
measured on a build that no longer exists, and a count is only comparable against its own
before-half.

### D-call-7 — CLOSED (2026-09-02, loft#1287): a forwarded plain parameter leaks the replaced store

`fn forward_plain(b: B) { replace_ref(b); }` where `replace_ref` takes `&B` and reassigns
leaked one store per call (`kt=79 B×50` over fifty iterations), on both backends. The
ANSWER was correct and was `(F-ParamRebind)` working as written — the replacement rebinds
`forward_plain`'s local and `main` keeps its value. What was missing is the free: the record
the callee allocated landed in a frame about to be dropped and nothing owned it. Neither
neighbour leaked — a `&` forwarder did not, and calling `replace_ref` directly did not.

**Closed 2026-09-02** by the fix that shipped for loft#1287, and re-measured here rather
than inferred from the issue being closed. Both the single call and the fifty-iteration loop
run clean on `--interpret` under `LOFT_STRICT_STORES=1` and on `--native` under
`LOFT_NATIVE_LEAK_CHECK=1`, and the answer is still `[0]`, so the free was added without
disturbing what `(F-ParamRebind)` says the value must be.

⚠ The measurement carries a positive control, because "no warning" is also what a broken
oracle prints. The released **2026.8.0** binary, run on the same two probes, still reports
`kt=79 B×1` and `kt=79 B×50` — the exact counts the issue filed. So the channel that would
report this leak is alive and simply has nothing to say about the current build.

Five deviations have been carried and closed (D-call-1 … D-call-5); otherwise
this is a *rules* doc — it shrinks operational.md's D-op-1 and adds no code deviation of its
own.

⚠ All three are the SAME rule, `(F-Return)` / `(F-Block)`, and all three were *"the tail's
value was dropped"*: D-call-1 dropped it because the block's type disagreed with the
signature, D-call-2 because a block reaching the expression parser is typed `Void`, and
D-call-3 because a var stood in for the tail expression and could not carry one of its
values. A zero here means no KNOWN survivor of that class, not that the class is closed —
each was found by moving an axis the previous one held fixed.

> **D-call-5 — OPENED AND CLOSED (2026-08-26, loft#1100).** `(N-Store)` did not hold for a
> nullable tail reaching a non-null `text` return: one backend RAN it and the other REFUSED
> to compile it.
>
> ```loft
> fn maybe(k: integer) -> text? { if k == 0 { null } else { "z" } }
> fn f(k: integer) -> text { a = "ab"; match k { -1 => maybe(k), _ => a } }
> ```
>
> `--interpret` warned and answered; `--native` — the DEFAULT backend — failed in rustc with
> `E0716` (`E0308` for the `if` spelling, once the arms' Rust representations disagree too),
> on a program the compiler had already type-checked, since it warned about it.
>
> **The issue filed this as a design call** — *"the two answers the language currently gives
> this program are warn-and-coerce and refuse, and it has to pick one"* — **and the rules had
> already picked.** `types.md` writes `(N-Store)` as *"a WARNING (nudge, compiles + runs, the
> slot holds null) when the null is REPRESENTABLE-AND-DISTINCT in τ's non-null form"*, and its
> per-type table puts `text` in that class (out-of-band on the heap); the narrow integer
> widths are the sole error case, because their sentinel collides with a real value. Refusing
> was never the other half of a choice — it was the deviation. This is the second time in a
> week an issue's *"design call"* was settled by a rule already written (loft#1002 and
> `(Slice-Open)` was the first), and the cheap move is to read the rule BEFORE deliberating.
>
> `do_if_acc` promotes a per-arm text accumulator, and its nullability term declined to
> promote exactly here — deliberately. The rewrite retypes the tail as the accumulator, after
> which `block_result`'s `(N-Store)` check compares two non-null types and says nothing, so
> declining was what KEPT the diagnostic; without the accumulator each arm stays
> `&*(callee(…))`, a borrow of the `Str` temporary the callee returned, dead at the arm's
> `}`. One tail type answered two questions, and the gate could only serve one.
>
> Closed by separating them: report the store from the tail's OWN type BEFORE the rewrite,
> then promote (`parse_block`, citing `@FR-N-Store`). **A diagnostic describes the SOURCE
> program; which lowering the compiler picks for it cannot decide whether the program is
> diagnosed** — that is the transferable half, and it is why one edit closed a refusal and a
> silence at once. It also removes the term loft#1099 found to be non-pass-stable, so the
> gate no longer depends on an inference the two passes disagree about.
>
> Measured, nine cells on both backends. Four were REFUSED natively and now compile and answer
> what the interpreter answers (`match`-call, `if`-call, three-arm, and the cell that actually
> REACHES the null — which answers `null` on both, the *"slot holds null"* half). Three cells
> that already compiled now also WARN where they were silent: the `match` null literal (the
> asymmetry D-call-4 recorded), a three-arm variant, and a call BOUND to a variable first.
> Untouched: a declared-`text?` return, and a non-null call arm.
>
> ⚠ **One silent cell survives and it is NOT this gate.** `if k == 9 { a } else { maybe(k) }`
> — the nullable call in the ELSE arm — compiles, answers `null` correctly on both backends,
> and reports nothing, because `parse_if` hands the else arm the THEN arm's type as its
> expected type (the loft#978 note), so the `Optional` is erased before any store check sees
> it. A value that is right with a diagnostic missing is a coverage gap, not a wrong answer;
> the fix belongs at the join, not here.
>
> Emitted IR over the corpus: **4 of 968** programs change, and three are this fix's guard, the
> loft#1101 guard and the loft#1099 guard, whose shape now promotes. The one existing program
> is `947-feature-worked-examples.loft`, where the delta is a `__work_cN` counter shift plus
> ONE `OpFreeRef` moving five places within a run of scope-exit frees — all of distinct
> stores, so their order is inert, and the file is green including the wrap leak gate.
>
> Guard `tests/scripts/1100-a-nullable-call-arm-in-a-non-null-text-return.loft` — eight cells,
> and its first job is to be a program `--native` ACCEPTS: on a control binary built at
> `159e0b42` the whole file fails in rustc before an assertion runs.

> **D-call-4 — OPENED AND CLOSED (2026-08-26, loft#1099).** `(F-Arity)` exempts a
> compiler-inserted slot from the user-facing requirement — *"a return buffer is not a user
> parameter"* — on the premise that the slot is THERE for every call. A two-pass parser owes
> that premise an invariant it does not state: **the compiler-inserted slots a function takes
> are fixed before any call to it is lowered.** A `-> text` function whose tail is a `match`
> with a `null` arm broke it:
>
> ```loft
> fn f(k: integer) -> text { a = "ab"; match k { -1 => null, _ => a } }
> ```
>
> ```
> H5 two-pass contract: def `n_f` (#710) grew a pass-2-only attribute `___acc_1`
> (pass1=2, pass2=3) that is not a documented lazy append — a real cross-pass divergence
> ```
>
> `do_if_acc` promotes a per-arm text accumulator and `text_return` makes it a hidden `&text`
> parameter, so the verdict decides ARITY. One of its terms reads the tail's INFERRED type,
> and that is not pass-stable: instrumented, it is `Optional(Text)` on pass 1 and `Text` on
> pass 2 from IR the two passes leave byte-identical. The accumulator was therefore minted on
> pass 2 alone, and the compiler aborted rather than lower a call against a signature that had
> moved. The `if` spelling of the same program was stable throughout, which is what says this
> is about the inference and not about the null arm.
>
> **The cure was already written down two blocks up, for the same hazard.** `do_tret_bind`
> promotes its own hidden `&text` buffer and carries a gate whose comment states the rule and
> the method: *"Rather than enumerate which tail shapes lower stably, make pass 2 FOLLOW pass
> 1: promote on pass 2 only if pass 1 already minted the `__tret` attribute."* `do_if_acc` now
> carries the twin (`def_has_acc_attr`). It generalises where a per-term repair would not:
> the unstable term is fixed for every tail shape at once, including ones nobody has hit.
>
> ⚠ **This is the second time in three days a fix was found by reading the code beside the
> defect rather than the defect.** loft#1096's belief was written in its own leg's comment,
> and this one's cure was written in its sibling's. `ownership.md`'s D-own-9 draws the first
> half of that lesson; this is the second.
>
> Guard `tests/scripts/1099-a-text-match-tail-with-a-null-arm.loft`, which fails on a
> pristine tree at `66fb9bb4` before it can run a single assertion (the parse aborts) —
> so its first job is to be a program the compiler accepts, and only then to check every
> arm's value on both backends. Controls: a DECLARED-nullable return, which keeps its
> accumulator on its own disjunct (loft#741 is what losing it costs), and a `match` with no
> null arm. Emitted IR over the corpus: **1 of 900** programs changes — the guard itself —
> so every existing text tail already answered the same on both passes.
>
> Two things it did NOT close, both measured and both pre-existing — **and D-call-5 below
> closed BOTH the same day, with one edit.** A nullable tail into a non-null `text` return
> reported `(N-Store)` for the `if` spelling and stayed SILENT for a `match` whose arm is the
> null literal; and a `match` arm that CALLS a `-> text?` function into a non-null `text`
> return compiled on `--interpret` and failed `--native` with `E0716` (loft#1100). They read
> as an accept/reject bug and a diagnostic-coverage gap, which is why they were recorded
> apart. They are one defect: the report and the promotion were reading the SAME tail type,
> so whichever the gate chose, the other was lost.

> **D-call-3 — OPENED AND CLOSED (2026-08-26, loft#1097).** `(F-Return)` did not hold for a
> COLLECTION tail join with a `null` arm:
>
> ```loft
> fn f(k: integer) -> vector<integer> { a = [1,2]; b = [3,4]; if k < 0 { null } else if k == 0 { a } else { b } }
> ```
>
> `f(-1)` answered `[1,2]` — `== null` read **false** while `len` read **2**, one value with
> two answers, on both backends and with no diagnostic. Two arms naming a local means there is
> a store to free before the return, so `scopes::free_vars` demotes the tail `if` to a
> STATEMENT and appends `Return(Var(ret_var))`: the expression still RUNS, and its value is
> discarded exactly as D-call-2's block tail was.
>
> `ret_var` comes from `returned_var_null_unified`, which folds a `null` arm onto its
> sibling's var — and states its own premise: *"the work-ref null-inits at function entry and
> a null arm never allocates into it, so `Return(Var(v))` yields the same null the sentinel
> did"*. True of a RECORD work-ref, which `gen_set_first_ref_null` sentinel-inits. **False of
> a collection**, whose owned local gets `OpInitRef` + `OpDatabase` and whose promoted buffer
> arrives ALIVE from the caller — so on the null path that var is a live, populated vector.
> `(E-Null)` is what it costs: the sentinel is a real, observable, RESERVED value, and a
> populated vector is not it. Closed by hoisting the tail's value to a temp when the fold
> lands on a collection (`scopes::free_vars`), the shape the null-arm RECORD join beside it
> already used — the frees still run between the value and the return.
>
> ⚠ **That same premise had already failed once, at a different site, and this is what makes
> it a class rather than a cell.** loft#1096 (`ownership.md` D-own-9, the day before) is
> `scopes::free_vars` reading *"a buffer not yet minted on this path is the null sentinel,
> which `free` ignores"* — the identical belief about a collection buffer's null-path
> contents, costing a use-after-free instead of a wrong value. **One wrong belief, two sites,
> two defects.** Grep the belief, not the symptom: any site reasoning that a collection slot
> holds the sentinel on a path that did not write it is suspect.
>
> Two more faults met at this tail and are fixed with it, both from the `Bind` leg's
> whole-tail copy `OpClearVector(buf); OpAppendVector(buf, <the join>)` — which answers the
> buffer on every path and evaluates the join AFTER the clear. An arm whose value IS the
> buffer answered what the clear had just emptied (`[]`), and an arm that had already
> delivered into the buffer was appended to itself and came back DOUBLED (`[3,4,5,3,4,5]`).
> Both are cured by the CONDITIONAL delivery that leg's own note names as what would close it
> — `materialize_vector_arms_into`, one arm at a time — plus leaving alone an arm whose value
> is a VIEW of the buffer, whose answer is already in it.
>
> Guard `tests/scripts/1097-a-null-arm-in-a-collection-tail-join.loft`: all three faults
> falsified on a pristine tree at `d98e60ef` (5 of 7 cells red), with a no-null-arm join and
> the RECORD family — where the fold's premise HOLDS — as the controls that keep the repair
> from widening. Fixes loft#1097. The leak left behind (a `match` tail needing a null arm, a
> local arm AND a literal arm, one store per call) is loft#1098: a lifetime fault with its own
> trigger, not this rule.

> **D-call-1 — OPENED AND CLOSED (2026-08-22).** `(F-Drop)` did not exist, and the edge it
> now names is where the two backends parted: a function DECLARED void whose body ends in a
> value ran on `--interpret` and would not compile on `--native`, which surfaced as a bare
> rustc `E0308` quoting a temporary `.rs` file under the message "native compilation failed
> (codegen bug)". `--native` is the default backend, so `loft t.loft` failed this way for an
> ordinary shape — a build-asset script whose last expression is a call returning `boolean`.
>
> Filed (loft#1075) as a design call between "emit it as a statement" and "refuse it", on the
> reading that the rules could not express the edge. Half of that was right: the RULE was
> missing, which is why `(F-Drop)` is written above. The choice was not open, because the IR
> had already made it — `parse_block` wraps a value-typed statement in `Value::Drop` when the
> enclosing function is declared void, so both backends receive `drop n_f();` and the discard
> is the shipped answer. What differed was the BLOCK's type: every statement but the last
> reaches the `t = Type::Void` at the foot of the statement loop, so a dropped TAIL left the
> block typed `boolean` in a function whose signature is `()`, and the native emitter takes
> the signature from the declared return and the trailing default value from the block's
> inferred type. The tell was next door — `f();` with a semicolon always worked on both
> backends, from the same IR, because the `;` sent the statement round the loop to that reset.
> One token deciding whether a program compiles is what says the block type, not the emitter,
> was the thing that was wrong.
>
> The fix is one statement — the block type follows the drop — and it repaired the
> interpreter too: a dropped struct-literal tail was held to program exit ("1 stores not
> freed"), which the same wrong block type had been keeping alive.
>
> It is GATED on the function-body context, and both attempts that were not are why. A
> `result` of `Void` reaches the drop meaning two different things, and only one of them is
> a decision. The other is a placeholder something else will fill in, and there are two of
> those: a LAMBDA declares no return type either, so its body carries the same `Void` while
> its return type is INFERRED from this very block type — flattening it gave every stored
> short `|x| { … }` a void return, which `parse_map` refuses with D-clo-2's *"cannot infer
> the type of the function passed to `map`"*; and a `{ … }` in STATEMENT position is parsed
> against `Void` even when it is the TAIL of an enclosing block, where it is the value that
> block yields — flattening that made `x = {{ …; n }}` infer void, which is the shape the
> Rust test harness writes around every `.expr(…)`. Both were found by the suite, not by
> reasoning.
>
> `unused_must_use` and `path_statements` also join the generated file's allow-list: a
> `#[must_use]` runtime op or a bare local reached as a statement is loft doing what the IR
> told it, and the warnings were reaching users quoting generated Rust — pre-existing on
> both trees, found by this matrix, and the same class of leak as the error. Guard `tests/scripts/void-fn-value-tail.loft`, confirmed
> to fail on a pristine tree at 655ff4dd with 13 `E0308`s on `--native` while `--interpret`
> ran it clean. Fixes loft#1075.

> **D-call-2 — OPENED AND CLOSED (2026-08-22).** `(F-Block)` did not hold: a `{ … }` block
> whose value someone reads dropped its own tail, so `fn f() -> integer { { 5 } }` answered
> `null` on `--interpret` and `0` on `--native`, and `fn g() -> integer { n = 5; { n } }`
> answered `5` on one backend and `0` on the other — silently, with the function
> type-checking, because the block's TYPE is its tail's type and only the value was thrown
> away. Every `{ … }` reaching `expression` is parsed against `Void` (a statement, as far as
> that site can tell), and the parse site cannot know which statement turns out to be the
> last one. The drop is undone after the statement loop, where the block's type is already
> the value it yields.
>
> Two boundaries, each measured rather than argued. **Depth**: a first version asked the
> question of the block one level DOWN and repaired `{ { 5 } }` while `{ { { 5 } } }` still
> answered null; asking it of the block's OWN tail holds at any depth. **Context**: the
> repair is restricted to a bare `{ … }`, the only context handed a `Void` it did not mean —
> a `for` / `while` / `parallel for` / `fields` body gets one because it IS a statement, and
> undoing the drop there leaked one store per round, reopening loft#725. Guard
> `tests/scripts/nested-block-in-value-position.loft`, confirmed to fail at the preceding
> commit on both backends. Fixes loft#1076.

- **Conformance is differential** — call/return is enforced across the two backends by the
  @PLN89 oracle (D-op-1); recursion, nested calls, and struct returns are in its corpus
  (`17-tuples-recursion`, `21-deep-recursion-large-data`, `08-nrvo-mixed-return-paths`). The
  parameter contract (`F-Param*`) is exactly what the ownership register (ownership.md, 0 open)
  and the sandbox raw-write rule ([capabilities.md](capabilities.md), 0 open) are built on, so it
  has the strongest standing cross-checks in the spec.

## Carried by calls.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [calls.md](calls.md) now states only what is open.

### the status line formal/README.md's area table carried until 2026-09-04

**0 open** (2026-08-22) — args left-to-right; scalar params by-value, heap params share (mutate-through visible, whole reassign local, `&` writes back); returns independent. `(F-Drop)` was added and D-call-1 opened and closed the same day: a function DECLARED void whose body ends in a VALUE ran on `--interpret` and would not compile on `--native` (a bare rustc `E0308` about a temporary `.rs` file). Filed as a design call; the IR had already chosen — a void tail is wrapped in `Value::Drop` on both backends — and only the BLOCK's type had not followed it. Gated on the function-body context, which is where two attempts broke: the same `Void` is a decision in a declared-void function and a PLACEHOLDER in a lambda (whose return is inferred from the block type) and in a statement-position block (which may be an enclosing block's value) (loft#1075). `(F-Block)` was written down beside it and D-call-2 opened and closed the same day: a `{ … }` block whose value someone reads dropped its OWN tail, so `fn f() -> integer { { 5 } }` answered null on `--interpret` and `0` on `--native` while the function type-checked — the block's type is its tail's type, and only the value was thrown away (loft#1076)

