<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# formal/tuples-history.md — the deviation register for [tuples.md](tuples.md)

> **The rules are next door.**  [tuples.md](tuples.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0** — D-tup-10 closed 2026-09-16 by an owner ruling (absence belongs to the `?`, so
`?`/`??` reach the in-flight tuple alone and a tuple a program writes down exists); its entry
stays in the chapter next door, where it was worked, with the measurements the ruling was made
on.  This line read `OPEN: 0` from 2026-09-05 until
2026-09-10 while D-tup-10 and D-tup-11 were live in [tuples.md](tuples.md) — a register's
headline is a claim about the chapter beside it, and it decayed the moment an entry was opened
somewhere else.  D-tup-15 opened and closed 2026-09-22 (loft#1590, below).  D-tup-14 opened and closed 2026-09-14 (below).  D-tup-12 opened and closed 2026-09-10 (below).  D-tup-9 opened and closed 2026-09-05 (loft#1365 — below: the record and scalar bindings by @PLN153 phase 1, the collection half by the @FR-F-Ret join).  (D-tup-8 opened and closed 2026-09-04, loft#1361 — below; D-tup-7 opened and closed 2026-09-04, loft#1350 — below; D-tup-4's KEYED half CLOSED 2026-08-31, loft#1230); D-tup-5 and D-tup-6 opened and closed
2026-08-28; D-tup-3 opened and closed 2026-08-26; D-tup-2 closed the day the
rule it needed was written down.  Bounded by the oracle note below — **and D-tup-3 is what that
note was warning about**: it was found by giving an element a HEAP type, which this doc's
all-`(integer, integer)` oracle cannot express, so the zero above never covered it.  D-tup-5 and
D-tup-6 are two more from the same blind spot, one axis further: a NULLABLE element, which the
all-`(integer, integer)` oracle cannot express either.

### D-tup-15 — OPENED AND CLOSED (2026-09-22): a tuple whose member 0 held a string was refused

`(T-Cons)` constructs a tuple from any members, and `t = (["a"], 1)` was refused on both backends:
*"Variable 't' cannot change type from vector<text> to (vector<text>, integer)"*.  An
assignment's destination is the accumulator a heap-building right-hand side adopts, and member 0
of `( … )` is parsed before a `,` proves the group a tuple, so a look-ahead asks first and a
tuple's member 0 gets a temp of its own.  That walk stopped at every string literal and answered
"not a tuple", so member 0 adopted the destination and typed it as the member.

Closed in two halves, both on 2026-09-22.  A string without a hole is crossed like any other
token, once every remembered token carried the scanner's hole state (`ScanState`).  A string that
OPENS a hole is followed the way `parse_string` follows it, because every token the look-ahead
reads is replayed to the real parse rather than scanned again: the hole's first token in the
string's mode, the hole's expression as code, the `}` resuming the string through `set_mode`,
and a spec's fill and flags in `Formatting` mode and its width as code
(`Parser::peek_tuple_literal`, `skip_format_spec_ahead`).  Answering "tuple" at every hole
instead would have refused `v = (["{x}"] + w)`, which needs the destination.  Guards
`tests/scripts/1590-a-tuple-whose-first-member-holds-a-string-binds.loft` and
`tests/scripts/1590-a-tuple-member-string-may-open-a-hole.loft`.

### D-tup-14 — OPENED AND CLOSED (2026-09-14): a tuple with an enum member linked as a record

`(T-Ref-El)` counts a value enum among the scalar elements, so `(T-Ref-Rep)` makes a `&` to a
`(Col, integer)` local the stack tuple itself.  The parser's `&` lowering asked its own scalar list,
which left the value enum out, so the tuple read as one with a heap element and took the
record-backed link: `t = (Col.Red, 5); c = &t; c.1 = 9` panicked in the allocator on `--interpret`
and did not compile on `--native` (rustc E0308).  The `&(Col, integer)` PARAMETER and a
`(Col, text)` local, which the rule makes record-backed, were already right, and so was a
`(boolean, integer)` local.  Closed by asking `data::is_scalar`, the shared predicate that counts
the value enum, together with `binding.md` D-bind-40.  Guard
`tests/scripts/an-enum-link-reads-and-writes-through-its-own-op.loft` (`tuple_link`, with a
boolean-member control).

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
observed).  The @FR-F-Ret walk (QUALITY-history.md B7t, calls-history D-call-13) boxes a generic's
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

> **The cell this entry's guard did not cross (2026-09-05, QUALITY-history.md B7t).**  A keyed member
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
values, nothing said so — the walk of `@FR-B-Copy` (QUALITY-history.md § B7n) found it by moving the
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
> use-after-free rather than a leak (QUALITY-history.md § B6k).
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

## Deviations carried by tuples.md until 2026-09-29

Closed entries moved here from the rules chapter's register (RELEASE.md § 5b), as written.

- **D-tup-19** *(CLOSED 2026-09-27, loft#1698)* — `(T-Proj)` / `(B-Copy)`: D-tup-18's faces
  at a tuple held DIRECTLY as a vector element.  `v[i]` unboxes the element's `__tuple<…>`
  record into a stack tuple, and `v[i].k` read the member off that copy, so it was no place:
  `v[i].1 = [66]` appended, `+=` concatenated the result with itself (also nested, in a
  field-held vector, through a `&` parameter), a text `+=` panicked on the interpreter and did
  not compile natively, a text `=` was refused as a constant, a scalar member write was "not
  implemented", and an index with a side effect ran twice.  A whole-element bind and a
  destructure left a heap member a second name for the element's store while the scalar
  members were copies — a mix no rule describes.  **Fix.**  `v[i].k` off an element read FROM
  A PLACE is the member read off the element's record (`Parser::stored_tuple_member_place`),
  the shape a struct element's `r[i].b` has, so the place machinery that was already right for
  it applies; a nested tuple keeps the address in its temp when the index cannot be repeated.
  The whole-element bind takes D-tup-18's member copy inside the unbox, and a destructure off
  an owned local's element copies as that bind does.  The whole read is a VALUE, so
  `(B-View-Depth)` does not reach it (binding.md).  Found on the way, and silent on every
  tree: a vector CONSTANT whose tuple element holds a record or a collection pre-built that
  member EMPTY (a struct member's fields read 0, a vector member `[]`), because the member is
  written by a copy the constant builder's literal field writes never mention.  A struct
  element holding a nested record was already refused for that reason; the tuple spelling is
  refused the same way now, and a literal-bodied function returning one keeps its call.
  Guards `tests/scripts/1698-a-tuple-held-as-a-vector-element-is-a-place.loft` (the silent
  cells), `1698b-…` (the member writes that were refused or crashed), `1698c-…` (the
  constant).

- **D-tup-18** *(CLOSED 2026-09-26, loft#1689)* — `(T-Cons)` / `(B-Copy)`: four faces of a tuple
  heap member that was not its own.  (1) `t = k.p` for a tuple-typed FIELD bound the record's
  own members — the field read is the tuple of its member reads and never reached the
  whole-tuple bind's member copy, which matched only a tuple VARIABLE — so a vector member was a
  second name for the field's store (both backends) and a text member a borrow of the record
  (interpreter): replacing the record, rewriting the field or removing the element showed
  through `t`.  (2) A TEXT member read from a place — `(s, 1)`, `(k.s, 1)`, `(v[0], 1)` — was
  never copied on the interpreter, where a tuple's text member is a borrowed string whose type
  records its place: a text local reassigned read the new text, a removed element `null`;
  `--native` holds a `String`.  (3) Writing a vector member through the record, `k.p.1 = [9]`,
  panicked on both backends: the refill used the parent-field append, and the member's byte
  offset is no field of `K`, so `Stores::field_nr` answered field 0.  (4) A whole-tuple write to
  the field, `k.p = (3, [9])`, APPENDED to the vector member (`[7,8,9]`), and `k.p = (4, [])`
  crashed (the interpreter read a corrupt reference, `--native` did not compile), on every tree.
  **Fix.**  The bind reaches `tuple_member_owned_copy` for a tuple-node source too; that home
  has a text leg (a frame-owned work text, the copy a format string gets), keyed on the
  member TYPE's deps; the append chooses the field form only where the access names a field of
  its owner (`Parser::field_access_names_a_field`); the member write replaces
  (`OpReplaceVector`, identity-safe per `heap.md (H-CopySelf)`) and an empty literal clears.
  Guard `tests/scripts/1689-a-tuples-heap-member-is-its-own-copy.loft`.  Found at the corpus's
  thinnest crossing in the rising `tuple` class (a tuple held in a keyed collection).

- **D-tup-17** *(CLOSED 2026-09-26, found with loft#1682)* — `(T-Absent)`'s ruling that an absent
  tuple is the present tuple of null members had no `--native` spelling for a STACK tuple: the
  emitter's typed null (`write_typed_null_in`) rendered a `Type::Tuple` as `()`, so an
  EXHAUSTIVE enum `match` answering `(integer, integer)` — whose parser fallback arm is `null` —
  did not compile natively at all (rustc E0308, *"expected (i64, i64), found ()"*), with no
  null member anywhere in the program; a `_ =>` arm hid it, and the interpreter answered.  On
  every tree since tuples were stack values (the sibling checkout's binary of two days before
  fails the same way).  **Fix.**  A stack tuple's null is the tuple of its members' nulls; a
  tuple with a heap or text member is a `__tuple<…>` record and was already `DbRef::NULL`.
  Cell `exhaustive enum match into a stack tuple` of loft#1682's guard.

- **D-tup-16** *(CLOSED 2026-09-25, loft#1673)* — `(T-Ref-El)` says of a `&(…)` binding
  *"Never a runtime fault and never an ICE"*, and a whole-value READ or WRITE of the
  STACK-backed form was an ICE on both backends: `take(p)`, `return p` and `q = p` panicked in
  the codegen link-read ladder, `p = (…)` in its write twin.  The RECORD-backed twin read whole
  correctly and REFUSED the whole write with a type error — so the two representations
  `(T-Ref-Rep)` gives differed in what a program could do, which `@FR-B-Ref-Uniform` rules out.

  ✅ **Closed at the rules' own answer, on both representations.**  A whole READ is the tuple of
  the element reads through the link (the interpreter's `generate_var`; native derefs a local
  link's `*mut (…)`, as it already did a parameter's `&mut`), so a bind `q = p` is a COPY
  (`(B-Copy)`) and the caller's tuple is untouched.  A whole WRITE writes THROUGH the link and
  REPLACES every member, the same as `p.0 = a; p.1 = b`.  The right-hand side is parsed
  against the tuple the link names, so a list literal becomes a `hash` member as it would for
  a tuple local of that type.  A stack-backed tuple is written element by element from a temp.
  A record-backed tuple is built as a record of the link's own `__tuple<…>` type and then
  copied over the linked record whole.  Each heap member gets its own storage before
  anything is overwritten, so `p = (p.1, p.0)` swaps.  The link bind `q = &t` stays a bind.
  ⚠ The fix's first version wrote a record-backed member with `set_field`, which only
  initialises a FRESH record. It appended to a live vector member (`["old"]` became
  `["old", "new", "pair"]`) and left a `hash` member empty. Nothing reported either: the guard's
  write cells started from an EMPTY member, where appending and replacing give the same
  result. A peer's cells found it (loft2-d9, 2026-09-25). The c-cells now start from a
  non-empty member and check element 0 as well as the length.
  The two smaller faces went with it: a record-backed link refuses `"{p}"` exactly as its
  value tuple does, and a diagnostic names it `&(text, text)` rather than the `__tuple<…>`
  record.  Guard: `tests/scripts/1673-a-tuple-link-is-read-and-written-whole-like-a-tuple.loft`
  (argument, return, bind-is-a-copy, write, swap, a write in a loop, a local link), both
  representations per cell.  The bind is a copy on BOTH representations, so they agree. The
  record-backed one already copied before this fix, and deeply: `q = p` over
  `&(vector<text>, integer)` copies the vector, so `q.0 += […]` leaves `p` alone. That is
  `(B-Copy)`'s whole-value row. It also means a record-backed `q = p` ALLOCATES, so it is not
  free, and a caller who wants to share the tuple should write `q = &p`.  The DESTRUCTURE half —
  `(a, b) = p` on both representations and from both sources — is guarded by
  `tests/scripts/a-destructure-unpacks-a-reference-tuple.loft`.

- **D-tup-10** *(CLOSED 2026-09-16, loft#1423 / loft#1451)* — `(T-Absent)` said no
  `Optional(Tuple)` exists while the code minted one wherever absence is synthesised:
  `Type::optional` wrapped a `Tuple` like any other type.

  ✅ **CLOSED 2026-09-16 by an owner ruling, because the rules could not settle it.**  The last
  live cell was the `?` discharge: it answered the members' defaults on the in-flight
  `(τ₁, …, τₙ)?` and was REFUSED on the `(τ₁?, …, τₙ?)` this entry's own rule called the same
  type.  One expression decided it — `u = v[i]; u?` answered while
  `u: (integer?, text?) = v[i]; u?` refused — and the axis was member-nullability, not the home:
  the in-flight spelling over a `vector<(τ?, τ?)>` refused too, and so did the boxed one.

  Measuring the cure is what showed the rules did not reach: `(D-Opt)` gives
  `construct_default(τ?) = null`, so a literal member-wise default of `(integer?, text?)` is
  `(null, null)` — the absent tuple itself, not `(0, "")`.  The case that separates the readings
  is a PARTLY PRESENT tuple, which the written spelling can hold and an index miss cannot
  produce: `??` and `==` are WHOLE-wise on both spellings and agree, so typing the discharge
  `(integer, text)` would put a live null in a non-null slot on the pass-through path — the lie
  `(N-Store)` exists to refuse.  So the `≡` above held at the two ENDS, all-present and all-null,
  and never as type equality.

  **The ruling:** absence belongs to the `?`, not to the members.  Only an out-of-range read
  makes a tuple that is not there; `(null, null)` written down is a tuple that EXISTS holding two
  nulls.  `?` and `??` discharge an absence, so they reach the in-flight tuple alone and are a
  compile error on a tuple that exists; `t == null` is false for one; `==` compares tuples that
  exist and answers false when a side is absent; and an in-flight tuple STORED into a slot whose
  members are each nullable is the sanctioned move.  `(T-Absent)` above is rewritten to that, and
  "every member null" is demoted from the DEFINITION of absence to how the in-flight value is
  represented.  Guards: `1477` (the member fold, its cells moved to the in-flight spelling),
  `1477b` (the null question in both spellings, plus the no-tag consequence),
  `a-tuple-that-exists-has-nothing-to-discharge.loft` (the refusal and its `v[0]` controls), and
  `an-absent-tuple-meets-the-type-its-author-declares.loft` (the landing).

  ⚠ **The refusal is gated on the SUBJECT, not the type, and that is measured.**  `(N-Index)`
  trusts a CONSTANT index, so `v[0]` is typed without the `?` while still being an element read
  that can miss — nine shipped scripts discharge one, and `823` asserts it outright.  A gate
  written on the type alone passes every refusal cell and breaks all nine.

  ⚠ **It also retired a close of its own.**  `??` over the boxed member-nullable spelling was
  made to work on 2026-09-16 (guard `a-boxed-tuple-discharges-its-null-like-the-stack-one.loft`)
  and the ruling makes it an error; that guard is deleted and its one surviving control, the
  generic `-> T?` route, moved into the refusal guard.  Unmerged, so the cost was one commit.

  That close left CODE behind, and it is recorded here rather than removed on the spot:
  `Parser::boxed_tuple_members_modulo_null` answers *"do this boxed tuple's members differ from a
  stack tuple's elements by nothing but each member's `?`"*, which was the gate widening that let
  the boxed `??` through.  It is not DEAD — the `??` refusal reports and CONTINUES, so the default
  still parses and the arm still runs — and neither clippy nor the gate flags it; what it no
  longer has is a reason.  Removing it is a separate measured step, because the predicate is also
  what keeps a coalesce result from being typed as the NON-null stack tuple, which `(N-Store)`
  refuses, and nothing currently distinguishes those two callers.  Whoever takes it should score
  the refusal cells AND `1477`'s partly-present cells, not the refusal alone.

  The record below is kept as it stood, because the measurements in it are what the ruling was
  made on.

  ⚠ **RE-MEASURED 2026-09-09, both backends: three of the four cells this entry called REFUSED
  now work, and only one still does.**  The entry read as a four-cell refusal and is a one-cell
  one.  What still fails is the LANDING type: `w: (integer?, integer?) = v[i]` is refused as
  *"cannot change type from `(integer?, integer?)` to `(integer, integer)?`"* — the two spellings
  of one notion meeting, which is the deviation itself.  What now WORKS: `v[i].0` by a variable
  index answers `null(oob)` where the entry says it is refused by name; `t == null` answers
  `true` on BOTH spellings, the member-nullable and the in-flight one, where the entry says
  *"No matching operator '=='"*.  `v[i] ?? d` and `v[i]?` still answer right (`1`, `0`).  The
  three were carried along by loft#1450's `(N-Chain)` work and @PLN25's null model rather than
  closed deliberately, which is exactly why a deviation's measured cells are a claim to
  re-measure and not a record to cite.

  ⚠ **RE-MEASURED AGAIN 2026-09-12: the one cell still stands, and both issues it names are now
  CLOSED.**  `w: (integer?, integer?) = v[i]` is still refused on both backends with the same
  *"cannot change type from `(integer?, integer?)` to `(integer, integer)?`"*.  So this entry is
  the case that says an issue closing is not a deviation closing — loft#1423 and loft#1451 each
  closed on their own cells while the notion the entry is about did not.  That is why
  `rule_tags.py registers --issues` reports such a pair to RE-MEASURE rather than calling it
  closed; four of the five entries it flagged the first time it ran were stale, and this one
  was not.

  ⚠ **TRIED 2026-09-15 and taken back out: building the member-nullable tuple in `Type::optional`
  does not close this entry, because the rules around it disagree once the in-flight spelling is
  gone.**  `Type::optional((τ₁, …, τₙ))` returning `(τ₁?, …, τₙ?)` — the rule's own sentence, at
  the one home every producer of absence asks — was built and measured on both backends:
  - it CLOSED the three refusals: `w: (integer?, text?) = v[i]` by a plain local index (the cell
    above; a LOOP-variable index already landed, so a probe written with one reads green on the
    unfixed build), `fn get(…) -> (integer?, text?) { return v[i]; }` (refused with a false
    "nullable stored into a non-null return" warning), and a generic `T?` instantiated at a tuple
    (`.0` refused on `__tuple<integer,text>?`);
  - and it BROKE `v[i]?` with a plain local index (*"`?` cannot build a default for
    `(integer?, text?)`"* — the refusal a written `t: (integer?, integer?)` has always met), and
    put two false warnings on `x: (integer, text) = v[i] ?? (0, "d")`.

  The `Optional(Tuple)` was carrying the one fact the member spelling cannot: an index miss nulls
  EVERY member at once.  Without it the rules meet head on.  `(N-Coal)` types `e ?? d` as `τ`
  (non-null members), `(T-Absent)` reads `t ?? d` as "every member null", and the partly present
  tuple fixed below (`(null, 2) ?? (9, 9)` keeps `(null, 2)`) then holds a null in a member typed
  non-null — the silent lie the refusal in `fields.rs` exists to prevent.  Typing the result
  member-nullable instead is the false-warning half.  So the entry wants a DESIGN call before a
  cure, and the three ways to decide it are:
  1. `??` and `?` on a tuple discharge MEMBER-wise — `t ?? d` is `(t.0 ?? d.0, …)`, typed
     `(τ₁, …, τₙ)`, the way `(N-Store)` already polices a tuple element by element.  No lie and no
     false warning; the observable change is a partly present tuple, `(null, 2) ?? (9, 9)` →
     `(9, 2)`.
  2. Keep the all-or-nothing fact in flight — `Optional(Tuple)` stays, and `(T-Absent)`'s "not even
     in flight" is amended to "never DECLARED"; the landing cell is then a `decl_accepts` arm.
  3. Keep both as they are and accept the member warnings after `??`.
  The probe matrix (thirteen cells with `text` members, both backends) is in the session record of
  2026-09-15 and is the starting guard for whichever is chosen.

  ⚠ **Owner ruling 2026-09-16: option 2 — the in-flight spelling STAYS and only the DECLARATION
  is refused, so the rule above now reads "never declared" rather than "not even in flight".**
  The three refusals closed by teaching each site that met the two spellings to read both, and
  every one of them was a SHAPE question asked without peeling (`@FR-N-Shape`):
  - the landing (`w: (integer?, text?) = v[i]`) — `change_var_type` admits the in-flight tuple
    when every declared member admits its own `τᵢ?`, through the same `decl_accepts` a slot
    already asks, so a member declared non-null still earns `(N-Store)`'s refusal;
  - the RETURN — the two boxing gates (`block_result`'s and `parse_return`'s) asked
    `matches!(t, Type::Tuple(_))` bare, so an absent tuple was never boxed into the declared
    record and then failed `convert` with *"expected `__tuple<…>`, got `(τ…)?` on return"*.  This
    was never about the member-nullable declaration: a plain `-> (integer, text)` refused the
    same read, and now takes it with `(N-Store)`'s per-member report;
  - a generic `T?` at a tuple — the record-backed member read peels now, and `Parser::tuple_elems`
    is the one home for "what are this type's tuple element types" across all six spellings.

  ⚠ **CLOSED 2026-09-16, the null QUESTION half: the record home answers by its MEMBERS.**  Both
  spellings now do, which is what `(T-Absent)` says — the rule is stated on the TYPE, not on
  where the value lives.  The defect reached that home by TWO routes, and measuring which one a
  cell took is what kept the cure from being written at the wrong site:

  - a **bare** `ref(__tuple<…>)` — a declared tuple return — matched no flag in the `== null`
    classification and fell to the generic `==`, which compared the reference against the null
    sentinel (`OpEqRef`);
  - an `Optional`-wrapped `ref(__tuple<…>)?` — a generic `T?` at a tuple — matched `ref_null`
    and took `OpRefIsNull`.

  Both answer by the RECORD, so both read a return buffer holding two nulls as PRESENT.  The
  second route was RIGHT wherever it was measured before, because the shapes reached for it had
  a genuinely absent record (`rec == 0`), where the record's answer and the members' agree — an
  agreement that made the guard's a3 cell pass over an open defect.  The cells that separate
  them are a record that EXISTS with every member null.

  Cure, one notion at one home: `is_tuple_shape` answers *"is this a tuple in ANY of its
  homes?"*, and both the classification and `null_test`'s own gate ask it, so a third spelling
  is added there rather than at either.  `coalesce_not_null` gained the record arm — members
  read at the synthetic struct's offsets through the same `get_val` that `.0` uses, since
  `Value::TupleGet` addresses a stack tuple by var and index and has no spelling for a record
  field.  The presence test comes FIRST and short-circuits, because a record that is not there
  has no members to read.  That arm precedes the `Optional(Reference)` one on purpose: a boxed
  tuple matches that too, and answering it there is the second route above.

  ⚠ **CLOSED 2026-09-16, the DISCHARGE half: `??` over the boxed spelling.**  `(T-Absent)` names
  `t == null` and `t ?? d` as ONE question, so closing only the first left the rule half-kept.
  The coalesce typed its result from the boxed subject, and the stack default then had nowhere
  to go — *"`??` default of type `(integer, text)` is not assignable to `__tuple<integer?,text?>`"*.

  The arm that answers this pair already existed (loft#1451): when the subject is boxed and the
  default is a stack literal, take the STACK spelling, because the value unboxes.  What declined
  was its GATE — `unboxes_stored_tuple` demands the record's members be `is_equal` to the
  destination's elements, and `__tuple<integer?,text?>` against `(integer, text)` differs by
  exactly the `?` per member that `(N-Coal)` is about to discharge.  Measured on the other side:
  the same `??` over a generic `-> T?` at a tuple has always worked, because that boxes
  `__tuple<integer,text>` with NON-null members and the equality holds.

  Cure: the result takes the record's OWN members in their stack spelling, `(integer?, text?)` —
  which is the type the stack home already produces for the same notion, so the two homes agree
  rather than one inventing an answer.  Nothing is widened: the subject's conversion is then the
  unbox that arm already names, with its equality holding on both sides.

  ⚠ **Typing it as the NON-null stack tuple would have been unsound, and loudly so:** the
  subject's conversion would have had to unbox NULLABLE members into NON-NULL elements, which is
  the store direction `(N-Store)` exists to refuse.  Keeping the members is what makes the cure a
  retyping rather than a widening.

  Three refusals were measured BEFORE the change and are byte-identical after, each at its own
  site: a local's retype (*"Variable 'z' cannot change type …"*), a declared non-null tuple
  RETURN (*"expected `__tuple<integer,text>`, got `__tuple<integer?,text?>` on return"*), and a
  mixed-home `if` join (*"… on else"*).  `(N-Store)`'s per-member report appears at none of them
  — every route refuses earlier, on a type comparison — so the boxed member-nullable tuple has no
  reachable path into a non-null slot for this close to have loosened.  Guard cell a5.

  ⚠ **Not this entry, and the cure must not absorb it:** `?` on a member-nullable tuple is
  refused in BOTH homes — `(integer?, integer?)` on the stack refuses identically to
  `__tuple<integer?,text?>`.  That is the pre-existing refusal recorded above, not a record-home
  fact, and a probe that reads it as one is measuring member-nullability while believing it is
  measuring the home.

  Guard: `tests/scripts/an-absent-tuple-meets-the-type-its-author-declares.loft` (a1/a2/a3 plus
  eight controls); its a2 cell now pins `q == null` on both the absent and the present return.

  ⚠ **loft#1478, CLOSED 2026-09-09, and its lesson is about this doc's own oracle.**  A `text`
  MEMBER of an element read by a variable index did not COMPILE on `--native` (E0308, `&str`
  into a `String` slot), and under that a nested tuple member emitted a move where a clone was
  owed (E0382).  Both from ONE omission: `generation::dispatch`'s assignment arm asks *"is this
  slot a tuple?"* NINE times to pick a member's coercion, and five of them asked it BARE — so
  the `Optional` this rule's own `(N-Domain)` wrapper puts on the slot hid the slot from its own
  coercions, while the Rust type rendered is the bare tuple either way.  `generation::
  var_tuple_elems` is the one home now.

  **A CONSTANT index could not reach either refusal** (`(N-Index)` trusts it, so no wrapper is
  built), and **an all-`integer` tuple could not reach them at all**, because the coercions
  missed are the ones only a non-`Copy` member needs.  That is the blind population: this
  entry's cells, and this doc's counts, have been read over `(integer, integer)` shapes.  **Any
  cell added here should carry a `text` member.**

  ✅ **That instruction is DISCHARGED for this entry's own cells, 2026-09-10.**  All six were
  re-run over `(integer, text)` on both backends and answer exactly as the `(integer, integer)`
  re-measurement above records: the LANDING type is still refused (*"cannot change type from
  `(integer?, text?)` to `(integer, text)?`"*), `v[i].0` / `.1` by a variable index answer
  `null(oob)` / `null`, `==` `null` answers `true` on BOTH spellings, `v[i] ?? (7, "def")` gives
  `(7, "def")`, `v[i]?` gives the members' defaults `(0, "")`, and a PARTLY present
  `(null, "keep") ?? (9, "def")` keeps the `"keep"` and reports `== null` false.  So the
  deviation is one cell over the heap population as well as the scalar one, and the count above
  is not an artefact of the shape it was read on.  The instruction still stands for cells ADDED
  here — it is the entry's measured cells that are now clear, not the class.

  A third defect came out from under it — the same read through a struct FIELD's vector leaks
  its work-ref record on `--native` (loft#1479) — which is newly REACHABLE rather than newly
  broken, since that cell did not compile before.

  ⚠ **And a cure that suggests itself is measured WRONG.**  `(N-Opt)`'s side condition got its
  home in `data::has_null` on 2026-09-09 (loft#1478), and the obvious next step — have the `τ?`
  constructors ask it, so the index stops minting `(integer, text)?` — makes this worse, not
  better: the read then types `(integer, text)`, non-null members holding nulls, with no
  diagnostic.  It trades a type the language cannot spell for one that LIES about what it holds.
  "No `Optional(Tuple)`" is not "no absence"; the tuple's absence has a form and the cure is to
  BUILD it.  `data::constructs_optional` is that carve-out.

  ⚠ **The carve-out is PERMANENT, and the sentence above it said otherwise until 2026-09-16.**
  It read *"it exists to be deleted when this entry closes — the gap between the two predicates
  IS this deviation, in code"*, and the owner's option-2 ruling retired that: `(T-Absent)` now
  refuses `(τ₁, …, τₙ)?` at every DECLARATION and has the compiler carry an arriving absence
  with the `?` on the OUTSIDE.  So the two predicates answer two different questions —
  `has_null` whether a `τ?` may be DECLARED for this τ, `constructs_optional` whether absence
  may be MARKED for it in flight — and both stay.  The same stale claim stood in two other
  homes (`types.md (N-Opt)`'s side-condition note and `QUALITY.md`'s unspan row, which also
  predicted the row would "go back down by one"); all three are corrected.  What this entry
  still carries is the `?` DISCHARGE, below — not the existence of the in-flight spelling.

  **Removal** (stale, kept for the reasoning it records): it named `Type::optional` as the one
  home a `Tuple` would map to its member-nullable form.  Option 2 declined exactly that — the
  in-flight spelling stays — so this is not the removal path any more.

  ⚠ **This entry claimed the member read, the store and the monomorph's return type would
  "follow with no per-site work".  Two of those three are FALSE, measured 2026-09-08 by making
  the arm and running the cells.**  The store/landing half does follow — `w: (integer?,
  integer?) = v[i]` starts being accepted.  The MONOMORPH does not: loft#1451's reproducer is
  byte-identical before and after on both backends, and it was closed separately as
  `D-call-16` in the return-promotion path, not here.  And the consumers REGRESS rather than
  follow — `?` on a tuple becomes *"cannot build a default for `(integer?, integer?)`"* (guard
  `1424`), `??` starts emitting a spurious *"stored into a slot of the non-null type
  `boolean`"*, and `== null` still has no home.  So closing this is six or seven pieces with
  `1423b` and `1424` as rewrites rather than passes, and `build_default` already refuses
  `(integer?, integer)` independently.  **The one-arm change must not be landed alone**: it
  half-migrates the representation and takes `?` on a tuple down with it.  The written `(τ, τ)?` stays
  refused — that half is `1423-a-nullable-tuple-type-is-refused-by-name.loft` and does not move.

  ⚠ **RE-MEASURED 2026-09-16 — and SUPERSEDED the same day by the ruling above, which closed the
  entry.**  What follows is the state the ruling was made ON, kept because it is the evidence
  behind it.  The re-measure said this entry did NOT close and that ONE cell remained, the `?`
  discharge.  It was owed because the option-2 ruling had turned the entry's own headline claim
  (*"the code still mints an `Optional(Tuple)`"*) into a description of what the rule
  PRESCRIBES rather than a deviation, so the count could only be settled by asking the cells
  again.  Its guards answered green on both backends
  (`an-absent-tuple-meets-the-type-its-author-declares.loft` a1-a4 + c1-c8, and the
  boxed-discharge guard the later ruling deleted), and the declaration refusal held.  What did
  not:

  - `?` on the IN-FLIGHT spelling answers the members' defaults and discharges to the NON-NULL
    tuple: `x: (integer, text) = v[i]?` lands, reading `0 ""`.
  - `?` on the WRITTEN `(integer?, text?)` is REFUSED on both backends — *"`?` cannot build a
    default for `(integer?, text?)` — discharge with `?? <default>` instead"*.
  - The same value decides it, so no other axis is in play: `u = v[i]; u?` answers, and
    `u: (integer?, text?) = v[i]; u?` refuses.  One expression, inferred versus annotated.
  - The axis is MEMBER-NULLABILITY, not the home — the in-flight spelling over a
    `vector<(integer?, text?)>` refuses too, and so does the BOXED one, naming
    `__tuple<integer?,text?>`.  A probe that reads this as a home fact is measuring the wrong
    thing while believing otherwise.

  **Mechanism:** `Data::has_default` admits `Type::Optional(_)`; `Parser::build_default`'s tuple
  arms recurse per member WITHOUT peeling and have no `Optional` arm, so one nullable member
  sends the whole tuple to `_ => None`.  The caller's own comment already names that class —
  *"`has_default` admitted the type and the builder cannot form its value — the two disagree,
  which is a compiler defect"*.  A top-level `τ?` never shows it, because the caller peels
  `base` before asking; only a MEMBER reaches the builder unpeeled.

  ⚠ **And the obvious cure is not obviously right, which is why this is a RULING and not a fix.**
  `(D-Opt)` says `construct_default(τ?) = null`, so a literal member-wise default of
  `(integer?, text?)` is `(null, null)` — the absent tuple itself, not `(0, "")`.  The entry's
  own sentence *"`t?` reads the members' defaults"* resolves two ways depending on which
  spelling's members are read.  The case that separates them is a PARTLY PRESENT tuple, which
  the written spelling can hold and an index miss cannot produce: measured, `??` and `==` are
  WHOLE-wise on both spellings and agree — `(null, "keep") ?? (9, "d")` keeps `(null, "keep")`
  and `== null` is false.  So typing a written tuple's discharge as `(integer, text)` would put
  a live null in a non-null slot on the pass-through path, the lie `(N-Store)` exists to refuse.
  `(T-Absent)`'s `≡` therefore holds at the two ENDS — all-present and all-null — and not as
  type equality.  Three admissible answers, none derivable from the rules as written:
  1. member-wise `?` → `(0, "keep")`, typed `(integer, text)`.  Sound, but splits `?` from the
     `??`/`==` pair `(T-Absent)` names as ONE question.
  2. whole-wise `?` with the result still `(integer?, text?)` — replaces an all-null tuple with
     the member defaults and passes anything else through.  Consistent with `??`, but then `?`
     does not remove nullability, which it does everywhere else in the language.
  3. keep the refusal and amend `(T-Absent)` to say `?` reaches the in-flight spelling only,
     recording that the written one has states the in-flight one cannot reach.

  Until one is chosen the refusal is the safe answer: it is loud, both backends agree, and no
  guard pins it, so whichever way it is ruled costs no expectation.  `OPEN` stays **1**.
