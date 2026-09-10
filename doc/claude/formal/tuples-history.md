# formal/tuples-history.md — the deviation register for [tuples.md](tuples.md)

> **The rules are next door.**  [tuples.md](tuples.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **1** — D-tup-10, whose entry lives in the chapter next door because it is still being
worked; this register carries the closed ones.  This line read `OPEN: 0` from 2026-09-05 until
2026-09-10 while D-tup-10 and D-tup-11 were live in [tuples.md](tuples.md) — a register's
headline is a claim about the chapter beside it, and it decayed the moment an entry was opened
somewhere else.  D-tup-12 opened and closed 2026-09-10 (below).  D-tup-9 opened and closed 2026-09-05 (loft#1365 — below: the record and scalar bindings by @PLN153 phase 1, the collection half by the @FR-F-Ret join).  (D-tup-8 opened and closed 2026-09-04, loft#1361 — below; D-tup-7 opened and closed 2026-09-04, loft#1350 — below; D-tup-4's KEYED half CLOSED 2026-08-31, loft#1230); D-tup-5 and D-tup-6 opened and closed
2026-08-28; D-tup-3 opened and closed 2026-08-26; D-tup-2 closed the day the
rule it needed was written down.  Bounded by the oracle note below — **and D-tup-3 is what that
note was warning about**: it was found by giving an element a HEAP type, which this doc's
all-`(integer, integer)` oracle cannot express, so the zero above never covered it.  D-tup-5 and
D-tup-6 are two more from the same blind spot, one axis further: a NULLABLE element, which the
all-`(integer, integer)` oracle cannot express either.

### D-tup-12 — OPENED AND CLOSED (2026-09-10): `_0` was a second spelling of `.0`, on one home only

`(T-Proj)` says a tuple's member is a LITERAL index and a tuple has no other member.  A
record-backed tuple is carried as the synthetic struct `__tuple<…>`, whose attributes are
named `_0`, `_1`, … — and that home's projection site claimed the member only when the next
token was ALREADY an integer.  A guard on the token rather than an answer about it: a named
member fell past it to the ordinary struct-field reader, where `_0` resolved.

So `t._0` READ a `vector<(integer, text)>` loop variable's element and `t._0 = 99` WROTE
through it into the vector, on both backends, while the same source over a plain local was
refused by name.  The five refusing homes (stack local, vector element by a constant or a
variable index, struct field, `&(…)` parameter, function parameter, all-integer return) and
the three admitting ones (`vector<(τ, τ)>` loop variable, nested loop variable, heap-carrying
return) differ by a REPRESENTATION choice with nothing in the source to show it — which is why
the boundary is worth writing down rather than the symptom.

Two more defects sat on the same fallthrough: a named member reported *"Unknown field
`__tuple<integer,text>`.name"*, naming a def the author cannot write (loft#1498's class, whose
predicate `Data::def_is_authored` this path never reached), and the refusal at all THREE homes
left the member in the token stream, so each dragged a second `Expect token ;` behind it.

Closed by giving both questions one home — `tuple_member_not_a_literal` (which also consumes
the member, and marks it `Value::Drop` so `t._0 = 9` does not collapse into `t = 9`) and
`tuple_index_out_of_range` — cited from all three sites.  The lesson is the shape rather than
the bug: THREE copies of one refusal is what let a fourth spelling of it be a token guard
instead, and a guard reads as an answer until you ask what happens when it does not hold.

### D-tup-9 — OPENED AND CLOSED (2026-09-05, loft#1365): a tuple literal member typed by a type variable

`(T-Cons)` copies a heap element INTO a tuple literal and `binding.md (B-Copy)` copies a plain
bind.  D-tup-8 made that hold for a member whose type is KNOWN at the literal; this is the
member whose type is a generic's type variable.  Neither rule has a clause for generics, and
that is the point of the entry: a monomorph is an ordinary program, so `(s, 1)` must behave
the same whether `s` is written `Ctr` or reached through a `T` bound to `Ctr`.

**Closed for a RECORD and for a SCALAR binding (2026-09-05).**  The difficulty is that the
template has to decide something that does not exist yet: a type variable is spelled
`Type::Reference` to its placeholder, so it looks exactly like a record, while what the member
IS — a record to copy, a collection to copy differently, or a scalar with nothing to copy —
exists only per instantiation.  Both one-sided answers were measured and both are wrong.
DECLINING the copy in the template (the shape that shipped for a day) left a struct-bound `T`
aliasing: `s = a; t = (s, 1); s.bump(); t.0.value()` answered `1` for a `Counter { n: 0 }`
where `Counter` written in place answered `0`.  Emitting it UNCONDITIONALLY allocated a record
with the type variable's own row — the layout escape loft#1070's guard refuses — an ICE on
both backends for `pair_sum<T: Addable>(a, b) -> (T, T)` with `T` an integer.

The cure decides per instantiation: the template emits the record copy, and
`Parser::collapse_parametric_tuple_member_copies` removes it again in each monomorph whose
bound type it does not fit.  The test is the block's own contents against that type
(`tuple_member_copy_shape_fits`), never a "this was a type variable" flag — a generic body
also builds tuples from CONCRETE members, and their copies are correct and must not be
touched.  Unwrapping the value is only half of undoing the guess: the template also gave the
tuple ELEMENT the backing's dep, and an element still naming a backing whose copy is gone is
owned by a variable nothing fills, so the store the member holds is freed by nobody — a leak
the first version shipped and the guard's absence would not have caught.  The dep therefore
goes with the copy (`Variables::make_tuple_members_independent`, `make_independent` one level
down, since a tuple carries no deps of its own and `deps_mut` on a `Type::Tuple` is `None`).

Guard `tests/scripts/1365-a-tuple-member-typed-by-a-type-variable-is-copied.loft`: nine
record cells, each against a CONCRETE twin rather than a literal — mutating the local, mutating
through the tuple, the member at index 1, both members typed by the variable, arity three, the
tuple as a return value, a two-field record so a wrong row shows in the value, the literal in a
loop and in an `if` arm — plus four scalar cells as the control, since keeping them green is
what the declining version bought by making the record cells wrong.

**The collection half, closed by the join rather than by the copy pass (2026-09-05).**  With
`T` bound to a `vector` or a keyed collection the template's record copy does not fit either,
so the collapse unwraps it — and the member is then a VIEW of its local
(`Variables::retarget_tuple_member_deps`, `(B-View)`; the first cut stripped the dep and
freed the caller's hash at the callee's exit, which the @FR-F-Ret guard's second call
observed).  The @FR-F-Ret walk (QUALITY.md B7t, calls-history D-call-13) boxes a generic's
`-> (T, …)` at instantiation exactly as a named function's is boxed, so the copy the concrete
path performs at the RETURN applies unchanged and happens once.  Measured on the joined tree,
both backends, no leaked store: `keep<T>(a: T) -> (T, integer)` with a `vector<integer>` reads
`len` 2 through the returned copy after the source grew to 3; with a `hash`, 1 against 2 — the
concrete twin's answers.  Two earlier readings of this entry named a missing free and a stack
tuple ABI as the cause; the ABI was the true one and the walk closed it.  The guard is the
walk's own `a-generic-instance-returns-what-its-concrete-twin-returns.loft` (`vector_tuplocal`,
`keyed_tuplocal`).  A generic's tuple built and NEVER returned keeps the view, and no program
can observe that — an opaque `T` bound to a collection has no mutator to reach it through — so
that last cell is unmeasurable rather than open.

### D-tup-8 — OPENED AND CLOSED (2026-09-04, loft#1361): a tuple with a heap member was shared where the rules say copy

> **The cell this entry's guard did not cross (2026-09-05, QUALITY.md B7t).**  A keyed member
> reaching the tuple literal through a LOCAL bound from a parameter and then RETURNED —
> `s = x; t = (s, 7); return t` — was written into the synthetic `__tuple` record by
> `emit_set_one_element` as a 4-byte header where a struct field write copies
> (`OpReplaceKeyed`): the interpreter wrote into a released, reused store and native refused
> the int for a `DbRef`.  Fixed in that leg; the cell lives in
> `a-generic-instance-returns-what-its-concrete-twin-returns.loft`.  The same walk boxed a
> generic's `-> (T, integer)` at instantiation, which is the return-ABI half of D-tup-9's
> collection case as the sibling stream named it.

`(T-Cons)` copies a heap element INTO a tuple literal and `binding.md (B-Copy)` copies a plain
bind, whole-value and heap alike; `layout.md (L-Tuple)` makes a tuple a synthetic struct, so a
struct's own boundary applies to it.  Three places kept the tuple's stack WORDS instead — and a
heap member's word is its handle.  A whole-tuple bind `u = t` copied the words (the interpreter's
`set_var_tuple`; native's `whole_tuple_clone` emitted `.clone()`, a pointer copy for a `DbRef`
leaf), so `t.0 += [9]` grew `u.0`; the literal's member copy (`tuple_member_owned_copy`) knew a
VECTOR and a KEYED local but not a STRUCT, so `(s, 5); s.v = 9` read 9 through `p.0.v`; and the
destructure's `T1.4` temp read its elements back the same way.  Both backends, the same wrong
values, nothing said so — the walk of `@FR-B-Copy` (QUALITY.md § B7n) found it by moving the
struct's bind through every position a bind can take.

Closed with ONE home rather than four: the whole-tuple bind and the destructure lower onto the
literal's per-member copy, which gained the struct (and struct-enum, and nullable, and nested
tuple) branches; a member read OUT follows `(B-View)` / `(B-View-Base)` — a collection member off
an owned tuple copies (`af = bx.v`), a struct member views, and off a parameter every member views
— through the same `classify_vec_bind` a struct field read takes.  The ownership oracle learned
that a heap member read out of a tuple is a view, which silenced a `lost-write` warning that had
been firing on a write both backends landed.  The return path's unwrap (`tuple_member_copy_source`,
loft#1109) matches the struct branch's shape too.  Guard
`tests/scripts/1361-a-tuple-with-a-heap-member-copies-like-any-other-value.loft` (15 cells,
falsified on both channels); the oracle note below stands — this was found one axis past it.

### D-tup-7 — OPENED AND CLOSED (2026-09-04, loft#1350): a lifetime-tuple result refused to join a tuple literal in an `if`

A lifetime-bearing tuple is ONE notion in two spellings: the stack tuple an author writes —
`([0], "d")`, or a local holding one — and the synthetic `__tuple<…>` record a function's
return is boxed into so `(F-Ret)` can hand every element out owned.  `if c { np(b) } else {
dp }` put the two spellings on the two arms: the then arm yields the record, the else arm is
parsed against the then arm's type, and `convert` has no route from a stack tuple to the
record — *"expected __tuple<vector<integer>,text>, got (vector<integer>, text) on else"*, for
a program that reads as one type to its author.  Written with the literal FIRST it compiled
(the record does convert to the tuple), and with both arms as calls it compiled; the refusal
was one direction of one join.  loft#1349 widened it: a lambda's tuple return is boxed the
same way now, so `if c { lam(b) } else { dp }` moved from an alias to this refusal.

**Closed in `block_result`, by the boxing a function tail already takes.**  An `else` arm
that yields a stack tuple whose element types spell the SAME synthetic name as the expected
record is boxed into its own work-ref (`rewrite_tail_tuple_with_work_ref`) and retyped as
the record, so `parse_if` joins two records — the arm kind a struct literal already is; a
tuple of a different shape keeps the refusal, which is then about the elements.  Guard
`tests/scripts/1350-a-lifetime-tuple-result-joins-a-tuple-literal.loft` (the four join
directions, a named and a fn-ref callee, a literal local and an inline literal, the
mismatched shape that must still refuse), falsified at `1bb5e1b8` on both backends.  Held
fixed and filed apart: a tuple local YIELDED by an arm is moved on `--native` and a later
read refuses to build (loft#1354).

### D-tup-5 — OPENED AND CLOSED (2026-08-28, loft#1122): a member was not parsed against the type its position names

The typing relation checks a tuple element against its member type, so LOFT.md's `⇐` rule —
*the expected type wherever there is one* — makes a member one of those places.  It was pushed
for a DECLARED LOCAL and for nothing else: a local reads its destination from `var_tp`, while a
`return` and a call ARGUMENT have only the channel, and `Type::Tuple` was in none of that
channel's admission lists.  A member whose parse NEEDS the expected type therefore had nothing
to resolve against one position over — a bare variant (`(Dot, 9)`) was REFUSED, and an empty
collection literal (`([], 9)`) answered `t.1 == null` for a member declared `integer`, leaked
the tuple's store, and would not compile on `--native`.

Closed by asking one predicate at each of those push sites (`Parser::tuple_hint_type`).

⚠ **The notion has two spellings and a return only ever shows the second.**  A source-level
`(τ₁, …, τₙ)` is a `Type::Tuple`; a tuple RETURN is promoted to `Reference(__tuple<…>)` — the
synthetic struct carrying the caller's `__retbuf` ABI — before the body is parsed.  Admitting
only `Type::Tuple` at the block tail changed nothing at all, silently, and the measurement is
what said so: the argument cells went green and every return cell stayed red.  Guard:
`tests/scripts/1122-a-tuple-member-is-parsed-against-its-type-in-every-position.loft`.

### D-tup-6 — OPENED AND CLOSED (2026-08-28, loft#1123): a nullable element did not earn the ABI its dense twin earns

`(T-Ret)` says a returned tuple is an INDEPENDENT value.  A tuple return is promoted to the
synthetic-struct ABI when any element carries a lifetime concern, and that predicate read its
argument directly — so `Optional(Reference(W))` answered NO where `Reference(W)` answered yes,
and `-> (W?, integer)` kept the by-value tuple ABI its DENSE twin did not.

On that un-promoted path a tail whose member BUILDS a value is dropped: the tuple is emitted as
a discarded statement and the function returns null.  `--native` read that back as `(null, 0)`
— both members lost, no diagnostic — while `--interpret` answered correctly off stack residue,
so a program passed its tests on one backend and was wrong on the other, and the other is the
default.  The axis is *nullable and PRESENT*: a `null` member was correct (its tail builds
nothing) and so was the dense twin.

Closed by reading through `Optional` in `has_lifetime_concern` — `τ?` has the same storage as
`τ`, which is why `element_stack_align` beside it already peels.  ⚠ That makes a tuple ELEMENT
a `__nullable<S>` slot, and `(N-Store)` read the synthetic wrapper as NON-null, warning that a
`W?` becomes null in `__nullable<W>` — the nullable type saying it is not one.  `τ?`'s second
spelling now has a home (`Data::is_nullable_wrapper`), and the doc there names the ten further
sites that still test it by hand.  Guard:
`tests/scripts/1123-a-nullable-tuple-member-returns-like-its-dense-twin.loft`.

⚠ **That ⚠ was a map, and the sites it pointed at were not swept (loft#1134, closed
2026-08-28).**  Giving the element a `__nullable<S>` slot changed the LAYOUT; nothing taught the
writers, so a member was copied in as a dense `S` — landing field `a` on top of the discriminant
at offset 0 and never setting it.  `(E-Null)`'s guarantee for this representation is *no
collision*, and the collision came straight back: a PRESENT `S { a: 0, … }` read absent, a
`float` first member read absent whenever its low byte was zero, and a member written `null`
read present.  The reason it survived a day is that the mistake was symmetric — the indexed read
projected offset 0 too, so write and read cancelled and the tag-consulting `for` loop was the
only route that looked wrong.

The rule the sweep owes, stated so the next layout change can be checked against it: **a tuple
element whose declared type is `τ?` and whose storage is the tagged `__nullable<τ>` is written
and read through the tag at EVERY position — a collection element, a struct field, a
reassignment, and a nested tuple.**  `Parser::emit_nullable_slot_write` and
`emit_nullable_slot_read` are the pair that hold it, and they spell the discriminant exactly as
`operators.rs::enum_null` does so a slot cannot be written by one and read by the other.  Guard:
`tests/scripts/1134-a-nullable-tuple-element-is-stored-behind-its-tag.loft`.

One position dropped the tag on the way OUT and was fixed straight after (**loft#1138**):
crossing a FUNCTION BOUNDARY.  `convert` unwrapped a `__nullable<S>` by sub-referencing the
`Some` payload without consulting the discriminant, and a sub-ref into an absent slot is a valid
`DbRef` — so an absent value arrived at a callee, and returned from a `-> S?`, as a present
record of zeroes.  Not a tuple question at all: a `vector<S?>` element and a plain struct field
reproduce it identically, so the axis is the boundary and the fix sits in `convert`.

One more consequence of the two spellings closed the same day (**loft#1139**): three sites
RE-DERIVED the synthetic `__tuple<…>` def from the element types they were handed, and the def
is NAMED by the source spelling — so a list read straight off the def's own attributes minted
`__tuple<__nullable<S>,integer>`, a different def with different offsets.  That is why
`v += [f()]` was refused for a tuple with a nullable member while its dense twin was accepted,
and why merely LIFTING the refusal writes the scalar member at byte 16 where the read looks at
24.  `Parser::source_spelling` is the normalisation; the rule it serves is the same one the
write side answers — **a tuple's offsets and its member types come from ONE def**, and any list
that will be used to re-derive that def has to be in the spelling the def is named by.

The split in the unwrap is worth keeping in mind too: only a NULLABLE target reads
through the tag.  A DENSE `S` target keeps the bare payload sub-ref, because `(N-Store)` has
already ruled that it cannot hold absence, and because two sites downstream recognise that
unwrap by its SHAPE — `tail_is_nullable_unwrap` (the #306 view-return materialise) and
`new_record_field_op` both match `Value::Call(OpGetField, …)`.  One spelling per question, rather
than a third spelling both would have to learn.  Guard:
`tests/scripts/1138-an-absent-nullable-struct-stays-absent-across-a-call.loft`.

> **D-tup-4 — OPENED 2026-08-26 (loft#1102); the VECTOR half CLOSED the same day, the KEYED
> half OPEN — a tuple literal ALIASED a heap local while both sibling constructors copied it.**
>
> ```loft
> vl: vector<integer> = [10, 20];
> t = (vl, 9);   s = S { v: vl };   vv = [vl];
> vl[0] = 41;
> t.0[0]  // was 41          s.v[0]  // 10          vv[0][0]  // 10
> ```
>
> Both backends agreed, so this was a shared semantic gap and not a parity bug. `(T-Cons)` said
> nothing about ownership, which is why nothing caught it: **an edge the rules cannot express
> means the RULE wants extending**, and `(T-Cons)` now states the copy.
>
> The struct literal deep-copies its member into the field's own storage and the vector literal
> copies its elements. A tuple has no such storage — its element slot holds a `DbRef` — so it
> stored the source's handle. The store the copy needs does not have to belong to the TUPLE
> though: a frame-local backing owns it and frees it at scope exit, exactly as a hand-written
> `o: vector<T> = []; o += vl; o` does, which is the shape now emitted at the literal.
>
> ⚠ **A shipped DIAGNOSTIC already asserted the fixed behaviour**, and that is the strongest
> argument here — stronger than the aliasing itself. `c = t.0; c[0] = 41` drew
> `warning[lost-write]: a whole-value bind COPIES the heap value (C86), so the mutation lands in
> the copy`, while the write reached `vl` through two levels of binding. A diagnostic that
> describes the contract wrongly is worse than a missing one, because it is believed.
>
> **CLOSED 2026-08-31 (loft#1230).** The keyed half is fixed: a keyed member is copied with
> `OpReplaceKeyed` — the op a STRUCT literal already emitted for its keyed field, and the reason
> both siblings this entry appeals to were independent while the tuple literal was not. The
> paragraph below records why it stayed open, and the blocker it names was removed by loft#1225's
> `TuplePut` arm; what remained was reaching for the copy rather than building one. **A plan was
> filed on the premise that no keyed copy existed anywhere in the language; the premise was
> wrong, and what disproved it was testing the struct-field route.** Three things were needed
> beyond the vector branch: the copy keeps the SOURCE's nullability (built dense it loses its
> ownership dep entering a `τ?` slot and leaks), its result type depends on the copy's own
> variable, and a tuple YIELD unwraps the copy exactly as `synthetic_tuple_return` already did
> for a RETURN — without that a generator leaked one store per keyed kind.
>
> **What WAS not closed, and why — the reason it stayed open until now.** A KEYED collection given to a tuple aliases in the same way
> (`hash<S[k]>`), and the fix here excludes a keyed local deliberately: that shape is a
> pre-existing codegen ICE (three of the four tuple emitters hand-spelled the `DbRef` type set
> and were short by the five keyed collections), reproduced identically on a control binary, and
> the emitter repair lives on a sibling branch. Copying cannot be added to a shape that does not
> compile, so the keyed half stays OPEN and this entry stays open with it.
>
> **A cost this pays and does not yet recover.** A tuple RETURN was already correct — it is
> rewritten to a synthetic `__tuple<…>` record, which copies like any struct — so a returned
> tuple now copies TWICE, once at the literal and once into that record. It is correct and
> measurable (`941-tuple-destructure-owns-its-element.loft` grows the second copy in its IR).
> The cure is the last-use elision `(T-Cons)` now admits — the source is dead after the
> construction, so nobody can tell — which is what the struct constructor already does and what
> `Value::Tuple` is not yet visible to. Not attempted here.
>
> **Measured.** Nine cells on both backends, five of them falsified on a control built at
> `9c1a0e4e`. Emitted IR: three existing corpus programs change, all of them tuple tests, all
> green. Controls: a PARAMETER member, which must keep aliasing its caller (`B-Ref-Alias`); a
> returned tuple after churn; a scalar-only tuple; and DESTRUCTURING — whose left side is parsed
> by the same branch as a literal, so a rewrite that does not exclude an assignment TARGET turns
> those names into expressions and the destructure reports *"left has 0 names"*. That cell needs
> its loop: the names only exist to be rewritten from the SECOND iteration on, which is why the
> first suite run caught it and a single-shot probe would not have. Guard:
> `tests/scripts/1102-a-tuple-literal-copies-a-heap-member.loft`.
>
> Unrelated and still open beside it: `t = ([10, 20], 9)` is refused as a type change
> (reproduced on the control; repaired on the sibling branch).

> **D-tup-3 — OPENED AND CLOSED (2026-08-26, loft#1104) — a tuple element is a projection that
> the ownership machinery could not read as one.** `(T-Proj)` says `t.i` is element `i`, and for a
> heap element that means a `DbRef` into the store the element lies in — the same thing `b.s` and
> `v[0]` are. The @P290 borrow-vs-owned bracket could not see it, so a call whose return may
> borrow the argument kept its conservative answer and LEAKED one record per call, both backends:
>
> ```loft
> fn pick(s: S, c: boolean) -> S { if c { s } else { mk() } }
> fn f(c: boolean) -> integer { s = S { a: 7 }; t = (s, 9); r = pick(t.0, c); r.a }   // 1 record / call
> ```
>
> `pick(q, …)`, `pick(b.s, …)` and `pick(v[0], …)` were all clean. The bracket protects a store by
> naming it through a variable whose VALUE is a `DbRef`, and `view_root_slots` walks a projection
> chain to that variable using `is_projection_op` — which is keyed on `OpGetField` / `OpGetVector`.
> A tuple element is neither: it is `Value::TupleGet`, not a `Call` at all.
>
> **Two cures are unavailable, and which ones is the useful part.** Widening the op list cannot
> reach a shape that is not an op. Naming the TUPLE cannot work either — the bracket protects the
> store a `DbRef` variable points at, and a tuple is not a `DbRef`; its ELEMENT carries the store.
> So the argument is bound to a temp, which is exactly the hand-written spelling that was always
> clean (`e = t.0; pick(e, …)`) and emits the same code — the argument loft#1029 used for the
> inline-construction family, one spelling over.  Closed in `Scopes::scan_args`, gated as its
> sibling is: a heap-carrying element, at a `returns_borrowed_view` callee, and nothing else —
> binding an argument reorders it relative to its left-hand siblings, which is a cost worth paying
> only where the alternative is a leak.
>
> ⚠ **THE SHAPE-SPECIFIC ARM IS GONE, AND MEASURING IT IS WHY.** It was written as
> `tuple_elem_borrow_source`, typing the temp as the tuple ELEMENT's own type, deps and all.
> loft#1105 then answered the same question in general (*can the bracket NAME this?*) and its arm
> sat AHEAD of this one in the chain, so a `TupleGet` — which is not a `Var` and which
> `bracket_can_name` refuses — never reached the tuple arm again: **0 reaches across the 875-file
> corpus.** Deleting it leaves the emitted IR byte-identical over all 875.
>
> And it was not merely dead. Forced ahead of the general arm it CHANGES the emit, in the one
> direction that matters: the tuple's declared element type still carries the dep of the local the
> literal was built FROM (`t = (s, 9)` types as `(ref(S)["s"], integer)`), so the temp came out
> `ref(S)["s"]` — while the hand-written `e = t.0` this cure exists to match measures
> `ref(S)["t"]`. `(T-Cons)` makes a tuple literal COPY its heap source (D-tup-4), so the element
> lies in the TUPLE's store and `["s"]` is a dep the copy already invalidated. The general arm
> reads the value's actual source and answers `["t"]`. **A shape-specific answer that agrees with
> the general one on every case it can still reach, and disagrees with the ORACLE on the one case
> it cannot, is not precision being preserved — it is a second derivation drifting.**
>
> ⚠ **The bare `t.0` is one cell of six, and the other five were found by moving the axes the
> first sweep pinned** — the chain's OP, the container the tuple sits in, and the index.
> `pick(t.0.s, …)`, `pick(t.0[0], …)` and `pick(t.1.s, …)` put a projection CHAIN above the
> element; `pick(t.0.0, …)` and `pick(vt[0].0, …)` read the element off something that is not a
> plain variable, which the parser lowers to a `tuple_tmp` block; and `pick(t.0.0.s, …)` is both
> at once, invisible until the block shape had a cure. **WHICH NODE gets the name is the whole
> distinction, because it decides the type the temp carries.** A chain is RE-BASED on the temp
> rather than bound: the ELEMENT's type is one the tuple declares, while the chain's RESULT type
> would have to be inferred, and a temp typed off the CALLEE'S PARAMETER instead carries no deps —
> it then reads as an OWNER of a store it only views, and the free that follows is a
> use-after-free rather than a leak (QUALITY.md § B6k).
>
> ⚠ **The class, and this is its fourth instance in a week: one notion, two spellings, one looked
> for.** A projection resolved by OP NAME cannot see the `TupleGet` spelling; the same blindness
> reaches `Parser::expr_borrows_local` (latent there — the deps leg covers what the op list
> cannot). The blindness is not findable from the symptom: searching for the spelling you DO match
> returns every site that gets it right, and the sites that get it wrong contain nothing to search
> for. `scripts/ir_walker_audit.py spellings` counts the class — 18 functions resolve a projection
> by op name and 2 handled the tuple spelling, one of which is the arm deleted above, so the
> handler count is now 1 against 18. See `IMPLEMENTATIONS.md` § *One notion, how many SPELLINGS?*
>
> **Measured.** Nine cells, both backends, values identical before and after — this is a pure
> leak, so `--interpret` under `LOFT_STRICT_STORES=1` is the instrument and the assertions score
> nothing. On a control binary built at `9c1a0e4e` the two record-element cells report
> `kt=78 S1104×50` over 25 rounds each; after, clean, and clean under `LOFT_POISON=1` too.
> Emitted IR over the corpus: **no existing program changes** — only the guard. Controls: the
> three already-nameable spellings, the hand-written binding, a SCALAR tuple element (which
> carries no store and must not be bound) and a callee that does not return a borrowed view.
> Guard: `tests/scripts/1104-a-tuple-element-argument-borrow-witness.loft`, scored by the wrap
> harness's leak gate — `loft --tests` cannot fail it even with `LOFT_STRICT_STORES=1`.

> **The rule extended (2026-09-03, with binding.md D-bind-11's close).** `(T-Ref-El)` said
> *"every τᵢ must be one of integer, float, single, character, boolean"* — a statement of the
> stack form's reach, and the deviation binding.md carried against `B-Ref-Alias`.  It now
> admits what a struct field can hold, and a new `(T-Ref-Rep)` says which representation a
> `&(…)` names: the stack for an all-scalar tuple, the `__tuple<…>` record otherwise — the
> record a heap-tuple return and a loop variable already were.  `(T-Ref-Src)` gained the
> parameter half of its source rule.  Spec-may-adjust in the ROADMAP's sense, and it adjusted
> TOWARD the more general rule, not away from it.
>
> **D-tup-1 — CLOSED (2026-08-20) — the reference tuple has a rule.** This doc specified
> construction, projection, destructuring and returns and said nothing about `&(τ₁, …, τₙ)` —
> the composition of `&` ([binding.md](binding.md)) with a tuple. Both halves were specified and
> their composition was not, which is how the two backends came to represent it differently with
> nothing to catch them (`--native`: a Rust stack tuple by `&mut`; interpreter: a record through
> a DbRef), and how loft#1006 reached codegen as an internal compiler error.
>
> `T-Ref` / `T-Ref-El` above now state what a `&(…)` denotes and which element types it admits.
> Extending the rule is what the [README](README.md) doctrine asks for at an edge the rules
> cannot express, and writing it down is what showed the admitted set had been **three lists that
> disagreed**: the signature guard admitted `single` and a function reference that codegen then
> died on, and refused `boolean`, which every layer could always have handled. There is one list
> now (`data::ref_tuple_element_ok`), read by the guard and by both `RefTupleGet` / `RefTuplePut`
> arms, so the rule and the implementation cannot drift apart again. Measured on both backends
> across all five admitted element types plus the four refused ones. Tracked against binding.md's
> D-bind-11, which carries the measurement.
>
> ⚠ **The last sentence was too strong, and D-tup-2 below is why.** One list is necessary and was
> not sufficient: a list is only consulted where somebody calls it, and only one of the two sites
> that build a `RefVar(Tuple)` does.

> **D-tup-2 — CLOSED (2026-08-23) — the admitted-element rule is now asked at every
> construction site, and the local path it exposed is implemented.** `T-Ref-El` names which
> element types a `&(…)` admits and `data::ref_tuple_element_ok` is the single list that answers
> it, but only the *signature* path consulted it. `Parser::ref_var_type` is now the one place a
> `&` in source becomes a `Type::RefVar`, so the parameter, the annotated local and the inferred
> `b = &a` all ask it, and a `&(…)` a signature refuses cannot be accepted at a local. Guard
> `tests/scripts/reference-tuple-local-binding.loft` (what must work) +
> `102-expected-errors.loft` (the four refusals); proven to fail on a pristine tree at
> `1e9d7910` — 6 of 7 cells on `--interpret`, 7 of 7 on `--native`.
>
> ⚠ **The entry named the ICE, and the ICE was the mild half.** Measured across positions and
> element types rather than at the filed cell, the whole `&(…)` LOCAL was unimplemented, at every
> element type including the admitted ones, and the loudness varied with what the tuple happened
> to hold:
>
> | written | was |
> |---|---|
> | `b = &a` | the `&` was **DROPPED**: the IR typed `b` a plain tuple and copied it, so `b.0 = 5` left `a` untouched, silently, on both backends |
> | `b: &(integer, integer) = a` | typed a reference over a value — the interpreter read an ELEMENT as a store index (`(7, 9)` gave *"index is 9"*) and `--native` handed the user a raw rustc `E0308` |
> | `b: &(boolean, boolean) = a` | answered `truefalse` where the swap says `falsetrue`, **exit code 0** |
> | `b: &(float, float) = a` | answered `null` for a present element |
> | `b: &(text, text) = a` | the filed ICE |
>
> So the register read `OPEN: 1` against a `silent-wrong` and a wrong-answer cell that no
> deviation named, because the entry inherited the ICE from the report that raised it. **Both
> backends agreed on every one of those**, which is why the tuple differential the doc leans on
> (D-op-1) was structurally blind: the two implementations were wrong in the same way.
>
> The fix is the one the rule asked for — the chokepoint, not a second call beside the first —
> plus the mechanism the chokepoint then had to have something to admit: a tuple local lives in
> the FRAME, so it joins the scalars at `OpCreateStack`, which is exactly the stack ref a `&(…)`
> PARAMETER is already handed at its call site. Native represents the local link as the raw
> `*mut (…)` @PLN87 L1 gives every local link (raw so the source stays readable beside it, which
> is legal loft and not legal Rust borrowing), and two sites now read one predicate,
> `generation::is_raw_tuple_link`, to decide it — the element base and the call that forwards
> the local to a `&(…)` parameter.
>
> ⚠ **`T-Ref-El` is a fact about this BINDING, not about tuples.** Measured while picking the
> chokepoint: the record-backed `RefVar(Tuple)` a `for` loop builds over a `vector<(text, text)>`
> reads and WRITES its elements correctly on both backends. It reaches a real record, so the
> layout limitation the refusal exists for does not apply to it. Putting the gate in a universal
> `RefVar(Tuple)` constructor would have refused a shape that works — which is why the
> chokepoint is *the `&` written in source*, and why `T-Ref` now says stack-backed out loud.
>
> The one shape left refused rather than linked is a tuple PLACE (`b = &v[0]`, `b = &s.pair`),
> now `T-Ref-Src`. It used to bind silently to a COPY — `b.0 = 9` wrote the copy and the source
> was unchanged, with no diagnostic and both backends agreeing. B-Ref-Reshape settles what to do
> there: loft declines rather than downgrading a reference to a copy.

> **D-tup-3 — CLOSED (2026-08-20) — a nullable element at a tuple POSITION.** This doc
> specified construction, projection, destructuring and returns, and `types.md` @PLN25
> `(N-Decl)` specified that a non-null `τ` stored into a `τ?` slot is not a type change.
> Their composition was not specified, and `(N-Decl)` peeled one `Optional` at the TOP, so
> a `τ?` sitting at a tuple position was never seen: `c: (text?, integer) = ("c0", 3)` was
> refused as a declared LOCAL while the identical type was accepted as a RETURN (loft#1034).
>
> That is D-tup-1's shape a second time — two specified halves, an unspecified composition,
> two sites answering differently with nothing to catch them. `(N-Decl)` now reads
> element-wise (`Variables::decl_accepts`, recursive through nested tuples), and the
> assignment path routes a tuple target through the SAME `convert` the return position
> always used, rather than growing a second opinion beside it.
>
> ⚠ **The refusal was the loud half.** The silent half was that a `null` ELEMENT was never
> converted to the element type's sentinel — it stored the empty text and answered `false`
> to `== null`. A fix that only widened the typing check would have turned a compile error
> into a wrong answer, which is why the guard's null-element cell is load-bearing.
>
> Direction preserved: the widening is `τ → τ?` only, so `(text, integer) ← (text?, integer)`
> remains the `(N-Store)` violation.

- **Conformance is differential** — tuples are enforced across the two backends by the @PLN89
  oracle (D-op-1): `17-tuples-recursion` carries construction, projection, destructuring, and
  tuple returns, precisely because the native layout (a synthetic `__tuple<…>` struct, inline
  bytes) differs from the interpreter's. A divergence in element order, value, or type is caught
  there.
- ⚠ **…and it carries no NESTED tuple with a `fn(…)` inside it — a second axis, measured
  2026-08-22.** `t: ((fn(integer) -> integer, integer), text) = ((dbl, 1), "z")` — a program
  with no assignment anywhere in it — panicked `fn_call_ref: fn_var=16 < 20` on the
  interpreter and was refused by rustc on `--native`, while the cell that touched no
  function at all (reading the plain members beside it) failed hardest, with an ICE. Depth
  was the axis loft#1069's own fix held fixed: it taught the tuple literal that a fn-ref
  member is the whole 20-byte pair and read the TOP-LEVEL members only, so everything it
  repaired was broken again one level in. Three sites had that shallow reading — the
  interpreter's literal push, the native emitter's declared-slot hand-down (and its gate),
  and the native fn-ref reachability walk — and all three now decide with ONE predicate,
  `data::tuple_carries_fn_ref`, which sees through nesting. That it is one function and not
  three copies is the D-tup-1 lesson applied before it could bite: three lists that
  disagreed is exactly what loft#1006 was. Guard
  `tests/scripts/fn-ref-in-a-nested-tuple.loft`, proven to fail on a pristine tree on both
  backends. The two REFUSALS left at this position — a short lambda not inferred inside a
  nested literal, and a forward-referenced fn name not resolving in any tuple literal — were
  loft#1073, and are closed (2026-08-22, guard
  `tests/scripts/tuple-literal-member-fn-inference.loft`). Both were the same shape one level
  in: `(T-Chk)`'s push read the TOP-LEVEL members, so a member that merely CONTAINS a
  `fn(…)` seeded nothing; and `change_var_type` accepted a bare `Unknown` source as pass 1's
  placeholder but not the same fact inside a composite, so `(later, 1)` was measured against
  the declared type and refused — the mirror of loft#944, which made that statement about the
  variable's own type.
- ⚠ **…but the oracle's elements are all `(integer, integer)`.** It carries no `text`, and that
  gap is measured, not theoretical: this doc read `OPEN: 0` through **two** live tuple deviations
  that the differential it leans on could not see — loft#1004 (a tuple's `text` element written
  one index too high: silent wrong element, silent lost write, SIGSEGV) and loft#1005 (a tuple
  `text` parameter that would not compile on `--native` at all). A `text` element is the first
  place the native layout stops being inline bytes, so it is exactly where a layout differential
  earns its keep. Widening `17-tuples-recursion` to a heap element type is the fix; until then
  the zero above is bounded by what the oracle covers.
- ⚠ **`(T-Cons)` says nothing about OWNERSHIP, and the third element type shows why that is a
  gap rather than a silence.** Given a heap LOCAL, a tuple literal stores its handle while a
  struct literal and a vector literal both COPY (`t = (vl, 9)` sees a later `vl[0] = 41`;
  `S { v: vl }` and `[vl]` do not, both backends). So a tuple element is aliased without the
  `&` that [binding.md](binding.md) `B-Copy` says aliasing requires — while `(T-Ref-El)` above
  REFUSES a collection element in the `&(…)` form that asks for it. Which of the two answers is
  the rule is an open design question (**loft#1102**); either way `(T-Cons)` owes a clause, and
  the `OPEN: 0` above does not cover this because the oracle carries no collection element
  either.

## Carried by tuples.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [tuples.md](tuples.md) now states only what is open.

### D-tup-4's keyed half, and why the zero stood over it

`D-tup-4`'s KEYED half closed 2026-08-31 (loft#1230): a keyed collection given to a tuple is now
COPIED like its vector twin, so `(T-Cons)`'s independence holds for every element type.

⚠ **The zero above is only as strong as the Conformance list below it, and that list checked
`(T-Cons)`'s copy with a VECTOR** — the one element type that already obeyed it. The keyed half
stood for five days after the vector half closed because the rule's own example exercised the
passing shape. A conformance entry that names one member of a family is a claim about that
member, not the family.

### the status line formal/README.md's area table carried until 2026-09-04

**0 open** (2026-08-31) — D-tup-1 closed 2026-08-20 (the reference tuple has a rule; `&(τ,…)`'s SCALAR-only restriction is now binding.md's D-bind-11, not an unspecified composition), and D-tup-4's keyed half closed 2026-08-31 (loft#1230: a keyed collection given to a tuple is COPIED like its vector twin, so `(T-Cons)`'s independence holds for every element type) — positional products (n≥2); `.i` a compile-time index; `(a,b) = …` destructuring; tuple returns. ⚠ its differential oracle is all-`(integer, integer)`: the doc read `0 open` through loft#1004 and loft#1005, both `text`-element deviations it could not see
