# formal/coroutines-history.md — the deviation register for [coroutines.md](coroutines.md)

> **The rules are next door.**  [coroutines.md](coroutines.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **1** (2026-09-22) — D-cor-5.  D-cor-4 opened and closed 2026-09-22 (loft#1589); D-cor-3 opened
2026-09-21 and closed the next day; D-cor-2 opened and
closed the same day (2026-08-28); D-cor-1 likewise on 2026-08-23.

> **D-cor-5 — OPEN (2026-09-22, loft#1601) — a generator held by a record in a keyed collection is
> never released.**
> `(G-Hold)` was written with loft#1585, which made a generator holdable at all: before it, a
> vector of them and a struct field holding one failed to compile.  A record, a vector, a
> nested record, a vector field and a tuple member now release every frame they hold, at death
> and at removal.  A KEYED collection (`hash`, `sorted`, `index`, `spatial`) does not: its
> records are released without a walk — `(H-Drop-Not)` keeps `OpDrop` hooks out of it — and
> the store walk cannot reach a frame, which lives in the coroutine table rather than in a
> store.  So its generators, and every heap local they allocated, stay allocated to program
> exit, on both backends, reported only by the store census.  A vector of vectors of
> generators is the same today, through loft#1597: no cascade reaches a vector's vector
> elements.

> **D-cor-4 — CLOSED (2026-09-22, loft#1589) — nothing said who owns a yielded record, and the
> two backends answered differently.**
> `(G-Next)` produces one value, but no rule said whose.  The interpreter kept a yielded record
> the generator's and released it with the generator, so a record returned past its generator's
> life read `null`.  `--native` handed a record built in a compiler temp to the consumer, whose
> `for` variable was a borrow of the generator (loft#481), so a drained `for` released nothing
> and every yield leaked.  A yielded CALL result leaked on both, because the callee minted a
> store the generator never learned of.  A consumer's value changed when the generator later
> wrote the local it had yielded (`p1` read `p5`), and a tuple yield of a local was a second
> name for it on both backends.  And a `match` over a generator collected into a buffer typed
> `vector<vector<E>>`, with drop hooks running before the arm read (interpreter) or never
> (native).
>
> **Closed** by writing `(G-Own)` — the reading `(G-Next)` and `(H-Move)` already gave — and
> making both backends keep it.  The parser copies an existing value at the yield
> (`Parser::yield_owned_value`) and refuses one whose type owns a droppable; both backends
> forget the temps a yield hands over (`coroutine_layout::yield_handed_temps`, one home), so
> neither the generator's tail nor its abandonment releases them; a finished generator answers
> `DbRef::NULL` for a handle on the interpreter too; a generator's `for` variable and a `yield
> from` item own what they receive (`coroutine_layout::yield_handed_over`); the `match` buffer
> is typed by its element and takes each record by a MOVE copy; and the native eager path,
> whose values share one snapshot store, snapshots a handed record as a move and copies it out
> to the consumer (`coroutine_snapshot_moved`, `coroutine_hand_out`).  A tuple hands over each
> member; a finished generator's tuple carries the reference null in each reference member
> (the interpreter types it from the frame, native initialises the transport buffer so), and
> a tuple an advance produces is its members' sole owner in the scope pass, so a tuple `for`
> runs each member's hook as its iteration ends.  A generic template's yield copy is decided
> again per monomorph.  Residual: an advance of an ALREADY finished tuple generator answers a
> zeroed tuple on the interpreter, whose frame no longer names the type.  Guard
> `tests/scripts/1589-a-yielded-record-is-the-consumers.loft`.

> **D-cor-3 — CLOSED (2026-09-22, loft#1586) — an endless `while` generator never yielded on
> `--native`.**
> `(G-Next)` says an advance runs to the next `yield` and produces one value.  Native loop
> generators outside CL-9 slice 1 (a `for` whose body ends in one unconditional `yield`) run the
> whole loop eagerly into a buffer, which the Conformance section accepts as an interleaving
> difference because the values still agree.  A `while` loop is outside slice 1, and when it is
> endless the premise fails: `while true { n += 1; yield n; }` answers `1 2` on the interpreter
> and on `--native` never answers — it fills its buffer until the process is killed for memory
> (measured at a 2 GB cap; three shapes: integer, boolean and float yields).  A `for` over a large
> range is lazy on both backends and is the workaround.  Closes with CL-9 slice 3 (COROUTINE.md
> § Design: lazy loop yields, axis A5).  Found by the Lua expressiveness measurement (LUA_BAR
> LB3 / LB4).
>
> **Closed** by lowering a `while` (a bare `Loop` in the generator body) exactly as slice 1 lowers
> a `for`, and by ROTATING a loop whose yield has statements after it: they run at the start of
> the next advance, before the header.  Three defects of the existing lazy `for` path surfaced
> once `while` loops reached it, and closed with it: a hidden record buffer (`__ref_*`) was
> re-declared on every advance, so a loop that built a record through a call leaked its last
> store; a local that adopted such a record was reset by its bare name (E0425); and the tail
> state bound no parameters (E0425 for a statement after the loop that read one).  A closure in
> the loop body keeps the eager buffer.  Guard:
> `tests/scripts/1586-a-while-loop-generator-yields-before-its-next-iteration.loft`, whose cells
> are all FINITE and assert the ORDER of side effects, so a regression fails an assertion rather
> than exhausting the machine.  Re-measured: LUA_BAR LB3 and LB4 pass on both backends.

> **D-cor-2 — CLOSED (2026-08-28, loft#1132) — a native transport channel was chosen for
> types it could not carry.**
> `(G-Next)` says an advance produces the yielded value; nothing says which loft types a
> backend may refuse, so the working rule is the one the whole doc rests on — the two
> backends agree, or the difference is a WRITTEN decided edge. `--native` had a third
> behaviour: it emitted Rust that rustc rejected, against generated source the author cannot
> read, for programs `--interpret` runs correctly.
>
> The channel ladder ends in an `as i64` catch-all whose unstated premise is *whatever is
> left is scalar-shaped*, and nothing tested it. Measured over yield type × position
> (straight-line / loop body), on both backends:
>
> | yield type | was | now |
> |---|---|---|
> | `(integer, text)`, `((integer, P), integer)` | `E0605` non-primitive cast | REFUSED, naming the type and the cure |
> | `(integer, integer)` / `(integer, float)` from a LOOP body | `E0308` | **works** — the eager buffer holds the elements' `i64` images flat |
> | `(integer, P)` / a fn-ref from a LOOP body | `E0308` | REFUSED — a store handle buffered per iteration aliases, the reason the struct/vector refusal already gives |
> | `hash`/`index`/`sorted`/`trie` from a LOOP body | `E0308` | **works** |
> | `(integer, boolean)`, straight-line | `E0308` | **works** |
>
> Four sites, one question each, and three of them had drifted from a home that already
> existed and already said so in a comment one screen away:
>
> * the ladder's catch-all and the eager collector's — premise is `data::is_scalar`
>   ([types.md](types.md)'s scalar/heap split); `coroutine_layout::channel_tag` now answers
>   `CHANNEL_NONE` and both ends read it, the producer emitting the `compile_error!` and the
>   consumer a diverging expression so exactly one diagnostic survives;
> * `lazy_yield_init` and the coroutine struct's `__values` element type — both spelled the
>   DbRef set as the SHORT three-variant list (@FR-Col-Store; one home is `data::is_dbref`),
>   so a keyed collection typed `__y` as `i64` inside a method returning `DbRef`;
> * `yield_slot_read`'s boolean — @PLN17 makes a boolean's storage form the tri-state `u8`
>   and the tuple rebuild writes into a Variable position, so reading the slot back as a bare
>   `bool` typed the rebuild against nothing.
>
> The eager tuple buffer is what makes the second row WORK rather than refuse, and it is the
> row the decided edge above had claimed all along.
>
> Guards: `tests/scripts/1132-a-generator-yield-rides-a-channel-that-carries-it.loft` (what
> must work, both backends) and `tests/native_yield_channel.rs` (the refusals, at the emit
> level — a refused program has no run to assert on).

> **D-cor-1 — CLOSED (2026-08-23) — `return` in a generator was accepted and DISCARDED.**
> `(G-Call)` makes any `-> iterator<T>` function a generator and the model gives a returned
> value no meaning, but nothing said so and nothing refused it.  Both spellings of the same
> act were accepted:
>
> ```loft
> fn make() -> iterator<integer> { return counting(1); }   // the keyword
> fn make() -> iterator<integer> { counting(1) }           // the tail value
> ```
>
> An author delegating to another generator got an **EMPTY sequence** on `--interpret`,
> exit 0, no diagnostic — and a panic inside `alloc_coroutine` ("RefCell already borrowed")
> on `--native`.  `(G-Return)` above now states the rule and both spellings are refused,
> naming `break` (early end) and `for v in g() { yield v; }` (forwarding).
>
> ⚠ **The message names `break` because the other candidate was MEASURED and does not
> work.** A bare `return;` was refused only as the generic *"Expect expression after
> return"* — the check reads "the declared type is not Void" as "a value is required" — and
> it looked like a capability the refusal was removing.  It is not one: `detect_lazy_for`
> (`generation/coroutine.rs`) rejects a `return` outright, so such a generator falls back to
> the eager buffer, whose factory cannot emit a mid-body return either (rustc E0308 — the
> factory's type is `Box<dyn LoftCoroutine>`).  Enabling it in the parser alone would have
> shipped a shape that runs on one backend and does not compile on the other, which is the
> divergence the refusal exists to prevent.  Naming an untested cure in a diagnostic is
> worse than naming none.
>
> Guards: `tests/scripts/generator-return-carries-no-value.loft` (what must work, both
> backends) and three cells in `102-expected-errors.loft`.

One thing the VERIFICATION worklist surfaced (2026-07-04) is a **decided edge**, not
a deviation — recorded here so it is not mistaken for a bug:

> **DECIDED EDGE — NARROWED (loft#836, slice 1, 2026-08-10): native is eager only for a loop body
> whose resume point one state cannot encode.** Laziness (G-Call / G-Next) holds for
> **straight-line** yields on both backends, and now for a loop whose body ends in ONE
> unconditional `yield`: the loop is lowered to a header+body state pair, the cursor persists in
> the coroutine struct, and one advance runs one iteration. So
> `for i in 0..2 { print("y{i} "); yield i }` consumed by `for x in gen() { print("g{x} ") }` is
> `y0 g0 y1 g1` on BOTH backends, and an early-`break`-consumed billion-iteration loop-generator
> stops when its consumer does — it no longer runs unboundedly on native. Eagerness was never a
> rustc restriction; it was the absence of a loop-to-state-machine transform, which is the same
> shape `async`/`await` uses and compiles on stable Rust.
>
> What REMAINS eager, and why each needs a resume point a single state cannot express: more than
> one yield per iteration (re-entry must land at the yield that suspended), a yield inside an
> `if`/`match` (the resume point depends on the branch), a nested loop (a cursor per level), a
> `continue`, and a statement AFTER the yield (the iteration would have to resume mid-body).
> A yield of a tuple / fn-ref rides the `next_into` channel, which has no suspend point of its
> own; a yield of a struct / vector builds its record into a work local that is not persisted, so
> lowering it lazily would leak one record per yield. Those keep the eager buffer; the observable
> difference stays side-effect interleaving.
>
> ⚠ *"and their values agree"* stood here until 2026-08-28 and was **never measured** — a tuple
> yield from a loop body did not COMPILE on `--native`. See D-cor-2 below.
>
> ⚠ **One of those reasons was firing on code the author did not write — FIXED 2026-08-23.** A
> generator that writes a **text field of a heap parameter** was eager:
>
> ```loft
> struct T { s: text }
> fn g(t: T) -> iterator<integer> { for i in 0..5 { print("p{i} "); t.s += "x"; yield i; } }
> //  --interpret : p0 g0 p1 g1     --native WAS : p0 p1 p2 p3 p4 g0 g1
> ```
>
> It has ONE yield, and the yield IS the author's last statement, so it reads as a shape the
> lazy lowering admits. In the IR it was not: a text field write lifts the field into a temp
> whose free is placed at block exit, which lands after the suspend —
> `OpSetText(t, 0, _field_1); yield i; OpFreeText(_field_1);` — so `detect_lazy_for` saw
> *"a statement AFTER the yield"* and demoted the whole generator. The same shape with an
> **integer** field was lazy (`OpSetInt(…); yield i;` — nothing trails), as were a text LOCAL
> written in the loop, a text-typed yield, and a text field READ. The author had no way to
> see the difference, and no way to remove the trailing statement.
>
> `hoist_trailing_frees` now moves a dead temp's free ABOVE the suspend, which is sound
> exactly when the yield does not read what is being freed — `_field_1` has already been
> copied into the record by the preceding `OpSetText` — and is strictly better for a
> generator the consumer ABANDONS, which used to strand the last iteration's temp. A yield
> that does read it (`yield t.s`) keeps the eager path. Widening the recogniser to accept
> arbitrary trailing statements would have been the wrong fix: the statement is not the
> problem, its position is.
>
> **This was the second time a generated cleanup op silently demoted a generator**: the
> `detect_yield_from` comment above records the first, where matching the op count exactly
> *"silently pushed every `yield from` onto the eager path the moment that free appeared"*.
> Both sites now tolerate a trailing free, by different means — that one ignores it, this one
> moves it — which is worth knowing before adding a third recogniser.
>
> Guards: `tests/scripts/generator-lazy-through-a-text-field.loft` (in every CI run) and a
> text-field cell in `tests/oracle/26-coroutine-laziness.loft` (the nightly cross-backend
> sweep), the latter proven to fail on a pristine tree at `415e7ba8` with exactly the eager
> trace `q0 q1 q2 q3 q4 w0 w1`.
> Slices 2-4 of [COROUTINE.md § Design: lazy loop yields (CL-9)](../COROUTINE.md#design-lazy-loop-yields-cl-9)
> close the rest. Tracked as [loft#836](https://github.com/loft-lang/loft/issues/836).

- **Conformance is otherwise differential, and this is the hardest case** — the two backends
  implement suspension by the most different mechanisms (interp: serialise the frame to a heap
  store; native: a compiled resumable state machine, still eager for the shapes named above).
  The @PLN89 differential oracle (D-op-1) carries `12-coroutine-generator`, but it checks VALUES
  only — which is exactly why it reported full agreement across the eager/lazy difference.
  `tests/oracle/26-coroutine-laziness.loft` pins the **interleaving** for both the straight-line
  and the lazy-loop shapes, and since 2026-08-23 it ASSERTS that order rather than only
  comparing the two backends' output: it records each event into a heap-struct parameter, which
  a generator can write across a yield. Giving it that assertion is what found the text-field
  cell above — the trace was written as text first, and recording it made the generator eager.
- **`yield from` is out of scope** — delegation (CO1.4) is deferred to 1.1+; when it lands it
  extends `G-Yield` (a delegated yield forwards the sub-generator's values) and gets its own
  rule + oracle case. Until then a `yield from` is a parse-level unsupported form, not an
  unspecified runtime behaviour.

## Carried by coroutines.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [coroutines.md](coroutines.md) now states only what is open.

### the status line formal/README.md's area table carried until 2026-09-04

**rules written (2026-07-04), 0 own** — lazy one-value-per-advance; straight-line yields lazy on both backends, and so is a loop body that ONLY yields; a loop body with a SECOND statement is eager on native (a DECIDED EDGE — rustc restriction, loft#836); conformance via the oracle

