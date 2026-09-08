# formal/closures-history.md — the deviation register for [closures.md](closures.md)

> **The rules are next door.**  [closures.md](closures.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0**.  This header read **2** until 2026-09-07, naming `D-clo-7` and `D-clo-14`; both
CLOSED on 2026-09-03 and the count was not carried here (the entries themselves say so, and
[ROADMAP.md](ROADMAP.md) has read 0 since).  `D-clo-23` below was opened and closed the same
day it was found.

### D-clo-23 — OPENED AND CLOSED (2026-09-07, loft#1409): `(L-CapWrite)` held at two integer widths, not at every one

`(L-CapWrite)` shares a captured scalar in the WRITE direction, and it is stated over "a
scalar" — no width in it, exactly as it carries no nullability (which is what loft#1408
restored one axis over).  A mutated scalar capture is boxed into a `__cell_<T>` record to give
the write somewhere both sides can reach, and the namer had templates for the two canonical
8-byte integers only.  Every other width returned `None`, the synthesis pass skipped it, and
the capture kept an un-boxed stack slot — which carries a DIRECT call's write and nothing
else.  So `f(0)` wrote through and `call_it(f, 0)` dropped the write, silently, on both
backends.

**The filed scope was the `forced_size` aliases and it was too narrow.**  The namer keyed on
storage WIDTH, so a bounds-only narrow type — `integer limit(0, 100)`, no `size()` anywhere —
fell in the same hole.  `boolean`, `character`, `single`, `float`, `text` and a plain enum were
never affected: each owns its storage on its own def, and it is the `integer` def, shared by
every width, that could not carry one.

The cure is width templates, and the rules chose it rather than the alternative the issue
offered.  A blanket REFUSAL was inadmissible — `(L-CapWrite)` says the write lands, so refusing
it is a second deviation, and it would have broken the direct-call spelling that was already
correct.  What the fix must not do is widen the capture on the way into its box, which is now
written down as `(L-CapBox)`: a cell is deduped by NAME, so two captures share storage exactly
when they share a name, and naming a narrow cell after its alias would have given
`integer limit(0, 100) size(1)` and `u8` one cell and silently widened the first one's declared
range to the second's.  The stem therefore carries the whole storage identity — width, bounds
and the null-flag — and the NAME is derived from the value type rather than restated beside it,
because those two had to agree and were two independent matches.

⚠ **Two things the cure exposed, both older than it.**  A cell's `value` attribute was flagged
nullable while its TYPE was dense — `add_attribute` marks every attribute nullable — and the
disagreement is invisible at 8 bytes and decisive at every narrow one: the width is read from
the type in one place and from the sentinel-reserving flag in another.  And the native
generator resolved a field's width from the attribute's ALIAS only, where the interpreter had
carried a second rung — the width the TYPE itself holds — since @PLN114 added it for
`__tuple<…>`'s synthesized fields.  A field built from a Type alone has no alias, so the two
backends registered one field two ways (`byte<0,true>` against `short<0,true>`, one byte
against two) and every type id past it was renamed (loft#739).  The generator's own comment
claimed it mirrored the interpreter exactly; it had been left behind by one rung.  Both are
fixed here, and the guard runs under `LOFT_STRICT_SCHEMA_IDS`.

**D-clo-18 left the register on 2026-09-03 without being fixed**, which is why the count fell to
2 with no code change: a `&` scalar parameter written from inside a closure is permanently
REFUSED, so per [ROADMAP.md](ROADMAP.md) it is a decided edge
([DESIGN_DECISIONS C115](../DESIGN_DECISIONS.md)) rather than distance from the spec. Its record,
and `D-clo-20`'s, are below.

⚠ **A rebind is not a mutation, and a capture is the one place the code could not tell them
apart (D-clo-20, loft#1281, closed 2026-09-02).** `(L-CapHeap)` shares a captured heap value
so a mutation-through is visible both ways; `(F-ParamRebind)` makes a whole-value REPLACE of
a heap parameter local to the callee. Written inside a closure, the replace took the
mutation's route: the closure record holds a COPY of the parameter's DbRef, the rebind
lowered to a clear plus a refill of the store that copy names, and `(F-ParamHeap)` makes that
store the CALLER's. So `fn f(p: vector<integer>) { g = fn() { p = [7,7]; }; g(); }` replaced
the caller's collection, on both backends, silently — every heap kind and every right-hand
side.

The cure is a refusal, and the measurement is why. Repointing the capture slot — the obvious
fix, and the one the keyed kinds appear to model — does not work HERE, because the callee
reads its own slot directly (`t_6vector_len(p(0))`, not a read through the closure record).
Two readers, one binding: a repoint moves the wrong answer from the caller to the callee
rather than removing it. Making it mean what it says needs the binding reachable from inside
the closure plus a write-back, which is precisely what `D-clo-18` (now a decided edge,
[DESIGN_DECISIONS C115](../DESIGN_DECISIONS.md)) records as unavailable for the `&`-scalar shape — the cell machinery that gives a mutated captured SCALAR that channel
cannot serve a heap value, because reads in the enclosing body would then see the cell while
the caller still sees its own slot. Same wall, one rule over, so the same answer.

Worth keeping for the next reader: the emitter was already building the fresh backing the
correct lowering would need (`__vdb_1`, `var__vec_1`), writing its length, pre-allocating it —
and then appending into the shared store instead and abandoning it. It does not leak (the
wrapper is freed at lambda exit), so it was wasted work rather than a second defect, but it
means the missing piece was never the allocation.

⚠ **Three entries had ONE cure between them, and it was not another ownership predicate.**
Each was a call site that allocated a return buffer and could not say whether what came back
IS that buffer: D-clo-13 across the two arms of a `??`, D-clo-12 across the frame a
forwarding function puts in the way, and loft#1183 across two assignments to one local. Every
static reading is right for one case and wrong for the other, which is what says the question
is not statically answerable at all — so the answer is not to answer it. **The store is owned
by the frame that HOLDS it, and ownership travels with the return value:** `fn_return` hands
the delivered buffer up one frame instead of forgetting it, and the caller's own return
releases it. BOTH backends do that now — the interpreter through `release_fnref_bufs`, and
`--native` through `codegen_runtime::FnRefBufGuard`, which reads the frame's declared return
type where the interpreter reads the store that came back (loft#1183, closed).

⚠ **That closed ONE of the three, and the difference named what the other two were about.**
The hand-up answers for a store the CALL SITE allocated; D-clo-12 and D-clo-13 hand back a
store the call site never made — the closure's capture, or the callee's own mint — and give it
to a caller whose type reads it as owned. D-clo-13 is now closed too, by the same
owner-by-possession rule reaching one step further: the callee already computes, at run time
and in one place, that the store it is handing back is the store it minted, and
`OpFreeRefOrHandUp` attaches an owner there (loft#1186). D-clo-12 remains, and its distance is
exactly the frame in the way: a forwarding function's return type is computed once for every
caller, so no per-argument fact reaches it. @PLN150. Measured repairs that do NOT work are
recorded on each issue.
D-clo-11 — a captured STRUCT taken by the caller's bind — D-clo-10 — a captured collection
taken the same way — D-clo-9 — a captured record FREED by a caller that lifted a fn-ref tail
— and D-clo-8 — a captured `vector<(…)>` unpacked rather than shared — were opened and closed
on 2026-08-29, 2026-08-29, 2026-08-29 and 2026-08-28. Closed: both
lambda forms capture identically (D-clo-1), the
stored-short-lambda combinator crash is now a clean diagnostic (D-clo-2) — both closed
2026-07-04 —
`L-Escape`'s *storage* half is complete (D-clo-3, opened and closed 2026-08-22), a lambda
now carries one text work buffer however many promotions ask for one (D-clo-4), a
combinator's inline callback is handed the buffer its ABI expects (D-clo-5), and a fn-ref
call carries every text buffer its target declares (D-clo-6) — all three opened and closed
2026-08-27.

⚠ This zero is only as strong as the axes the corpus below varies, and it has now been
re-measured TWICE and broken both times. D-clo-3 found the *first-Set vs re-Set* axis;
D-clo-4 found the axis inside the BODY — every `L-Apply` cell returned through a single
delivery, so nothing varied *how many* promotions the body asks for. The axes now varied
are destination (local, struct field, vector element, tuple member, return) ×
first-Set/re-Set × source (bare name, non-capturing lambda, capturing lambda, local, call,
`if`/`match` arm) × host (named local, `&` parameter, vector element, field chain) ×
**buffer count asked for by the body (one, two)**. Two that remain HELD FIXED, and are
therefore where a next re-measurement should look: the number of DISTINCT capturing
lambdas per attribute (one, by a shipped rule), and the nesting of the holder itself (a
struct that holds a capturing closure cannot go in a collection at all, by #318).

A THIRD axis was held fixed and is now varied: WHERE the lambda is applied. Every
`L-Apply` cell called the closure directly, and a combinator lowers its own call — so a
capturing lambda passed INLINE to `map` and returning text faulted on `--interpret` while
`--native` ran it. That is D-clo-5, closed the same day.

> **The re-measurement, and what the corpus was holding fixed (2026-08-22).** The
> Conformance section below verifies `L-Escape` at three destinations — a local, a struct
> field, and a return — and every one of them writes into a place being **initialised**.
> The axis it never varied is therefore not the container at all but *first-Set vs re-Set*,
> and on that axis a live crash was sitting under the zero: a fn-ref written by a
> NON-CAPTURING source (a bare name, or a lambda capturing nothing) lowers to the 8-byte
> d_nr while the slot is the 20-byte pair, and only the initialising paths topped it up.
> `g = inc` on a live `g` panicked `fn_call_ref: fn_var=16 < 20` on `--interpret` while
> `--native` ran the same program — a backend SPLIT, so neither backend alone could see it
> — and `t.0 = inc` panicked on one backend and handed the user a raw rustc E0308 on the
> other. Fixed at the three destination-aware sites (`set_var`, the `TuplePut` arms of both
> backends, and the native reachability walk), guarded by
> `tests/scripts/fn-ref-reassignment-tops-up-the-pair.loft`, which was confirmed to fail on
> a pristine tree on both backends.
>
> The rest of the destination sweep came back clean and is recorded here so it is not
> re-run: vector element (literal and `+= [f]`), keyed-collection value, struct-enum
> variant payload read per-variant, nested struct-in-vector, and an un-inferrable stored
> short lambda through `map`/`any`/`all`/`sort_by`/`filter` (D-clo-2's fix named
> `parse_map` alone, but the diagnostic fires at the LAMBDA, so it was never the
> single-site risk it looked like).

> **D-clo-22 — OPENED AND CLOSED (2026-09-04, loft#1353): a nullable record answered by a
> FN-REF and reassigned, or chosen by an `if`, kept the raw pointer on the interpreter.**  The
> reassign copy the interpreter emits for a borrowed-view call result asks
> `use_analysis::callee_of` which function a fn-ref call reaches, and that resolver declined
> every fn-ref whose return is `τ?`, because admitting the nullable spelling had been measured
> as a use-after-free — on `1114-a-nullable-heap-capture-…`'s `fn(p2_n) -> P2s? { p2_st }`, a
> lambda returning a CAPTURED store, which no caller variable names and the argument bracket
> (`protectable_ref_args`) cannot protect.  So `j = if c { hr(b2) } else { d }` with `hr =
> fn(q: Bag) -> P? { … q.rec … }` aliased `b2.rec` on the interpreter while `--native`, whose
> arm has no such exception, copied — against `(B-Copy)` and `(O-NoDiverge)`.  Closed by
> telling the two returns apart: a return that borrows a visible ARGUMENT is admitted (the
> bracket protects it, and the copy is what the rules ask); one whose return dep names no
> visible parameter — the closure record — is still declined (`fnref_return_borrows_closure`).
> Guard `tests/scripts/1353-…loft` (the join, a reassigned nullable local, the plain and the
> captured-store controls, which the 1114 guard pins as well), falsified at `1bb5e1b8` on
> `--interpret`, native inert by construction.  The named twin is loft#1346
> (ownership-history D-own-29).
>
> **D-clo-21 — OPENED AND CLOSED (2026-09-04, loft#1349): a lambda returning a lifetime tuple
> handed its vector element up as a view of the argument's field.**  `(F-Ret)` says a returned
> whole heap value is owned, never a view.  A NAMED function declared `-> (vector<integer>,
> text)` takes the lifetime-tuple boxing at its declaration — the return becomes the synthetic
> `__tuple<…>` record and every element is copied out through the return buffer.  A LAMBDA
> declared the same way stored its annotation verbatim, so its tail `(q.items, q.nm)` was
> handed up as the bare tuple the arms yield: the vector element a view of `q.items`, and
> `t = hp(b); b.items[0] = 99` read 99 through `t` on both backends while the named twin
> read the original.  Closed by one helper both lambda forms take at the same point on both
> passes (`Parser::boxed_tuple_return`, the rule the named declaration already applied:
> `has_lifetime_concern` → `tuple_def`).  The boxed lambda then joins with a tuple literal
> exactly as a named function does, which is loft#1350's territory and closed beside it.
> Measured on both backends; the cells are in loft#1350's guard.
>
> **D-clo-18 — RECLASSIFIED as a decided edge (2026-09-03, loft#1276): a `&` SCALAR parameter
> written from inside a closure is REFUSED, and no code change closes it.**
>
> `(L-CapRef)` + `(F-ParamRef)` read together say the write should reach the caller. It does not,
> and the reason is one rule further in: the value lives in the caller's slot and `(L-CapScalar)`
> gives the closure a COPY of it, so there is no shared record for the write to land in. Before
> the refusal existed the program COMPILED and answered quietly wrong —
> `fn bump(p: &integer) { g = fn() { p += 1; }; g(); p = p + 10; }` on `n = 5` answered **15**
> where 16 is correct, the closure's increment dropping through a parameter whose whole purpose
> is the write-back.
>
> Making it mean what it says needs the REF itself in the closure record plus a write-back, and
> the cell machinery — the mechanism that gives a mutated captured LOCAL scalar exactly that
> channel — cannot supply it: reads in the enclosing body would then see the cell while the
> caller still sees its own slot.
>
> **Why it left the register rather than closing.** It was carried as an OPEN deviation for two
> days while the refusal it describes was already permanent, so the count read one higher than
> the distance actually was. [ROADMAP.md](ROADMAP.md) already says what to do with that: a row
> that turns out **spec-may-adjust** leaves `formal/` and becomes a decided edge. It is
> [DESIGN_DECISIONS C115](../DESIGN_DECISIONS.md), which carries both halves — this one and
> `D-clo-20`, the heap twin below, whose refusal is the same answer one rule over. The rule keeps
> the carve-out in its own text: `(L-CapRef)` now states the refusal and cites C115 instead of a
> deviation number.
>
> Guard `tests/scripts/1276-reject-a-ref-parameter-a-closure-cannot-write.loft`
> (*"Cannot write to the `&` parameter 'p' from a closure"*), whose read-only cells are the
> control saying that widening the write-walk to credit a lambda's WRITES did not also credit
> its READS.

> **D-clo-20 — OPENED AND CLOSED (2026-09-02, loft#1281), as a REFUSAL: a whole-value rebind of
> a captured heap PARAMETER reached the caller.**
>
> `(F-ParamHeap)` makes a whole-value rebind of a heap parameter local to the callee, and a
> rebind written inside a CLOSURE that captures that parameter reached the CALLER instead:
> `fn repl(p: vector<integer>) { g = fn() { p = [7,7]; }; g(); }` left the caller's `[1,2]` as
> `[7,7]`, on both backends, with nothing reported, while the identical rebind written without
> the closure correctly left it alone. Every heap kind did it — vector, keyed and struct alike —
> and every right-hand side: a literal, a call, another local.
>
> The two rules meet here and the code followed only one. `(L-CapHeap)` is right that the closure
> and the callee body see one collection; what does not follow is that the CALLER does. The
> closure record holds a COPY of the parameter's DbRef, so the rebind lowered to a clear plus a
> refill of the store that copy names — and `(F-ParamHeap)` makes that store the caller's. A
> capture has no route back to the parameter SLOT, which is the binding `(F-ParamRebind)`
> rebinds.
>
> It is REFUSED, which is the call `D-clo-18` makes for the `&`-scalar shape and for the same
> reason: making it mean what it says needs the binding reachable from inside the closure PLUS a
> write-back, and the cell machinery that gives a mutated captured SCALAR exactly that cannot
> serve a heap value. Measured rather than assumed: repointing the capture slot alone does not
> fix it, because the callee reads its own slot directly (`t_6vector_len(p(0))`, not a read
> through the record), so a repoint moves the wrong answer from the caller to the callee instead
> of removing it.
>
> The refusal is narrow, and each exclusion still works: a captured LOCAL (no caller to reach
> past), a `&` parameter (`(L-CapRef)`, where the write-back is the point), a scalar or text
> parameter (the cell machinery), and every mutation-THROUGH — `p += [x]`, `p[i] = v`,
> `p.clear()`, a field write — which `(L-CapHeap)` and `(F-ParamGrow)` require to reach the
> caller.
>
> Reject twin `tests/parse_errors.rs::a_closure_cannot_replace_a_captured_heap_parameter` (all
> three right-hand sides × vector, hash, struct); the shapes it must NOT reach are
> `tests/scripts/1281-a-closure-cannot-replace-a-captured-parameter.loft`, which cannot hold the
> refused spelling because the fixed compiler will not parse it. Both halves are recorded as one
> decided edge, [DESIGN_DECISIONS C115](../DESIGN_DECISIONS.md).

> **D-clo-19 — OPENED AND CLOSED (2026-09-01, loft#1279): `=` on a captured collection was two
> other operations, decided by the SOURCE of the right-hand side.**
>
> A literal APPENDED (`b = [7,7]` over `[1,2]` read back `[1,2,7,7]`); a variable, a call and
> an empty literal were DROPPED entirely, the statement collapsing to a bare read of its own
> right-hand side — the emitted lambda for `c = src` is one `OpGetDbRef` and no store at all.
> Both backends, no diagnostic.
>
> One cause under both. A captured collection is reached through the closure record's shared
> DbRef, which resolves to `OpGetDbRef` and not to the `OpGetField` a struct field gives.
> @PLN93 taught the APPEND path that difference — `is_captured_dbref` exists for exactly it —
> and the whole-value REPLACE path was never told. So a LITERAL right-hand side still had
> @PLN93's build-into-the-target lowering to run and appended, because nothing had cleared
> first; every other right-hand side had nothing to run at all.
>
> That selector has now been too narrow three times, and the lowering was right each time:
> P261 (a literal into a struct field appended), loft#917 (a `vector<τ>?` field, whose
> `Optional` wrapper it did not match), and this. The cure is the same clear-then-fill in all
> three; only the shape of the destination kept changing.
>
> ⚠ The literal needs the clear BEFORE it and no append after it, because it builds INTO the
> destination — the first version of this fix appended and answered `[]`, having cleared away
> what it had just built. `value_mentions` is what asks that question: an RHS that names its
> own destination is one that constructs in place.
>
> Guard `tests/scripts/1279-a-captured-collection-rebind-replaces.loft`. Its sharp cell is
> `bx.items = [7,7]`: the same rebind reaching the same kind of collection through a captured
> STRUCT was correct throughout, which is what said this was a missing lowering rather than a
> limit of capture. The remaining question — which BINDING a captured rebind names, where a
> captured PARAMETER's rebind still reaches the caller — is D-clo-20 (loft#1281).

> **D-clo-17 — OPENED AND CLOSED (2026-08-30, loft#1202): a captured record ENUM was TAKEN by
> the caller's bind, because no delivery was ever classified for it.** `@FR-L-CapHeap` says a
> captured heap value is SHARED — the caller may read it, never take it — and D-clo-11 made
> that hold for a `Reference`. A record enum is the SECOND spelling of a struct-like heap
> store, and it kept the old behaviour: `r = g(1)` on `g = fn(v: integer) -> Shape { cap }`
> adopted the captured record, the next iteration's rebind released it, and the arena guard
> reported the use-after-free directly while `cap` still named the store.
>
> The cause is one arm above everything the issue had ruled out. `block_result` picks a
> return's DELIVERY from a chain of `else if`s keyed on the type former, and the record arm
> spelled `let Type::Reference(td, ls) = t.base()` by hand. `Type::Enum(td, true, _)` matched
> that arm, the vector arm and the keyed arm alike: **none of them**. With no delivery there
> is no `ref_return`, with no `ref_return` no return dep, and an empty dep list is what
> `Def::returns_borrowed_view` reads as OWNED. That is loft#1140's story one former over —
> *"the five KEYED kinds reached no arm above, so no delivery was classified for them"* — and
> the pass BELOW the arm had been built for both spellings all along (`ref_return` rebuilds
> `Type::Enum(td, true, dep)` on the line after its `Reference` twin), which is why every
> function-shaped repair the issue proposed was looking downstream of a gate that never
> opened. The arm now asks `Type::heap_def_nr`, the one home for *"which record definition
> does this type name"*, so a hand-written pattern cannot drift from it again.
>
> ⚠ **Opening that gate exposed two more sites where the same two spellings had drifted,
> and closing the UAF without them would have traded it for a leak.** Both were found by
> the acceptance corpus rather than by reading, and both are the same shape as the first:
>
> - the **ownership-transition free** (`scopes.rs`, @FR-O-Latest): a local that OWNED a
>   store and is then assigned a VIEW must release what it displaced. Two of the four
>   blocks in that chain paired `Reference` with `Enum(_, true, _)` and two did not — and
>   one of the two was the `owned_refs` TRACKING that licenses the others, so the paired
>   blocks were dead for a record enum as well.
> - the **inline-call lift** (`scopes.rs`): the `Reference` arm lifts when the callee
>   delivers through a `__retbuf`, because the temp is then the caller's own buffer; the
>   record-enum arm beside it asked only `!returns_borrowed_view()`, which a `__retbuf`
>   delivery fails. The two arms are now ONE arm over `heap_def_nr`, with the lifted temp
>   keeping the spelling it arrived with.
>
> The lesson is the entry's own: a promotion pass that had been built for both spellings
> all along was gated by a hand-written pattern that admitted one, and the sites DOWNSTREAM
> of that gate had quietly specialised to the traffic it let through. Widening the gate is
> what makes them visible, so the widening and the three repairs are one change.
>
> ⚠ **The filed cell was one of four, and the four broke together.** Varying the tail shape
> over the forms that READ the closure — a bare capture, a field projected out of a captured
> holder, a capture on one arm of an `if` join, and a capture handed back from a lambda passed
> INLINE to `map` (D-clo-5's axis) — every one was a use-after-free, and every shape that does
> NOT read the closure (a mint, a forwarded parameter, the struct twin) was clean. So the
> boundary is the type former, not the tail: the issue's single repro was the cell that
> happened to be in a test file.
>
> ⚠ **`--native` answered correctly on all four throughout**, which is what kept this out of
> sight: the release is silent until something reuses the slot. The channel is a
> debug-assertions build or `LOFT_STORE_GUARD=1`, and the guard's header says so, because
> `make falsify` builds a plain dev binary where those assertions are compiled out.
>
> Guard: `tests/scripts/1202-a-captured-record-enum-is-not-the-callers-to-take.loft`, whose
> controls are the two minting shapes (reading either as a borrow leaks one store per call —
> the leak that narrowed the earlier `Reference` repair to collections), a named function
> forwarding an enum parameter, and both arms of a `-> Shape?` return, which reaches the arm
> through the same `.base()` peel.

> **D-clo-16 — OPENED AND CLOSED (2026-08-29, loft#1188): a declared-RECORD lambda aborted the
> compiler when the holder struct was declared before its field's type.** `(L-Escape)` promises
> `g = fn(v: integer) -> P { q.p }` works, and it did — written one way. Moving `struct P` below
> `struct Q { xs: vector<integer>, p: P }`, a change with no meaning in a language whose
> declaration order is free, aborted on the two-pass contract: *"grew a pass-2-only attribute
> `__ref_1`"*.
>
> D-clo-15's sentence, one rung out. What pass 1 can classify is a property of what was RESOLVED
> when it read the body, never of the SPELLING: a field typed by a forward reference is `Unknown`
> while pass 1 reads the tail, so the #306 view materialisation that gives this lambda its return
> buffer never fires there, and pass 2 — which has the resolved field — mints the buffer and grows
> the arity. No predicate over the pass-1 tail can separate the two orderings, because the two
> passes are not reading the same type. So the reservation goes where every type IS resolved,
> which is what `reserve_late_return_buffers` exists for (#675), and every declared-record lambda
> is reserved for.
>
> A CAPTURE tail is included here where D-clo-14 excludes it from the collection leg, and the
> asymmetry is a fact about the two deliveries rather than an oversight: a `-> P` return hands
> back an owned COPY (#306 materialises the view into the buffer), while `-> vector<…> { q.xs }`
> hands back the capture's own store and has nothing to place.
>
> The reserved buffer is BOUND, not renamed onto, and that half is load-bearing. The placeholder
> is minted between the passes, before pass 2 appends the `__closure` argument, so renaming the
> attribute onto the work-ref the tail mints puts the callee's argument slots out of the attribute
> order the CALL SITE lowers against — measured, `CallRef` wrote the closure into the buffer's slot
> and every call answered a zeroed record. Guard
> `tests/scripts/1188-a-declared-record-lambda-gets-its-buffer.loft`, whose cells assert the VALUE
> for that reason: a fix that only stops the abort still passes an exit-code channel.

> **D-clo-15 — OPENED AND CLOSED (2026-08-29, loft#1178): a declared-collection lambda whose
> tail pass 2 REPLACES aborted the compiler.** `(L-Escape)` says a closure is an ordinary
> value that may be stored, passed and returned, and `(L-Apply)` that calling one is a call;
> `g = fn(v: integer) -> vector<integer> { xs = [1, 2]; xs.map(…) }` is both, and it did not
> compile at all — `H5 two-pass contract: grew a pass-2-only attribute __vdb_2`.
>
> The reservation was read off the PASS-1 tail, and this body defeats that read outright: its
> pass-1 tail is `Var(xs)`, a named local that already owns a store — the exact spelling of
> the bodies that must NOT get a buffer — while pass 2 lowers the `map` into a fresh one. The
> two passes are not looking at the same tail, so no predicate over the pass-1 one can
> separate the rows. Reserving for EVERY declared-collection lambda is what compiles them
> all, and the two things that blocked it are now closed: `State::fn_return` releases the
> buffer a callee did not hand back (D-clo-7's fix) and the native dispatch now asks the same
> question of the VALUE that came back rather than of the deps that declared an intent. The
> one exception is a fact rather than a prediction — a CAPTURE tail has nothing to deliver
> (D-clo-14).
>
> Two defects had to come out of the way, and each is its own sentence:
>
> - a lambda nested in another lambda's body left `last_closure_work_var` set, so the OUTER
>   fn-ref was mapped to a closure variable living in the INNER lambda's table and `--native`
>   emitted `var_??` for it. The named-function reset states the same rule one scope out
>   (*"a lambda inside make_adder leaks last_closure_work_var into the next function
>   parsed"*); a lambda inside a lambda is that leak within one body.
> - `--native` could not compile the map row: the desugar's `_map_result_1` is built INSIDE
>   the comprehension block and handed back from outside it, and a Rust `let` lives where the
>   emission first reaches it. The interpreter cannot have that — a local is a frame slot
>   wherever it is written — so it is a property of the EMISSION. Every VIEW a `return` names
>   is now bound up front, the cure loft#731 gave the iteration scratch for the identical
>   error. A view only: #354 measured the other half, and hoisting a heap local that OWNS its
>   store re-inits a fresh one per call that the matched free no longer covers.
>
> Guard: `tests/scripts/1178-a-declared-collection-lambda-gets-its-buffer.loft`, which carries
> all seven rows of the issue's table because the reservation is now unconditional and the
> rows that must NOT fill a buffer are what says the runtime free carries them.

> **D-clo-14 — OPENED AND CLOSED (2026-08-29, loft#1182): a lambda handing back a place read
> out of a CAPTURE reserved a return buffer it then ignored.** `(L-CapHeap)` says the captured
> store belongs to the frame that made it, so there is nothing for the callee to place — and
> `ref_return`'s ladder had no verdict for that and fell through to `Grow`, so
> `fn(v: integer) -> vector<integer> { q.xs }` grew a hidden `q` buffer the body never fills.
>
> The two backends then disagreed, and that disagreement is the entry. `--interpret` was clean
> because `State::fn_return` releases any buffer the callee did not hand back (D-clo-7's fix),
> a RUNTIME check that does not care what the deps claim. `--native` reads the deps:
> `arm_frees_buf` frees an unfilled `__vc_hbuf` only when the candidate's return deps do NOT
> name a hidden heap attr, and they do, because the buffer exists. One store leaked per call.
>
> `classify_text_dep` has answered this exact question since @PLN85 — `TextDep::SkipCaptured`,
> *"captured closure var — read from the closure record; never promoted"*. One notion, two
> ladders, and only the text one could see it. The ref ladder now carries the same verdict.
>
> Guard: `tests/scripts/1182-a-captured-place-tail-reserves-no-buffer.loft`, whose native row
> moves on the leak channel and whose interpret row is INERT — a backend divergence can only
> move one, which is why `make falsify`'s conservative AND reports NOT falsified for it.

> **D-clo-11 — OPENED AND CLOSED (2026-08-29, loft#1181): a captured STRUCT was TAKEN by the
> caller's bind, and the same dep was dropped TWICE on its way to the call site.**
> `(L-CapHeap)` names struct and vector in one breath, so D-clo-10's *"only for a COLLECTION
> return"* was never a rule — it was where the measurement stopped. `r = s(1)` on
> `s = fn(v: integer) -> P { cap }` adopted the captured record and the rebind released it;
> `LOFT_STRICT_STORES=1` reported the use-after-free, and a SECOND capturing lambda in the
> same function turned it into a wrong answer by landing its closure record on the freed slot.
>
> That entry's stated reason — *"a struct return is MATERIALISED into a fresh copy before it
> leaves the callee"* — is false, and the IR says so in one line: the lambda's body is
> `return OpGetDbRef(__closure, 0)`. Nothing copies.
>
> Two independent drops, and the issue's two recorded repair attempts each failed on the
> other one:
>
> - **the fn-ref VARIABLE kept pass 1's type.** Pass 1 has not parsed the body, so the type
>   it publishes says the result is owned; pass 2 knows better. `is_equal` collapses deps, so
>   `change_var_type`'s equality early-return kept the uninformed answer and the call site
>   never saw the dep at all. A fn-ref slot now ADOPTS a refined return dep, for the reason
>   the `#663` element width beside it is adopted — same base type, so the frame the two
>   passes lay out is unchanged.
> - **`fnref_result_type` read *"an index naming no visible argument"* as the closure.** True
>   of `__closure` and false of `ref_return`'s `__ref_N`, and BOTH are out of range: `{ cap }`
>   borrows and `{ sr_make(k) }` owns, spelled identically. D-clo-10 recorded that as
>   *"no dep-index test can separate them"*, and that is true of a RANGE test and false of a
>   NAME test — `Argument::hidden` already carries the distinction and its own doc already
>   states the conclusion (*"should be excluded from dep propagation"*). A lambda now
>   publishes a return type whose leftover out-of-range index can only be the closure, which
>   is what lets the borrow be read without over-approximating the mint into a leak.
>
> ⚠ **The closure is read only where the lambda's tail is a PLACE** — a slot, or a field /
> element / capture read out of one. A tail that JOINS hands back the capture on one arm and
> a fresh store on the other while carrying ONE dep list, and neither reading is right twice:
> as a borrow the minting arm leaks four stores, as owned the capture arm is released while
> its variable is live. That is D-clo-13, and the restriction is what keeps this entry from
> trading one defect for the other.
>
> Guard: `tests/scripts/1181-a-captured-struct-is-not-the-callers-to-take.loft`, whose
> falsification row moves on ONE cell — the over-free is silent until something reuses the
> slot, so the direct-rebind cells pass on the control build and are scored by
> `LOFT_STRICT_STORES=1` instead.

> **D-clo-12 — OPENED AND CLOSED (2026-08-29 / 2026-08-30, loft#1185): a FORWARDING function
> froze the capture its fn-ref argument handed back.** `fn call_it(f: fn(integer) -> P, v: integer) -> P { f(v) }`
> called with a capture-returning lambda releases the captured record on the caller's rebind.
> Inside `call_it` the slot is a PARAMETER with a DECLARED fn-type, which carries no deps
> whatever closure is passed, so the closure read D-clo-11 installed is inert one frame down
> — the same predicate seen from the other side that D-clo-9 measured for monomorphs.
> D-clo-9 resolved it at the CALL SITE, where the caller named the closure it passed; here
> the forwarding function's return type is computed ONCE for every caller, so no per-argument
> fact can reach it. Both routes that entry proposed were measured and neither works: reading
> a fn-typed PARAMETER as capturing moves nothing, because the dep still never reaches the
> published `-> P`.
>
> **So the value is COPIED before it escapes** — `classify_reference_delivery` answers
> `MaterializeView` for a tail that calls through a fn-ref parameter, the same rewrite it
> already applies to a tail pointing into something the callee frees. The caller then owns an
> ordinary fresh record and the capture is untouched, at the cost of one record copy on the
> forwarding path, which is the cost this entry named.
>
> The copy alone would orphan the other arm: a forwarded lambda that MINTS its return leaves a
> store nobody owns once the copy is taken. `Store::alloc_serial` — a monotonic stamp compared
> against a snapshot taken when the fn-ref call began — separates a store minted DURING the
> call from a capture that predates it, and the minted one joins the hand-up list loft#1183
> established. That comparison is the whole of what no static dep list could answer, and slot
> numbers being reused is why nothing cheaper can stand in for it.
>
> Guard: `tests/scripts/1185-a-forwarded-fnref-result-is-not-the-callers.loft`, with the mint
> row as its control.
>
> ⚠ **The BOUND spelling is not closed**: `{ r = f(v); r }` binds before returning, so the tail
> is a `Var` and no tail-shaped rule reaches it — 4 use-after-free reads either side. Reaching
> it means unpicking NRVO, since `r` is itself the return buffer there and a copy would target
> its own source. @PLN150.

> **D-clo-12 / D-clo-13 — the two are ONE question, and every static reading of it has now
> been measured in both directions (2026-08-30).** The question is: *does a fn-ref call hand
> back a store the caller may free?* The callee answers it per RUN — a capture on one arm, its
> own mint on the other — and each static answer trades one defect for the other:
>
> | reading | capture arm | mint arm |
> |---|---|---|
> | OWNED (today) | use-after-free (loft#1186 present, loft#1185) | clean |
> | BORROW (`place_tail` true) | clean | leaks one store per call |
>
> Both rows are measured on both backends. The mint row is not an artefact of the `??`: the
> forwarding case's `call_it(fresh, 1)` — the row loft#1185 records as clean — is the same mint
> under the same reading, and a borrow there has nothing to free it either.
>
> A third reading was tried and is ruled out for a different reason: letting
> `capturing_fnref_var` answer for a fn-typed PARAMETER, so a forwarding frame's tail borrows
> the closure its argument carries. Measured, it moves nothing — loft#1185 keeps its seven
> use-after-free reads on both backends — because the dep never reaches the forwarding
> function's PUBLISHED return: `fn call_it(f: fn(integer) -> P, v) -> P` still declares a bare
> `-> P`. The tail's dep is a FRAME dep on `f`, and nothing converts it to the attribute index
> a caller could map. So the forwarding case is not one predicate short; the fact has no route
> through a return type computed once for every caller.
>
> **So the answer is not a dep list, and it is not a return BUFFER either** (see the D-clo-13
> entry below: the store handed back was never the call site's buffer). The fact exists at
> RUN time and in ONE place — the callee's own `OpFreeRefIfDistinct(w, ret)`, which compares
> the store it minted against the store it is returning. What is missing is a channel from
> there to the caller's free, and the tree already has the shape of one: `@PLN90` #495's
> `witness_vars` / `_own_store_<name>`, a per-run witness for a local whose ownership differs
> per PATH. Here it would differ per CALL, so the witness has to be set from the callee's
> answer rather than from the caller's own assignments. That is a plan, not a predicate —
> **@PLN150**, which carries the measured table above, the three ruled-out repairs, and both
> candidate channels.

> **D-clo-12 / D-clo-13 — the static reading was RE-MEASURED against this tree (2026-08-29),
> and the cure is now decided.**  Setting `published_ret_type`'s `place_tail` unconditionally
> true — that is, answering the closure question for a JOIN tail as well — makes loft#1186's
> PRESENT arm clean on both backends and leaks four stores on the ABSENT arm, one per call, on
> both.  So D-clo-13's claim is not a historical note about the tree it was written on: it
> holds after loft#1179's runtime free and after loft#1183's hand-up, and no reading of one dep
> list serves both arms.  loft#1185 is unmoved by that switch (still seven use-after-free reads
> on both backends), because a forwarding frame's return type is computed once for every
> caller and carries nothing about the closure its ARGUMENT held.
>
> **The cure is the one loft#1186 names, and it is an ABI change rather than a classification
> fix: the fn-ref call site mints its heap return buffer as a CALLER LOCAL** — the symmetric
> twin of `push_fnref_text_buffers` / `fnref_text_buffer_vars`, with
> `Data::fnref_text_buffers`' widest-candidate-then-trim shape as the precedent.  Today the
> buffer is allocated inside `State::fn_call_ref` at run time, which is why `fnref_bufs` has to
> track it by frame depth at all.  With a caller local:
>
> - the call's RESULT is published as a borrow of that buffer, so a destination local never
>   adopts whatever store came back — which is what closes both of D-clo-13's arms at once:
>   the absent arm's fresh store is the buffer (freed at the caller's scope exit) and the
>   present arm's capture is simply not the caller's to free;
> - a FORWARDING frame gets the same buffer in its own scope, so `return f(v)` is a return of a
>   borrow of a local and the existing #306 materialise copies it into the forwarder's own
>   return buffer — D-clo-12 closes as a consequence, at the cost of one record copy;
> - `--native`'s dispatch arm stops needing `__vc_hbuf` at all, which is loft#1183's remaining
>   half.
>
> The cost is one heap buffer per fn-ref call site that may receive a heap delivery, and one
> extra record copy on the forwarding path.  The `&text` half has paid exactly that since
> loft#1116.
>
> **D-clo-13 — OPENED AND CLOSED (2026-08-29 / 2026-08-30, loft#1186): a lambda whose tail
> JOINS a capture with a mint had one dep list for two ownerships.** `fn(n: integer) -> P { cap ?? P { v: -1 } }` hands
> back the captured record when the subject is present and a store of its OWN when it is
> absent.
>
> ⚠ **The absent arm was written here as handing back the call site's return buffer, and it
> does not** (re-measured 2026-08-30, `7dfafc22`). The emitted body takes `__retbuf`, never
> writes it, mints `__ref_p2_1` with `OpDatabase`, and keeps that store precisely when it is
> the one being returned — its own `if __ref_p2_1 != __ret_1 { free }`. So the store the
> borrow reading leaks is a CALLEE MINT, and no rule about who owns a call site's return
> buffer can reach it: a caller-owned buffer the callee ignores changes nothing about the
> store that actually comes back. The re-measurement is on the issue, together with what it
> leaves standing — the callee computes *"the store I minted is the one I am returning"* in
> one place, which is where an owner could be attached without a new ABI. Read as owned, the present arm is a use-after-free; read as
> a borrow, the absent arm leaks one store per call. The NAMED twin is clean on BOTH arms and
> says what the cure is: a direct call site mints the return buffer as a caller LOCAL that
> scope exit frees, so whichever arm runs the buffer has an owner. The fn-ref path has no
> such local. The cure is the symmetric twin of `push_fnref_text_buffers` — a fn-ref call site
> that may receive a heap delivery owns that buffer the way it already owns its `&text` ones,
> with `Data::fnref_text_buffers`' widest-candidate-then-trim shape as the precedent for the
> adaptive ABI.

> **The cure was not a third reading of the dep list.** The callee already computes the
> answer, at run time and in one place: the two operands of the free that guards its own mint
> name ONE store exactly when it is handing that store back. `OpFreeRefOrHandUp` is that free
> with an owner on the adoption leg — the store joins the list a delivered return buffer uses
> and `release_fnref_bufs` carries it up, by the rule loft#1183 already established. The
> distinct leg is untouched, so a function whose return does not borrow keeps the op it had.
>
> With the mint owned, the BORROW reading is right for both arms, and `published_ret_type` now
> keeps the `__closure` index for a JOIN tail with a place arm as well as for a tail that
> cannot join. `--native` needed one more thing to agree: a REFERENCE-returning fn-ref call
> was handed the null sentinel where the interpreter has always allocated a buffer, so the
> store the callee materialised had no owner on that backend alone. It gets a real buffer now,
> which is also what makes the comparison askable there.
>
> Guard: `tests/scripts/1186-a-join-tail-hands-its-mint-an-owner.loft`, whose both-arms cell —
> one closure, one call site, the arm decided by the argument — moves on BOTH channels at
> `c3545888`: use-after-free reads interpreted, a leak natively.
>
> ⚠ **loft#1185 is NOT closed by this**, and the difference is the point: a forwarding frame's
> return type is computed once for every caller, so the fact never reaches it. That is D-clo-12
> and @PLN150's second channel.

> **D-clo-10 — OPENED AND CLOSED (2026-08-29, loft#1180): a captured COLLECTION was TAKEN by
> the caller's bind.** `(L-CapHeap)` says a captured heap value is SHARED — the caller may read
> it, never take it. `r = g(7)` on `g = fn(v: integer) -> vector<integer> { cap }` adopted the
> store and released it at scope exit, so `cap` answered EMPTY from the second call onward, on
> both backends, with nothing saying so.
>
> `fnref_result_type` maps a fn-ref call's return deps through the caller's actual arguments
> and DROPPED any index naming no visible one, on the stated grounds that *"the adaptive fn-ref
> ABI allocates those buffers at runtime, so the value arrives OWNED"*. That is true of a
> hidden work buffer and false of `__closure`, which is the CALLER's own record — D-clo-7's
> sentence one more time, *a dep dropped as uninteresting is not a dep that was never there*,
> in a third position after the `??`-lift (loft#1114) and the fn-ref tail (loft#1176). The
> dropped index now becomes a dep on the fn-ref VARIABLE, which is where the caller reaches
> its closure.
>
> Two restrictions, both measurements rather than caution:
>
> - only for a CAPTURING slot, read off the fn-ref TYPE's own deps. That predicate means what
>   it says HERE, where the slot is a caller local whose type was INFERRED at the bind; it is
>   inert one frame down, where the same slot is a parameter with a DECLARED fn-type
>   (loft#1176 measured that, and the two entries are the same predicate seen from both sides).
> - only for a COLLECTION return. ⚠ **Both halves of this restriction were wrong, and D-clo-11
>   closed it a few hours later.** A struct return is NOT materialised into a fresh copy —
>   the lambda's body is `return OpGetDbRef(__closure, 0)` and nothing copies — so
>   `fn(i: integer) -> P { cap }` was a use-after-free, not a value that "was always right".
>   And *"no dep-index test can separate them"* is true of a RANGE test only: the
>   out-of-range index is `__closure` for `{ cap }` and `__ref_N` for `{ sr_make(k) }`, and
>   `Argument::hidden` tells them apart by NAME. The leak this restriction was avoiding —
>   eleven stores in `717-closure-struct-return.loft` — is real and is what the name test
>   removes.
>
> ⚠ The captured-FIELD spelling (`{ q.xs }`) answers correctly now and still LEAKS one store
> per call on `--native` — loft#1182, a different mechanism: `ref_return` promotes the borrowed
> local into the return attribute, so the callee declares it delivers through a buffer it then
> ignores. The INLINE spelling was correct throughout, which is why this was first filed as a
> leak — nothing binds the result, so nothing adopts it.
>
> Guard: `tests/scripts/1180-a-captured-collection-is-not-the-callers-to-take.loft`.

> **D-clo-9 — OPENED AND CLOSED (2026-08-29, loft#1176): a captured record was FREED by a
> caller that lifted a fn-ref tail.** `(L-CapHeap)` says a captured heap value is SHARED, and
> a value the outer scope still names cannot be released by somebody else's scope exit.
>
> `fn once(x: P, f: fn(P) -> P) -> P { f(x) }` hands back a fresh store, the caller's own
> argument, or a record the closure CAPTURED, and its `-> P` reads the same in all three.
> The caller decided from `returns_borrowed_view`, the DEPS proxy: a capture-returning
> lambda's return dep names the hidden `__closure` attribute, and a hidden attr reads as
> *"not a borrow"*. So the caller lifted the result and freed it — the captured record
> answered another value on the next iteration and garbage once the scope ended, on BOTH
> backends. This is D-clo-7's licence exactly (*"a dep dropped as uninteresting is not a dep
> that was never there"*), in the direct-`Call` position rather than the `??` one that entry
> closed, and the `__retbuf` exemption made it worse: `{ f(x) }` never delivers INTO that
> buffer, so the premise that the lifted temp is the caller's own allocation is false there.
>
> The mirror image was live at the same time and is what the issue was filed for: the
> GENERIC spelling of the same source under-lifted, because the freshness proof it uses is
> read off the monomorph's body and a fn-ref's callee is a runtime value there — one leaked
> record per inline call. **One resolution answers both.** The callee's fact is unreachable
> from inside the callee and reachable at the CALL SITE, where the caller named the closure
> it passed: `fnref_target` resolves the definition and its own body-shaped freshness proof
> decides. Both ownership reads are needed and neither is redundant — the deps proxy catches
> a lambda handing back its own PARAMETER, the body proof catches one handing back a CAPTURE.
> An unresolved or ambiguous slot declines, which costs the leak that was already there.
>
> ⚠ **The fn-ref must be a caller LOCAL.** `fnref_target` maps variable slots, so one held in
> a struct field (`once(P { n: 41 }, h.f)`) resolves to nothing and declines — one leaked
> record per call, unchanged by this fix and recorded here rather than left implicit.
>
> Guard: `tests/scripts/1176-a-monomorph-whose-tail-is-a-fn-ref-call.loft`, whose two halves
> fail on DIFFERENT channels (the over-lift on an assertion, the under-lift on the exit leak)
> and whose header says which of them the falsification row can and cannot score.

> **D-clo-8 — OPENED AND CLOSED (2026-08-28, loft#1131): a captured `vector<(…)>` was
> UNPACKED instead of shared.** `(L-CapHeap)` says a captured heap value is SHARED, and the
> mechanism is a 12-byte `DbRef` in the closure record. `closure_attr_type` types every
> collection capture as `Reference(<element def>)` carrying the #328 share marker — the def
> is a stand-in for *"some DbRef"*, not a claim about what the slot holds.
>
> For a `vector<(…)>` that stand-in def is `__tuple<…>`, which is exactly what loft#821's
> per-element tuple write in `set_field_check` matches on. It read the DESTINATION slot's
> spelling while its own comment says the arm must be chosen by *"the SOURCE's
> representation rather than by which spelling the slot happened to carry"* — so the capture
> emitted the vector's own bytes as two integers:
>
> ```loft
> xs: vector<(integer, integer)> = [];  xs += [(1, 11)];  xs += [(2, 22)];
> s = c0(fn() -> integer { a = 0; for t in xs { a += t.0 * 1000 + t.1; } a });
> //  --interpret: 0, silently.  vector<(integer, P)>: len 0 then SIGSEGV.  --native: E0308.
> //  the same loop OUTSIDE the closure: 3033.
> ```
>
> A tuple of SCALARS fails too, and it carries no store — which is what rules an ownership
> explanation out and names the capture's SHAPE as the axis. The arm now also asks whether
> the slot holds a `DbRef` (`deps.contains(&u16::MAX)`, the spelling three neighbouring sites
> already read for the same question), which routes a capture to the auto-Reference store
> directly below it.
>
> Guard: `tests/scripts/1131-a-captured-collection-is-stored-as-a-handle-not-unpacked.loft`,
> which keeps the struct / nested-vector / keyed element types as controls — those are the
> @PLN93 shapes the tuple row fell outside of, and a fix that took one of them down would be
> worse than the defect.

> **D-clo-7 — CLOSED (2026-09-03): the witness was a SLOT, and the slot was in the body.**  The
> open half read *"the return's dep names `__closure` and never which slot"*, and that was the
> whole of it: `closure_capture_base` answered only for a closure with ONE capture
> (`Some([only])`), because the callee's base is the `__closure` variable and the mapping the
> build writes — `OpSetDbRef(___clos_N, off, var)` — was collected with its offsets thrown away.
> The callee's `??` subject reads `OpGetDbRef(__closure, off)` at exactly one of those offsets.
> `capture_return_offsets` collects the offsets a return can hand back, skipping a read whose
> result is consumed on the spot by an op that answers no store (`OpGetInt(OpGetDbRef(__closure,
> 12), 0)` is a field of another capture, not the capture); `fnref_captures` keeps `(off, var)`;
> one offset resolves to one caller variable, and the routes already shipped take it from there
> — `OpBindOrCopy` for a record (loft#1248), store identity for a collection (loft#1257).
>
> Measured, both backends, every value unchanged, `LOFT_POISON` and `LOFT_STRICT_STORES` clean:
> two store-bearing captures with a record return 499 stores at N=500 → 3 and 70 000 calls
> under the ceiling; a captured collection 65535 (abort) → 3; two captured collections 499 →
> 3; a store-bearing capture beside a pure mint 500 → 3; the borrow arms leave both captures
> intact and the bound spelling is a copy.
>
> ⚠ **Three things the matrix caught, each a wrong cut before it shipped.**  (1) Answering
> `Borrowed { u16::MAX }` from `classify` for an unnameable base — instead of the `Owned`
> fallback the readers were gating around — broke the direct nullable-capture return
> (`fn(n) -> P? { return c; }`, guard 1114); the fallback stays, and the free-deciding sites ask
> the predicate below instead.  (2) A collection `??` over a capture never hands the capture
> back — its chosen arm is COPIED into the caller's `__retbuf` — but a first cut read that as
> *every* collection return, and `fn(k) -> vector { c }` plus 1180's captured field were then
> freed under the closure.  The callee's own ownership verdict cannot separate them either: it
> answers a callee LOCAL (`__vdb_1`, the mint's backing).  What does is the return's DEP LIST:
> it names `__closure` only where the capture can come back.  (3) A capture variable assigned
> again after the build keeps naming the NEW store while the closure holds the build-time one
> (`L-CapHeap`), so `Defs::multi_assigned` refuses it as a witness — and reassigning such a
> variable leaks a store at exit today regardless (loft#1324, pre-existing).
>
> Declined on purpose: `c ?? d`, where either capture may come back and one witness cannot
> answer for two.  Guard: `tests/scripts/1248b-a-capture-witness-is-the-slot-the-return-reads.loft`,
> six cells, falsified at c6239cbf.

> **D-clo-14 — the traded leak CLOSED, one spelling residual (2026-09-03, loft#1257).** The
> decline above cost the mint arm one store per call — 389 live at N=400, a store-table abort
> at scale. It no longer does, on both backends, with every borrow-arm value unchanged.
>
> **The cure needed no witness, and the entry below is where that was got wrong.** It reads
> *"a cure has to carry the Join base as a witness to BOTH sites"*, and both halves of that
> sentence were measured — but `(O-Oracle)` already says what a `Join` means at run time
> (*"adopt iff the value's store ≠ `base`'s store"*), and the dep NAMES that base. So the
> owner is decidable by store IDENTITY, which is `ownership.md` D-own-16's route one shape
> over, at no witness slot and no IR temp.
>
> **What answers for BOTH frees is one thing, not two: the base rides on the temp's TYPE.**
> `callref_owned_return` types `__lift_N` with `Deps::frame1(base)` instead of `Deps::none()`,
> and a non-empty dep stops `state/codegen.rs` emitting the unconditional pre-Set free at all —
> the RE-SET that left the interpreter wrong when only the scope-exit free was guarded.
> `get_free_vars` then emits `OpFreeRefIfDistinct(__lift_N, base)` for the one that remains.
> One guarded free per evaluation.
>
> ⚠ **The container KIND broke it once, and only on one backend.** The keyed arms were written
> blind and kept `Deps::none()`, so the pre-Set free survived for them alone: the hash cell
> emptied its caller's collection on the INTERPRETER while `--native` stayed green. Found by
> moving the axis, not by reading the code — `matrix_axes.py` reported `A1 container kind 2/6`
> for the probe set that had already passed.
>
> **RESIDUAL, and it is a statement CONTEXT rather than a shape:** `r = if c { g(some) } else
> { g(none) }` still leaks the mint arm — 246 stores at N=500, attributed to the closure's own
> mint line — because the lift is not consulted for a call in an `if`/`match` arm at all. Not
> a regression: it leaks identically under `LOFT_NO_LIFT_JOIN_WITNESS`.
>
> **Not reached, and measured so:** the CAPTURE witness (`c ?? [7,8]` over a captured `c`)
> aborts at the ceiling with the route on exactly as with it off — the return's dep names
> `__closure` and not which slot, so there is no base. That is D-clo-7's open half, and this
> is the measurement the ROADMAP asked for when it said whether the identity route reaches the
> two closure rows was untested.
>
> Guard: `tests/scripts/1257b-a-lifted-collection-return-is-freed-by-identity.loft`, eight
> cells, falsified at d9a2ec21 (interpret exit 101 -> 0, native exit 1 -> 0, both panicked ->
> clean). Opt-out `LOFT_NO_LIFT_JOIN_WITNESS` keeps the pre-lift form as the A/B leg.

> **D-clo-14 — the OVER-FREE closed, the leak it traded for OPEN (2026-09-01, loft#1257).**
> `g = fn(q: vector<integer>?) -> vector<integer> { q ?? [7, 8] }` used INLINE inside a LOOP
> answered `null` and left the caller's own vector EMPTY — `len(some)` reached 0 with nothing
> saying so, on both backends. Both axes were required: the named twin was right, the bound
> spelling was right, and one inline call outside a loop was right, so every single-axis probe
> passed.
>
> **The same sentence as D-clo-7, at the collection arm, with the sign reversed.** There the
> deps proxy called a JOIN a borrow and the mint arm went unowned; here it calls the same JOIN
> *owned* and the borrow arm gets freed. A collection return is delivered through a HIDDEN
> buffer, so its dep names only hidden attributes and `returns_borrowed_view()` reads
> *"minted into its own buffer, the caller adopts"* — right when the closure mints, wrong when
> the `??` hands back the argument, and those are the same call. `callref_owned_return` then
> lifts it into a `__lift_N` typed with `Deps::none()`, and an empty dep list is what makes
> `get_free_vars` emit the free that empties the source.
>
> `(O-Oracle)` answers what the proxy cannot: a `Join` whose base the @P290 bracket can NAME is
> *"this may be that caller variable"*, so the collection arms decline it. The `Reference` /
> record `Enum` arms still lift, because `OpBindOrCopy` settles it per execution (loft#1248).
>
> **OPEN: the mint arm of that same closure now leaks**, one store per call — measured peak 4
> → 403 at N=400, a store-table abort at scale. Taken deliberately, and the reason is the
> label doctrine rather than a preference: a leak announces itself and a silently emptied
> container does not, so `silent-wrong` outranks `sev:`. It costs only the JOIN shape; a pure
> mint classifies `Owned`, or `Borrowed` of a hidden buffer with no nameable base, and
> loft#1177's cells all keep their lift.
>
> **A WITNESSED lift is the end state, and it was built and measured rather than proposed.**
> `OpFreeRefIfDistinct` — the machinery `paired_witness` already drives for a work-ref — fixes
> `--native` and leaves the INTERPRETER wrong. The interpreter's damage is not the scope-exit
> free but the RE-SET: one iteration is correct and two are not, so the transition-free on
> `__lift_N`'s reassignment releases the borrowed store before any scope-exit free runs. Both
> halves are needed, and the next attempt should start from that measurement.
>
> Guard: `tests/scripts/1257-a-lifted-collection-return-does-not-empty-its-source.loft`, whose
> last cell is the TRADE rather than a pass, and whose one- and two-iteration cells are what
> located the interpreter's half. Falsified at `ca1a829e`. ⚠ Scored by VALUE: the leak channel
> read `NO leak` on the broken build throughout, because a container emptied by a free of a
> store that IS freed is not a leak.

> **D-clo-7 — value half CLOSED, leak half OPEN (2026-08-27, loft#1114).** `(L-CapHeap)`
> says a captured heap value is SHARED. A NULLABLE one was not: `closure_attr_type`
> recognised `Reference`, the keyed collections and `Vector`, and let `S?` fall through — so
> the capture kept its `__nullable<S>` enum type, was COPIED into the closure record INLINE
> while its dense twin was SHARED as a `DbRef`, and the body's read then applied the enum's
> payload offset on top of a record the write had placed without one. The lambda answered
> `4294967199`, with nothing saying so.
>
> `S?` IS a `DbRef` whose `rec == 0` means absent, which is why the cure is a peel and not a
> new storage class. `Data::nullable_struct_payload` answers the one-sided question in BOTH
> spellings — the `Optional(Reference(S))` the author writes and the `Enum(__nullable<S>,
> true)` the field rewrite produces — and that is the whole of it: **recognising only the
> spelling a site happens to see is what gives one value two layouts.** The same gap wore an
> ICE (the tail's type changes KIND between the passes, so the delivery arms differ and pass
> 2 grows an attribute pass 1 never minted) and a REFUSAL of a legal program (`Type::is_equal`
> had a peel for eight wrappers and none for `Optional`, so derived `==` compared the inner's
> deps and printed one type as two).
>
> ⚠ **The refusal was MASKING the wrong answer.** With the `Optional` peel applied and the
> capture still copied, a refused cell stops being refused and starts answering
> `4294967199` — a loud refusal traded for a silent wrong one. The peel is restricted to
> inners that CARRY DEPS, because a scalar has none and derived `==` then compares the SPEC,
> whose integer half is the layout-bearing WIDTH (loft#663): without that restriction `u8?`
> and a wider `integer?` become one type and `overflow(300)` answers `300`.
>
> ⚠ **And fixing the capture exposed a use-after-free behind it.** With the store shared, a
> caller's `??` over the fn-ref return LIFTED that join into a temp and freed it, releasing
> the captured record while the outer variable was still live — so a second lambda over the
> same variable read a released store. The licence was an empty return dep, and it is empty
> because `fnref_result_type` DROPS an index naming a hidden attribute on the stated grounds
> that *"the value arrives OWNED"*. `__closure` is a hidden attribute, and a captured value
> does not arrive owned: **a dep dropped as uninteresting is not a dep that was never there.**
> The lift now declines for a CAPTURING fn-ref and still fires for one that captures nothing.
>
> **The leak — first half CLOSED (2026-08-29, loft#1179), second half OPEN.** Both halves
> are the same sentence: *a direct call site mints the return buffer as a caller LOCAL it
> frees at scope exit, and the fn-ref path had no equivalent.*
>
> CLOSED — a lambda that BINDS its return to a local (`d = q ?? P{}; d`) leaked one store per
> call. `fn_call_ref` allocates one store per hidden return attribute because it cannot know
> which function the slot holds, and a callee that delivers its return some other way — it
> minted its own store, or the delivery slot was rebound to a borrow — left that store owned
> by nobody. `--native` never had it: its dispatch passes the null sentinel for a Reference
> return and frees an unfilled `__vc_hbuf` for a vector one, which is the same fact this side
> was missing. `State::fn_return` now releases every buffer the returning frame's call site
> allocated, keeping the one the callee handed back — identified by STORE, because a callee
> that delivered through the buffer may answer a record or a position inside it.
>
> That one free also closed loft#1180 (a lambda returning a captured struct's vector FIELD,
> both spellings) and made loft#1178's reservation safe to widen: reserving a return buffer
> for EVERY declared-collection lambda was already correct on `--native`, and the only thing
> wrong with it here was the unowned buffer.
>
> **The `??`-default store — the ARGUMENT witness CLOSED (2026-09-01, loft#1248), the
> CAPTURE witness OPEN.** `g = fn(q: P?) -> P { q ?? P{} }` leaked the default arm's store,
> one per call, on both backends, released only when the CALLER's frame exited — so a loop
> reached `store table exhausted: 65535 stores live at once`.
>
> ⚠ **Two claims in the previous wording were wrong, and both were measured that way.** *"the
> BOUND spelling is clean"* holds only for a LITERAL `null` argument: from a variable,
> `r = g(none)` leaked at exactly the same rate as the inline spelling, because the dep then
> has a caller variable to name. And *"discarded INLINE"* named a symptom rather than the
> axis — the two spellings reach two different sites (the lift, and `scan_set`'s deps strip
> with the first-bind dispatch), so a fix at either alone moves one of them.
>
> **The cure was the one this entry already named, and the reason it had not been taken is
> that the oracle could not answer.** `(O-Oracle)` says the own-vs-borrow verdict is computed
> by ONE oracle and that a call resolves through the callee's return summary — but a CALL HAS
> TWO SPELLINGS. `Value::Call` names its definition; `Value::CallRef` names a runtime value,
> so `Ownership::classify` had no arm for it and it fell to `_ => Own::Owned`, the one answer
> that licenses a free. Nothing crashed on that, because every reader gates on the `Call`
> spelling first — so the effect was not a wrong free but a whole family that never got the
> oracle's answer at all, and was left to `(O-Proxy)`, which `ownership.md` says in as many
> words is UNSOUND ALONE.
>
> `classify` learns the second spelling (resolving the target through
> `scopes::collect_fnref_targets`, shared rather than re-derived), and three readers learn it
> together through one predicate, `use_analysis::callref_join_first_bind`: `scan_set`'s deps
> strip, so a free is emitted at all, and both backends' `OpBindOrCopy`, so that free is right
> on the borrow arm too. `callref_owned_return` reads the oracle beside the proxy, exactly as
> the direct-call branch does.
>
> ⚠ **The leak and the over-free are ONE axis, and stripping the deps without the guard turns
> this defect into the other one.** Measured mid-fix: with the strip in and native's guard
> still keyed on the `Call` spelling, `--native` freed the caller's own record and answered
> `s=null src=-2401053088876216593` — while the LEAK channel read clean throughout, because a
> freed store still holds plausible bytes. `LOFT_STRICT_STORES=1` plus a read of the SOURCE
> after the loop is what separates them, and both are cells now.
>
> **The CAPTURE witness closed the same day, and the mapping was already in the IR.** A
> capture arrives through the hidden `__closure` attribute, so `caller_arg_base` answered
> `u16::MAX` and the conservative no-lift stood — `c: P? = null; fn(k) -> P { c ?? P{} }` held
> one store per call to frame exit. But the closure BUILD emits `OpSetDbRef(___clos_N, <slot>,
> <caller var>)` for every capture, so *"which caller variable is capture slot k"* is written
> down where the closure is made; `collect_fnref_captures` reads it, beside the target
> resolution and shared with it. 404 → 5, both backends.
>
> ⚠ **Ask the callee's VARIABLE space, not its attribute space.** `callee_base` is a variable
> number, and the two are not the same numbering: in the closure this fix is about `__closure`
> is variable 3 and attribute 2, so an attr-indexed test reads OUT OF RANGE and answers "not
> the closure" for the single case it exists to catch. `caller_arg_base` beside it indexes
> attributes and is right to; they answer different questions about the same value.
>
> **STILL OPEN, and both are the same missing fact.** A LITERAL `null` argument has no slot to
> compare against, and a closure with TWO store-bearing captures is ambiguous — the return's
> dep names `__closure` and never which SLOT, so the borrow arm may be either and comparing
> against the wrong one would adopt a store the caller still holds. One capture is decidable
> and is taken; two decline and keep the leak, which is this gate's standing direction when it
> cannot name what it would be freeing. Closing it needs the dep to name the CAPTURE rather
> than the record.
>
> Guarded by `tests/scripts/1248-a-closure-join-return-owns-its-minted-arm.loft` (falsified at
> `212bf82c`: both backends abort in the first cell) and
> `tests/scripts/1114-a-nullable-heap-capture-is-shared-like-its-dense-twin.loft`.
>
> **PARTLY CLOSED (2026-08-31, loft#1245) — a fn-ref call is a call in BOTH spellings.**
> The (B-Copy) value half is closed: the bind now copies like its named twin.  The LEAK
> half this entry was filed for is CLOSED TOO once the two checkouts joined (2026-09-01):
> the lift is admissible on either route — a witness the oracle can NAME (loft#1248) or a
> witness set that is COMPLETE with no capture (loft#1245) — and neither branch had both.
> Measured flat at 70 000 inline calls, and clean under `LOFT_POISON=1` +
> `LOFT_STRICT_STORES=1` including the loft#1114 capture guard.  Re-measured
> 2026-08-31, and the scope recorded here was wrong in BOTH directions, so the entry is
> restated rather than edited.
>
> What was written: *"a lambda's `??`-default store discarded INLINE leaks one store per call
> on BOTH backends; the BOUND spelling is clean, and so is the named twin."* Measured against
> `LOFT_ALLOC_REPORT=1` at N calls, "bound" named the wrong bind — binding inside the lambda
> BODY (`d = q ?? P{}; d`) leaks exactly as the inline tail does, and what is clean is binding
> the RESULT at the call site.
>
> **It is not a leak, it is a crash.** The entry was scored on the exit-leak channel, which
> reads ZERO for it: `cr_fnref_minted` / `State::release_fnref_bufs` do release the buffers,
> but the unit is the FRAME, so a loop holds one store per call until the frame exits. Nothing
> leaks at exit, the peak grows linearly, and both backends abort at `store table exhausted:
> 65535 stores live at once`. An exit-leak gate cannot see a frame-lifetime accumulation.
>
> **And the leak was the smaller half.** The same missing dispatch dropped the (B-Copy) copy:
> `r = g(a)` on a fn-ref ALIASED the argument, so `r.n = 99` wrote through to the caller's `a`
> where the named twin copies. It needed no nullable, no `??` and no capture — a lambda
> returning its parameter was enough — and both backends agreed on the wrong answer, so no
> differential oracle could see it. The compiler even emitted `lost-write` on that line,
> telling the reader the mutation *"lands in the copy, not the source"* while it corrupted the
> source. **That half is closed.**
>
> **The cure is one question asked once.** `use_analysis::callee_of` answers *which definition
> does this call reach?* for `Call` and `CallRef` alike, and the sites that decide what a
> returned store means now read it instead of matching one spelling: both backends' heap
> first-bind dispatch, `scopes`' deps strip (which its own comment already required to name the
> SAME callees as the dispatch, loft#810), and the @P290 bracket's two halves
> (`protectable_ref_args` / `call_return_frees_source`). The `CallRef` route did not need a new
> answer — it needed to be asked.
>
> **What is still open, and why it is not a matter of finishing the job.** The INLINE spelling
> holds its store to frame exit. Closing it means the bind must COPY, which is
> `gen_set_first_ref_call_copy` — reached when the callee does not adopt-fresh, or through a
> witnessed JOIN — and the lift that would arrange it is measured unsound three separate ways,
> each invisible without `LOFT_POISON=1`:
>
> 1. **The SUBJECT arm.** `h(have).n` in a loop hands back the caller's own argument; the
>    lifted temp adopts it and its scope-exit free releases it. The loop then answers `null`.
> 2. **A CAPTURE.** Neither reading can see it: `returns_borrowed_view` treats the hidden
>    `__closure` dep as *"the callee minted this"*, and `protectable_ref_args` reports its
>    witness set COMPLETE for a call whose arguments are all scalars — vacuously, having
>    witnessed nothing. Together they say *"owned and fully bracketed"* about a store the
>    enclosing scope owns. `use_analysis::callref_captures` is the one home for that exception.
> 3. **`τ?`.** `Optional(Reference)` reaches the copy-or-adopt split only through
>    `nullable_join_first_bind`, which is itself `Call`-only and wants a JOIN with a nameable
>    witness. Admitting the nullable spelling without that twin turned
>    `1114-a-nullable-heap-capture-…`'s `fn(q: P2s?) -> P2s? { q }` into a use-after-free.
>
> ⚠ **A fourth thing, and it is the one worth carrying forward.** Classifying a `CallRef` in
> `Ownership::classify` — so the join arm could see it — looked necessary and is not: with it
> removed every value cell is still correct and every peak still flat, while WITH it a direct
> `p2_named(p2_st)` read another cell's recycled store on `--native`. A change that is not
> load-bearing can still be load-bearing for a regression.
>
> ⚠ **The capture hazard was found by `scripts/matrix_axes.py`, not by the suite.** Every cell
> written by hand held closure capture at `none` (A10 1/5), and the fix was green on all of
> them, on both backends, under poison, while silently breaking every capturing lambda.
>
> Guard: `tests/scripts/1245-a-fn-ref-bind-copies-like-its-named-twin.loft`, falsified at
> `ee199301` on both backends, carrying the named twin as its control plus the capture and
> argument-spelling axes.  The leak half has no cell there because one would have to abort.
>
> Guarded by `tests/scripts/1114-a-nullable-heap-capture-is-shared-like-its-dense-twin.loft`.


> **D-clo-6 — CLOSED (2026-08-27).** `(L-FnRef)` says a bare function name is a first-class
> value. It was not, for a function that carries TWO hidden `RefVar(Text)` work buffers:
> `g = nb; g()` crashed the interpreter. loft#1116, both halves closed the same day.
>
> A function acquires two the ordinary way — a text local AND a discharge accumulator, each
> promoted to a hidden `&text` out-param. That is legal for a NAMED function, whose own call
> sites lower against its known signature, and D-clo-4 records why forbidding it is not the
> cure (it moved five suite results). But the fn-ref ABI passes exactly ONE buffer, because
> a call site cannot know which function a fn-typed slot holds — so through a fn-ref the
> callee is entered short.
>
> **The `--native` half is closed.** Its dispatch arms are chosen by SIGNATURE, so a function
> nobody takes a reference to was reddening the build whenever some lambda shared its shape
> — the arm spent one buffer argument on both parameters (`E0499`). Extra buffers now get
> their own temporaries, which is sound on that backend and only there: native returns text
> OWNED and never threads the value back through the buffer, so the buffers type-check
> rather than deliver. Guarded by
> `tests/scripts/1116-a-fn-ref-arm-does-not-spend-one-buffer-twice.loft`.
>
> **The interpreter half is CLOSED too (2026-08-27, loft#1116).** There the buffer IS the
> delivery, so an extra temporary would have swallowed the result — and a `&text` is a
> pointer into the CALLER's frame, so the dispatcher cannot supply one that outlives its own
> return either. The count had to travel outward: the call site pushes what the WIDEST
> candidate of that signature could want (`Data::fnref_text_buffers`) and `fn_call_ref` pops
> what the actual target does not take, which is the same trim it already did for a target
> wanting none. One count, two readers.
>
> ⚠ **The other admissible cure on the issue — declining the fn-ref (`B-Ref-Reshape`'s
> precedent) — rested on a premise that had expired.** It was recorded as costing nothing
> *"since every such call faults today"*, and that was true when written; by the time it was
> taken up the `--native` half had landed and `g = nb_two; g()` ANSWERED there. Declining
> would have removed a working capability from one backend to make it match the other, and
> `(L-FnRef)` says the value is first-class in the first place. Re-measure a filed
> "nothing is lost" before building on it — a sibling fix can have made it false.
>
> Guarded by `tests/scripts/1116b-a-fn-ref-call-carries-every-text-buffer-its-target-wants.loft`,
> whose wide target holds DIFFERENT text in its two buffers on purpose: the obvious
> two-buffer function (`loc: text = "x"; return loc ?? "fb"`) has both buffers holding the
> same value, so reading the wrong one is invisible and that shape can only score a crash.

> **D-clo-5 — CLOSED (2026-08-27).** The third route to the same fault line, found by
> varying where `(L-Apply)` happens. `xs.map(fn(n: integer) -> text { return s; })` on a
> CAPTURING lambda panicked the interpreter with a corrupt `DbRef` while `--native`
> answered — a backend split, so neither backend alone could see it. loft#1115.
>
> Cause: the caller allocates the one hidden `RefVar(Text)` work buffer a text-returning
> fn-ref call hands its target, and `parse_operators` appends it for the ordinary `f(args)`
> spelling. `map` lowers its own `CallRef` and never appended it, so the callee was entered
> one DbRef span short and read its `__closure` from the wrong offset. The closure argument
> itself is NOT part of that injection — `fn_call_ref` reads it back from the 20-byte fn-ref
> slot — which is exactly why the same shape returning an integer, a struct or a boolean was
> always correct, and why the fault looked like a capture problem when it was a buffer one.
>
> Fixed in `parse_map` through one `callback_call_ref` helper. The buffer is drawn from
> `caller_text_buf`'s `__work_c<N>` sequence, not `work_text`'s `__work_<N>`: the map family
> early-returns on pass 1, so this mint is pass-2-only, and a pass-2-only mint on the shared
> counter shifts every later `__work_N` — loft#662's class. Guarded by
> `tests/scripts/1115-an-inline-callback-gets-the-text-buffer-its-abi-expects.loft`, whose
> native half is INERT by construction and says so.

> **D-clo-4 — CLOSED (2026-08-27).** `(L-Apply)` makes applying a closure a call, and
> [calls.md](calls.md) `(F-Return)` says `return e` exits the call with `e` — the same
> program as the tail spelling. A lambda whose body both `return`ed and discharged a null
> (`fn(n: integer) -> text { return s ?? "fallback"; }`) instead SIGSEGV'd the interpreter
> and failed to compile on `--native` (`E0499`), so the two spellings of one program
> disagreed and the rules settled which one was wrong. loft#1113.
>
> Cause: a text-returning lambda is handed **exactly one** hidden `RefVar(Text)` work
> buffer by the fn-ref call ABI — a call site holding a fn-typed slot cannot know which
> lambda is in it, so it injects one and the callee either uses it or has it popped
> (`State::fn_call_ref`). Two promotions can meet inside one body and neither consulted the
> other: `parse_return` promotes at the `return`, and the block tail promotes the `??` / `?`
> / `if` accumulator afterwards. The callee then carried TWO, the frame came up one DbRef
> span short, and it read its `__closure` slot from the wrong offset — loft#717's fault
> line, reached by a second route. Fixed where the buffers are minted (`text_return`, one
> `holds_text_work_buf` predicate now shared with the P227 placeholder it always had):
> the first promotion to ask takes the buffer, and a later text local stays a local,
> delivered by copy exactly as `SkipOwnedLocal` already prescribes.
>
> **The filed scope was a third of the defect.** It named three conditions — a closure, the
> `return` keyword, and a `??` yielding `text`. Only the closure is real: `?` reaches it
> (the other discharge rule), the null branch reaches it, and two plain text locals with no
> discharge anywhere reach it. What the shapes share is a SECOND buffer, not the spelling
> that asked for one. Guarded by
> `tests/scripts/1113-a-lambda-carries-one-text-work-buffer.loft` (falsified at `20e25e9a`:
> interpret exit 139 → 0, native exit 1 → 0).
>
> **Measured and rejected:** applying the one-buffer rule to NAMED functions too. It is the
> same ABI on paper — a named function whose signature matches a fn-ref's does reach the
> generated dispatch arm, which forwards one buffer twice and does not compile — but it
> moved five suite results, and `float=0.25` came back `0` through the sqlite bridges. A
> named function's ordinary call sites lower against a known signature and carry as many
> buffers as it declares. The named-function half is therefore still open, filed separately,
> and older than this fix.

> **D-clo-3 — CLOSED (2026-08-22).** `L-Escape` says a closure "may be stored in a
> variable or struct field", and said nothing about the slot being fresh — so **assigning**
> into a fn-typed struct field or vector element that already held one (`h.f = inc`,
> `v[0] = inc`) had to work, not merely fail better. It was refused on both backends, and
> refused by the wrong rule: the fn-ref read lowers to a `fn_ref_field_read` Block rather
> than the `Call`/`Var` place shapes the assignment dispatcher knows, so it was not
> recognised as writing ANYWHERE and fell through to *"Not implemented operation = for type
> function(…)"* — a message about the `=` operator, contradicted by the same field
> accepting the same value one line earlier.
>
> Fixed by peeling the read back to its place and handing each destination to the writer
> the LITERAL already uses — a struct field to `set_field`, a vector element to
> `fn_ref_slot_dnr` — so the two positions cannot come to different conclusions about what
> a fn-ref source may be. The P215/@P213 refusal for a non-inline source and the #247
> refusal for a capturing source in a collection are unchanged shipped decisions; what
> changed is that the assignment now REACHES them, which was loft#1072's "small half".
>
> Three things a slot that already holds a value needs that a fresh one does not, each
> measured failing on the way:
>
> * the closure half must be RELEASED. A non-capturing source left the previous closure
>   record in place, so the field read back as the new function paired with the old
>   closure, and `fn_call_ref` entered a callee that declares no closure with one pushed as
>   its hidden argument — a corrupt frame, not a stale value: the call returned misaligned
>   and the next read of an unrelated field faulted in `get_int`. A capturing source
>   orphaned the old record in the host's store instead — a leak that grows with the loop.
>   One `OpClearKeyed` against the `child_rec<…>` field closes both, through the same
>   `remove_claims` cascade that frees it when the host dies;
> * pass 1 must RECORD a capturing source, because the attribute's split layout comes from
>   `assigned_lambda_d_nr` being set in that pass and the read's byte offset is `u16::MAX`
>   there (the struct has no layout yet). The read site's own record of the attribute is
>   the only answer available on pass 1;
> * the host may be a `&` parameter — `RefVar(Reference)`, not `Reference`.
>
> Guarded by `tests/scripts/fn-ref-assigned-into-a-field.loft` (15 cells, both backends,
> value + a plain field read beside it + a 200-iteration loop for the leak), confirmed to
> fail on a pristine tree at 655ff4dd with 19 errors per backend. Fixes loft#1072.

> **D-clo-1 — CLOSED (2026-07-04).** The `|…|` short form now captures outer variables exactly
> like the `fn(){}` form — the two are pure syntactic sugar (L-Fn), the maker's intent.
> `parse_lambda_short` gained the closure-param setup block its sibling `parse_lambda` already
> had (add the `__closure` attribute + set `closure_param` so the body reads captures from the
> closure record), and builds its public `Function` type from the DECLARED params only (excluding
> the hidden `__closure`, so a `.map(f)` arity check still sees one param). Inert for a
> non-capturing lambda (no captures ⇒ no closure record ⇒ the block is a no-op). Guard
> `tests/scripts/85-short-lambda-capture.loft` (scalar + heap capture, both backends); 625 lib +
> native_scripts + interp suite green. (Residual, minor: a zero-arg `|| { … }` closure *assigned
> then called* has a separate parse edge — the `.map`/inline capturing forms all work.)

> **D-clo-2 — CLOSED (2026-07-04).** A stored short `|x|` lambda whose types could not be inferred
> (assigned without a type context, `g = |y| { y*2 }`) got a GARBAGE signature (a `text`/`void`
> default), and passing it to `.map` built a `vector<void>` result → a panic at `data.rs:4569`
> (`def(u32::MAX)`). The root cause was a crash where a **clean diagnostic** was already the intended
> outcome (the same lambda used standalone / called directly already errors "Cannot infer type for
> lambda parameter"). Fix: `parse_map` now guards a `void`/`Unknown` return (or `Unknown` param)
> from an un-inferrable fn-ref and emits the guiding "pass it inline / use `fn(x: T) -> R`"
> diagnostic instead of building the invalid result vector. The inline `.map(|y| …)` form (which
> has the element-type hint) and the long `fn(y: T) -> R` form are unaffected. Regression guard:
> `tests/leak.rs::dclo2_stored_short_lambda_map_no_crash` (parses without panicking, the guard
> diagnostic fires); 625 lib + interp + native_scripts green. (Making it *work* — inferring the
> stored lambda's types from the later `.map` source — is cross-statement inference, a separate
> enhancement; the crash → clean error is the fix.)

## Carried by closures.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [closures.md](closures.md) now states only what is open.

### D-clo-7 / D-clo-14 closure summaries, the cluster premise, D-clo-18/20's exit

**D-clo-7 CLOSED 2026-09-03.**  The last open half — a `??` whose borrow arm hands back a
capture the caller could not NAME — resolved by reading the SLOT off the callee's body: the
subject's `OpGetDbRef(__closure, off)` and the build's `OpSetDbRef(___clos_N, off, var)` share
an offset, so one offset over a variable assigned once is a witness as good as an argument's
(`use_analysis::capture_return_offsets`, `closure_capture_base`).  A collection `??` over a
capture turned out never to hand the capture back at all — its chosen arm is COPIED into the
caller's `__retbuf` — and is separated from a capture returned DIRECTLY by whether the return's
dep names `__closure` (`callref_capture_blocks`).

**D-clo-14 CLOSED 2026-09-03** (loft#1257, and its bound-spelling mint arm with loft#1320): a
closure's collection `??` return is freed by store IDENTITY against the `Join` base the temp's
own dep names — inline, bound, as an argument, in a branch arm, at every collection kind. The
record is in [closures-history.md](closures-history.md).

**The cluster's premise is now measured false, and D-clo-14 is what measured it.** Both rows were
recorded as *"the same missing mechanism — a per-execution ownership witness"* together with
[ownership.md](ownership.md)'s `D-own-16` ([QUALITY.md](../QUALITY.md)'s cluster register). All
three closed or narrowed WITHOUT one: a `Join`'s owner is decidable at run time by store
IDENTITY against the variable the dep already NAMES, which costs no witness slot, no IR temp and
no deps strip. The sharper question the cluster should have asked is whether a row has a NAMEABLE
base — and D-clo-7's remaining half is exactly the case where it does not.

**D-clo-18 is no longer here.** A `&` SCALAR parameter written from inside a closure is REFUSED,
and refusing is deliberate: `(L-CapScalar)` gives the closure a COPY of the caller's value, so
there is no shared record for the write to land in and no code change closes it. Per
[ROADMAP.md](ROADMAP.md), a row that turns out **spec-may-adjust** leaves `formal/` and becomes a
decided edge — it is [DESIGN_DECISIONS C115](../DESIGN_DECISIONS.md), together with `D-clo-20`,
its heap twin, which took the same refusal for the same reason one rule over. Counting a
permanent refusal as distance from the spec overstates the register by one.

### the status line formal/README.md's area table carried until 2026-09-04

**2 open** (D-clo-7, D-clo-14 — one `??`-default leak in two positions: the borrow arm's witness cannot be NAMED, so the mint arm's store leaks; they are the SAME missing per-execution ownership witness as ownership.md's D-own-16, and QUALITY.md carries them as one cluster. D-clo-18 and D-clo-20 left as REFUSALS, DESIGN_DECISIONS C115) — the `fn(){}` and `\|…\|` forms capture IDENTICALLY (pure sugar, D-clo-1); first-class (store/pass/return/escape); scalar-by-value / heap-shared capture; a stored un-inferrable short lambda in `map` is now a clean diagnostic, not a crash (D-clo-2). `L-Escape`'s STORAGE half is complete (D-clo-3, opened and closed 2026-08-22 by re-measuring the previous zero): a place that already holds a fn-ref — a local, a tuple member, a struct field, a vector element, a `&`-parameter's field — now takes a new one, releasing the closure record the old one owned, and a source the LITERAL refuses is refused identically

