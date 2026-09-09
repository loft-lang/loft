# formal/ownership-history.md — the deviation register for [ownership.md](ownership.md)

> **The rules are next door.**  [ownership.md](ownership.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0** — D-own-38 CLOSED 2026-09-06 (loft#1388: the release is by STORE IDENTITY now, `@FR-O-Witness`, with the hand-off at the closure build; below); D-own-39 OPENED AND CLOSED 2026-09-06 (loft#1398: the `is` payload binding recorded no borrow in its TYPE, so the empty-deps proxy read it as OWNED and `--native` copied where the interpreter aliased — #429's cure applied to one of its two sites, below); D-own-38 OPEN 2026-09-06 (loft#1388 residual: the release of a store a closure record adopted, and of one orphaned beside it, is decided per BINDING where the question is per STORE, below); D-own-37 OPENED AND CLOSED 2026-09-06 (loft#1389: the degenerate self-dep was stripped for one of the two RECORD kinds, so an annotated struct-enum local read as borrowed and never freed what a join displaced, below); D-own-36 OPENED AND CLOSED 2026-09-06 (the `@FR-O-Detach` walk: a collection literal's detach ran before its reads on every destination, and `--native` declined a value-`if`'s displaced free, below); D-own-35 OPENED AND CLOSED 2026-09-05 (loft#1370: the per-path fact had no home for a VECTOR local — every value-branch bind aliased the chosen arm — closed at the parser's selector, below); D-own-34 OPENED AND CLOSED 2026-09-05 (the owner witness did not survive the cache, and a nullable bind from a borrow-returning call aliased); D-own-33 OPENED AND CLOSED 2026-09-05 (the per-path fact was short of four homes, every one a nullable local not treated as the heap local it is: a literal buffer adopted inside a loop, the branch pre-init, the loop hoist, a keyed `match` bind, below; a fifth face is loft#1367, owned by @PLN153); D-own-32 OPENED AND CLOSED 2026-09-05 (the oracle called a minted variable Owned regardless of its other definitions, and its shadow re-derived the base translation, below); D-own-31 OPENED AND CLOSED 2026-09-05 (the never-free contract named one spelling of five and forbade a release the language ships, below); D-own-30 OPENED AND CLOSED 2026-09-05 (a nullable local holding a projection VIEW freed the store it displaced, below), after D-own-29 2026-09-04 (loft#1346, below) and D-own-28 the same day (loft#1335).  D-own-8 CLOSED 2026-09-03 (opened 2026-08-24, NARROWED 2026-08-25 to a
single cell, its Face B CLOSED the same day, that cell's one known SYMPTOM closed 2026-08-26
with the FACT still wrong, loft#1098, and the fact itself closed by giving every path of a
value branch its own binding — below).  D-own-26 CLOSED 2026-09-03: its gate existed all
along and was measuring nothing, and every proxy site now declares which of the four facts it
reads — the "eleven of seventeen" was a hand count.
D-own-16 CLOSED
2026-09-03, its BOUNDARY corrected and its wider half closed 2026-08-30, with three cures
measured and ruled out along the way, loft#1200 —
D-own-23 opened and closed 2026-08-29 with loft#1154; D-own-24 the same day with loft#1156, and D-own-21 with
loft#1150 — the three-faced one, whose entry records that a DEFERRAL is a missing
measurement rather than a closed question; D-own-22 opened and closed 2026-08-29 with
loft#1142; D-own-20 opened and closed 2026-08-29 with loft#1143;
D-own-19 was opened 2026-08-28, narrowed the same day to its path-sensitive half (loft#1126)
and CLOSED the same day with loft#1128; D-own-17 and D-own-18 both opened and closed
2026-08-28;
D-own-15 opened and
closed 2026-08-27 with loft#1119; D-own-14 opened and
closed 2026-08-27 with loft#1118; D-own-13's second face
closed 2026-08-27 with loft#1107 and its first face the day before; D-own-12 records the two
witness spellings closed there and points at D-own-11 for the other two; D-own-9, D-own-10 and
D-own-11 opened and closed 2026-08-26, D-own-7
opened and closed 2026-08-23, and D-own-6 before it; D-own-25 opened and closed
2026-08-30 with loft#1201, and D-own-16 was NARROWED the same day by loft#1200 to the one
condition its two surviving shapes share — the assigned value READS the local it assigns; the
five original D-own deviations remain resolved.  Read those entries for what their oracles vary before treating any zero
here as a measurement: each rested on a Join corpus that pinned one axis, and moving that
axis found a fresh family every time — which is exactly how D-own-8 arrived, from a consumer
rather than from an oracle at all, and how its second face was found by varying the POSITION
of the same join.  Face B is also this register's clearest case of a leak MASKING a wrong
answer: the interpreter retained what `--native` recycled, so the defect was filed at its
mildest symptom and the `silent-wrong` half only appeared once the retention was removed.

### D-own-39 — OPENED AND CLOSED (2026-09-06, loft#1398): #429's borrow dep reached the `match` binding and not the `is` one

`(O-Proxy)` reads an empty dep list as OWNED, and that proxy is only ever as good as the deps a
binding actually carries — which is why #429 gave a HEAP struct-enum payload binding a frame dep
on its subject, closing an interp-vs-native divergence with it.  It was applied to
`parse_match_enum_field_bindings` alone.  The `is` spelling is the same bind at the sibling
site, and carried `set_skip_free` without the dep:

```loft
w = W{st: Holder{inner: Pay{a: 1}}, t: "w"};
if w.st is Holder { inner } { w.st = Empty{z: 0}; g = inner.a; }
// --interpret 0, --native 1 (+ kt=82 Pay×1 not freed)
```

With no dep the type read OWNED, so `--native` deep-COPIED the payload where the interpreter
aliased it — and then leaked the copy.  `(O-NoDiverge)` forbids the split either way, and
`(B-Disturb)` says which number is right: overwriting a place is not disturbing it, so the
binding still names the slot and 0 is the answer.  The `match` twin gave 0 on both backends
throughout, which is what located the gap at the SITE rather than at the rule.

⚠ **These two sites have now diverged TWICE on the same axis**, which is the thing to carry
forward rather than either fix.  The `@FR-O-Owner` walk found the `is` capture carrying no dep
where the `match` twin does, from the MATERIALISE side — its gate had to admit an empty list to
cover it (loft#1394's D-bind-19).  This entry is the same asymmetry from the OWNERSHIP side: an
empty list is `(O-Proxy)`'s reading of OWNED, so the same missing dep that made a view
invisible to one walk made it a copy to the other backend.  A third reader of these two sites
should assume the asymmetry rather than discover it.

**Closed by giving the `is` site the same dep**, under the same shape test #429 states: a
`Reference` / `Vector` / `Enum` payload takes a frame dep on `match_borrow_source`'s answer, a
SCALAR takes none (it carries no `DbRef`), and a TEXT payload is untouched — it is an owned copy
with its own write-back route (`record_text_payload_view`).

The two sites are not folded into one home here, and that is the honest state rather than a
claim: the `match` path binds through the subject expression and the `is` path through a
stabilised `_is_subj_N` local, and the surrounding code differs enough that a shared helper
would be a third thing to keep in step.  What is shared is the PREDICATE — both call
`Parser::match_borrow_source` and both test the same three type kinds — so the next divergence
of this class is a grep for that call.

Guard `an-is-payload-binding-borrows-its-subject-like-its-match-twin` (8 cells: the overwrite
shape, the `match` twin beside it, a plain read, a write THROUGH the binding, and four controls
— vector and text payloads, a LOCAL subject whose reassignment `(B-View)` materialises, and two
arms over one subject).  Falsified at cb2dca92: `--native` 1 where `--interpret` reads 0, with
`kt=82 Pay×1` not freed.

### D-own-38 — OPENED AND CLOSED (2026-09-06, loft#1388): every release a captured local owes is by STORE IDENTITY

`(O-Latest)` says ownership belongs to the LATEST assignment *at that point*.  A closure record
adopts the store its capture named AT THE BUILD, and the frame's scope-exit free is suppressed
so the record's cascade is the sole owner (#323).  loft#1324 established that the suppression
must name the same STORE the adoption does, and closed it for the COLLECTION half — a capture
that names a VIEW, whose backing local `capture_build_backings` finds.  The DIRECT half asked
`is_captured(v)`, a fact about the BINDING, and kept suppressing the free of whatever the local
named LAST:

```loft
s = S{a: 1, b: "x"};
h: fn(integer) -> integer = |i| { s.a + i };
s = build(h);                    // the store `s` ends up holding is freed by nobody
// Warning: 1 stores not freed at program exit: kt=81 S×1
```

**Closed for that half** by giving `capture_build_backings` the second fact read off the same
walk — which captured locals are ASSIGNED AGAIN after their build — and having
`capture_adoption_owns_free` (the one home all three consumers read: `get_free_vars`,
`check_ref_leaks`, `ownership_cfg`'s oracle) decline the suppression for them.  The two build
spellings differ in ORDER and both are handled: a STORED closure is built by an earlier
statement, while an INLINE argument is built inside the very right-hand side that reassigns
the local (`s = build(|i| { s.a + i })` hands the record the store `s` is about to stop
naming).  `Value::walk` is pre-order, so the inline build is reached after its own assignment;
`captures_built_in` reads it off the right-hand side against the map as it stood BEFORE the
assignment, and the walk skips it when it arrives.  Measured over 9 cells on both backends:
7 clean where 6 leaked, values unchanged in every one.

**What is still per binding, and why it is one deviation and not two.**  Both residual cells
are the same question asked at a different moment — *does anything still hold this store?* —
and both are answered today by a static fact about the BINDING:

* **A stored closure and TWO reassignments** (`s = build(h); s = build(h)`) leaks the
  INTERMEDIATE store.  The record holds the first store forever, so the first reassignment must
  not free what it displaces — but the second displaces a store no record ever adopted, and
  `owns_displaced_store`'s `!is_captured(v)` vetoes both alike.  The two sites are
  indistinguishable statically; what tells them apart is whether the displaced store IS the one
  the record holds, which the record can be asked at run time
  (`OpFreeRefIfDistinct(<displaced>, OpGetDbRef(___clos_N, <offset>))` is the existing op pair).
  Dropping the veto instead is measured WRONG: the free then runs before the right-hand side is
  evaluated and the closure reads a released store (`a=1` where `a=2` is right).
* **A closure record REUSED across loop passes.**  `OpDatabase` on an already-allocated record
  variable records into the same store, so each pass RE-ADOPTS a new capture and only the last
  adoption is ever released — the previous pass's store is orphaned the moment the capture slot
  is overwritten.  A 5-pass loop leaks 4.  The release belongs at the hand-off: a capture slot
  overwritten with a different `DbRef` gives up what it held.

  **The obvious cure was BUILT and measured WRONG, which is why it is not here.**  Freeing the
  record before the rebuild (`OpFreeRef(w)` ahead of the build's `OpDatabase`) clears the leak
  in every loop cell, and the escape argument holds: a capturing closure may not be stored in a
  collection, a fn-ref struct field takes one capture shape, and a factory's record escapes by
  RETURNING, which leaves the loop.  The struct 5-pass loop went clean with its values
  unchanged.  The VECTOR loop went clean and its values did NOT hold: `v = build_v(|i| { (v[0]
  ?? 0) + i })` over three passes answered `4,5` on the interpreter and `1,2` on `--native`,
  where `4,5` is right and both backends agreed on it before.

  What it collides with is the fix ABOVE.  `free_named`'s cascade releases an ADOPTED capture,
  and the frame now frees the same local's store for a local reassigned after its build — so
  the two free one store between them.  A leak traded for a backend split and a wrong answer is
  the trade this project refuses, and it is what `get_free_vars`' own note says out loud:
  *suppress without adopting and the store is never freed at all, adopt without suppressing and
  it is freed twice.*  The release is admissible only together with `record_adopts_capture`
  reading the same verdict `capture_adoption_owns_free` now reads — one decision, as that note
  requires.

Neither is reachable from a per-binding fact, and that is `(O-Latest)` saying so rather than an
observation about this code: the fact belongs to the LATEST assignment at that point, and a
type-level `deps` list can express neither which assignment nor the loop depth.  So the veto is
wrong in KIND, not merely imprecise, and its two sites are statically indistinguishable by
construction.

**CLOSED the same day, by the mechanism the rules already licensed.**  `(O-Witness)` is release
by STORE IDENTITY, and a captured local reassigned after its build has exactly the mixed
ownership it models — the record owns the store the capture named AT THE BUILD, the frame owns
every store after it.  Three parts, and each answers one of the failures above:

* **The witness**, extended to such a local.  `owner_witness_locals` admits it off
  `CaptureBuilds::reassigned_after_build`, in both spellings of "the record took this local's
  store": a struct capture names the local outright, a collection capture names a view whose
  backing local is this one.
* **The HAND-OFF at the closure build, emitted FIRST.**  The record becomes the owner there, so
  the witness gives that store up — a clear, never a free.  Ahead of the guarded release,
  because for an INLINE closure argument the capture and the target are the same local: the
  record adopts the old store and the local moves on in one statement, so a release asked first
  frees what the record has just taken.  That is `(O-Detach)` in ownership clothing, and it is
  exactly what made dropping the veto answer `a=1` where `a=2` is right.
* **The record's own release**, beside it and GATED on every capture in the build having a
  witness.  The gate is the soundness argument: a capture without one is still owned by the
  frame, so the cascade and the frame would free one store between them.  `(O-Derived)` — one
  decision, one home.

Eleven cells, both backends, all clean with their values unchanged — every row of the issue's
matrix including the two that stayed open at 3360fb93 (a stored closure called twice, and
either spelling in a loop), plus a three-call cell, a vector capture in a loop, and the
never-reassigned control the whole thing must not disturb.

**The COLLECTION half needed the same sentence, and finding it was the last step.**  A closure
capturing a VECTOR inside a loop kept one store, and `2026.9.0` did not — so this was a
REGRESSION from the more accurate backing map above, not a residual: resolving a build against
the map as it stood BEFORE the assignment gave `backing[v] = __vdb_1` where the pre-order walk
had previously given nothing, and `backs_an_adopted_capture` then suppressed `__vdb_1`'s free
on the strength of the FIRST pass's adoption while the record's slot had been rewritten twice
since.  `(O-Latest)` says exactly that: the record holds what the capture named at the build,
and a build that RE-RUNS holds only its last.  So the backing's suppression is declined where
the build sits inside a LOOP.

**And the narrowing is loft#1324's guard, not a precaution.**  The first cut declined on
"the capture was reassigned after its build", which is also true of #323's escaping factory —
a build that runs ONCE, whose record outlives the frame and must keep its suppression however
often the capture is reassigned afterwards.  Declining there frees the store the escaped
closure still reads:
`1324-a-reassigned-capture-suppresses-the-store-the-record-holds` failed with `null(oob)` on
the first run of the `scopes` subject.  A build that re-runs and a build that escapes are
opposite cases, and only the loop test separates them.

The instrument that caught it was the shipped binary: `2026.9.0` frees `#2 #3 #4 #5` here and
the branch freed `#2 #3 #5`, which named the missing free before any reasoning about why.

**A vector-typed WITNESS was built and measured wrong on the way**, and is recorded because it
is the obvious next idea: admitting a vector local to `owner_witness_locals` with a
`Vector`-typed `__own_` handle removes the leak and makes `--native` answer `1,2` where `4,5`
is right, because the witness then releases a store the record still holds.  The witness stays
record-typed; the collection half is answered by the suppression clause instead.

Guard `a-captured-local-reassigned-after-the-build-frees-its-own-store` (7 cells: both build
spellings, the struct and vector captures, and the two controls that must NOT move — a capture
never reassigned, whose store is the record's, and a literal right-hand side, which rebuilds in
place and mints nothing).  Falsified at ac412a96 — `kt=81 S×3, kt=25 main_vector<integer>×1` ->
clean on both backends.

### D-own-37 — OPENED AND CLOSED (2026-09-06, loft#1389): the degenerate self-dep was stripped for one RECORD kind of two

`(O-Proxy)` reads a binding's dep list to answer *does this local own the store a reassignment
displaces?*, and #328 established that a dep on the binding ITSELF is not a borrow at all — it is
a degenerate edge that flips the variable into the dependent-view codegen class.  `Parser::change_var`
strips it, and it read `Type::Reference` alone:

```loft
e: Sh = Circle{r: 1};                              // deps=[e] — a dep on itself
e = if e is Circle { circle(2) } else { e };       // the minting arm runs
// Warning: 1 stores not freed at program exit: kt=82 Circle×1
```

A struct-enum is a record reached through a `DbRef` exactly as a struct is — `data::is_dbref`
names `Reference` and `Enum(_, true, _)` together — so the annotated literal bind kept `[e]`,
`owns_displaced_store` read the non-empty list as BORROWED, the join's displaced free was never
emitted, and one store leaked per execution on BOTH backends, growing 1:1 with a loop.  Values
were right throughout; the leak announced itself only at exit.  The un-annotated form, the same
local bound from a CALL, and the plain struct twin were all clean, which is what located the
gap at the type kind rather than at the join.

**Closed by stating the strip over both record kinds** — `Reference | Enum(_, true, _)` — and by
reading the deps through the generic accessors, so the stripped list keeps the dep SPACE it
arrived in rather than being rebuilt as `Frame`.

The COLLECTION kinds stay outside it, and that boundary is measured rather than assumed.  For a
`vector` or a `text` the self-dep is the @P302 re-init-in-place ownership marker that `(g)` in
[ownership.md](ownership.md) reads as `Owned` (`t = "{t}x"`), not a degenerate borrow.  An
env-gated probe over `tests/scripts`, `tests/docs` and `default/` counted **6418** self-deps
reaching this site on a collection kind — 3760 `Vector`, 2658 `Text`, 42 keyed/optional — and
**zero** on `Enum`.  So widening past the two record kinds would re-answer a question that is
already right, at the cost of every one of those readings.

Guard `an-annotated-struct-enum-local-owns-what-it-mints` (7 cells: the `if` join, the `match`
join, a 4-pass loop, the hand-back arm, the annotated-from-a-call, un-annotated and plain-struct
controls), falsified at 32e36462 — `kt=82 Circle×6` -> clean on both backends.  It carries a
`main` that calls every cell because `--tests` does not leak-check, so a `main`-less leak guard
reads INERT on the build it was written to catch (`scripts/falsify.sh` § ONE CHANNEL IS BLIND).

### D-own-36 — OPENED AND CLOSED (2026-09-06, the `@FR-O-Detach` walk): a collection literal's detach ran before its reads, and `--native` declined a value-`if`'s displaced free

`(O-Detach)` sequences a binding's detach AFTER every read of it by the value being assigned.
Walked as a rule (QUALITY.md B8a): its eight sites ask one static question — *does the value
read the binding?* — with one home, `Value::reads_var`, and answer it by one of three
placements (hoist the reads into temporaries; defer the free past the assignment; release by
store identity after the `Set`).  A 37-cell matrix over 14 binding kinds × 20 right-hand-side
shapes found the eight in agreement and two shapes that never reached one:

* **The vector literal.**  `v = [v[1]?, v[0]?]` answered `[0, 0]`; `len(v)` inside the literal
  read `0` then `1`; a struct element read its `?? default`; a parameter, a typed local, a
  struct field and a `+=` all read the result being built — sixteen spellings, both backends,
  silent.  The build's detach (`create_vector`'s `=` repoint, `clear_vector_field`) was emitted
  at the head of the build's ops, before the element expressions that read the destination.
  The comprehension had been held to the rule three times (`I-Comp`, D-iter-1..3) with a
  snapshot of its own; the literal is the same build without the loop.  Closed by
  `Parser::snapshot_read_destination` — one home, which the comprehension's deferred route now
  calls too: copy the destination before the first write, rename every read in the parts to
  the copy, and let the two detach sites insert after it (`Parser::build_snapshot_len`).  Guard
  `a-vector-literal-reads-what-its-destination-held`, falsified at 6f9c0886 (14 assertions → 0
  on the interpreter, the native run's first → 0).  Residual: a field reached through an
  element and a captured collection are destinations the snapshot cannot name (loft#1391).
* **A value-`if` on `--native`** (`(O-NoDiverge)`).  `s = if s.a > 5 { mk(7) } else { s }` — the
  interpreter stashes and post-frees through `rhs_reads_v`; native's `owned_ref_reassign`
  listed the right-hand sides that produce a store and `Value::If` was not among them, so the
  then arm's displaced store leaked on that backend alone, one per execution.  Declining the
  detach is the rule's own forbidden third option.  Added to the list; the identity guard
  already makes the else arm — the same store — a no-op.  And the `match` spelling of the same
  shape did not compile natively at all: `output_if_inner` peeled a `Span` to decide not to
  open a brace for a block arm and asked the bare value when closing one.  Guard
  `a-join-reassignment-whose-other-arm-is-the-binding-frees-and-compiles`, falsified at
  6f9c0886 (a rustc refusal → runs; the leak by hand under `LOFT_NATIVE_LEAK_CHECK=1`, 1 → 0).

Filed, not folded in: loft#1388 — a captured struct local's reassignment from a call retains
the displaced store (`owns_displaced_store`'s `!is_captured` veto is the rule's *declined
detach*: a per-binding answer to a per-store question); loft#1389 — an annotated struct-enum
local bound from a variant literal carries a dep on itself and never frees what a join
displaces; loft#1390 — a variant literal does not join with a binding of its enum type.

### D-own-35 — OPENED AND CLOSED (2026-09-05, loft#1370): the per-path fact had no home for a VECTOR local, and the parameter rebind read the proxy's carve-out as ownership

`(O-Complete)` asks that every path of a bound value branch be its own binding, and `(B-Copy)`
that a plain whole-value bind copy.  B7v gave the RECORD spelling its home (D-own-34, the
statement-form sink in `scopes.rs`); the vector spelling had none, because the vector copy is
the PARSER's (`classify_vec_bind` and its copy arm), and a value-branch RHS never reached that
selector.  Measured (QUALITY.md B7w, 33 cells, both backends): every value-branch bind of a
vector local handed it the chosen arm's STORE — `if`, `else if`, `match`, `??`; dense, nullable
and null-initialised; every element kind; projection, mixed and parameter-source arms; inside a
loop; and the first bind through a `match` or `??` wrapper.  Beside it, `x = s.v ?? va` viewed
an owned projection that `x = s.v` copies.  The keyed twin copies (`OpReplaceKeyed`) and is the
control.

**Closed** at the selector: `Parser::sink_vec_bind_into_arms` writes the bind out per arm and
classifies each tail by the same selector, a copy inside an arm always mints (the join's deps
make the ownership proxy unreadable there), a `??` hoist is judged by its source, a
buffer-yielding block is bound whole, a wrapper-block first bind is declared at the statement,
a promoted return buffer keeps the value form (F-Ret's adopt-or-materialise), and a returned
local the rewrite sank keeps its own store at the return (`Bind`, carried by
`branch_sunk_vectors` since the `Set(v, If)` the ladder read it from is gone).  Guard
`a-vector-local-bound-from-a-value-branch-copies-the-chosen-arm.loft`, falsified at faa38979.
The parameter half is calls-history D-call-14.  Corpus census: 20 of 1260 files moved, all
green both backends under strict stores, poison and the native leak check.

### D-own-34 — OPENED AND CLOSED (2026-09-05): the per-path fact was short of three more homes, and the fact the emitters read did not survive the cache

The `@FR-O-Witness` walk (QUALITY.md B7v) built the caller-side and cache matrices B7u's had
not, and each red was a nullable local not treated as the heap local it is — none the mixed-path
join the matrix was drawn for:

- **The owner witness did not survive the startup cache.**  `owner_witness` was maintained in
  the IR and restored by no snapshot field, so a WARM program-cache run served the pre-witness
  copy arm: `s: S? = a; s = a.next; s = a` wrote the second copy INTO the record `s` was
  viewing, `b == 7` on both backends where a cold run read `b == 2`.  A fact the emitters read
  must survive the snapshot exactly as `skip_free` does — `__own_<name>` is now the tenth stored
  `Variable` field (`VAR_OWNER_WITNESS`), through the JSON codec, the store codec and the schema
  source, and `CACHE_FORMAT_VERSION` is bumped to 5 so a stale bundle is not read.  Guard:
  `tests/arc_e_program_cache.rs::a_warm_run_keeps_the_owner_witness`, cold vs warm, both backends.

- **A nullable local bound from a call that answers a BORROW of its argument aliased it.**
  `x: S? = keep(a); x.value = 9` wrote through to `a` on both backends while the dense twin
  copied; a witnessed local first-bound that way pointed its owner witness at the CALLER's store
  and released it at the first view.  The heap first-bind dispatch asks its shape against the
  bare type, and its one nullable arm (loft#1106's join guard) admitted only a `Join` verdict —
  a callee that ALWAYS hands its argument back is a pure `Borrowed`, so it stayed a plain alias.
  `nullable_join_first_bind` now admits a `Borrowed` whose witness is the ONE argument the return
  deps name (a two-source return keeps the plain adopt — loft#1368 — because one witness would
  adopt the other), and the reassignment strip and the var-copy strip peel `base()` so the copy
  is emitted for the nullable spelling too.  Guard:
  `a-nullable-local-bound-from-a-borrow-returning-call-copies-it.loft` (5 fns, both backends).

- **A `-> S?` callee freed a PARAMETER on its null path.**  A record source of a return with a
  reachable null arm is paired with the hoisted return value (`OpFreeRefIfDistinct(src,
  __ret_N)`), which is right for a LOCAL but freed the CALLER's argument on every `null` answer,
  both backends.  A parameter is no longer a null-arm source (its store is the caller's,
  F-ParamHeap; a REBOUND parameter keeps its own release by identity against its entry stash).
  Guard: `a-null-answer-does-not-free-the-argument-the-other-arm-hands-up.loft` (3 fns).

- **A heap-record local reassigned from a value branch handed up the chosen arm's STORE.**
  `x = if c { a } else { b }` on an owned `x` aliased `a` and then freed it as its own at scope
  exit; a witnessed local the same.  The FIRST bind already lifts each arm into a temp the
  binding borrows, but a binding assigned elsewhere cannot borrow those (`@FR-O-Latest`), so the
  REASSIGNMENT is now lowered to the statement form `if c { x = a } else { x = b }` — each arm's
  `Set` gets the copy a single bind of its tail would.  Records only; the vector/keyed twin is
  loft#1370.  Guard: `a-record-reassigned-from-a-value-branch-copies-the-chosen-arm.loft` (6 fns).

Every guard `make falsify`'d at e575a33f; the corpus IR census moved 7 of 1241 files, every one
green on both backends under strict stores and poison.  A trap the walk caught in its own work:
peeling the var-copy strip through `base()` widened it onto a CAPTURED nullable local, whose
closure holds the store — freeing it read `null` through the capture; the strip now excludes a
captured or never-free local (`@FR-L-CapHeap`), and `1181`/`1202` are the controls.  Held FIXED
and filed apart: the two-source nullable return (loft#1368) and the vector value-branch reassign
(loft#1370).

### D-own-33 — OPENED AND CLOSED (2026-09-05): the per-path fact was short of four homes, every one a nullable local not treated as the heap local it is

`(O-Complete)` requires the fact PER BINDING and PER PATH — every binding, every arm.  Measured on
the `@FR-O-Complete` rule-led walk (QUALITY.md B7u) with the matrix the rule states and its
guards had not crossed — the STATEMENT form, a local assigned on two paths with different
ownership, every cell called twice — the record, vector and keyed columns held (81 of 81) and
the nullable column did not, in four ways that are not the mixed-path join at all:

1. **A binding that adopts a literal's work-ref inside a loop body had two owners.**  `y: S? =
   S { n: 3 }` (and a struct-enum literal, and a literal in an `if`/`match` arm) builds in a
   function-scoped `__ref_p2_N` the binding aliases.  loft#1317 paired the buffer's forced
   exit free with the local and declined the pairing where the local is inner-scoped — a loop
   body.  There the binding's per-iteration free returned the store, the buffer kept the
   number, and the next iteration's `OpDatabase` reused it in place after another record had
   taken it: the second iteration's literal was written over that record, both backends,
   nothing reported.  Closed by giving the literal buffer the pairing @P378(a) already gives a
   CALL buffer (`witness_buffer`): the adopter's free declines while they alias and the buffer
   keeps, reuses in place and frees once at exit — carried for every arm's buffer.  A MOVE at
   the adopt was tried first and reverted: it was right for the loop and contradicted every
   site that reads the buffer as the owner (the owner witness, loft#1200's flag, the `??`
   lift), four leaks from one reset.
2. **A nullable local first assigned inside a branch held no null on the other path.**
   `scopes::needs_pre_init` listed the bare heap spellings and did not peel `Optional`, so the
   second arm's `Set` was a reassignment whose guarded displacement free read an
   uninitialised frame word — a refused free, or the free of a live store the previous frame
   left there.  Closed by the peel.
3. **A nullable local first assigned inside a loop body stayed body-scoped** — the hoist reads
   the same predicate — and the read after the loop was a use-after-free (interpreter) or an
   unresolved `var_x` (rustc).  The same peel; a nullable VECTOR's null-init then needed the
   sentinel rather than the dense arm's store or placeholder (`gen_set_first_nullable_collection_null`).
   A keyed nullable local reads present-and-empty on the untaken path on both backends — its
   assignment copies INTO its own store, so the init allocates — and what absence means for it
   is @PLN153's question, recorded and not frozen.
4. **A keyed local bound through a `match` never freed the taken arm's store**: `join_arms`
   (loft#1154's per-arm licence for the free-source bit) took the `scalar_match` block as one
   arm.  It now reaches a value block's tail, where every `match` keeps its chain.

A fifth face is OPEN elsewhere: two spellings of `S?` meeting in one local (loft#1367, the
tagged field projection and the pointer), owned by @PLN153 phase 3 through `(L-Null-Which)`.

Guards: `a-binding-that-adopts-a-literal-buffer-inside-a-loop-frees-it-once` (falsified at
64437246 under `LOFT_POISON=1` — plain mode hands the stale buffer its own number back; the
poison sweep is the CI leg), `a-nullable-local-first-assigned-inside-a-branch-or-loop-holds-null-on-the-other-path`,
`a-keyed-local-bound-through-a-match-frees-the-arm-that-ran`.  Corpus IR moved in 20 of 1241
files, all green on both backends under strict stores.

### D-own-32 — OPENED AND CLOSED (2026-09-05): the oracle called a minted variable Owned regardless of its other definitions, and its shadow re-derived the base translation

`(O-Oracle)` says the answer is a function of the VALUE, computed by one derivation, and that a
translation which cannot name a base must not upgrade the verdict.  Two things fell short of it,
found because the @PLN94 shadow derivation (Check A) disagreed with the oracle in 14 places over
the 1247-file corpus (QUALITY.md B7r).  First, `classify` read *"a var `OpDatabase` minted a
fresh store into is Owned regardless of any other def"* — right for the retbuf a
`materialized_view_return` fills, and an UPGRADE for a local minted once and then rebound by a
call that may hand back its own argument (`c = M {…}; c = cond(c, 3)`, 1017b) or by a capture
read (`__kvb_1` inside a closure, 1326/1331): `Owned`, the verdict that licenses a free, where
the value is a `Join` or a view.  Masked at run time by the distinctness guard and by
loft#1331's detach, which is the caveat's exact shape.  Second, the shadow's private copy of the
callee-to-caller base translation, written to *"mirror"* the oracle's, carried none of
loft#1318's three fixes, so a call delivering through a hidden buffer read `Join(MAX)` there
and `Borrowed(buffer)` in the oracle (37-stress).

Closed on both sides.  The mint arm now JOINS the mint with the variable's other definitions
(a bare-`Var` right-hand side is a copy per `(B-Copy)` and so Owned; a call or projection is
what the oracle says of it; a minted variable with no `Set` stays Owned), and the translation
has ONE home, `use_analysis::structural_arg_base`, read by the oracle and by the shadow — the
shadow's independence is in the flow, not in the translation.  Check A reads 0 over the corpus;
the emitted IR, bytecode and Rust are byte-identical across all 1247 files, so the change is in
the facts and not in the programs.  The A1b gate, whose asserted disagreement was these two
defects meeting on one fixture, now asserts the runtime failure of the wrong plan and Check A
clean on both plans, with an injected true positive (`LOFT_OWN_INJECT_FACT_OWNED`).

### D-own-31 — OPENED AND CLOSED (2026-09-05): the never-free contract named one spelling of five, and forbade a release the language ships

`(O-Override)` read *"no `OpFreeRef` is ever emitted for this binding — exactly that sentence
and nothing weaker."*  Measured on the `@FR-O-Override` rule-led walk (QUALITY.md B7q) over
the 1247-file corpus, the sentence was wrong in both directions.  It named ONE of a free's
five spellings, and the two backends intercept the flag downstream for two of them only
(`OpFreeRef`, `OpFreeRefTag`; a bare variable operand), so a never-free binding freed by
`OpFreeText`, `OpFreeRefIfDistinct`, `OpFreeRefOrHandUp` or through a tuple element would
have honoured the letter of the rule while releasing the store — and the question "which ops
free their first argument?" was a hand-spelled list in nine places, no two agreeing.  And it
FORBADE a release the language ships and tests: a `??` text subject (`__ncc_N`) and a text
return stage (`__ret_N`) are marked never-free so the scope-exit sweep does not free the value
their block yields, and the pass that staged them frees them after the statement that copied
the value out — 217 function–binding pairs in the corpus, every live-spelling free of a never-free binding
there was, and not one anywhere else.

Closed by extending the RULE, per the doctrine's other half: an edge the rules cannot express
means the rule wants extending.  `(O-Override)` now forbids every free DERIVED FROM OWNERSHIP
in any spelling — `use_analysis::OpSets::frees` is the one home of the list, and all nine
matchers read it — and names the one admissible free: the release the marking pass places on
a fact of its own, `Function::is_staged_text_temp`.  `ownership_cfg`'s Check D
(`LOFT_OWN_ORACLE=check`) is the gate — a free of any other never-free binding by any live
spelling is a RED, and `LOFT_OWN_INJECT_FREE_SKIPFREE` proves it fires.  Found and closed
alongside: a local that took BOTH the loft#1200 displacement flag and the loft#1336 owner
witness, the witness's never-free mark dropping the flag's free at codegen on both backends —
right by accident, and 172 lines of dead IR in the 1200 guard; the witness now runs first and
the flag's own never-free exclusion keeps a witnessed local out.

### D-own-30 — OPENED AND CLOSED (2026-09-05): a nullable local holding a projection VIEW freed the store it displaced

`(O-Owner)` says one thing owns each store and only it frees it; `(O-Latest)` places the free
from the LATEST assignment.  The D-own-16 residual `borrows_one_argument` reads a nullable
local's single-ARGUMENT dep as ownership and frees the store it displaces at a reassignment —
correct for the WHOLE-value argument borrow it was written for (`d: S? = p`, whose store is
free-protected on the borrow path), wrong for a PROJECTION.  `d: In? = q.inner` aliases q's
NESTED store, which carries no free-protection, so the reassignment released the CALLER's
record; a view of a LOCAL's field or a vector element failed the same way, the dep still
naming its base.  A SILENT-WRONG: the freed store read correct until a later allocation reused
its slot, then returned the filler's value (`777` for `71`, both backends); the vector-element
shape crashed out of bounds under `LOFT_POISON`.

Found on the `@FR-O-Latest` rule-led walk (QUALITY.md B7p), 1266 cells KIND × first source ×
second source × position, scored under poison because the plain build hid it.  Closed at the
FACT: a view owns no store, so the empty/argument-dep proxy is wrong for a view-holder and
`(O-Override)` vetoes it.  `scopes::nullable_view_locals` marks such locals never-free before
the scan; all three free-site twins (`state/codegen.rs`, this file's scope-exit, `generation/
dispatch.rs`) already consult `is_skip_free`.  The two mixed-ownership shapes that DO own a
store are excluded and keep their machinery: a solely-owned minting call the loft#1200 runtime
flag, a view+mint mix the owner witness (loft#1336).  Guard
`a-nullable-view-local-does-not-free-what-it-displaces.loft`, falsified at `51646648` on both
backends via the value channel.

### D-own-29 — OPENED AND CLOSED (2026-09-04, loft#1346): the interpreter kept a nullable record's borrowed-view result raw where native copied it

`(O-NoDiverge)` says both backends read one fact and cannot disagree; `(B-Copy)` says a plain
bind is independent of its source.  A nullable record answered by a call that views its
argument (`fn nr(q: Bag) -> P? { if … { q.rec } else { null } }`) took the bind-or-copy on its
FIRST bind, and on a REASSIGNMENT — a nullable local assigned again, or the `__lift_N` temp an
`if` join binds its arm into, which is declared nullable and assigned inside the arm — reached
the interpreter's set lowering, whose borrowed-view copy is skipped when *the call reads the
destination* (the self-passing shape `g = idp(b, g)`, whose result may BE `g`'s own store).
That question was asked through `stash_old_for_post_free`, the flag that routes the post-free
through the guarded form — and that flag is also raised for a NULLABLE local and for a fresh
record, neither of which means the call reads the destination.  So every nullable record
local reassigned from a borrowed-view call kept the raw pointer: `j = if c { nr(b) } else
{ d }; b.rec.x = 99` read 99 through `j` on the interpreter, and `--native`, whose arm has no
such exception, copied — the same IR, two answers.

**Closed by asking the question itself** — `rhs_reads_v` — and leaving the post-free routing
to its own flag.  The guard then found a second defect beneath the copy: `OpCopyRefOrNull`,
which binds a callee's null result, wrote `Stores::null()` into the slot — a constructor that
ALLOCATES an owned empty store, which is what a variable's default-init wants and what an
absence is not — so the null arm read as PRESENT to `== null` (`OpRefIsNull` tests the
sentinel's store number) while rendering as null, and one store leaked per null result.  It
writes `DbRef::NULL` now, the sentinel every other absence site uses; the census of the
constructor's other readers found only default-inits.

Guard `tests/scripts/1346-a-nullable-record-from-a-call-copies-when-reassigned-or-joined.loft`
(the `if` join, a reassigned nullable local, the null arm; the first-bind, self-passing and
fn-ref-plain controls), falsified at `dd46146c` on `--interpret`, native inert by construction.
Held fixed and filed apart: the FN-REF callee's join (loft#1353) — `use_analysis::callee_of`
declines the nullable fn-ref spelling by design until its `CallRef` twin lands, a measured
use-after-free otherwise.

### D-own-28 — OPENED AND CLOSED (2026-09-04, loft#1335): a fn-ref return of a keyed, nullable or tuple shape kept the callee's attribute indices

`(O-Deps)` says every store-lifetime decision reads the one carried `deps` fact, and
[DEPS_INVENTORY.md](../DEPS_INVENTORY.md) says a callee's ATTRIBUTE indices are bridged to
caller FRAME variables at the call site — that bridge is what makes the fact readable by the
frame that holds it.  The named-call path bridges every dep-carrying shape.  The fn-ref path
(`fnref_result_type`) listed four — text, vector, record, record enum — and returned every
other shape UNTOUCHED:

```loft
h = fn(q: Bag1245) -> hash<K1245[k]> { q.m };
r = if s > 0 { h(bag) } else { d };     // `h(bag)` arrives typed  hash<K[k]>["q"]  — attr 0
```

A keyed collection, any return under `?`, a tuple of them — each reached the caller with
the callee's attribute index in place, and the `if` join then unioned attribute 0 into a
frame-space list.  The nightly debug-assertions gate stopped there ("dep-space violation:
union of Attr deps with Frame deps", `Deps::union`), every night since the guard for
loft#1245 landed; a release build reads the index as whatever caller variable happens to be
number 0, a borrow of a variable the value never touched.  It is the class IMPLEMENTATIONS.md
records under *the DbRef set drifts short*: a hand-written list of the shapes that carry a
list, restated beside the keystone that already knows.

**Closed at the mapper, by asking the keystone.**  `fnref_result_type` now routes every
shape through `Type::borrow_deps` / `Type::rewrap_deps` — a text return (under `?` too)
through the visible-argument map, a tuple element by element, every other dep-carrying shape
through the closure-aware map — and lists nothing.  A second assertion then surfaced on the
same gate: `Deps::renumber_frame` refused an EMPTY attribute-tagged list, which a `&text`
parameter's declared type carries into the variable table and which the retbuf renumber
walks; an empty list has nothing to corrupt, so the assertion exempts it as `union` already
did.  Both are visible only under `RUSTFLAGS='-C debug-assertions=on'`; the whole gate
(`--lib`, `issues`, `wrap`, `strings`, `frame_vars`) is green on this tree.

Guard `tests/scripts/1335-a-fn-ref-return-of-any-shape-maps-its-deps-into-the-caller.loft`
(hash, sorted, index, nullable record, nullable text; join, plain bind, loop with filler; the
vector and record controls), falsified by hand under the debug-assertions build at
`3d8f2b9e` (abort → ok) — inert on the six channels `make falsify` scores, because the wrong
index landed on a variable the values never read.  Held fixed and filed apart: a nullable
VECTOR return, a vector inside a returned tuple, and a nullable record chosen by an `if`
ALIAS the field on the baseline as well — `(B-Copy)` deviations the mapper fix is not what
closes.

### D-own-27 — OPENED AND CLOSED (2026-09-04, loft#1336): a local that OWNS after one assignment and VIEWS after another has no static owner

`(O-Latest)` memoises ownership per assignment at its loop depth, and `(O-Proxy)` reads the
binding's dep list — and both are STATIC readings of a binding that carries ONE dep list,
recording whichever assignment parsed last.  A walker breaks them:

```loft
cur: Node? = a;                                   // (B-Copy): cur mints a copy of a
while cur != null { total += cur.value; cur = cur.next; }   // (B-View): cur views b, then c
```

`cur`'s dep list reads `[cur]` (the self-dep the view rebind leaves), so `owned_ref` is false
at every site, the pre-`Set` free and the guarded post-free are never emitted, and the
scope-exit sweep declines — the copy leaks, on both backends, values right throughout.  The
inverse order, `s = a.next; s = a`, leaves the list EMPTY, so the reassignment copies "in
place" into whatever `s` names — the viewed record `b` — on `--native`, and the interpreter's
copy arm, asked on the bare `Reference` type, skipped a `Node?` altogether and ALIASED `a`.

**The filed scope was wrong three ways, and the matrix found each.**  The `reference<Node>?`
field is not the axis (`x: Leaf? = l0; x = t.l` leaks the same), a COPY-bind is not the axis
(`cur: Node? = mk(1, b); cur = cur.next` leaks the same, through either a dense or a nullable
return), and the `?` on the local is not the axis either: the dense twin `x: Pair = a; x =
x.other` released the copy at the rebind through codegen's post-free AND freed `x` at exit
through the proxy sweep, which landed on `b`'s store — a masked over-free that read clean
only because nothing read `b` after it.  The class is *a heap-record local with MIXED
ownership across its assignments*, and the fact that decides its frees is per RUN.

**The cure is the native emitter's own tracker, lifted into the IR.**  `generation/mod.rs`
already kept `_own_store_<name>` for a dense, deps-empty local with an owned init and a
borrow reassign (@PLN90, loft#495): pointed at the store the local owns after an owned
assign, freed by identity at a borrow reassign, freed at exit.  That was the reference route
— the dense twin was right on `--native` because of it — and it was private to one backend
and blind to a nullable local.  `(O-Witness)` puts the same slot in the IR (`__own_<name>`,
`scopes::owner_witness_locals`): `scan_set` classifies each assignment (`witness_set_kind` —
a whole-value copy, a loft callee whose return both emitters copy, an `Owned` oracle answer
that is not a literal into a work-ref) and emits the release before a mint, or after the
`Set` by `OpDistinctStore` where the value reads the local or is a view; `get_free_vars`
releases at scope exit; the local is marked never-free.  Two ops carry it: `OpDistinctStore`
(the identity test — two sentinels compare EQUAL) and `OpRefAlias` (a reference as a VALUE,
so the witness can NAME the local's store where `Set(w, Var(v))` would copy it).

**Four measurements shaped it, each a wrong first cut:**

1. *The witness's entry init was `= null`.*  A heap local's `= null` lowers to `OpInitRef`, a
   stack-record placeholder, and the first release met the `#306` refusal.  It is the
   sentinel call now.
2. *`OpDatabase` reuses the slot's store, and at a FIRST bind that slot is the `null_named`
   placeholder.*  Allocating fresh unconditionally leaked one untyped store per witnessed
   local on `--native`; fresh only on a REASSIGNMENT.
3. *A never-free local reads as the `__ncc_` hoist to native's adopt rule* (`is_borrowed_view
   && skip_free → adopt`), so `cur = keep(other)` aliased the argument and the witness then
   claimed `other`'s store.  The rule now excludes a witnessed local; the interpreter's twin
   arm wanted the `?` peel (loft#1106's family) for the same cell.
4. *A projection whose deps a LATER copy strips is materialised at codegen* (loft#778's `k =
   a[0]; for x in a { k = x }`), and the witness — which classifies a projection as a view —
   never learned of that store.  Both emitters now decline the materialise arms for a
   witnessed local: its projections stay views, the container-wide free those arms guard
   against is one it never emits, and `collect_views_to_materialise`'s bindings are never
   witnessed, so the two mechanisms do not meet on one binding.

And one trap in the GUARD, not the fix: a `reference<Node>?` field is a POINTER, and a helper
returning `Node { next: b }` with `b` its own local hands back a dangling one — five cells
reported use-after-free from the test's own chain builder.  Every chain is built in the frame
that walks it.

**Measured** on both backends with `LOFT_STRICT_STORES=1`: the eleven filed cells plus the
call-mint, nested-field, dense-twin, loop-declared, one-arm, nullable-source,
borrowing-call (over a view, and reading the local), alternating and 200-round cells — values
right, no store retained, no over-free.  `scripts/find_problems.sh --subject scopes|store|
codegen|runtime` green; the two `LOFT_NO_JOIN_OWN` positive controls hold the witness off
too (`LOFT_NO_OWNER_WITNESS`), because the witness closes their `local_source` leak on its
own and a control that cannot fire proves nothing.  Held FIXED: a returned VIEW of a local
through a `-> S?` return (no delivery buffer, so nothing materialises it — filed apart), a
witnessed local at a callee's hidden buffer position, and a captured local, which the
witness declines (@FR-L-CapHeap holds).

### D-own-25 — OPENED AND CLOSED (2026-08-30, loft#1201): one delivery buffer, two owners, because a vector reads the adopt flag the other way round

`@FR-O-Owner` says every heap store has exactly one owner.  `xs.map(|x| { [x, x + 1] })` gave
one store two.  The caller allocates a single `__ref_N` delivery buffer, hoists it out of the
loop and reuses it every iteration; the lambda fills it and hands it back, so the
comprehension's per-iteration yield slot IS that buffer.  Read as an owner, the slot took a
plain `OpFreeRef` at the end of each iteration — releasing the caller's own buffer, which the
next iteration then wrote into.

**The fact was already computed and the two type formers need OPPOSITE readings of it.**
`Definition::return_adopts_fresh_store` answers *does the callee mint its own store, or fill
the one I passed?*  For a `Reference` its FALSE case is safe on its own, because
`gen_set_first_ref_call_copy` interposes a deep copy and the slot cannot alias the buffer.  A
vector has no such copy path — it is PutRef-ALIASED to the work-ref argument — so for a vector
FALSE is precisely the case where the two DO alias.  The witness pairing that emits the
runtime-conditional `OpFreeRefIfDistinct(slot, buffer)` was gated on
`adopts_fresh_store || publishes_through_ref` and on a `Reference | Enum(_, true, _)` shape,
so the vector spelling reached neither.  It now pairs whatever the flag says, which is
conservative in the direction that matters: `OpFreeRefIfDistinct` frees exactly as the plain
free did when the stores DIFFER and only skips when they alias.

⚠ **There are TWO pairings and only one of them is sound here — measured, not reasoned.**
`paired_witness[buffer] = slot` makes the BUFFER's free conditional on the slot;
`witness_buffer[slot] = buffer` makes the SLOT's free conditional on the buffer.  They are
not symmetric.  The second is right for this shape: the slot is inner-scoped and dies every
iteration while the buffer is function-scoped and released once, so skipping the slot's free
in the aliasing case loses nothing.  The first is the opposite trade, and for a vector
admitted by the alias case it is wrong — the slot may carry no free of its own, and then
NEITHER store is released.  Widening both branches at once was tried first and leaked across
sixteen test binaries (`placement_parity`, `n2_cdylib`, `leak`, `leak_cases`,
`nullable_ret_buffer`, `ownership_oracle`, `alias_link_baseline` and the script corpus), while
every cell of the hand-built boundary matrix stayed green — the suite found it and the matrix
could not, because the matrix varies the DEFECT's axes and not the fix's blast radius.

⚠ **The named-function twin was clean by ACCIDENT, and that is the finding worth keeping.**
`xs.map(pair)` passed throughout, which is what made this look like a lambda question.  Its
yield slot carried a dep — but the index was a CALLEE ATTRIBUTE number resolved against the
CALLER's variable table, so the name it pointed at was whatever local happened to occupy that
slot.  Adding two unrelated locals to the caller moved the dep from `_elm_1` to `b`, a `text`.
A dep in the wrong space is not a fact; it was non-empty, and non-empty is what suppressed the
free.  So the corpus contained a passing cell whose pass meant nothing, and the axis it
appeared to establish (*lambda vs named function*) was not the axis at all — the axis is the
RETURN FORMER, and the boundary was measured at struct-clean / vector-broken.

⚠ **It is a wrong ANSWER on `--native`, not only a latent hole.** The issue was filed from
`--interpret` as *"a latent soundness hole, not a wrong answer today"*, and that is true
there — the released store is not reused before it is read, so poison is the interpreter's
only channel.  On `--native`, the default backend, the recycled buffer is handed straight back
and appended to: a `map` asking for six rows of three answered row 1 with SIX elements.  A
wrong LENGTH, silently, with nothing to say so.

Guard: `tests/scripts/1201-a-mapped-lambdas-collection-does-not-own-the-buffer.loft`, whose
controls are the named function and the stored fn-ref (a `CallRef` allocates its buffer at run
time, so nothing there can alias), the other return formers, `filter`, and an explicit
comprehension.

### D-own-21 — CLOSED (2026-08-29, loft#1150): three faces of one list that read `Hash` and not `Optional(Hash)`

Opened the same day and closed here.  A `-> hash<T[k]>?` did not hand back what it built, and
the three defects hid one another in sequence: the value was DISCARDED and the sentinel
returned; once D-own-22 made it come back, the arm buffers were freed twice — conditionally by
that fix and unconditionally at scope exit — so the store being returned was released; and a
genuinely ABSENT return panicked in `copy_claims` on the way into the caller's local.

**All three are one miss wearing three faces.**  `@FR-L-Null` gives `τ?` the same layout and
the same store as `τ`, so every site deciding *"does this carry a store?"* must peel:

* `get_free_vars`'s `suppress_source` asked `is_dbref(tp(v))` BARE, so a nullable arm buffer
  failed the `return_sources` suppression and took an unconditional scope-exit free.
* `is_protectable_store_type` asked it bare **while its own caller peels**, so a `τ?` argument
  left the @P290 witness set incomplete and the minting arm leaked.
* `OpReplaceKeyed` dereferenced an ABSENT source, where its own FREE leg had carried the
  sentinel guard all along — the copy leg simply never got one.

⚠ **The second of those was written down and deliberately deferred**, on the ground that *"the
change has no measurement asking for it"*: it is not inert (it moves emitted code in six corpus
programs, every one a guard for this machinery) and it moves in the direction where a mistake
is a use-after-free rather than a leak.  This issue is that measurement — one program, two
spellings, and only the wrapper between them.  All six named guards (1021, 1029, 1105, 1106,
1107, 882) are green under `LOFT_STRICT_STORES=1` + `LOFT_POISON=1` on both backends with the
peel in place, which is the check the deferral asked for.  **A deferral records a missing
measurement, not a closed question; take it up when the measurement arrives.**

Absence needed a representation, and one already existed: `DbRef::ABSENT_REC` in the collection
slot (loft#917) means null for an ALLOCATED store, where zero means the EMPTY collection.  So an
absent source leaves the destination's store in place — there is nowhere else for a later `+=`
to build — and marks the slot absent.  `Stores::mark_collection_absent` is the one home, called
from both `OpReplaceKeyed` bodies.

**This is the sixth and seventh site where this same list has drifted short by the wrapper** —
`is_dbref` here (twice), at D-own-13, `deps_mut` (loft#1106), `is_keyed` (loft#1140's
`94ae617f`) and `depend` (loft#1143).  `is_dbref`'s own doc records that it drifts when
RESTATED; it drifts when asked BARE just as reliably.

Guard: `tests/scripts/1150-a-keyed-return-that-can-be-absent-hands-back-what-it-built.loft`,
falsified at `d47714d9` on both backends (a PANIC to clean).  `absent_is_not_empty` is the cell
that separates the two states — every other cell passes if they are conflated.

⚠ Its results are all BOUND before they are read: consuming a nullable keyed return INLINE
retains the callee's store, which is **loft#1157**, pre-existing and measured unchanged across
this fix, with the dense spelling clean.  An inline cell would lock that leak.

### D-own-24 — OPENED AND CLOSED (2026-08-29, loft#1156): a body local died at the block, and was read after it

`@FR-O-Owner` places a free where the value DIES.  A local a loop BODY assigns is scoped to the
body block, so `get_free_vars` released its store at the end of every iteration — and a read
after the loop then read a freed record.

```loft
for p in as1 { e = p; }
// … anything that allocates …
println("{e.v}");     // answers 100 — a `Junk`'s bytes, through `A`'s layout
```

⚠ **Three things make this the register's clearest case of a defect no instrument reports.**
The obvious repro answers CORRECTLY — with nothing allocated in between, the freed slot still
holds the last value, so the churn is what makes it visible at all.  `LOFT_STRICT_STORES=1`
does NOT catch it: the slot really is free, so no detector is violated; the read simply lands
on whatever the allocator handed out next.  And `--native` REFUSED the program (`E0425`),
which reads like a native limitation and is the opposite — the free analysis had already
placed the local's death at the block's end and native additionally scoped the Rust `let`
there.  **One decision, expressed twice, half of it visible.**  Making it compile without
moving the scope would have shipped the interpreter's use-after-free to native.

Closed by hoisting the SCOPE, not by moving the free: pre-initialised at the enclosing scope
the local gets ONE store that each iteration copies into.  That is byte-for-byte the IR a
hand-written `e: A = A { … }` before the loop already produces, which is the strongest evidence
available that the target shape is right — a program that already ran correctly on both
backends.  It also settles the design question the issue left open (*extend the lifetime, or
refuse the read?*) without choosing either: a refusal would have to explain why
`for i in 0..2 { n = i * 10; } println(n)` is legal, and it is, on both backends — the scalar
works precisely because a scalar slot is already function-wide, which is the scope the hoist
gives the heap handle.  The rule was not extended; it reached the case that was missing it.

The exclusion is `was_loop_var`, and it is what keeps loft#1135 closed: a loop's own VARIABLE
is read after the loop routinely (LOFT.md documents it) and reserving a slot for it at the
enclosing scope orphans one store per program.

Guard: `tests/scripts/1156-a-loop-body-local-read-after-the-loop-is-not-freed-per-iteration.loft`,
falsified at `8d3245eb` on both backends — **on different channels**, which is the signature:
the interpreter fails an ASSERTION (it ran, and answered wrong), native fails to COMPILE and
has no assertion to fail.  Its cells sweep a copy-bound local, a BORROW-bound one, zero
iterations (`null`, the answer loft#915 gives), and a local first assigned in an INNER body and
read after the OUTER loop — that last because hoisting only one level puts it in the outer
loop's body, where every single-loop cell still passes.

### D-own-23 — OPENED AND CLOSED (2026-08-29, loft#1154): the CALL-SITE mirror — a join whose arm is a call

D-own-22's residual, and the same rule from the other side.  A keyed local bound from a JOIN
whose arm is a fresh-storage CALL retains that call's store:

```loft
fn mk(k: integer) -> hash<Hr[hk]> { [Hr{hk: k, hv: k * 11}] }
g = if c { mk(1) } else { mk(2) };     // one store retained per evaluation of a call arm
```

`OpReplaceKeyed`'s `0x8000` source-free bit is set from `is_struct_returning_call(code)` —
*is the RHS a call* — and a `Value::If` is not one, so the store the callee minted is copied
out of and abandoned.  The deep copy itself is correct, which is why this is a pure retention.

Measured against D-own-22 on one program: `origin/main` ×6, the D-own-22 build ×3 — the callee
half is closed and this half is untouched, which is what says they are two deviations and not
one.

⚠ **The obvious cure is an OVER-FREE**, and that is what shaped the fix.  Widening the gate to
*"a join whose arms are calls"* breaks the mixed shape `if c { mk(1) } else { m }`: on the local
arm the source is `m`'s store and freeing it takes the caller's collection.  The static bit
cannot separate the arms and which arm ran is a runtime fact.

Closed by deciding PER ARM (`Parser::join_source_frees`): a fresh-storage CALL's store is
nobody else's and may be freed; a NAMEABLE arm is marked for the @P290 bracket, which then
refuses its free at runtime; an arm that is neither leaves the decision unmakeable and the
conservative never-free stands.  `view_root_slots` already unioned a join's arm roots for the
ARGUMENT case, so the witness half needed no new derivation — only to be reached.

Guard: `tests/scripts/1154-a-join-of-calls-frees-the-store-its-callee-minted.loft`, falsified
at `7f80c305` on both backends (ten retained stores to clean, with `exit` and `asserts` reading
`0|0` on both trees).  `call_or_local` is the OVER-FREE control and fails LOUDLY — it reads the
local back empty — where the defect itself is silent.

⚠ **No `??` cell.**  `??` is a join in the LANGUAGE and not one in the IR: it lowers to a block
named `ncc` holding its subject in a `skip_free` `__ncc_N` temp, so its arms are not
`Value::If` arms and this gate does not reach them.  `X ?? <call>` retains the call's store —
recorded on loft#1157, whose subject is exactly that: a keyed call result with no owner.
Protecting `__ncc_1` as a witness would make it WORSE, since that temp is the one needing the
free.

### D-own-22 — OPENED AND CLOSED (2026-08-29, loft#1142): a Join answered the ownership fact per FUNCTION

`(O-Complete)` requires the fact PER BINDING and PER PATH.  `get_free_vars` answered it per
FUNCTION: `in_ret` suppresses the scope-exit `OpFreeRef` for every member of `return_sources`,
and a join puts EVERY arm in that set — while at most one arm is the return on any given run.
A keyed return through a branch join therefore retained the arms that did not run, one store
per call, growing without bound in a loop.  Values were correct throughout, so only the leak
channel could score it.

**Two proximate mechanisms, and that is what decides where the cut goes.**  With LITERAL arms
the untaken arm's `__kvb_N` is allocated before the branch is tested — `scan_if`'s pre-init
prefix emits `Set(v, Null)` per assigned variable, and for a keyed local that is an
`OpDatabase` store rather than a cheap null (the same belief loft#1135 was, one prefix over).
With NAMED-LOCAL arms nothing is pre-inited and it leaks identically.  So removing the
allocation closes one shape and leaves the other: **the defect is the free.**

Closed by widening loft#1022's runtime leg rather than adding one — hoist the join to
`__ret_N`, then `OpFreeRefIfDistinct(src, __ret_N)` per owned candidate.  Which arm ran is not
a static fact and that comparison is the only thing that can decide it.  The record gate asks
for an arm that is NOT a source, because a record's orphan is the arm that fails to deliver; a
keyed join orphans with every arm a source, so the keyed gate asks for **an owned keyed source
plus more than one arm**.  Counting owned sources alone was measured short:
`if c { x } else { [lit] }` has exactly one and still orphans it on the `x` path.

Guard: `tests/scripts/1142-a-keyed-return-through-a-join-frees-the-arm-that-did-not-run.loft`,
falsified at `caa35d27` on BOTH backends — 5 leaked store kinds (94 stores) to clean, with
`exit` and `asserts` reading `0|0` on both trees, which is why the header says the leak is the
channel that moves.

⚠ **The CALL-SITE mirror is open as loft#1154**, and the same program measures it: `origin/main`
leaked ×6, this build ×3.  A keyed local bound from a join whose arm is a fresh-storage CALL
retains that call's store, because `OpReplaceKeyed`'s `0x8000` gate asks *is the RHS a call*
and a join is not.  Its obvious cure is an over-free — an arm that is a bare local would have
the caller's store freed — so it needs the @P290 bracket widened to protect arm terminals and
not only ref arguments.  Found by `scripts/matrix_axes.py` reporting *statement context:
MISSING if-arm* on this fix's guard; the cell did not exist until the axis tool asked for it.

### D-own-20 — OPENED AND CLOSED (2026-08-29, loft#1143): the `?` spelling of a keyed return borrowed nothing

`(O-Move)`'s borrow clause — *a return that hands back a parameter is recorded in the return
type, and the caller COPIES* — was implemented for `hash<T[k]>` by loft#1140 and not for
`hash<T[k]>?`.  `@FR-L-Null` settles that they are the same question: `layout(τ) = layout(τ?)`,
so a `?` changes what the slot may HOLD and not what it reaches.

```loft
struct Hr { hk: integer, hv: integer }
fn nz(x: hash<Hr[hk]>?) -> hash<Hr[hk]>? { x }
//  203/203 0/0 0/0        expected 203/203 203/203 203/203 — both backends, no diagnostic
```

**Two sites, and fixing either alone is worse than fixing neither.**  `Type::ret_dep_shape`
did not peel `Optional(<keyed>)`, so the borrow went unrecorded and the caller freed a store
it had been lent.  Peeling it there and stopping turns the wrong answer into a `--native`
PANIC, because the second site — the keyed assignment's dep-strip in `parser/expressions.rs`
— restated the five keyed variants inline where `Type::depend` is the declared home for
*"which vars does this type borrow?"*, and that function is dep-transparent through
`Optional` (@PLN25).  The nullable destination therefore kept the borrow it had just
deep-copied away from, was typed as owning no store, and `OpReplaceKeyed` wrote through the
`u16::MAX` null sentinel.

The write side of that same list had already been fixed once, for the same reason:
`make_independent` reads `Type::deps_mut`, which peels the wrapper, *"spelled inline here,
this arm list had drifted behind that one by an `Optional`"* (D-own-13 first face, loft#1106).
The READ side beside it kept its hand-rolled match for another three days.  **A restated type
list drifts SHORT, and the direction is always the wrapper** — this register now records the
same miss at `is_dbref` (D-own-13), `deps_mut` (loft#1106), `is_keyed` (loft#1140's
`94ae617f`) and `depend` (here).

Guard: `tests/scripts/1143-a-returned-nullable-keyed-parameter-is-still-the-callers.loft`,
falsified at `d496ace4` on BOTH backends (exit 1 -> 0, 1 assertion failure -> 0 each).  Its
cells sweep the three signature spellings, all five keyed kinds and the nullable-vector
control; **it deliberately carries no cell for a keyed return that can be ABSENT** — see
D-own-21, which that would lock rather than guard.

### D-own-21 (original entry) — superseded by the CLOSED entry above (2026-08-29, loft#1150)

The nullable half of the keyed return delivery that D-own-20 did not reach.  When the value a
`-> hash<T[k]>?` hands back can be absent — either literally (`{ null }`) or because the tail
is a BRANCH JOIN — the body frees every arm's store and returns the null sentinel:

```loft
fn f(c: boolean) -> hash<Hr[hk]>? { if c { [Hr{hk: 9, hv: 900}] } else { [Hr{hk: 8, hv: 800}] } }
//  --interpret answers 800 by READING THE FREED STORE (LOFT_STRICT_STORES=1 names the
//  use-after-free); --native panics in `allocation.rs` indexing allocations[u16::MAX].
```

The emitted IR shows it directly: the dense twin ends `return if c { __kvb_1 } else { __kvb_2 }`
and the nullable one drops the `if` to a statement, emits `OpFreeRef` for BOTH arm buffers and
ends `return null`.  `vector<T>?` and `S?` joins are clean, so the boundary is exactly the five
keyed kinds behind `?` — the kinds whose `block_result` arm (added by loft#1140) records the
borrow fact and dispatches no delivery, where the vector and reference arms each dispatch one.

Ruled out by measurement, so nobody re-derives them: `is_dbref(result.base())` and
`tail_is_if` are both TRUE for the nullable spelling, `unify_if_branches_work_refs` returns
`None` for the DENSE spelling too (its arm terminals are `__kvb_N`, not `__ref_N`), and
`get_free_vars` reports byte-identical `ret_var` / `return_sources` for both.  The divergence
is therefore downstream of the scope pass's inputs, not in the `if_unified` gate that names
this exact symptom in its own comment.

### D-own-19 — OPENED AND CLOSED (2026-08-28, loft#1126 + loft#1128): ownership read off the BINDING, not the latest assignment

`(O-Latest)` says ownership is a property of the LATEST ASSIGNMENT to a binding, at the loop
depth that assignment was taken.  A function whose TAIL is a call hands the callee its own
return buffer so the result can be built straight into the destination — and when that callee
mints a store of its own instead, the buffer variable stops holding what it held.

The interpreter did not free the displaced store, because the buffer variable is the hidden
return-buffer PARAMETER and `state/codegen.rs`'s `is_hidden_buf_arg` reads exactly that: *an
argument, so the CALLER owns this store, so never free it*.  True at function entry and false
from the first assignment onward — the binding-level reading `O-Latest` exists to replace.  One
orphan per CALL, so a hot path grew the heap without bound; the answer was right throughout.

Both halves are needed: an earlier `return` of the variable (which is what makes it the buffer
variable) AND a tail call that allocates for itself.  Either alone is clean.

`--native` was already correct, and by reading the same fact a different way: it stashes the
caller's buffer at function entry as `_rb_w_<name>` and guards the displaced free with `_old !=
_rb_w_<name>` — a RUNTIME answer to "is this still the caller's store?".  So the two backends
agreed on the value and disagreed on the heap, which is the asymmetry `(O-NoDiverge)` forbids
and the reason only the interpreter's leak channel could see it.

Closed in `scopes.rs::scan_set`, beside the `#316` transition free that already answers this
question for the borrow-rhs shape: `owned_refs` IS `O-Latest` (the oracle memoised per path and
per loop depth, intersect-merged at every join per `O-Complete`), it lives there, so the free is
emitted there.  Gated on the callee's carried adopt-vs-copy fact
(`Definition::return_adopts_fresh_store`) — a callee that DELIVERS through the buffer displaces
nothing, and a pre-Set free would then destroy what it is about to write into.  Guard:
`tests/scripts/a-tail-call-frees-the-store-its-buffer-var-stops-holding.loft`.

**The residual is CLOSED (2026-08-28, loft#1128): the interpreter carries the fact per RUN.**
Where the prior assignment is CONDITIONAL — `if c { r = mk(1); … } mk(3)` — the intersect-merge
correctly answered "not owned on every path" and emitted nothing, so that shape leaked on
`--interpret` while `--native`'s runtime witness got it right.  Conservative in the safe
direction (a leak, never an over-free) and incomplete, which is the half `O-Complete` names.

Closed by giving the interpreter its own witness, as a BOOLEAN rather than native's `DbRef`
snapshot: there is no IR spelling for a raw `DbRef` copy — `OpCreateStack` yields a pointer to
the variable's SLOT, which tracks the current value rather than the entry one — while a
`__rbo_<name>: boolean` mirroring `owned_refs` needs nothing new.  It is written after every
assignment to the buffer variable (left UNCHANGED where the call DELIVERS through the buffer,
since the variable then holds what it held), the displaced free becomes
`if __rbo_<name> { OpFreeRef(v) }`, and it starts FALSE — on entry the buffer is the caller's,
and a transition site is reachable with no prior assignment at all (`fn g() -> Res { mk(2) }`),
so an uninitialised slot would release the caller's store.  Minted only for a body that
actually reaches a displacing site, so nothing else pays a slot.

The same witness makes the LATENT over-free in the mirror direction moot rather than needing
the hazard proven: at the FIRST assignment of a buffer variable with a non-S1 rhs, codegen's
`owned_ref` was true and it emitted an UNCONDITIONAL pre-Set `OpFreeRef` on the caller's buffer
(measured directly — `owned_ref=true s1=false hidbuf=false arg=true` — and never reproduced as
a fault, because every shape tried has the caller's `__ref_N` still null).  A flag that is
false until this function mints something cannot free what it did not mint.

Guard: `tests/scripts/1128-a-conditionally-assigned-return-buffer-frees-what-it-displaces.loft`,
whose loop cell alternates the branch fifty times — the leak is one store per CALL, so a single
call cannot witness its size, and a fix that simply freed unconditionally would over-free on
half of them.

### D-own-17 — OPENED AND CLOSED (2026-08-28): a mint carried the DESTINATION's deps

`(O-Deps)` reads a value's deps to place its free, so what a value's deps SAY is the whole of
what the sweep knows.  A keyed collection literal in value position (`[K { … }]` as a return,
a call argument, a `??` default) builds into a function-scoped `__kvb_N` accumulator whose
store is its own — a keyed collection has no wrapper record — and the accumulator was minted
at the DESTINATION's type, deps and all.  `??` is the position where that destination is a
BORROW: the subject of `b.c ?? []` is a field read typed `["b"]`, so the mint inherited `["b"]`
and `get_free_vars`' `dep.is_empty()` ownership test read someone else's store.  Nothing freed
it, and the mint sits inside the arm — one store per EVALUATION, unbounded in a loop, on BOTH
backends, with every value right.

Closed by minting at `var_tp.without_deps()`: a deps list describes where THIS value's storage
comes from, and a freshly minted store's comes from the mint.  The `vector` twin three lines
away has always minted dep-free, which is why the defect was keyed-only.  Guard:
`tests/scripts/a-keyed-literal-default-owns-the-store-it-mints.loft`.

### D-own-18 — OPENED AND CLOSED (2026-08-28, loft#1121): a store allocated for a value that overwrites it

`(O-Deps)` places a free from the deps a value carries, and the deps are also what decides
whether a slot gets a store at all.  A `??` vector-literal default has two shapes: one where
`_vec_N` owns its store and the literal fills it in place, and one where `vector_db` mints a
wrapper record and writes `_vec_N = OpGetField(__vdb_N, 0)` — a VIEW.  Both took the owning
preamble, so the second overwrote a live store the moment the null arm ran: one orphan per
EVALUATION, unbounded in a loop, `--native` only, values right throughout.

`--interpret` was clean for a reason that is not this rule: its assign path frees the displaced
store before it rebinds an owning local (`src/state/codegen.rs`, the `owned_ref &&
!s1_substituted` arm).  So the backends agreed on the answer and disagreed on the heap, which is
the same shape as D-own-16 above and the reason only the native leak channel could see it.

Closed by giving the backed shape `inline_ref` rather than `skip_free`.  Those two bits were
conflated: `skip_free` says *do not allocate* AND *never free*, and only the first half is
wanted here — a borrowed subject's `??` in return-tail position hands `_vec_N` to the
return-delivery materializer, which owns its free.  The site already applied `skip_free` for an
OWNED subject and withheld it entirely for a borrowed one; `inline_ref` is what the borrowed
case could always have had.

⚠ **Which shape a site is cannot be read from the deps.** The same line strips them for BOTH,
so by the time anything asks, the two agree — gating on the dep list left the owning shape
building into a null sentinel and every in-place `?? [lit]` answering length 0.  It is read off
the emitted block instead, which says so: the wrapper shape contains the `OpGetField` assignment
and the owning shape does not.  Guard:
`tests/scripts/1121-a-backed-default-does-not-allocate-a-store-it-overwrites.loft`, which scores
that 0 beside the leak.

### D-own-26 — CLOSED 2026-09-03 (opened 2026-09-01, narrowed twice): eleven free-deciding sites never consult O-Override, and the corpus cannot tell

`O-Proxy` states an obligation in so many words: *"A site that FREES on the proxy MUST also
consult O-Override — otherwise it frees a store someone else owns."* Measured in the 2026-09
bug review ([BUG_REVIEW.md](../BUG_REVIEW.md)): **38** functions test
`depend().is_empty()`, **17** of them decide a free, and **6** consult `is_skip_free`. The
obligation is undischarged at eleven sites.

**What makes it a deviation rather than a bug is that nothing fails.** A probe at every
push into `free_vars`'s free list, run over `tests/scripts` and `tests/docs`, reported
**zero** `skip_free` bindings arriving — so today the rule holds by accident, at sites that
do not ask. A site that frees on the proxy and happens never to meet a marked binding is
byte-for-byte indistinguishable from one that asks correctly, which is the same invisibility
`O-Proxy`'s own ⚠ paragraph is about, one level down. It becomes a wrong answer the first
time a `set_skip_free` call and one of these eleven meet, and the failure is an over-free:
freeing a store the rule says must never be freed.

**Narrowed on the day it opened.** Three of the seventeen were folded onto
`Scopes::owns_freeable_store`, which discharges the override consult and the parameter
carve-out together; emitted IR was verified byte-identical across all 1052 corpus files, so
the fold changed nothing and the guard is now structural at those three. The remaining
eleven are the open half.

**Closing it needs a check, not eleven edits.** The honest cure is a way to fail a build in
which a free-deciding site reads the proxy without the veto — the sites are recognisable
(`depend().is_empty()` reaching an `OpFreeRef` decision), and enumerating them by grep is
what produced the numbers above. Until such a check exists, each new free site restates the
obligation or silently skips it, which is how the count went from the 24 written into
`ownership.md` to 38.

**NARROWED 2026-09-03 — the check existed and was measuring nothing, and the eleven do not
survive re-measurement.** `scripts/o_proxy_check.py` had shipped on 2026-08-24, a week before
this entry was written, and the entry's own sentence *"closing it needs a check"* was true of
what that check could see rather than of whether one existed. It matched only free EMITTERS
inside the region a condition gates — but `get_free_vars` is what emits `OpFreeRef`, so these
sites conclude ownership in one function while the free lands in another. **25 of its 29
`ok` verdicts came from an empty region, not from a proof**, and it reported `0 violations`
across two sites that had no veto at all.

Three discriminations closed the gap — a free is REACHED (a write to the fact the sweep
reads: `make_independent` / `without_deps` strip the deps, `set_skip_free` on the proxied
binding is a MOVE) rather than only emitted; a writer counts only when it NAMES the binding
the condition concluded about; and a negated read whose region writes the fact is a positive
site, not the is-it-a-borrow question. The check now also prints its own control, `N of M
reach a free`, so the state it shipped in is visible rather than silent.

Re-measured against that predicate: **6 of 24 positive sites reach a free, not seventeen**,
and all six now discharge the veto. The eleven was a hand count that could not separate
*asking* the proxy from *freeing* on it; spot-reading the rest finds classifiers
(`materialises_element`, `classify_set`, `classify_vec_bind`), copy-vs-alias decisions
(`assign_refvar_vector`) and the @PLN94 oracle, which owe the veto nothing — but also
`parse_field_iteration` and `inline_struct_return`, which do reach a free by a route no
lexical window follows, so the eighteen are UNDECIDED and not cleared. Two of the six were
genuinely undischarged and now consult it: `scan_set`'s displaced-owned dep strip and `gen_set_first_ref_var_copy`'s move,
where a wrong proxy hands the target an interior pointer that its scope-exit `OpFreeRef`
then releases — loft#823's shape reached through the flag instead of through the deps.
Both changes are guards in the withholding direction and were measured INERT: a differential
probe printing whenever the added conjunct changes the outcome reported **zero hits over
1119 corpus files** on `--interpret` and a 60-file `--native` sample.

**CLOSED the same day by the declaration pass**, which is the cure that entry named. The
lexical route is exhausted at the seventeen sites whose free-reach cannot be decided from the
region a condition gates, so those sites now SAY which of the four facts they read, in a
fixed vocabulary the gate parses: `free` (9 sites — and @FR-O-Override is required with it),
`copy` (8), `alloc` (4), `oracle` (3). A declaration is a claim, so the gate contradicts one
it can disprove: declaring anything but `free` while a free is visible in the gated region is
reported, not trusted. Five falsifications, each run by breaking one thing at a time and
confirming red — the three veto sites, a deleted declaration, and a free site re-declared
`copy`.

**What the close does not cover, stated plainly:** a site that declares `copy` and frees
somewhere no lexical window reaches. That risk is real and much smaller than the one it
replaces, which was *"nothing in the source distinguishes them, and both compile."*

⚠ **The pass corrected one of its own verdicts, and that is the transferable lesson.**
`parse_field_iteration` reads like a free site and its own comment asserts the veto belongs
there — *"a borrow/skip_free binding owns no allocation"* — so it was declared `free` and
given `!is_skip_free(v)`. The differential probe then reported **8 of 1119 corpus files**
arriving with a `skip_free` binding: a live behaviour change, not the latent guard the other
sites were. The mechanism settled it — `copy_variable` + `remap_var_deep` give each field
block a FRESH binding, and the frees that follow are of those, never of the binding tested,
which is the same reason discrimination 6 excludes minting. The site is `copy`, the veto was
removed, and the comment now records that its own prose overstates its filter. **A site's own
comment is not a measurement**, and a rule citation is not a licence to change behaviour
without one.

### D-own-16 — CLOSED 2026-09-03 (opened 2026-08-27, narrowed 2026-08-30): a value that READS the local it assigns never frees the store it displaces

`(O-Deps)` places a free from the deps a value carries.  A local reassigned from a join over
ITSELF gets none placed: `c = mk(i) ?? c` retains every displaced store, nine of ten over ten
rounds, both backends, values right throughout — so only the leak channel speaks.

```loft
c: SN? = SN { x: 5 };
for i in 0..10 { c = mk(i) ?? c; }   // kt=78 SN×9 at exit
```

It is genuinely the hard shape rather than an oversight: the borrow arm IS the variable being
assigned, so freeing the displaced store before the assignment is a use-after-free on the arm
that takes it, and only a per-execution comparison can tell the two apart.  That is what
`OpBindOrCopy` exists for, and the reassignment does not reach it.

⚠ **One candidate cause is ELIMINATED, measured 2026-09-02.**  `callref_join_first_bind` — the
site that emits `OpBindOrCopy` — refuses anything that is not a `Value::CallRef`, and
`c = mk(i) ?? c` calls a NAMED function, so the call SPELLING looked like the whole of "does
not reach it".  It is not: admitting `Value::Call` there leaves the leak at exactly `SN×9` on
both backends, under `LOFT_STRICT_STORES=1` as well.  So the next attempt should not spend
itself on the spelling — the dispatch is a FIRST-BIND one and this is a reassignment, which is
a different question from which call form produced the value.

Re-measured the same day, and the entry's own numbers still hold: `c = mk(i) ?? c` over ten
rounds leaks `kt=81 SN×9`, while the plain `c = mk(i)` — the wider half loft#1200 closed —
is clean.

**NARROWED 2026-08-30 (loft#1200): the wider half is CLOSED, and the route to it is this
entry's own conclusion carried out.**  The entry reads as though the JOIN were the mechanism.
It is not — the plain reassignment `c = mk(i)` leaks identically with no join anywhere, and so
does the straight-line spelling with no loop — so what it described is *a nullable
heap-record local never releasing what its reassignment displaced*.  The dense twin is clean
because its callee is handed a `__retbuf` and fills the store the local ALREADY owns, so
nothing is displaced; a nullable RECORD return gets no buffer (`-> S?` is a synthetic
`__nullable<S>` carrying its own delivery, and giving it a buffer as well leaks one record per
call), so every call mints.  `vector<T>?` and `text?` are clean for the dense record's reason:
both reuse one buffer.

**A STATIC free was tried first and is wrong — do not re-run it.**  The local's first store is
normally an inline mint into a work-ref (`c: S? = S { x: 5 }` lowers to
`c = { Object -> __ref_p2_1 }`), so the local and that work-ref name ONE store.  Freeing
through the local before the reassignment double-frees it against the work-ref's own
scope-exit free: latent everywhere, and an observable wrong ANSWER where the local is returned
(`fn build() -> S? { c: S? = S{x:5}; for … { c = mk(i); } c }` handed back garbage under
`LOFT_POISON=1` on both backends).  One static site cannot separate the first iteration, where
the store is still the work-ref's, from the rest.

**An UNGATED peel of the two shape tests was tried too, and is unsound — do not re-run it.**
Peeling the `?` fixes every leak cell on both backends, and the empty dep list those tests
stand on (@FR-O-Proxy) is a PROXY: for a nullable `Reference` local it reads "owner" for at
least three unrelated kinds of borrow.  Each was found by the REFUSAL channel — `BUG (#306)`, a
whole-store free of the eval-stack store — which is the channel a widened free moves and the one
a leak matrix cannot see:

| slot | what it really holds | found in |
|---|---|---|
| the `__lift_N` of an inline `f(x) != null` | the eval-stack record — a `-> S?` return is NOT delivered into a caller-owned buffer the way its dense twin is | `1085-ret-buffer-passthrough-free.loft` |
| a local a lambda CAPTURES | a slot shared with the closure record | `1114-a-nullable-heap-capture-is-shared-like-its-dense-twin.loft` |
| a local bound from a reflection builtin (`t = type_named(name)`) | a borrowed handle into a store the runtime owns | `pln127-reflect-consumer.loft` |

Excluding the first two still left the third, and three unrelated borrow kinds reaching one
predicate is what says the predicate is the wrong place: the shape cannot license the free, so
the cure below peels it only behind a per-RUN witness that none of these three borrows can set.

**The cure is this entry's own sentence, carried out.**  A per-RUN witness — a boolean per
qualifying local (`__lbo_<name>`), false at entry, set true only by a MINTING CALL and false by
anything else — makes the free conditional on the local actually being the store's sole owner.
It is `rbuf_witness` one scope in, and D-own-19's path-sensitive half is the precedent.  Note
the flag records SOLE ownership, which is strictly narrower than `owned_refs`: an inline mint
into a work-ref is `Owned` and still not solely owned.

Two halves were needed beside it and both were real: `owned_refs` is keyed on an UNPEELED
shape, so a nullable local was never tracked at all (@FR-L-Null — `layout(τ) = layout(τ?)`, so
a `?` cannot change who frees a store), and the free's gate wanted ownership established at the
CURRENT loop depth where this shape establishes it one level out.

What REMAINS open here is narrower than the entry was filed with, and it is the same condition
in both surviving shapes: **the assigned value READS the local it assigns** — a call taking it
(`c = bump(c, i)`) and the self-referential join below.  The free is emitted before the
assignment, so taking it there hands the callee, or the join's borrow arm, a store that is
already gone; closing them needs the release to happen after the value is computed.

**Measured and REVERTED — do not re-run this.**  `Ownership::classify`'s var-cycle back-edge
answers `Borrowed { base: u16::MAX }`, which reads as *"no nameable witness"*, and the obvious
reading is that the cycle arm's base is the variable itself, so naming it would make the join
witnessed.  It changes nothing: the leak is identical on both backends with `base: *v`.  The
missing free is therefore not the witness's to license, and the next place to look is the
reassign-site machinery that excludes it — `owned_slot_reassignments` skips a var that is an
ARGUMENT, and its comment records that param-slot displaced frees *"stay with the witness
mechanism"*, which is the sentence this measurement falsifies for the self-referential case.

Found while building loft#1119's boundary matrix; distinct from D-own-15, which was the
oracle answering differently per caller rather than a free that is absent for everyone.

**The join was never the axis (2026-08-30).**  The plain reassignment leaks
identically — `c: S? = S { x: 5 }; for i in 0..10 { c = mk(i); }` retains nine stores in ten
rounds, with no `??` anywhere — so *"genuinely the hard shape rather than an oversight"* was a
reading of the repro rather than a measurement of the boundary, and the witness experiment
above changed nothing because the shape never reaches the witness machinery at all.  Both
backends decide this from a shape test over the local's TYPE, and both named `Reference` and
the record `Enum` without peeling the `?`: `S?` is `Optional(Reference(S))`, which holds the
store exactly as its dense twin does.  Every OTHER former was already right in its nullable
spelling — `vector<T>?`, `hash<K[k]>?` and `text?` all release — because `Optional` is
transparent to `depend()` and to `is_keyed`, and only the bare `matches!` was not.  `Vector`
stays out of the peel on purpose: a nullable vector releases through its own path, and the
comment saying so is beside the test that had to be widened.

**And the obvious cure is measured and RULED OUT, which is the more useful half.**  Peeling
the `?` in both shape tests fixes every leak cell on both backends and is unsound: the empty
dep list those tests stand on (@FR-O-Proxy) is a PROXY, and for a nullable `Reference` local it
reads "owner" for at least three unrelated kinds of borrow.  Each was found by the REFUSAL
channel — `BUG (#306)`, a whole-store free of the eval-stack store — which is the channel a
widened free moves and the one a leak matrix cannot see:

| slot | what it really holds | found in |
|---|---|---|
| the `__lift_N` of an inline `f(x) != null` | the eval-stack record — a `-> S?` return is NOT delivered into a caller-owned buffer the way its dense twin is | `1085-ret-buffer-passthrough-free.loft` |
| a local a lambda CAPTURES | a slot shared with the closure record | `1114-a-nullable-heap-capture-is-shared-like-its-dense-twin.loft` |
| a local bound from a reflection builtin (`t = type_named(name)`) | a borrowed handle into a store the runtime owns | `pln127-reflect-consumer.loft` |

Excluding the first two still left the third, and three unrelated borrow kinds reaching one
predicate is what says the predicate is the wrong place: the fix belongs where the ownership
FACT is known (@FR-O-Oracle), not in a widened shape test.  So the widening was measured,
reverted, and recorded here rather than shipped.

What the original entry got right is the reason the leak channel was the only one speaking: the
values are correct throughout on both backends.  What it got wrong is the boundary — a filed
repro shows the shape someone happened to write, never the shape the defect covers, so the
first probe of a filed leak should be the same program with the interesting feature REMOVED.

**NARROWED AGAIN 2026-09-03: three of the five cells CLOSED, and the route is not the witness
this entry proposed.**  `MintOnly` (a minting call that reads the local) 9 → 0, the
self-referential join `c = mk(i) ?? c` 9 → 0, and the conditional borrow 4 → 0 — on BOTH
backends, every value unchanged.  Guard:
`tests/scripts/1085b-a-nullable-local-frees-what-it-displaces.loft`.

The cure is REACHABILITY plus LICENCE, and the release-after-the-value-is-known machinery was
already correct — the entry above says the reassignment "does not reach" `OpBindOrCopy`, but
the machinery it actually never reached is `stash_old_for_post_free` → `OpFreeRefIfDistinct`,
gated shut by `owned_ref`'s UNPEELED shape test.  Three things had to hold together:

1. the peel (`Reference` / record `Enum` only — `Vector` stays out, as recorded above);
2. `!is_captured` — @FR-L-CapHeap;
3. a nullable local takes the GUARDED post-free rather than the unconditional pre-`Set`
   `OpFreeRef`, whose whole-store release is sound only for a local that ALWAYS holds a store.

**That third point answers the table above, and answers it differently than the table
predicted.**  *"Excluding the first two still left the third"* assumed each borrow kind needs
excluding BY SHAPE.  It does not: routing the nullable local to the guarded free means
`free_displaced` consults distinctness, free-protection AND @FR-H-Free's `store(r) ≠ 0` side
condition, so the `__lift_N` row and the reflection row both DECLINE at runtime with no shape
test at all (`pln127-reflect-consumer.loft` and `1085-ret-buffer-passthrough-free.loft` clean
under `LOFT_STRICT_STORES=1`).  ⚠ Which of those guards declines the reflection handle was not
instrumented — only that it declines.  The CAPTURE row is the one that genuinely needs the
static test, because there the displaced store IS distinct and IS unprotected while still
being shared; that is why `!is_captured` is load-bearing and why the bare peel answers 2 where
cell 6 must answer 1.

Each of the three is falsified: drop the capture test and cell 6 breaks; drop the post-free
routing and `1085` refuses at `op=OpFreeRef`; drop `free_displaced`'s stack-ref guard and the
same file refuses at `op=OpFreeRefIfDistinct`.  The opcode MOVING between those last two is
what says they are two sites rather than one defect seen twice.

**CLOSED the same day — the last row went with a THIRD option this entry never considered.**
`d: S? = p; d = mint(d, i)` — a local that first BORROWS a parameter — leaked ten stores in ten
rounds, because its dep list names `p` for the whole frame and the empty-deps clause therefore
declines forever.  The entry above concludes a per-RUN boolean witness is the cure, on the
grounds that a STATIC dep-strip is unsound.  The second half is true and is measured: with
`d: S? = p; if take { d = mint(d) }`, the not-taken branch still holds the CALLER's store, and
freeing it there is a use-after-free two frames up.

But witness and strip are not the only two options.  The dep list is not merely an obstacle —
it NAMES the variable the local might still be aliasing, so ownership is decidable at RUNTIME
by store IDENTITY, with no witness slot, no IR temp and no strip:

| the local's state | vs its dep | outcome |
|---|---|---|
| still borrowing (never minted, or the not-taken branch) | one store | declines — correct |
| minted its own | distinct | freed — correct |

Two halves, both reusing machinery that already existed.  **Scope exit** emits
`OpFreeRefIfDistinct(v, dep)` — against the @PLN87 entry stash (`rebind_orig`) where the
parameter is REBINDABLE, since a rebound param's slot stops naming the caller's store.
**The transition free** routes through the same guarded post-free the other cells use, and its
safety on the FIRST round — where the displaced store IS the caller's — is @FR-H-Free's
free-protection side condition: `free_displaced` declines a protected store.  Measured rather
than reasoned: `LOFT_POISON=1` answers identically to `LOFT_POISON=0`.

⚠ Restricted to an ARGUMENT dep on purpose.  A parameter's slot is stable for the frame (or has
that entry stash); an arbitrary LOCAL dep can itself be freed before the scope ends, and the
comparison would then name a store that is already gone.  This is also why the `p462` warning
recorded in `displaced_owned_slots` does not apply: that one is about stripping deps on a PARAM
SLOT, and this predicate excludes `is_argument(v)` and strips nothing.

**What remains is not a leak.**  A lambda-CAPTURED local retains what it displaces, and that is
`(L-CapHeap)` holding: a captured heap value is SHARED, so declining is the right answer and its
right answer keeps a store.  It can never enter the script corpus, whose leak gate is absolute —
it lives in the plan's matrix probe, whose cell 6 asserts `g() == 1`.

### D-own-15 — CLOSED (2026-08-27, loft#1119): the ORACLE answered differently depending on who asked

`@FR-O-Oracle` is the claim that there is ONE own-vs-borrow derivation. That claim needs the
answer to be a function of the value alone; here it was a function of the value AND of which
caller happened to be asking.

`Ownership` carries a set of variable slots that are already in flight, so a self-referential
chain (`c = t[k] ?? c`) yields a conservative answer instead of recursing for ever. A slot
number only names a variable within ONE function's variable space, and `return_ownership`
walks the CALLEE's body — with the caller's set still in hand. The caller's `__ncc_3` and the
callee's `__ret_1` are both var 3, so the walk read the callee's own temp as self-referential
and answered `Borrowed { base: MAX }` for an arm that borrows nothing. The callee's return
then read `Join { base: MAX }` — "no nameable witness" — where the identical call in a
different statement context answered `Join { base: 0 }`.

Everything downstream reads that one fact, so everything downstream declined: D-own-14's lift
(`scopes::inline_struct_return` via `ncc_join_is_witnessed`) left the block inline and the
store the callee minted was owned by nothing, one record per EVALUATION on both backends. The
filed symptom was a discarded call statement inside a loop; the loop and the discard were
neither of them the condition. What decided it was which SLOT NUMBER the caller's temp got.

```loft
fn pick(a: SN?, c: boolean) -> SN? { if c { a } else { SN { x: 9 } } }
fn main() { p = SN { x: 7 }; for i in 0..4 { use_it(pick(p, false)?.x); } }
```

Closed by scoping the in-flight set to the function whose body is being walked — the callee
gets a fresh one and the caller's is restored after. The FUNCTION-level guard (`visiting`)
is untouched and still stops genuine recursion.

**What generalises past this entry, and it is the same lesson D-own-12 closed on from the
other end:** an oracle whose answer depends on the ORDER it was asked in is not one
derivation, whatever its doc says. The give-away was in the issue before the cause was — the
same call site, the same arguments, the same callee, two different answers — and that shape
is a statement about the ANALYSER's state, never about the program. Reading it as a gap in
the gate cost a day; the gate was doing exactly what it said.

⚠ **The guard for this is a numbering SWEEP, and it has to be.** The collision needs the
caller's in-flight slot to equal a slot the callee's tail walk reaches, and the callee reaches
exactly one — measured, and measured again with a seven-armed callee, which widened nothing.
So which caller hits it is a coincidence of how many locals that caller declares first: one
extra compiler temp anywhere moves every number by one and a single hand-written cell goes
quiet without saying so. `tests/scripts/1119-a-callees-var-slots-are-its-own.loft` is twelve
callers over two callees for that reason, and its header says what to do if every cell ever
goes quiet at once (widen the sweep — do not delete the file).

### D-own-14 — CLOSED (2026-08-27, loft#1118): a JOIN return used INLINE had no owner

`(O-Deps)` places a free from the deps a value carries. A callee whose return may be its
ARGUMENT or a store it MINTED answers `Own::Join`, and which one it is cannot be settled
statically — only per execution. Bound to a local that already works: the bind goes through
`OpBindOrCopy`, which adopts the minted arm (so the scope-exit free is right) and
materialises the borrowed arm (so the caller's argument is intact).

Used INLINE it did not. The `??` / `?` discharge lowers to an `ncc` value-block whose temp is
`skip_free` — the block's result ALIASES it — so the minted store was owned by nothing: one
leaked record per EVALUATION, unbounded in a loop, and the values right throughout, so only
the leak channel spoke. `scopes::inline_struct_return` is the lift that cures exactly this
(loft#879), and its `dep.is_empty()` guard refused the shape, because a `Join` return carries
a dep on the argument it may borrow.

The guard was not careless: lifting a value that really IS a borrow and freeing the temp is a
use-after-free, the direction that cannot be recovered from. What makes the lift admissible
is that the bind which follows is the runtime guard and not a static bet — a lifted temp is a
DENSE `Reference`, so the heap first-bind dispatch reaches it and emits `OpBindOrCopy`. The
lift therefore asks the same `Own::Join`-with-a-nameable-witness question that decides
whether the guard is emitted at all, so it cannot fire where the guard would not.

**The narrowing is the load-bearing half, and it was measured rather than reasoned.** loft's
IR spells every operator as a `Value::Call`, so "the subject is a call" also matches an
ELEMENT READ — `t[p] ?? d` is an `OpGetVector` — which is a view into a container the caller
still owns. Admitting it made the ownership fuzz gate's `local_source` cell answer WRONG on
`--native` with the two backends diverging. Only a call to a LOFT-DEFINED function is lifted,
and only the block's FIRST statement is read: a default arm is frequently a call of its own
(`t[p] ?? dflt()`), and searching the block for ANY call re-admits the cell just excluded.

Three narrowings were tried against that gate before this one held, which is the record worth
keeping — the gate falsified each in turn, and none of the hand-built cells could.

Guarded by `tests/scripts/1118-an-inline-join-return-is-lifted-and-guarded.loft`, whose
borrow-arm cells are scored by the caller's variable AFTER the loop rather than by the leak
channel: a leak channel cannot see an over-free, because freeing more always reads as an
improvement. Falsified at `aed98943` — interpret leaked `SN×750` → clean, native `SN×3` →
clean. That native count is the second thing worth keeping: a small repro is clean on
`--native`, which reads as "interpreter-only" until the loop counts show otherwise.

### D-own-13 (second face) — CLOSED (2026-08-27, loft#1107): the ELEMENT position witnesses its ROOT

`(O-Complete)` requires the ownership fact PER BINDING.  A nullable value in an ELEMENT position
now has one: `caller_arg_base` maps a PROJECTION argument to the root container it reads out of,
through the same `view_root_slots` walk the @P290 bracket protects through, so the join guard has
a store to compare the return against and frees the minting arm.

```loft
fn pickn(s: Sn?, c: boolean) -> Sn? { if c { s } else { mkn() } }
fn elem(c: boolean)  -> integer { v: vector<Sn?> = [Sn { a: 7 }]; r = pickn(v[0], c); r?.a }
fn local(c: boolean) -> integer { q: Sn? = Sn { a: 7 }; r = pickn(q, c); r?.a }
```

Twenty records over ten rounds at `kt=__nullable<Sn>` on the control, clean on both backends
after, values 180 throughout.

⚠ **This entry read as an OWNERSHIP gap on the strength of one discriminator that was itself
broken.**  It recorded *"binding `v[0]` to a local first does NOT cure it — a witness gap is
cured by a name, an ownership gap is not"*, and that is a sound test whose input was wrong: on
that build the hand-bound spelling did not merely fail to cure the leak, it CORRUPTED the
caller's container, because the join bind read its witness out of a slot its own sentinel had
just overwritten.  The name was being taken and then destroyed.  With the read ordered before
the write the discriminator answers the other way and the gap is the witness gap it always was.

**What generalises:** a discriminator is only as good as the build under it, exactly as a filed
negative is.  This one was run on a tree carrying an unfixed defect in the very mechanism it was
discriminating on, so it could not have answered anything else.

### D-own-13 (first face) — CLOSED (2026-08-26, loft#1106): a nullable heap local carried no ownership fact

`(O-Complete)` requires the ownership fact PER BINDING.  An `Optional(Reference)` local has
none: `data::is_dbref` lists the eight store-carrying kinds and not the `Optional` wrapper, so
`--show-ownership` renders a `P?` variable as `— (scalar)`, nothing frees it, and the @P290
bracket cannot name it either.

```loft
struct P { x: integer = 3 }
fn mk() -> P? { P { x: -1 } }
fn pick(a: P?, c: boolean) -> P? { if c { a } else { mk() } }
fn plain(c: boolean) -> integer { q = P { x: 7 }; r = pick(q, c); r?.x }   // leaks one per call
fn declared(c: boolean) -> integer { q: P? = P { x: 7 }; r = pick(q, c); r?.x }  // clean
```

Both backends, values correct throughout.  The axis is the ARGUMENT'S OWN declared type: with
`q` declared `P?` the result's deps come back empty and the caller frees it, and with `q`
inferred `P` they name `q`, so the caller reads a borrow and the minted store is orphaned.

⚠ **This is not D-own-11 with a different argument, and the test that separates them is the
hand-bound spelling.** Every witness gap in D-own-11 is cured by binding the argument to a
local first; this one is not — `e = q; pick(e, c)` leaks identically.  A witness gap is about
what the bracket can NAME; this is about what the type system says anyone OWNS.

The obvious cure was measured and rejected: peeling `is_protectable_store_type` to `.base()`
(matching the `heap_dep` gate three lines above it, which peels for exactly this reason) leaves
the leak untouched, because the missing free is not the bracket's to license.  Resolving it is
a decision about which of `is_dbref`'s callers see through the `Optional` wrapper — and
`is_dbref`'s own doc already records that this list drifts SHORT when restated, with
`Parser::is_heap_handle` named as "the same question with a `.base()` peel".

### D-own-12 — CLOSED (2026-08-26): the witness list was short by two OPS, and the count of homes for that list was wrong

D-own-6 closed on the claim that *"the runtime Join witness now covers every argument it can
name."*  Four spellings have been found since that it could not name, all on the same axis its
own closing paragraph identifies as the one its oracle never varied.  Two of them are this
entry's; the other two are D-own-11's, closed in the sibling checkout by the general
`bracket_can_name` question rather than by a fifth shape.

| spelling | what the walk answered | where it closed |
|---|---|---|
| `pick(t.0, …)` — a tuple ELEMENT | `None`: `Value::TupleGet` is not a call, so no op-name list can see it | loft#1104, bound at the call site like the construction family |
| `pick(t.0.s, …)`, `pick(t.0[0], …)`, `pick(t.0.0, …)`, `pick(vt[0].0, …)` — a CHAIN over one | `None` for the same reason, one or two nodes down | loft#1104 — the ELEMENT is bound and the chain RE-BASED on it, so the temp carries the type the tuple declares |
| `pick(h[k], …)` — a KEYED lookup at hash, sorted and index alike | `None`: `OpGetRecord` was absent from `is_projection_op` | **here** — the op list merged onto one home |
| `pick(v[i] ?? mk(), …)` — a join whose arm MINTS | `None`: the arm is a call, and a call is deliberately not a projection | loft#1105, D-own-11 |

**The list `view_root_slots` reads was short by two ops, and three other homes already had them.**
`OpGetRecord` is declared `-> reference[data]`, so a keyed lookup answers a record living in the
collection's own store and the root variable is exactly the witness the bracket wants.
`is_projection_op`'s doc said *"One list, two readers … Two lists of the same two ops would
drift."*  Measured: **seven sites spell that list by hand across six files, in four distinct
memberships**, and the two that had the right answer (`scopes::base_container_var`,
`generation::container_element_base`) are byte-identical copies of each other.  Merged onto the
one home rather than adding the op a fourth time; the doc now also states the criterion it is NOT
(*"the return deps on parameter 0"*, which `OpNewRecord` and `OpInsertVector` also satisfy — they
GROW the store rather than read it).

⚠ **The chain rows are why binding is not always the answer.**  A chain's temp must carry the
projection's RESULT type, which this pass cannot compute; binding its BASE needs only the type the
tuple already declares.  A temp typed off the CALLEE'S PARAMETER instead carries no deps and so
reads as an OWNER — and a free emitted for a store the tuple base still owns is a use-after-free,
not a leak.  That is the one direction this machinery's own comments warn about, and it is why the
element is bound and the chain re-based rather than the whole argument being bound.

**What generalises past this entry:** D-own-6 named the argument spelling as the axis its oracle
pinned, and then closed on a fix that enumerated the spellings *it had thought of*.  An axis named
in a closure is not an axis measured by it.  Each found since was reached by moving one more thing
— a tuple base, a projection above it, a keyed container, a `??` — and each took one probe.
D-own-11's closing sentence is the generalisation both halves arrived at from opposite ends:
**a predicate that enumerates SHAPES will keep being one shape short.**

### D-own-11 — CLOSED (2026-08-26, loft#1105): an argument the borrow bracket could not NAME leaked the callee's minted store

The model claims **no leak**.  A call whose return may BORROW an argument decides borrow-vs-owned
at runtime with the @P290 bracket, and the bracket protects a store by naming it through a
variable whose VALUE is a `DbRef`.  Where it cannot name one the witness set reads incomplete and
the caller keeps the conservative never-free answer — which is SOUND but copies the returned store
and orphans the one the callee minted, one record per call on both backends:

```loft
fn pick(s: S, c: boolean) -> S { if c { s } else { mk() } }
fn f(c: boolean) -> integer { v: vector<S?> = [S { a: 7 }]; r = pick(v[0] ?? mk(), c); r.a }
```

A `??` in argument position lowers to an `ncc` BLOCK whose tail is a join with a CALL arm.
`view_root_slots` walks a bare `Var`, a projection chain and a join, and neither a
multi-statement block nor a call is nameable — the call deliberately so, since its returned store
may be the argument's or one it minted, which is the very split the bracket exists to decide.

Closed by binding the argument to a temp before the call (`Scopes::scan_args`,
`unnameable_borrow_source`), which is the hand-written spelling that was always clean
(`e = v[0] ?? mk(); pick(e, …)`).  **The preamble runs BEFORE the bracket is emitted**, so the
temp holds a real `DbRef` by then — which is exactly why a WIDER WITNESS is the wrong cure and a
bind is the right one: marking the block's own work-ref would read as covered while it still held
its null, and the source-free that licenses would release a store the caller still reaches.  That
trades a leak for a use-after-free (loft#981).  A bind cannot make that mistake, because the value
is already computed when the name is taken.

⚠ **This is the THIRD cell of one class, and the fix is finally the class rather than the cell.**
loft#1029 hoisted an inline CONSTRUCTION, `tuples.md` D-tup-3 bound a TUPLE ELEMENT, and this
binds anything else the walk cannot name — asking `bracket_can_name` instead of enumerating a
fourth shape.  **A predicate that enumerates SHAPES will keep being one shape short; the question
it stands for does not run out.**

⚠ **AND IT SUBSUMED ONE OF THEM, WHICH IS THE CLAIM THIS ENTRY ORIGINALLY GOT WRONG.**  Only the
CONSTRUCTION case stays ahead in the chain, and it has a reason: a construction is HOISTED rather
than bound, for the loft#981 reason above.  The tuple arm was ahead of nothing — this general arm
precedes it, a `TupleGet` is not a `Var` and `bracket_can_name` refuses it, so the tuple arm was
unreachable from the moment this landed: **0 reaches across the 875-file corpus, and deleting it
leaves the IR byte-identical over all 875.**  Forced ahead it disagrees with the hand-written
oracle (`tuples.md` D-tup-3 has the measurement), so it is removed rather than reordered.  Neither
guard could tell: both pass with it live, dead, or gone.  **A shape arm behind a question arm is
dead code that still reads like a safety net.**

⚠ **A bind is not free of consequences, and this entry's first form paid both.** The temp takes
the CALLEE'S PARAMETER type, and a parameter declaration carries NO DEPS — so a temp holding a
VIEW read as an OWNER and the frame freed a record the CALLER still reached. `use_hash(h, true)`
then `h[7].tag` answered null; the tuple spelling answered `12884901900`; `LOFT_POISON=1` panicked
on a corrupt reference. And the arm as first placed came BEFORE the tuple-element one, so a tuple
element — not a `Var`, not nameable — never reached the arm that types its temp off the TUPLE's
own declared element type. Both are fixed here: the arm is ORDERED LAST, and its temp is
`skip_free`, because **a witness OWNS NOTHING** — something else already owns whatever it holds
(the `??`'s own work-ref on the minting arm, the container on the view arm).

⚠ **Neither checkout's matrix could see it, and the axis both pinned is worth the sentence:
every cell built its container INSIDE the function that called.** A free that should not happen
then lands on a store dying at the same scope exit, `H-FreeTwice` absorbs it as a silent no-op,
and neither the value channel nor the leak channel says anything. The general form —
**a leak channel cannot score an over-free**, because the gate is monotone and freeing MORE always
reads as an improvement — is QUALITY.md § B6k. Cells:
`kb_outlives_*` and `tb_outlives*` in the two `…-can-witness-the-bracket` guards.

**Measured.** Six cells, both backends, values IDENTICAL before and after — a pure leak, so
`LOFT_STRICT_STORES=1` is the instrument and the assertions score nothing.  A control at
`9c1a0e4e` reports `kt=79 S1105g×12` over 12 rounds; after, clean, and clean under `LOFT_POISON=1`.
Controls: the hand-written binding, a bare `Var` argument (already nameable — most calls in the
language, and re-binding it would reorder an argument for nothing), a projection the walk already
resolves, and a callee whose return does not borrow.  Guard:
`tests/scripts/1105-an-unnameable-argument-borrow-witness.loft`, scored by the wrap harness's leak
gate.

⚠ **THE TEMP TAKES THE DEPS OF THE VALUE IT HOLDS, AND THAT HALF IS LOAD-BEARING.** The bind's
type is the callee's parameter SHAPE carrying `lift_view_deps(arg)` — what the argument itself
borrows.  The parameter's DECLARED type has no deps, and a temp typed that way reads as the OWNER
of a store it only VIEWS: `get_free_vars` gives it a scope-exit free that releases the caller's
container.  `pick(h[k], …)` over a `hash` passed in as a PARAMETER then read back as `null`, and
`pick(t.0.s, …)` as another type's bytes, on both backends and under `LOFT_POISON=1` as a corrupt
dereference.  A value whose source `lift_view_deps` cannot name is NOT bound at all: a leak is the
better of the two, and it is the one that was already there.

⚠ **AND TAKING A FREE AWAY MOVES A SLOT.** The deps that stop the temp being freed also SHORTEN
its live interval — a variable with no scope-exit free is dead earlier — and the slot allocator
then hands its slot to the local the call's result is bound to. That is legal only while nothing
writes the shared slot between the temp's last write and its read as the ARGUMENT, and the join
bind wrote it first: `gen_set_first_ref_join` sentinelled its destination BEFORE evaluating the
call, so the argument arrived `null` and the borrowing arm answered the field DEFAULT
(interpreter only; `--native` gives each local its own Rust binding and never noticed).

The rule broken is the one `generate_set` already states for @P290 — *evaluate the call before
touching the destination* — and it reads as inapplicable here for a real reason: it is gated on
the RHS naming `v`, and a FIRST bind cannot name its own destination. **But the call can name a
NEIGHBOUR the allocator gave that slot to, and that is not the same question.** The sentinel is
written after the value now; `OpBindOrCopy`'s precondition is unchanged, since the slot still
holds a sentinel before the guard writes it. Guard:
`test_the_lifted_argument_survives_the_binds_own_slot`, which needs a NULLABLE-return callee to
reach the join bind at all — with a non-null return the local takes another path and the cell is
inert. It is also the only cell in that file the VALUES score: everything else there is a pure
leak.

⚠ **AND A SHARED PREDICATE IS ONLY SHARED IF ITS CALLERS AGREE ON THE ARGUMENT.** The strip and
the two emitters read ONE question (`nullable_join_first_bind`) so a strip always has a guard
under it — and they still disagreed, because `scan_set` asked it against the RAW right-hand side
while codegen only ever sees the SCANNED one. Between them sits the very rewrite this entry is
about: `scan_args` LIFTS an argument the bracket cannot name into a temp, and that temp IS the
witness the join resolves. Read before the lift the call answers *"no nameable witness"* and the
strip declines; read after it the guard goes in. The local then owned a store with no free — one
leaked record per call on the minting arm, from two readers of one predicate. It asks about
`set_value` now. **One home secures the QUESTION and says nothing about WHICH VALUE each caller
hands it; a pass that rewrites the IR sits between two readers of the same fact, and the one
upstream of the rewrite is asking about a program that will not exist.**

⚠ **AND THE AXIS THAT HID IT IS GENERAL: A LEAK CHANNEL CANNOT SCORE AN OVER-FREE.** Every cell of
the six above builds its container INSIDE the calling function, so a free that should not happen
lands on a store dying at the same scope exit — `H-FreeTwice` absorbs it and neither the values nor
the leak gate says anything.  That gate is monotone the wrong way: freeing MORE than you should
always reads as an improvement.  **So a fix that ADDS a free needs at least one cell where the
freed store OUTLIVES the frame that freed it** — the container arriving as a parameter, read back
after enough allocation to recycle a released record.  Those cells are in the guard now
(`test_a_coalesced_argument_leaves_the_callers_vector_intact`,
`test_a_keyed_argument_leaves_the_callers_hash_intact`), and they fail outright on a binary built
at `15be379a`.

### D-own-10 — CLOSED (2026-08-26, loft#1101): a BOUND projection was renamed onto the caller's buffer, and it owned nothing to rename

`(O-Move)` says a returned heap value's ownership TRANSFERS, and that a return which merely
BORROWS is copied instead.  A collection return that views another local did neither — it was
renamed onto the caller's buffer, which is the promotion ladder saying *this local IS the
buffer*:

```loft
fn f() -> vector<integer> { vv = [[11, 22, 33], [44, 55]]; e = vv[0]; e }   // answers []
fn g() -> vector<integer> { v = [11, 22, 33]; t = (v, 7); e = t.0; e }      // answers churn's bytes
```

Writing the same projection AS the tail was always correct: the gates on that path
(`return_projects_into_local`, the H12 predicate) read the tail's SHAPE, and `returns_own_field`
suppresses the rename for `return d.value`.  Its own comment names the hole — *"a local-bind
(`v = d.value; return v`) returns `v` itself (not a projection)"* — and once the projection
happens at a binding the tail IS a bare `Var`, so no shape gate can see it.

**This is `O-Proxy` read as an ownership answer.**  `fresh_owned_vector_deps` accepts a local
whose dep list is non-empty as *"a named non-argument local vector with a backing store"*, and
a view reads non-empty too.  `O-Oracle` is the fact both sites want and it is STRUCTURALLY
UNAVAILABLE here for the reason this file already records for `vector_needs_db`: the oracle
classifies a finished body from `data.def(d_nr).code`, and the parser has no def handle.  What
IS available is the sharpened reading of the proxy this file states three sections up — the
dep list carries THREE meanings, not two: empty (no store yet), a dep on the binding's OWN
mint (`__vdb_N`, which says *I own one*), and a dep on ANOTHER LOCAL (*I borrow that one*).
Closed by reading the third case as the borrow it is, citing `@FR-O-Move` / `@FR-O-Borrow`
(`Parser::var_views_local`, `src/parser/control.rs`), which leaves the candidate on `Bind` —
the copy-into-a-separate-`__retbuf` leg an owning local already takes.

⚠ **Skipping the mint is not a refinement of that reading, it is what makes it USABLE**, and
that is the entry's transferable half.  This verdict decides whether the function takes a
hidden buffer argument, so it must agree on both parser passes — the obligation
`var_bound_to_branch` states, and the one loft#1099 had just cost.  Measured: `vector_db` adds
the mint dep on pass 2 ONLY, while a borrow dep comes from the projection and is present on
both, so an owning `o` reads `[]` then `["__vdb_1"]` and a viewing `e` reads `["vv"]` twice.
Bare non-emptiness would therefore answer *owns* on pass 1 and *borrows* on pass 2 for one
body, moving the ABI between the passes.  The mint test is what collapses that to one answer.

**Two facts, because neither covers the other.**  A read out of an inline call's result
(`e = mk().items; e`) carries an EMPTY dep list: loft#882/#889 record the container dep at the
SUBSCRIPT only, leaving a bare field read to *"the delivery machinery already copies out"* —
true of the tail `mk().items`, false the moment it is BOUND.  So the second leg reads the
DEFINING STATEMENT (`var_defined_by_projection`), and it carries the same mint test, because
the vector backing rewrites an owned literal into `OpGetField(__vdb_N, 0)` on pass 2 and a
verbatim shape read called that a view on one pass and not the other.

**Measured, not reasoned.**  The filed scope was two cells; the boundary matrix found six.
Beyond the two reported: a `-> vector<T>?` return, which additionally raised an internal
`BUG (#306): refused to free the stack store`; an `if` ARM that binds the view; the explicit
`return e` spelling, which answered the RIGHT length while leaking one store per call, because
its classifier handed the promotion the local's BORROW SOURCE — `vv`, a `vector<vector<T>>`,
renamed onto a `vector<T>`-shaped buffer; and the inline-call read, which diverged, answering
stale-but-right interpreted and EMPTY on `--native`.

The IR sweep bounds the cut from the other side: **1 of 967 corpus programs emits different
bytecode, and it is this fix's own guard file** — no existing program's code moves, so the
rungs fire only on the shapes that were broken, and the corpus had no coverage of them at
all, which is why they survived.  Normalise the stdlib paths before believing a sweep: the
control worktree resolves `default/` through its own prefix, which reads as a diff in every
program and reported 29 false changes before it was normalised away.

Guard: `tests/scripts/1101-a-projection-bound-to-a-local-then-returned.loft` — fifteen cells,
five of them falsified on a HEAD-built control binary, and the file is scored on both backends
plus the wrap leak gate (the explicit-return cell is the one only the leak gate can fail).
Controls: the tail projection, an owned literal and an owned build (which must KEEP the
rename — refusing it is the over-fire a bare-emptiness reading produces), the issue's
copy-out workaround, an ARGUMENT-rooted projection (the caller owns that store, so the view
outlives the call), a copied rebinding, and the RECORD twin, which reaches its own view repair
through `classify_reference_delivery`.

⚠ **The structural leg resolves a projection by OP NAME, so it cannot see a `TupleGet`
spelling.**  `expr_borrows_local` matches `OpGetField` / `OpGetVector`; a tuple element read
that lowers to `Value::TupleGet` reaches none of them.  It is latent rather than live — five
tuple spellings (`t = (v,7); e = t.0`, `e = mk_tup().0`, `t = mk_tup(); e = t.0`, the explicit
return and the tail) all answer correctly on both backends, because the DEPS leg covers each of
them — but the two legs are not interchangeable, and a future tuple shape carrying no dep would
fall between them exactly as `e = mk().items` did.  `scripts/ir_walker_audit.py spellings`
counts this class repo-wide: 18 functions resolve a projection by op name and 2 handle the
tuple spelling.

⚠ **A stale `target/release/loft` is not a control.**  It answered the guard file GREEN — not
because the shapes were fixed there, but because it predates the code under test; a
freed-then-reused store also depends on the build's own allocation pattern, so a binary that
merely *looks* older can report either verdict.  The control has to be BUILT from the commit
under test (`git worktree add` + a separate `CARGO_TARGET_DIR`).

### D-own-9 — CLOSED (2026-08-26, loft#1096): a COLLECTION return's promoted buffer is the CALLER's store, and the callee freed it

`(O-Owner)` says a free is for a store the value OWNS.  A `-> vector<T>` function whose tail
may deliver `null` freed one it did not:

```loft
fn f(n: integer) -> vector<integer> { if n == 0 { null } else { [n] } }
fn main() { t = 0; for i in 0..2 { r = f(i); t += len(r); } }
```

`ref_return` renames the value arm's backing ref `__vdb_1` onto the hidden `__retbuf`, and
`scopes::free_vars`' loft#688 leg then enrols that renamed argument as *a local this function
minted* and emits `OpFreeRefIfDistinct(__vdb_1, __ret_1)`.  On the null arm the witness is
the sentinel, the store numbers differ, and the free fires — on the CALLER's store.  The
caller still names it (`__ref_1`, freed at its own scope exit) and still passes it to the
next call, whose entry `OpClearVector` then reads a freed record: `rec=0xDEADBEEF` under
`LOFT_POISON`, on BOTH backends, from the second iteration on.  One call is clean, which is
why it took a loop in the poison corpus to surface it — a newly-armed guard reporting old UB.

**The premise that failed is written in the leg's own comment**: *"a buffer not yet minted on
this path is the null sentinel, which `free` ignores."*  That is true of a RECORD return,
whose caller-side work-ref reaches the call as a bare `OpInitRef` sentinel.  It is false of a
collection: `codegen::gen_set_first_vector_null` gives an owned vector work-ref `OpInitRef` +
`OpDatabase`, so the buffer arrives ALIVE — and the callee's own `OpDatabase` then reuses that
store in place (`alloc_record_at` clears and re-claims a live slot rather than minting beside
it), so there is never a distinct callee-minted store for this free to reclaim.  Closed by
excluding a collection return from that leg, citing `@FR-O-Owner` (`src/scopes.rs`).

**Measured, not reasoned:** the free was emitted in **44 of ~1000** corpus programs and
FIRED 375 times across 20 of them, so removing it is live rather than theoretical; every one
of those 44 is green under `LOFT_POISON=1` + `LOFT_STRICT_STORES=1` with no leak, which is
what says the store it reached was always the caller's.  Emitted IR is otherwise identical:
the whole diff is the `__ret_N` hoist and the free it existed to carry.

⚠ **The fix is NOT at the promotion, and that corrects an extrapolation this file invited.**
D-own-8's Face B (loft#1081) closed *"at the promotion, which is where the unsound step is"*,
and the loft-codegen skill's loft#1096 note reads that line as naming where THIS fix belongs.
Refusing the rename was built and measured, and it is the wrong cut twice over:

* it **over-fires**.  The gate has to be *"the tail may deliver the sentinel"*, and a `match`
  with no catch-all lowers its fallthrough to exactly that sentinel — so an ordinary
  `match e { A { xs } => { xs }, B { ys } => { ys } }` loses its NRVO and copies each arm
  through a second store (`tests/use_analysis.rs::ownership_pins_match_return_resisting_cases`
  is what caught it).
* it needs a **second half** to stay correct.  Dropping to `Bind` copies the WHOLE tail into
  the buffer and answers the buffer on every path, so the null arm delivered an EMPTY vector
  instead of the sentinel and `f(0) == null` read false — loft#936's contract, broken by the
  repair for loft#1096.

The rename is sound here: building into the caller's buffer is the NRVO the design wants, and
it produces correct values with no leak.  What was unsound is the free that read the rename as
*minted here*.  **A closure names where ITS unsound step was; the next defect in the same
machinery has to be measured, not inherited.**

Guard: `tests/scripts/1096-a-null-return-must-not-free-the-callers-buffer.loft` — twelve
cells, five of them falsified on the pre-fix binary under `LOFT_POISON=1` (and none without
it: the freed bytes are still usable, so the poison job is what scores this file).  Both
halves are pinned, because neither implies the other — a fix that only refuses the rename
passes every use-after-free cell and fails `the_null_arm_still_answers_null`, and a fix that
only changes the delivery still faults on cell one.  Controls: the `[]` arm (the issue's
workaround, which keeps its rename), the RECORD family (which keeps rename AND free), and a
join with no null arm at all.

⚠ **A second defect found by the same probes — and the SAME dead premise, one site over.
Closed as `calls.md` D-call-3 (loft#1097).**
`fn g(k) -> vector<integer> { a = [1,2]; if k < 0 { null } else if k == 0 { a } else { [k] } }`
answered `g(0) == []`; dropping the null arm answered `[1,2]`.  The null arm makes `__vdb_1` a
second promotion candidate, which takes `Bind`'s whole-tail copy —
`OpClearVector(a); OpAppendVector(a, <the join>, 0)` — and `a` IS the promoted buffer, so the
clear runs before the join is evaluated and the `k == 0` arm answers the buffer it just
emptied.  That is loft#1078's *"the re-mint destroys the store the copy is about to read"*
with a CLEAR in place of the re-mint, and `classify_ret_promotion`'s `tail_reads_buffer`
guard against exactly that shape is RECORD-only.  Both backends agree, so backend agreement
is again not an oracle.

**And the null arm of that same tail never reached the caller.**
`returned_var_null_unified` folds a null arm onto its sibling's var on the belief that the
var holds the sentinel on the null path — *the same belief this entry's free leg holds, at a
different site*.  For a RECORD it is true; for a collection buffer it is false in both
places, and it cost a use-after-free here and a wrong value there.  **One wrong belief, two
defects, one day apart** — so grep the belief, not the symptom: any site reasoning that a
collection slot holds the sentinel on a path that did not write it is suspect.  Both are
fixed; `calls.md` D-call-3 carries the return half.  What is left is loft#1098, a per-call
leak on a `match` tail that needs a null arm, a local arm and a literal arm all three.

### D-own-8 — OPEN (2026-08-24, loft#1082 / loft#1081): a Join's ownership fact is true on one path only

`(O-Complete)` requires the fact PER BINDING, PER PATH — "every binding, including every
`match`/`if` arm".  A join whose arms disagree produces ONE fact for BOTH paths:

```loft
line = if len(pts) > 2 { smooth_pts(pts, flags, false) } else { pts };
```

The then-arm is a call returning a freshly-owned vector; the else-arm is a bare local.
`LOFT_VAR_TABLE` shows the binding typed `def deps=[pts]` — a BORROW.  On the owning path
the fact is false.

⚠ **The mechanism this entry originally named is NOT the one in play, and that was
falsified 2026-08-25.**  It read: *"`arm_join_type` strips only the deps an arm MINTED
(loft#978), and `joined_deps` then UNIONS, so `{} ∪ {pts}` = `{pts}`."*  Two probes against
the `if` shape above — the entry's own example — say otherwise:

* removing the strip from `arm_join_type` entirely leaves the binding's deps **unchanged**;
* a tracer in `arm_join_type` **never fires** for this shape at all.

The `if`-expression joins through `merge_dependencies(&true_type, &false_type)`
(`parser/control.rs`), which is a different path; `arm_join_type` serves the `match` arms.
So an attempted fix aimed at `arm_join_type` — the obvious reading of the old text — would
have changed code that never runs for the reported program.

**The real mechanism, found 2026-08-25 by instrumenting `merge_dependencies` and then
`LOFT_LOG=type_timeline:line`.**  The union is computed CORRECTLY and is then collapsed by a
setter that replaces where its caller assumes it accumulates:

```
[MD]  a=[5] b=[0] -> [5, 0]                    merge_dependencies: the union is RIGHT
[type_timeline] line  Unknown -> [5, 0]        change_var_type stores both
[type_timeline] line  [5, 0]  -> [5]           depend  (variables/mod.rs:1797)
[type_timeline] line  [5]     -> [0]           depend  — last one wins
```

(var 5 is `__ref_1`, the owning arm's mint dep; var 0 is `cp`.)

`Function::change_var_type`'s early-return does

```rust
for on in type_def.depend() { self.depend(var_nr, on); }
```

and `Function::depend` is `Type::depending(on)` = `with_deps(&Deps::frame1(on))` — it
REPLACES the whole list with `[on]`.  So a type carrying N deps collapses to its LAST one.
**Six sites in `variables/mod.rs` share that loop.**

**It is not join-specific, and that is the wider finding.**  A two-BORROW join loses one
source outright:

```loft
pick = if c { a } else { b };   // pick def deps=[b] — the dep on `a` is gone
```

`pick` aliases `a` on the taken path, and nothing records it.

⚠ **Still no symptom.**  The two-borrow shape was probed with the dropped source going out
of scope first, under `LOFT_POISON` + `LOFT_STRICT_STORES`, and answers correctly — so
something downstream keeps the dropped source alive.  The collapse is a real defect in the
FACT with no demonstrated consequence, which is the same position Face A has always been in,
now stated one layer deeper and at the right function.

**FACE A NOW HAS A SYMPTOM (2026-09-03, loft#1320), and it is a LEAK rather than a wrong answer.**  The
entry above says *"a real defect in the FACT with no demonstrated consequence"*; the
consequence is what a binding joined from TWO arms does with the store the minting arm handed
it.  `r`'s type carries BOTH arms' deps, so `dep.is_empty()` — @FR-O-Proxy — reads it as a
borrow, `get_free_vars` emits nothing, and the mint arm's store is owned by nobody.

```loft
g = fn(q: vector<integer>?) -> vector<integer> { q ?? [7, 8] };
for i in 0..500 { r = if i % 2 == 0 { g(some) } else { g(none) }; s += r[1]; }
```

`r(5):vector<integer>["none", "some"]` — the union is right, and being right is what suppresses
the free.  246 live stores at N=500, both backends.

**The matrix says it is a JOIN-ARITY question, not a container-kind or a call-spelling one.**
Each row measured on both backends, values correct throughout:

| the call in the arm | `vector` | a struct |
|---|---|---|
| fn-ref `q ?? default` | leaks, 246 stores | leaks, 245 stores |
| NAMED fn, same body | clean | leaks, 250 records in ONE store — at PROGRAM exit |
| one arm a pure mint (so ONE dep) | clean | — |
| both arms borrow | clean (nothing to free) | — |
| value CONSUMED inside the arm, never escaping | clean | — |

Two of those cells are worth their own reading.  **The single-dep row is the boundary**: where
the join names ONE base the existing machinery already answers, and the failure starts at TWO.
And the record/NAMED cell is the only one visible on the program-exit leak channel, so it is
the only one the script corpus's absolute leak gate could ever have caught — the other three
are freed at FRAME exit and need `LOFT_ALLOC_SITES=1` at the peak.  ⚠ It reads as an
interpreter-only defect and is not: `--native`'s leak check is OFF by default, and
`LOFT_NATIVE_LEAK_CHECK=1` reports the identical 250 records.  Present in released 2026.8.0.

**FACE A's SYMPTOM CLOSED (2026-09-03, loft#1320) — by giving each path a binding, not by an
N-witness free.**  Every qualifying arm tail `g(x)` of a value branch is rewritten into the BOUND
spelling on a temp homed where the joined binding lives — `{ __lift_N = g(x); __lift_N }` — so
the join borrows from the temps and each temp answers for ONE arm with ONE base: a collection
temp keeps that base as its dep and is freed by store identity against it (loft#1257's route,
`OpFreeRefIfDistinct`), a record temp owns unconditionally through `OpBindOrCopy` (loft#1248).
Three arms need nothing N-ary.  The same rule, applied at the bind, also closed the BOUND vector
spelling in a loop (`t = g(none)`, 470 stores at N=500), which `1257b-…` had recorded as covered
and had measured only on the borrow arm.  Reached in three statement positions, each its own
site: a `Set` RHS (`scan_set`, temps homed in the BINDING's scope — homed in the statement's
they were freed under a binding declared outside the loop, a use-after-free the poisoned build
named), a keyed branch (`OpReplaceKeyed(if …, r, tp)`, seen in `scan`) and a branch consumed as
a call ARGUMENT (`scan_args`).  Flat at 3 stores on both backends for `vector`, every keyed
kind, a struct, text / struct / nested elements, a field and an element argument, an argument
base one frame up; `LOFT_POISON` and `LOFT_STRICT_STORES` clean on every borrow-direction cell.

Two shapes are DECLINED on purpose and each is a cell asserting the value only.  A named local
bound at two sites from two DIFFERENT bases gets no witness — one static witness cannot answer
for a store the OTHER site handed it, and freeing on the wrong one released a caller's store
(measured: sum 4034 for 12500 before the gate).  And a base assigned at more than one site in
the function is not offered as a witness, because at scope exit it may name a store already
gone.  Both keep the leak they had.  ⚠ The `callref_join_bases` gate is computed off the RAW
body before the scan, because a conflict found at the second Set cannot retract the free the
first Set already emitted.

⚠ **Under `LOFT_STRICT_STORES=1` the outer-declared cell fills the store table at 70 000 with no
violation reported.**  Not a defect in the free: the callee frees its own mint under two names
(`_vec_1` and `__vdb_1` are one store — loft#1322, pre-existing, a no-op in every mode), and
strict's slot reuse then differs from the plain allocator's on that one shape.  Plain and
poisoned runs are flat, and the corpus is never run under strict.

Guard: `tests/scripts/1320-a-branch-joined-binding-frees-the-arm-that-minted.loft`, 11 cells,
falsified at e949f943 (interpret exit 101 -> 0, native exit 1 -> 0, both panicked -> clean).

**What closing it needs.**  @FR-O-Proxy's *"empty deps means owned"* has a runtime form that is
sound for exactly this shape — *free iff the store is distinct from EVERY variable the deps
name* — which is @FR-O-Oracle's own per-execution sentence for a `Join`, generalised from one
witness to N.  loft#1257 shipped the N=1 case (a lifted collection return, `OpFreeRefIfDistinct`
against the single base).  N>1 has no op: every free op in `01_code.loft` takes ONE witness.
The two candidate cures are (a) give a `CallRef` in an arm the caller-side `__ref_N` buffer a
direct call gets, so the deps name reusable BUFFERS rather than caller variables — which is
exactly why the vector/named row above is clean — or (b) an N-witness guarded free.  (a)
follows the precedent already in the tree and is the recommendation; it is also the one that
changes call-site arity for a resolved fn-ref target, which `parser/mod.rs`'s
`h5_has_lowered_caller` deliberately avoids today, so it wants a design pass rather than a
patch.

**CLOSED 2026-09-03 — neither cure above; the structural one, widened to every arm KIND.**
loft#1320's own principle closes the rest: *"give each path a binding"* was written for a
fn-ref `??` arm, and the residual was every other arm whose SINGLE bind would leave the
binding owning a store while the join read the arm as a borrow.  `scopes.rs::arm_bind` now
answers for the whole table, and the temp is always bound by the single bind's own lowering —
nothing re-derives a copy or an adoption:

| arm tail | before (both backends) | now |
|---|---|---|
| fn-ref call, `Join` | lifted (loft#1320) | unchanged; a multi-assigned base no longer declines it |
| fn-ref call, `Owned` (`m(0)` beside `cp`, or beside `g(some)`) | store table exhausted at 140 000 | owned temp, freed |
| fn-ref call, `Borrowed` record (`h(bag) ?? d`) | interpreter VIEWED, native COPIED and leaked | owned temp; codegen's `callee_of` arm copies on both |
| fn-ref call, `Borrowed` collection delivered into the call's buffer (`{ q.items }`) | leaked one store per call, even at a plain bind | owned temp / the plain bind's deps stripped (`callref_delivers_collection`) |
| fn-ref call, raw keyed or index VIEW (`{ q.m }`, `{ w[0] }`) | — | never lifted, never freed (see the witnessed-lift regression below) |
| named call, record `Borrowed` / `Join` (`get(bag) ?? d`, loft#1323's `d(some)`) | interpreter viewed / 250 records accumulated in one store | owned temp; codegen copies |
| named call, record `Owned`, or any named collection | clean — the caller's `__ref_N` buffer is the owner | unchanged |
| a plain VARIABLE (loft#1321), local / parameter / loop element — for a binding the join is the ONE assignment of | ALIASED the arm | record: `{ __lift_N = x; __lift_N }`, copied at the bind; vector: refilled into a function-scoped buffer by `OpReplaceVector` |
| a plain VARIABLE where the binding is ALSO assigned elsewhere as an owner (`r = x; for … { r = v[i] ?? x }`) | the runtime join bind (`OpBindOrCopy`) copies for a record; a vector's arms are materialised into the local's own store | unchanged — lifting there turned one binding's fact into a borrow at every Set and orphaned the plain copy (`85-runtime-join-loop-copy-view` said so, one store per call) |
| a `??` hoist of a projection (`vv[0] ?? [0]`) | view | unchanged — `(B-View-Depth)`'s own spelling |
| a literal / comprehension | owned by its per-site `__vdb_N` / `__ref_p2_N` | unchanged |

**The joined binding's dep list now names the temps** — for a binding the join is the one
assignment of.  The variables the arms copied are removed from it where no other arm still
reads them, and a `??` hoist an arm hands back is added — so `LOFT_VAR_TABLE` reads
`line def deps=[__lift_1, __vdb_2]` for the shape this entry opened with, which is true on
both paths.  A binding assigned elsewhere keeps the parser's fact, because a type-level list
carries ONE fact for every Set of the variable (`(O-Latest)` is the rule that says why), and
the runtime join bind already answers for it.  The `match` spelling of the literal-mint
arm still drops the `__vdb_N` dep (`arm_join_type`'s loft#978 strip) and that is the one cell
where the two spellings' FACTS still differ; its consequence was measured absent a fourth time
(the buffer is function-scoped and reused in place) and it stays as a note, not a deviation.

**The two declined shapes close by a witness SNAPSHOT, not a witness slot per binding.**  A
collection local bound from a fn-ref `Join` is freed by identity against the store its base
named AT THE BIND.  Where the base variable still names that store at every later free —
assigned once, one base per local — it is the witness, as before.  Where it does not — the
base is reassigned in the function, or the local is bound at two sites from two bases — a
`__wit_N` slot (one per local, a never-freed borrow of the base's type) is written beside each
bind from that bind's base, after the transition free and before the value is computed, and
both frees compare against it.  Two stale numbers still agree and decline, which is what makes
a base RE-MINTED while the borrower is live safe (measured with a fresh allocation between
passes so a wrong free would be reused and read back).  This is `(O-Latest)` — the fact
belongs to the assignment — carried the way @PLN87's entry stash already carries a
rebindable PARAMETER's, which is also why loft#1320's parameter-base cells were clean all
along.

**The `??` hoist temp owns a CALL subject.**  `parser/operators.rs` marked every record
`__ncc_N` never-free, on the reading that the join's binding would own what the block handed
it.  It never did: `r = g(none) ?? d` held one record per call to frame exit on the
interpreter, and `r = mk(i) ?? d` was clean ONLY because `r` freed on the mint arm a store it
merely borrowed on the other — the same one-path fact this entry is about.  A call subject's
hoist now owns what a plain bind of that call would (a fresh mint adopted, a borrowed or
`Join` return deep-copied by codegen, a fn-ref's answered by `OpBindOrCopy`), and releases its
previous store in the IR before the re-bind, so `--native` — which does not release a
displaced store on a fn-ref re-bind of a user local (loft#1328, pre-existing) — stays flat
too.  A projection subject stays the view it is.

**Two regressions the first cut introduced, and what each taught.**
* *A variable arm in a call ARGUMENT was copied.*  `c = maybe_b(c ?? M {}, i)` (D-own-16's
  own guard) then handed the callee a temp that died at the statement while `c` still named
  it — a refused free on the interpreter, `0` for `8` on native.  An argument ALIASES the
  caller's variable (calls.md F-ParamHeap); the rewrite in argument position now lifts calls
  only.  The lesson is the peer session's from loft#1318 the same afternoon: statement
  CONTEXT is a live axis, and a cell written in the wrong one is vacuous.
* *The named-owned `??` subject leaked once `r` stopped owning it.*  The buffer's exit free
  was CONDITIONAL on the hoist no longer naming it (loft#1317's pairing), which is sound only
  where someone else releases the store — the hoist itself, or the caller when the hoist or a
  binding that borrows it is what a return hands out.  A never-free hoist that is not handed
  out leaves that free as the store's sole release, so it is plain there.
* *A lifted nullable record temp tripped the stack-store net at exit.*  Its preamble
  `Set(tmp, Null)` reached `gen_set_first_at_tos`'s Reference/Enum arm asked BARE, fell to the
  generic fallthrough, and held `Stores::null()` (a real slot, `rec == 0`) instead of the
  sentinel.  Peeled — the same class as the D-layout rows; user code never reaches it because
  a `P? = null` is parsed to `OpNullRefSentinel`.

**A FALLBACK IS NOT A VERDICT — the defect this closure introduced, found and fixed inside
the branch (2026-09-04).**  `use_analysis`'s `CallRef` arm answers `Own::Owned` for a base it
cannot NAME, and its own doc already says readers must not take that at face value.  The arm
lift read it as one.  The shape was unreachable until loft#1329 made a captured fn-ref
resolvable, and then a FORWARDING lambda —

```loft
inner = fn(q: vector<integer>?) -> vector<integer> { q ?? [7, 8] };
fwd   = fn(q: vector<integer>?) -> vector<integer> { inner(q) };
for _ in 0..4 { r = if true { fwd(c) } else { fwd(none) }; t += r[1]; }   // len(c) == 0
```

— reached its own return through `__closure`, so the summary lost the base, answered `Owned`,
and the arm took an UNWITNESSED free of the caller's collection.  Two iterations empty the
source while the value still reads right, so a values-only cell cannot see it; and the
CONTROL for it has to be loft#1329's build, because at 26d17f4b the target does not resolve
and the cell passes for the wrong reason.

⚠ **Declining the lift on that fallback is NOT the cure, and measuring is what said so**: it
closes the over-free and then leaks worse than the release — 70 000 forwarding mint arms
exhaust the store table where 2026.8.0 is flat.  Trading a silent wrong answer for a leak the
shipped compiler did not have is not an improvement.

The cure is the fact the summary lost, which the callee still DECLARES: `fwd`'s type reads
`vector<integer>["q"]`, naming the visible parameter its return borrows.
`callref_declared_borrow_base` maps that dep to the caller's argument through the SAME
`caller_arg_base` a resolved base takes, and `callref_collection_join_base` asks it wherever
the oracle answered the fallback — so one identity free serves both answers rather than two
mechanisms serving one question.  Both directions measured: the borrow arm keeps the source,
the mint arms are flat at 70 000, on both backends, for a vector and a record and for the
no-branch rebind.

**Found beside it, filed, not fixed here.**  loft#1327: a fn-ref whose target cannot be
RESOLVED — a fn-typed parameter — reads `Owned` at the oracle's fallback and is typed owned by
the parser (the fn TYPE's return carries no deps), so `u = g(a)` inside `fn plain(a, g)` frees
the caller's collection on the borrow arm; present on 2026.8.0, `silent-wrong`.  loft#1328:
the native re-bind release gap above.  And a regression closed on the way (guard
`1245b-a-witnessed-lift-does-not-free-a-keyed-view`): the witnessed lift loft#1245 opened
licensed a free for every fn-ref return whose argument set was witnessable, and a collection
answered as a raw keyed VIEW has no guarded release — `t = h(bag)` with `h = fn(q) { q.m }`
emptied `bag.m` after one call on both backends, every answer still right.  The witnessed
route now reaches a record (the bracket refuses the source-free) or a collection `Join` (freed
by identity) and nothing else.

Guards: `1323-every-arm-of-a-value-branch-has-its-own-binding.loft` (every arm kind, the two
declined shapes, the hoist, the controls one axis away; falsified at 26d17f4b — both backends
exit 101/1 → 0, `store table exhausted` → clean), `1321-a-joined-binding-copies-what-a-plain-
bind-copies.loft` (the copy face; falsified at 26d17f4b, one assertion each backend) and
`1245b-…` (falsified at 26d17f4b).  The corpus: the scopes subject suite and the full suite
green; `1085b-…` (D-own-16) red under the first cut and green under the second, which is the
measurement that named the argument-position axis.



**FIXED 2026-08-25 — `Function::depend_all`.**  All six sites now route through one setter
that keeps every incoming dep instead of the last:

```rust
fn depend_all(&mut self, var_nr: u16, type_def: &Type) { … }   // variables/mod.rs
```

Two properties make it a replacement rather than a widening, and both are guarded in
`variables::loop_binding_dep_tests`:

* a value borrowing two sources records BOTH (`a_binding_that_borrows_two_sources_records_both`);
* an EMPTY incoming list is a **no-op, not a clear**
  (`an_empty_incoming_dep_list_does_not_clear_the_established_ones`) — the loop it replaces
  did nothing for an empty list, and *"the types agree, adopt the deps"* never meant *"and
  drop what you had"*.

It adopts the incoming list WHOLE; it is deliberately **not** a `Deps::union` with the
variable's existing deps.  The loop replaced, so replacing-without-collapsing is the minimal
change that fixes the loss and nothing else.  For the same reason it keeps dropping the
`u16::MAX` share-marker (#328), which `depend`'s own guard has always skipped: two downstream
decisions read that marker's presence (a struct field's layout via `deps.contains(&u16::MAX)`,
and `deps == [u16::MAX]` as a predicate of its own), so carrying it through would have changed
answers well outside this defect.

⚠ **Guarded on the predicate, not through a program, and the reason is the entry above:**
the collapse has no observable symptom, so a script-level guard would assert nothing while
the fact is plainly wrong — and the fact is what every free-placement decision reads.  The
two tests above it in that module carry the same disclaimer for their own reasons.

Measured on the two shapes this entry names:

```
[vartable]  pick  vec<ref>  def deps=[a(0), b(2)]        ← was [b]
[vartable]  line  vec<ref>  def deps=[__ref_1(5), cp(0)] ← was [cp]
```

**Sibling audit — the class, not the site.**  The collapse is *a loop over a dep list whose
body calls a REPLACING setter*, so it was swept as that shape rather than as six known lines.
21 loops in `src/` iterate a dep list; 18 only read it.  The other **three** had the identical
defect and are fixed with the same `depend_on_all`:

| site | shape |
|---|---|
| `parser/expressions.rs` (@PLN85 cluster V) | save/restore around `change_var` — **strips every dep in a correct loop and restores only the last** |
| `parser/vectors.rs` | an element binding adopting its parent's deps |
| `parser/objects.rs` | a temp adopting the written value's deps |

The first is the one to notice: its SAVE side loops over the whole list and its RESTORE side
collapses, so the asymmetry was visible in the same six lines the whole time.  **And the union
fix makes these siblings more dangerous rather than less** — multi-dep lists were previously
rare *because* the six sites kept flattening them, so fixing the six is what puts lists of
length > 1 in front of the other three.  A class-wide sweep was therefore a precondition for
the fix being safe, not a tidy-up after it.

**The blast radius is bounded by a property worth checking rather than trusting.**  Writing
`n` for the incoming list's length after the `u16::MAX` filter:

| `n` | before | after |
|---|---|---|
| 0 | no-op | no-op |
| 1 | `[d]` | `[d]` |
| ≥ 2 | `[last]` | **`[all]`** |

Before and after are non-empty in exactly the same cases, so **`depend().is_empty()` answers
identically at every site**, and the fix changes *which* deps a value carries — never
*whether* it carries any.  That matters because at least three decisions read that predicate
AS AN OWNERSHIP TEST (`vector_needs_db`, `classify_vec_bind`, and the `[]`-means-owner reading
in `minted_vars`), and each is measured load-bearing: neutralising the first breaks
`tests/scripts/03-text.loft`, and inverting the second corrupts (#426).  None of them can move
under this change.

**How often the collapse actually fired, and where — this is the part that reflects on the
register itself.**  Counted with one env-gated `eprintln` on the `n >= 2` arm, over all 858
corpus programs: **48 events in 12 files** (47 of two deps, one of three).  So the fix is live
rather than theoretical — the corpus was dropping a dep 48 times — and the whole suite passes
either way, meaning nothing had come to depend on the collapse.

The 12 files are the finding.  Almost every one is a regression guard written for an EARLIER
ownership deviation:

```
11  h7-loop-retbuf-alias        7  1081-a-join-bound-to-a-returned-local
 9  848-value-block-local…      5  1051-tuple-destructure-ownership   (the 3-dep case)
 2  981-split-ownership-return  2  139-drop-cascade
 1  1019-join-owned-arm-owner   1  172-store-confinement-soundness
```

Those guards were passing while the fact underneath them was incomplete — which is what
[README § deviations](README.md) means by an `OPEN: 0` line being only as strong as its oracle.
They still pass now that the fact is complete, so none of them was ever *scored* on the dep
list; they were scored on values, and the collapsed list was invisible to them.  Treat the
earlier D-own zeros accordingly: they were measured over a corpus in which multi-source deps
could not survive to be measured.

⚠ `vectors.rs` keeps one pre-existing behaviour deliberately: a `self.vars.depend(elm, vec)`
immediately above is still overwritten when the parent carries deps.  Whether `elm` should
depend on BOTH `vec` and the parent's list is a separate question this fix does not answer —
adopting the parent list whole changes only what the loop lost.
**One fact, two questions — and it is only right for one of them.**  `joined_deps`'
own doc-comment justifies the union as the reading "no arm can contradict: it can only
keep a store alive longer than one arm needed, never free one another arm still holds."
That is true of the question the union was written for — *what must stay alive?*  It is
false of the OTHER question the same `deps` list answers — *does this binding need a
backing store of its own?*  `vectors.rs::vector_needs_db` asks it as
`self.vars.tp(vec).depend().is_empty()`, so a union that is non-empty says "borrows,
needs no store" about a value that OWNS on the path that runs.  Conservative for
liveness is anti-conservative for allocation.  Two named hazards meet here: an empty dep
list read as *owned*, and one derived fact with two homes.

**Face A — NARROWED 2026-08-25 to one cell, by the `depend_all` fix above.**  The entry
below predicted the closure would need *"a lowering change — making the join's own result
carry a mint dep"*.  It did not: the lowering **already produced** that mint dep, and the
collapse was discarding it.  With the collapse fixed, the filed shape reads

```
pf_line  def deps=[__ref_1(10), pf_cp(7)]   ← was [pf_cp]
pf_wids  def deps=[__ref_2(12), pf_cw(8)]   ← was [pf_cw]
```

— the owning arm's mint marker beside the borrowing arm's dep, which is exactly what
`arm_join_type`'s own comment calls *"what says which store the result owns"*.  The fact is
no longer true-on-one-path-only for this shape.

⚠ **One cell survives, and it makes the two spellings DISAGREE.**  Varying the owning arm
between a CALL and an INLINE mint, across `if` and `match`:

| owning arm | `if` | `match` |
|---|---|---|
| a call (`smooth(cp)`) | `[cp, __ref_1]` ✓ | `[cp, __ref_1]` ✓ |
| an inline mint (`[for v in cp {…}]`) | `[cp, __vdb_3]` ✓ | **`[cp]`** ✗ |

Values are correct in all four cells; only the FACT differs.  The `match` row is
`arm_join_type` stripping the contributed arm's minted vars — which is why the call cell
passes for the wrong reason: `minted_vars` **cannot see a mint inside a callee**, so the
strip finds nothing to remove and the union survives by accident.  Move the mint into the
arm and the strip engages.

That strip is deliberate (loft#978: publishing an arm's mint as a dep made the return
machinery read the result as a view of a local, and `deliver`'s return went to `["??"]`), so
removing it trades this deviation for that one.  It is the entry's *"one derived fact, two
homes"* hazard in its sharpest form: the strip is RIGHT for the delivery question and WRONG
for the ownership question, and one dep list answers both.  **Face A stays OPEN for the
inline-mint `match` arm only**, pending a way to separate those two readings.

**2026-08-26 (loft#1098): the surviving cell's SYMPTOM is closed; the FACT is not, and the
two are worth keeping apart.**  The cell had a consequence after all, at the RETURN position:
because the stripped mint never reaches `ls`, the arm that minted it is never a promotion
candidate, so nothing ever DELIVERS it into the caller's return buffer.  It is returned
instead, the callee's `OpFreeRefIfDistinct` sees the store it is about to return and keeps it,
the caller's binding is typed as a borrow of ITS buffer and frees that, and one store orphans
per call — the 65,535-entry exhaustion class.

⚠ The dep list still reads `line def deps=[cp]` for an inline-mint `match` arm where the `if`
spelling reads `[__vdb_1, cp]` (re-measured with `LOFT_VAR_TABLE` after the fix), so
`(O-Complete)` is still not satisfied for it and **Face A stays OPEN**.  What changed is that
its one known symptom is gone, which puts the cell back in the position the entry describes
above: a false fact looking for a symptom.  The BOUND-local shape the register reduces it to
(`line = match … { 0 => cp, _ => [for v in cp {…}] }`) was probed at 200 rounds under
`LOFT_POISON=1` + `LOFT_STRICT_STORES=1` on both backends and is correct with no leak, so the
next symptom, if there is one, is not there.

**The cure does not need the two readings separated after all, and that is the finding.**  The
strip's harm is that ONE arm's store goes undelivered; delivering every arm removes the
question rather than answering it.  `block_result`'s #416 per-arm materialiser already does
exactly that, and was excluded for a tail with a direct `null` arm — an exclusion written for
the return TYPE (64bd0984: *"materializing would set `returned = Vector[__retbuf]` on a path
that yields null, which native cannot represent"*), which @PLN25 had already relaxed for a
DECLARED-nullable return.  The rule that replaces it is the one the promotion itself states:
**at most ONE arm can BE the buffer**, so a tail with two or more value arms must materialise
the rest.  One value arm keeps its rename and its NRVO; two or more take the per-arm delivery.

**The fix is at the DELIVERY, not at the fact, and that is what makes it small.**

**Measured, and the filed scope was a third of it.**  The report needed a null arm, a LOCAL
arm and a LITERAL arm together.  Sweeping the arm KINDS against the arm COUNT over `-1 =>
null` tails says the local arm is not required and the count is:

| tail | before |
|---|---|
| `-1 => null, _ => [k]` | clean — the rename covers the one value arm |
| `-1 => null, 0 => [7], _ => [k]` | **one store per call** |
| `-1 => null, 0 => a, _ => [k]` | **one store per call** (the filed cell) |
| `-1 => null, 0 => [7], 1 => [8], _ => [k]` | **one store per call** |
| `-1 => null, 0 => a, _ => b` | clean |
| `-1 => null, 0 => [7], _ => a` | clean |

The two clean multi-arm cells are clean by other routes, not by the rule, so they are controls
rather than evidence — they now take the per-arm delivery like the rest.  The `if`-chain
spelling of every cell was clean throughout, because `if` and `match` reach this tail through
different legs; a sweep of one spelling would have found nothing.

Emitted IR over the corpus: **1 of 898** programs changes, and the change is one duplicated
entry `OpClearVector` removed.  So the gate was live for exactly the shape it was written for
and nothing else, and the NRVO on the single-value-arm shape — loft#1096's own — is untouched.
Guard: `tests/scripts/1098-a-null-arm-tail-with-two-value-arms.loft`, falsified on a pristine
tree at `0df2ca45` by the wrap leak gate (1198 stores).  ⚠ `loft --tests` on that file alone
does NOT falsify it: the leak gate lives in `tests/wrap.rs::run_test`, so `cargo test --test
wrap loft_suite` is what scores it.

A residue and a sibling, both filed: the **`text`** family aborts before it can be measured at
all (loft#1099, an H5 two-pass ICE on `-> text` + `match` + a null arm + a local arm), and the
`-> vector<T>?` spelling still leaks one store on `--native` (loft#948).

**Face A — the allocation answer (the original statement).**  A borrow-typed slot owns no store, so a
whole-value assignment into it has nowhere to land.  The false fact reduces to ~55 lines — a
`for` over a vector of structs whose vector fields are copied into locals, then the mixed join
— reproducing `pf_line def deps=[pf_cp]` and `pf_wids def deps=[pf_cw]` exactly as filed.
No wrong answer or crash is yet attributed to it; it is a false FACT looking for its symptom.

**Symptom hunt, 2026-08-25 — the fact REACHES its site, and still nothing breaks.**
Re-reproduced in ~20 lines (`LOFT_VAR_TABLE` shows `line def deps=[cp]` for
`line = if len(cp) > 2 { smooth(cp) } else { cp }`).  Instrumenting `vector_needs_db`
confirms the decision is reached and answers with the false fact: `[VNDB] line deps=1 →
false`, i.e. *no backing store allocated*.  So these probes are not vacuous — they arrive at
the named site, take the branch the false fact selects, and are still correct:

| probed shape | result |
|---|---|
| whole-value reassign into the joined slot (`line = other()`) | correct, source intact |
| build a comprehension into it (`line = [for p in cp {…}]`) — the `op == "=" && !needs_db` → `OpClearVector` path, which builds into the EXISTING store | correct, **source and `cp` both intact** |
| append after that reassign | correct |
| 40 rounds under the leak gate + `LOFT_STORES=timeline` | 4 allocs / 2 frees, **no leak** |

⚠ **`depend().is_empty()` is not an ownership test, and that is sharper than the union
story.** Printing the dep NAMES at that site separates two cases the emptiness test
conflates:

```
[VNDB2] out    deps=["__vdb_1"]   ← dep on its OWN mint var: it OWNS a store already
[VNDB2] result deps=["__vdb_1"]
[VNDB2] shapes deps=["__vdb_1"]
[VNDB2] line   deps=["cp"]        ← dep on ANOTHER LOCAL: a borrow
```

`minted_vars`' own doc states the first reading: *"`[]` lowers to `OpDatabase(__vdb_N, …)`
and the value then types as a dep on `__vdb_N`, which says I own this store — the opposite
of the borrow a dep normally records."*  So the list carries THREE meanings, not two: empty
(no store yet), a mint dep (owns one), a local dep (borrows one).  `vector_needs_db` reads
only emptiness, and answers "needs no store" for the mint case correctly and for `line`
correctly-by-accident.

(An earlier note here said the false answer was "the well-trodden branch" because the other
vars reach it the same way.  That was wrong: they reach it with a MINT dep, which is a
different case with a correct answer.  `line` is the only anomaly in the run.)

**The fix direction the rules make sayable.** This is `O-Proxy` and `O-Oracle` meeting:
`vector_needs_db` asks an OWNERSHIP question (*do I own a store?*) using the dep list, while
`joined_deps`' union answers a LIVENESS question (*what must stay alive?*).  The obvious
closure is for the allocation site to read `O-Oracle` instead of the proxy.

⚠ **That closure was attempted 2026-08-25 and is STRUCTURALLY UNAVAILABLE at that site.**
`O-Oracle` (`use_analysis::ownership_of`) classifies from `data.def(d_nr).code` — a
POST-PARSE analysis over a finished body.  `vector_needs_db` runs inside the parser, which
(measured) has **no current-def handle at all** and **never calls the oracle**: every one of
its 20-odd consumers lives in `scopes` / `codegen` / `generation` / `ownership_cfg`.  There is
no def_nr to pass and no completed body to classify.

Two further measurements bound the problem:

* **The proxy term is load-bearing.**  Neutralising `depend().is_empty()` in
  `vector_needs_db` breaks `tests/scripts/03-text.loft` — it cannot simply be dropped.
* **The false fact reaches that site and still does not decide the allocation.**  Instrumented,
  `line` arrives as `deps=1 → false` ("no backing store"); yet the emitted IR shows
  `OpDatabase(__vdb_2)` at the reassignment repointing `line` to a FRESH store, so the
  borrowed one is never cleared.  Something downstream allocates regardless, which is why
  no probed shape bites.

**The second-fact route was attempted next, and the blocker is placement, not machinery.**
The flag has to be SET where the mixed join is visible and READ where the var is known, and
no single point has both:

* the join sites (`control.rs`, six of them) build a `result_type` and have **no destination
  var** — the binding happens later, at the assignment;
* the bind site has the var and the joined type, but the union has already erased which arm
  owned, and the arms' TYPES are gone — a structural re-check of the IR cannot recover it
  either, because the owning arm here is a bare CALL and `minted_vars` sees no mint (the
  mint is inside the callee);
* carrying the flag on `Deps` would travel correctly but **does not survive the store
  round-trip** — `ir_schema` reconstructs a dep list as `Deps::unknown(vec![…])`, so a
  warm-loaded program from the startup cache would lose it and answer differently from a
  cold one.  A correctness flag that a cache drops is worse than none.

So the flag wants to be set at the join and read at the allocation, and the two are separated
by a bind that discards the distinguishing information.  Closing Face A means first giving the
join a way to reach the binding — most plausibly by making the join's own result carry a mint
dep (the `["__vdb_N"]` form above already MEANS "owns"), which is a lowering change rather
than a flag.  Face A stays OPEN pending that design, with the placement constraints above
recorded so the next attempt does not re-derive them.

⚠ **loft#1082's panic was NOT this, and is now CLOSED elsewhere.**  Measured in a scratchpad
copy of the `drawing` package: replacing BOTH joins with imperative `for`-append loops — either
alone, or both — left `index out of bounds … 65535` exactly where it was.  The cause was a
two-pass work-ref collision with nothing to do with ownership at a join: `ref_return`'s
`Bind { substitute: true }` unregisters a substituted-away `__ref_N` (which also sets
`skip_free`), the `__ref_N` numbering DRIFTS between passes when a callee declared later in the
file mints a buffer on pass 2 that pass 1 did not, and `work_refs` re-minting that name
re-registered the ref while leaving `skip_free` standing.  `gen_set_first_vector_null` reads
`skip_free` as "do not allocate", so the buffer reached the callee as `DbRef::NULL`.  Fixed by
clearing the flag on re-mint; guard
`tests/scripts/1082-a-re-minted-work-ref-is-not-the-one-substituted-away.loft`.
A mechanism that explains the var table is not thereby the cause.

**Face B — a returned local the promotion should never have renamed (loft#1081, CLOSED
2026-08-24).**  The same one-path fact, at a join BOUND to a local the function returns:

```loft
fn pick(m: boolean, a: float, b: float) -> vector<float> {
  v: vector<float> = if m { [a, b] } else { [a] };
  v
}
```

`ref_return` NRVO-**renames** `v` onto the caller's return buffer.  That is right for
`v = [a]`, where the literal BUILDS into the buffer — and wrong here, because a join does
not build into its destination: each arm mints its own backing and the assignment REBINDS
the slot (`PutRef`).  So the buffer is abandoned the moment the join runs and the arm
store is handed back with no owner.  `(O-Owner)` is violated twice by one return: two
stores, zero owners.  The same join written at the function TAIL was always clean, because
there each arm materialises into `__retbuf` and frees its own backing — the BOUND spelling
simply never reached that path.

It surfaced as three symptoms, and only the smallest was filed:

* **one leaked vector per call**, both backends — the arm store nobody owns;
* **an untyped `kt=65535` store per call**, interpreter only — the un-taken arm's
  `__vdb_N`, eagerly allocated by `gen_set_first_ref_null` and never named again;
* **a silent wrong answer on `--native`, the DEFAULT backend** — the sibling arm was also
  freed at scope exit on the path that returns it, so the allocator handed the slot
  straight back and three calls answered the THIRD call's values for all three bindings.
  The interpreter answered correctly *because it leaked*: the leak was masking the
  use-after-free, which is why this arrived as a leak report.  Once the eager-allocation
  half was fixed the mask came off and the wrong answer showed on both backends.

Closed at the promotion, which is where the unsound step is: `classify_ret_promotion`
refuses the rename for a Vector local bound to a branch join
(`Parser::var_bound_to_branch`, citing @FR-O-Owner / @FR-O-Move), so the candidate drops
to `Bind` — the local keeps its own store and is copied into a separate `__retbuf` at the
return, the shape the tail join already used.  Companion: a `__vdb_N`'s entry null-init is
now the non-allocating sentinel (`gen_set_first_ref_null`, @FR-O-Derived), because its
`OpDatabase` sits at a BUILD site that may be conditional — the function prologue already
sentinel-inits every `__vdb` slot (#260 Fix B) and this was the one site that undid it.

The verdict is STRUCTURAL (does the body contain `Set(v, If …)`) rather than
ownership-based, because it is needed on PASS 1: `vector_db` runs only on pass 2, so on
pass 1 the binding's deps are still empty and no arm has minted anything.  A verdict that
differed across passes would move the hidden buffer argument between them.

⚠ **A first fix here was reverted as inert.**  Making the scope-exit free a runtime witness
(`OpFreeRefIfDistinct(v, ret_var)`) removed the wrong answer and left both leaks — a trade,
not a closure.  Once the promotion was fixed at its source, no control could falsify the
witness any more, so it came out: a guard that cannot fail proves nothing, and it had
already cost one native regression (`E0425` — a block-local `ret_var` is not in scope where
the free is emitted).

Guard: `tests/scripts/1081-a-join-bound-to-a-returned-local.loft` — values AND the wrap
harness's leak gate, both halves falsified by disabling the fix (57 leaked stores, and both
value cells red on both backends).  Neither half implies the other: silencing the leak by
freeing the DELIVERED store passes the leak gate and fails every assertion.


**The cure Face A needs is a rule decision.**  A binding whose value OWNS on some path
must own a store on every path.  That means a mixed-ownership
join types as OWNED and the borrowing arm MATERIALISES a copy — which is not a new rule
but `(O-Move)`'s existing sentence for the callee case ("if the return *borrows* a
parameter … the caller COPIES to obtain its own store"), and the model's own doctrine that
the compiler always finds a lowering, "copying when it cannot prove an alias is safe".
Half of it has been tried and measured to fail: making the owning arm win the union types
the binding owned but emits no `OpDatabase` for it, so the destination is still absent.
Both halves have to land together.

**What is NOT the cause, each eliminated by its own run.**  Reassigning over a field view;
passing local copies instead of field views; `LOFT_NO_CONF_RECOVER=1` (store confinement);
loft2's move-elide / DbRef-set work (`812aac5d` fixed the INVERSE — a borrow read as an
owned store); and blanket `mark_inline_ref` on every `__vdb_N` to stop the eager
allocation, which ALSO relocates the null-init and broke the tail-`if` return promotion
(a tail join answered `0` instead of `5`).  The eager-allocation fix therefore needs a
marker that changes alloc-vs-sentinel WITHOUT changing init order.

**Reductions.**  Face B reduces to nine lines (above).  Face A's false FACT reduces to
~55 lines.  loft#1082's PANIC does not reduce yet: the same tail-return-out-of-a-loop shape
written out on its own runs clean on BOTH backends, with the `const` parameter, the nested
caller loop and the struct-with-vector-field source all present — four constructed reductions
now.  The reliable oracle is a scratchpad COPY of the `drawing` package driven through `--lib`
(`Fronds … bow=0.16`, parse only, no render), which bisects freely and is how the tail-return
boundary was found; their tree stays untouched.


### D-own-7 — CLOSED (2026-08-23, loft#1078): every arm of a Join that OWNS a store is a candidate the free must name

`(O-Derived)` says free a local iff it owns its store and does not transfer it out.  A tail
`if`/`match` whose arms each own a store transfers exactly ONE of them, so the others are
locals that must be freed — and the promoted NRVO buffer is one of those arms.

`fn pick(c) -> S { w = S { a: 7 }; if c { S { a: 9 } } else { w } }` renames `w` onto the
hidden return buffer, so the `else` arm delivers the buffer and the `if` arm delivers a
different store.  `scopes::free_vars` reaches the losers through three legs — a null arm
(@PLN85 A.1), a promoted buffer no arm names (loft#688), and arms that disagree about
ownership (loft#1022) — and the multi-source leg that covers *"several owned candidates, one
winner"* excluded every ARGUMENT.  That exclusion is right for a user parameter, which belongs
to the caller, and wrong for the one argument that is really a local this function minted.
loft#1022's own comment had already named the carve-out and applied it inside its own gate;
the multi-source leg needed the same one.  One orphan per call, both backends, invisible in a
single call — `loft_planet` retained ~16,000 records per planet and four planets exhausted the
65,535-entry `store_nr` table.

**What the oracle held fixed, and what moving it found.** The filed report varied *what the
non-taken arm names* (a local, a parameter, a vector element) and held the RETURN POSITION and
the arm COUNT fixed.  Moving those two found two `silent-wrong` defects the leak had hidden,
neither of them an ownership fact:

* **Two owned locals** — the first is renamed onto the buffer, and the second's copy leg emits
  `OpDatabase(buf); OpCopyRecord(<tail that reads buf>, buf)`.  The re-mint destroys the store
  the copy is about to read, so the renamed arm answered a zeroed record.  A three-arm `match`
  broke only its FIRST arm, which is the tell that the buffer RENAME is the mechanism and the
  join is not.
* **Bound, then returned** (`r = if c { … } else { w }; r`) — not a tail join at all.  This is
  loft#848's class one arm over: the pass-2-only object-literal mint still drew from the shared
  `__ref_N` counter, so pass 2 handed it the name pass 1 had left on the return buffer, and
  `return_buffer()` resolves that buffer BY NAME.  The arm's record and the return destination
  became one slot.

Both answered wrong IDENTICALLY on the two backends, so `(O-NoDiverge)` held while
`(O-Owner)` did not — a reminder that backend agreement is not an oracle.  Guard:
`tests/scripts/1078-join-arms-that-each-own-a-store.loft`, both halves falsified on a pristine
worktree at `f7a57124` (the value cells by assertion, the leak cell by the wrap leak gate).

### D-own-6 — CLOSED (2026-08-20, loft#1029): the runtime Join witness now covers every argument it can name

> ⚠ **Read D-own-11 and D-own-12 with this.**  The heading's claim did not hold: four further
> argument spellings have been found that the witness could not name, on the very axis the closing
> paragraph below identifies as the one its oracle never varied.  All four are now closed, and
> D-own-11 records why the cure had to become a QUESTION rather than a fifth shape.


`(O-Complete)` accepts the Join as *inherently runtime*: a callee whose return may borrow a
parameter is completed per-path by the @P290 bracket — `protect_store_frees` marks each ref
argument's store, and a returned store that is marked is refused the source-free while a
callee-minted one is freed.  The register closed D-own-2 on that basis.

The witness was not total.  The bracket needs a slot to name, so
`use_analysis::protectable_ref_args` accepted only a bare `Var`; for any other argument
spelling `covers_all` went false and the caller fell back to the conservative never-free
answer, orphaning the store the callee minted — one record per call, both backends.  The
axis is the ARGUMENT SPELLING, not what the borrow arm names: a vector-element borrow arm
leaks with a literal argument, and a parameter borrow arm is clean with a variable one.

The rule that closes it: **the witness names a STORE, not the argument.**
`protect_store_frees` marks an allocation and reaches it through any `DbRef` in that store, so
an argument only has to be DERIVED from a nameable slot by operations that stay inside one
store.  Two families, and they need opposite cures:

* **A view of a live slot** — `b.s`, `d.b.s`, `w[0]`, `vb.v`, `o ?? q`, `if c { q } else { r }`.
  The root of a projection chain is the witness, and a join witnesses every arm.  Nothing is
  hoisted; the slot already holds its `DbRef` when the bracket runs.
* **A construction block**, which MINTS the store it yields — a struct or collection literal.
  This one cannot be witnessed in place: the bracket is emitted before the arguments evaluate,
  so the work-ref still holds its null and marking it would protect nothing while reading as
  covered — trading the leak for a use-after-free.  It is hoisted into the enclosing statement
  list instead, which is the spelling (`q = S { a: 7 }; pick(q, …)`) that was always clean.

`null` in either spelling holds no store and needs no witness (loft#1021).

**The oracle that missed this** varied the instantiating TYPE and the join SHAPE and never
varied how the argument was SPELLED — every cell in `1019-join-owned-arm-owner.loft` binds its
argument to a variable first, and a corpus that sweeps four axes impressively is read as
coverage.  `tests/scripts/1029-inline-argument-borrow-source.loft` now moves that axis across
eleven spellings, each asserting BOTH arms plus the source's own value and, for a collection,
its length — because a cure that freed the DELIVERED store answers the same number on the
owning arm, and only a length or a source read can witness it.  The type-variable half of the
same gap is recorded in [interfaces.md](interfaces.md).

---

OPEN: ~~0~~ (2026-07-04, superseded above) — **the ownership register was at zero.**  All five D-own
deviations are resolved: D-own-3 (typed `Deps`) CLOSED; D-own-4 RECLASSIFIED as the
decided edge C86 (whole-value binds copy; aliasing is a last-use elision —
`classify_vec_bind`); D-own-5 (the `&` borrow rides `deps`) CLOSED; **D-own-2
(O-Complete) CLOSED** (the ownership fact is total — oracle covers every value, the free
side reads it, the inherently-runtime Join completed per-path by the `_own_store`
witness; validated by the 6-shape sweep + full gates + the `program_ownership` fuzzer);
and now **D-own-1 (O-Deps) CLOSED** — an audit of every store-lifetime DECISION site
(dispatch.rs / state/codegen.rs / ops/ref_ops.rs / scopes.rs / control.rs) found the
free/copy/adopt/drop decisions read the ONE canonical fact
(`ownership_of` / `returns_borrowed_view` / `return_adopts_fresh_store`) on the shipped
path — the last inline shape-scan (the interp adopt-vs-deep-copy visible-ref-param scan)
was unified onto `return_adopts_fresh_store()` matching the native sibling (commit
`0234cbbb`).  **The floor (honest):** the pre-fact scans survive ONLY under the
`LOFT_NO_JOIN_OWN` opt-out (differential-control machinery, not shipped behaviour); the
runtime Join witnesses (`_own_store`/`OpBindOrCopy`) are inherently-runtime (spec-accepted,
not a re-derivation); and collapsing the return-ownership readers into ONE physical funnel
is code-DRY, not a re-derivation (each already reads the fact).  Those are reclassified as
non-deviation cleanup — the O-Deps SUBSTANCE (no shipped decision re-derives ownership; the
fact is carried and read everywhere) is met.  Validated: full suite 2601/2601 (env flakes
only), `native_scripts`, `LOFT_POISON`, the `ownership_fuzz_gate` control pairs, the
differential oracle, and the fuzzer.

### D-own-1 — CLOSED (2026-07-04): ownership is carried as one `deps` fact, read (not re-derived) per-site
- **Violated:** O-Derived / O-Deps
- **Where:** the store-lifetime bug class — `has_ref_params`, the return-source set, the
  free-suppress / return-buffer logic, etc. ([OWNERSHIP_MODEL.md § Why](../OWNERSHIP_MODEL.md)).
  Each fix added a codegen condition rather than completing a fact.
- **Effect:** the recurring store-lifetime bugs (Cluster A, #426, #429, …) — "N forests,
  one root". The class cannot be closed by more conditions.
- **@PLN85 note (2026-07-04):** the store-lifetime BUG class is retired (@PLN85 closed) —
  the load-bearing re-derivations are ELIMINATED (return-delivery + reassign thicket
  collapsed behind `classify_X`/`dispatch_X`; the `ownership_of` oracle default-on, 0/54
  over-free; the free side reads `returns_borrowed_view()`) and no re-derivation produces
  a live bug (closed by construction: fuzz/poison/DA + leak-gate).
- **@PLN90 note (2026-07-04):** the LAST per-site ownership re-derivation is now GONE —
  `scan_set`'s owned-vs-view TRACKER (`ref_rhs_ownership`) no longer re-derives from the
  RHS shape; it reads the ONE canonical `ownership_of` oracle (Owned → track; Borrowed
  AND Join → View, since a borrow/join reassignment displaces the prior owned store and
  must not be tracked as owned).  So O-Derived is SATISFIED: every store-lifetime
  decision now reads the one canonical fact, not a per-site shape scan.  Validated: full
  suite + `native_scripts` + DA + `LOFT_POISON` + differential oracle green; the p462
  conditional `?? m_none()` transition and the C86 copy-return cases all clean both
  backends.  **The D-own-2 residual is now CLOSED too** (see below): the `_ => Owned`
  tail is correct (it covers only fresh-owned / scalar / payload-less values, not a
  hole), the value-vs-bind gap is INERT for the free decision (the reassign pre-free +
  type-based scope-exit free cover it), and the inherently-runtime Join is completed
  per-path by the `_own_store` witness — so the ownership fact is TOTAL.  O-Derived:
  **CLOSED** — the re-derivation is deleted.  What stays under D-own-1 is only the
  *single-fact* unification: the free/copy/move decisions read the canonical fact at
  their chokepoints, but three cooperating mechanisms (the static oracle read + the
  runtime Join witnesses + the return-buffer machinery) are not yet ONE `deps` read.
- **Status:** CLOSED (2026-07-04) — the audit + `0234cbbb` unification landed the last
  shipped shape-scan onto the fact (see the header for the close + the honest floor).
  History below.  Landed: the return-delivery
  collapse is COMPLETE — `block_result` 459→328 lines, **45→21 helper calls**, the 15
  tail-shape classifiers down to ~3 genuinely-distinct entry guards; EVERY delivery
  mechanism routes through a pure `classify_X` selector + `dispatch_X` (vector
  `Delivery`, Reference `RefDelivery`, text `TextDep`, `ref_return`'s
  `classify_ret_promotion`); the #416/#448 cells folded; class swept dry over ~41
  probes.  The `ownership_of` oracle chokepoints are **DEFAULT-ON**
  (`keys.rs::join_own_enabled`; 54-cell over-free map 0/54 default).  And the FREE
  side began reading the canonical fact: `scan_set`'s #316 ownership tracker
  (`ref_rhs_ownership`) and codegen's owned-ref reassign gate now call
  `returns_borrowed_view()` instead of re-scanning the return deps inline (2026-07-04,
  both byte-identical over the 8 D-own-1/C86/462 corpora).
  **AUDIT 2026-07-04 — the consumption side is now ~fully fact-reading.** A sweep of
  every store-lifetime DECISION site (dispatch.rs, state/codegen.rs, ops/ref_ops.rs,
  scopes.rs, control.rs) found the free/copy/adopt/drop decisions read the canonical
  fact (`ownership_of` / `returns_borrowed_view` / `return_adopts_fresh_store`)
  everywhere but ONE genuine residual, plus two non-violations:
  - **THE ONE RESIDUAL — `state/codegen.rs:1786-1789`**: the interp `v = call()`
    deep-copy path still gates on an inline *visible-ref-param scan* to decide
    adopt-vs-deep-copy, while the NATIVE sibling (`dispatch.rs:405`) already reads
    `return_adopts_fresh_store()`.  For a fresh-return-with-ref-param callee
    (`fn mk_from(seed) -> Box { Box{..} }`) interp deep-copies where native adopts —
    same value + leak-clean on both, but a mechanism divergence.  Unifying it onto
    the fact is a COPY-ELIMINATION small-step (adopt instead of deep-copy), not
    byte-identical — best done as a dedicated @PLN90 slice on this most-reverted
    path, with the corpus+matrix gate, NOT rushed.
  - NOT violations: `dispatch.rs:403-404` (`.starts_with("n_")` / `code()!=Null` are
    call-KIND eligibility filters, the ownership decision reads the fact at 405);
    `scopes.rs collect_return_sources` (the return-source SET is the row-268 fact
    PRODUCER for the match/if union, not a consumption re-derivation).
  REMAINING: (1) the single copy-elim unification above + the architectural funnel of
  the 3 return paths (row 273) into one return-ownership computation — mechanical, no
  live bug; (2) the `??`-JOIN
  runtime witness (`OpBindOrCopy`/`OpFreeRefIfDistinct`/`_own_store`) is inherently
  runtime (the
  arm taken is unknown at compile time), not a re-derivation to delete.  D-own-5's
  `&`-borrow fact is CLOSED (folded).
- **Removal — DONE:** every free/copy/move reads `deps` (via `ownership_of` /
  `returns_borrowed_view` / `return_adopts_fresh_store`) on the shipped path; the
  per-site heuristics survive only under the `LOFT_NO_JOIN_OWN` opt-out (control
  machinery).  Non-deviation cleanup left: DELETE the opt-out scans once the differential
  controls retire, and collapse the return-ownership readers into one physical funnel
  (pure DRY — each already reads the fact).

### D-own-2 — CLOSED (2026-07-04, @PLN90): the ownership fact is TOTAL
- **Violated:** O-Complete
- **Where:** the row-100/102 holes — adopt-vs-copy for arbitrary borrowing returns; the
  general dep-driven caller copy. (The struct-field and value-`if`-return facets closed
  earlier — #415, a7.)
- **What CLOSES it — the analysis is now total, and validated total.**  O-Complete's
  failure mode is *incompleteness → a silent miscompile or leak* (line 64-66): a
  binding/path with NO computed ownership fact, falling back to a heuristic/stopgap.  That
  is now eliminated on three fronts:
  1. **The static fact is total and correct.**  `ownership_of` (use_analysis.rs) computes
     an `Own` for EVERY `Value`: `OpDatabase`/`OpNewRecord`/literals/scalars → `Owned`;
     a projection → `Borrowed{base}`; a user call → the interprocedural `call_ownership`;
     `??`/`if` → the `join` of its arms; block/insert → its tail.  The `_ => Owned` tail
     is not a hole — it covers only literals / scalar-void ops / payload-less control,
     which ARE fresh-owned or heap-irrelevant (verified against the classifier).
  2. **The free side READS that one fact** (the D-own-1 fold): `scan_set`'s #316 tracker
     (`ref_rhs_ownership`) is a pure `ownership_of` read — `Owned → Owned`, `Borrowed`/
     `Join → View`.  The three-valued gap is closed: `RefRhs::Unknown` is DELETED (dead
     once the oracle covers every value), so the free side is a total 2-valued read of
     the oracle, not a separate structural walk.
  3. **The inherently-runtime JOIN is completed per-path at runtime.**  Where a binding's
     ownership genuinely differs per path (`r = x; for { r = v[i] ?? x }` — owned copy on
     the empty path, a borrowed view once the ncc runs), a static per-binding fact CANNOT
     decide (the spec accepts this as inherently runtime, see D-own-1 residual (2)).  The
     `_own_store_<name>` witness (generation/, @PLN90 loft#495 / commits 44fd7d72 +
     a4bcad5b) is exactly the "set-and-reconcile across arms" O-Complete's removal
     criterion asks for — done at runtime: it tracks the store r actually owns, so BOTH
     the displaced-free and the scope-exit free release the owned store and never the
     view.  This is the last binding-shape whose free decision was previously incomplete.
- **The residuals — all COMPUTED and SAFE, not holes** (probed both backends,
  [plans/85 D-own-2-completeness.md § Sweep](../plans/85-store-lifetime-retirement/D-own-2-completeness.md)):
  (i) the **value-vs-bind gap** (`ownership_of(x)=Borrowed` for a `r = x` whole-value
  COPY that owns) is INERT for the free decision — the reassign pre-free + type-based
  scope-exit free release the displaced/final store regardless of the tracker's read;
  and for the transition class the witness's `is_var_copy` reads the bind as owned.
  (ii) the **deps-carried-join** (`r = pick(v,i)`, `pick = v[i] ?? Box{..}`) is a
  COMPUTED `Own::Join`, classified conservatively as a view — correct: the OWNED arm is
  materialised into the return buffer whose own lifetime frees it, so `r` views it (no
  leak / no double-free, both arms exercised).
- **Validated total:** the transition class swept dry over 6 shapes (2 live over-frees
  found + fixed, 4 safe), the value-vs-bind + deps-join residuals probed clean+poison,
  the full suite 2600/2600 (env flakes only), `native_scripts`, `LOFT_POISON`, native
  leak-check, DA, the differential oracle, AND the `program_ownership` fuzzer (3108 execs,
  0 findings — the "unfuzzed axis" concern discharged).  No binding/path produces a live
  miscompile; the analysis is total.
- **Not this deviation:** unifying the runtime witness + return-buffer machinery INTO the
  single `deps` read (rather than three cooperating mechanisms) is the *single-fact*
  ideal — that rides **D-own-1 (O-Deps)**, which stays open.  And the adopt-vs-view
  *optimisation* for a Join return (view is correct; adopt would save a copy) is
  copy-elimination — **@PLN90's LINT charter**, not an O-Complete correctness item.

### D-own-3 — CLOSED (2026-06-12, recounted into the register 2026-07-03): typed `Deps`
The dep list was a raw `Vec<u16>` overloading five meanings across two address spaces.
The H2 migration ([DEPS_INVENTORY.md](../DEPS_INVENTORY.md), steps 1–5) landed the
`Deps` newtype with named constructors at every creation site, space-checked queries
(`frame_vars` / `as_attr_indices`, debug space tags), and the `CALLEE_FRAME_BIT` VALUE
tag (0x8000) so the one cross-space provenance (the vectors.rs lambda propagation)
survives the IR codec unambiguously.  Residual (not a deviation): the newtype `Deref`s
to `Vec<u16>` for read convenience — writes go through the typed constructors.

### D-own-4 — RECLASSIFIED (2026-07-03, C86): the #415 copy IS the semantic; derive it, don't reverse it
The entry claimed the #415 struct-vector-field copy-on-bind was a stopgap contradicting
reference-default.  The reversal attempt found the premise false: on BOTH backends every
WHOLE-VALUE heap bind copies (`p = o`, `b = x`, `af = bx.v`) and only projections alias —
the written law, not the code, was wrong.  The maker's call
([DESIGN_DECISIONS C86](../DESIGN_DECISIONS.md#c86--whole-value-heap-binds-copy-aliasing-is-a-last-use-elision-the-rustc-rule)):
whole-value binds COPY by contract; `p = o` becomes an alias only when the source is
provably dead afterwards — the rustc last-use rule, as an OPTIMIZATION
(`use_analysis::ElidePlan` is that analysis).  `O-Borrow` scopes to projections /
params / `&τ`.  (binding.md D-bind-3 was already closed — the old "blocks" claim was
stale.)  The implementable RESIDUAL — the copy/alias/elide decision at the bind site
derives from the ownership fact instead of the syntactic `struct_vec_field` branch —
folds into **D-own-1**.  **Narrowed 2026-07-03:** the decision is now the pure
`classify_vec_bind` selector (`VecBind`, parser/expressions.rs — byte-identical
extraction over the C86 bind corpus): the verdict reads the base var's
incrementally-maintained `deps` (the same fact `ownership_of` reconstructs post-parse
via its whole-body `Defs` walk — Owned ⇒ copy, Borrowed/Join ⇒ view; agreement
witnessed by `LOFT_MATERIALIZE_DUMP` over the corpus), and the ELIDE half is already
live post-parse (`elision_plans` → `scopes::elide_borrows`).  What remains of D-own-1
here: the mid-parse deps read and the post-parse oracle are two implementations of one
fact — they unify when ownership is carried as one typed `deps` fact end-to-end.

### D-own-5 — CLOSED (2026-07-03, folded): the `&` borrow now carries its source in `deps`
- **Was:** @PLN87's ladder L1–L6 realised live references ([binding.md](binding.md),
  verified), but the `&τ` borrow's source was carried by a side-flag (`skip_free` on the
  L5 heap whole-value alias), not the `deps` fact the checker reads.
- **The fold (executed):** the L5 bind (`p = &o`, the only `&` binder with a free
  decision) now types `p: &Reference(td, [o])` via the standard `depending()` carrier —
  free suppression derives from `owns = dep.is_empty()` (`scopes::get_free_vars`), the
  same O-Borrow read every other borrow uses; the `set_skip_free` side-channel at the
  bind is deleted.  Proof: the ladder introspects change ONLY in the type display
  (`&ref(Pair)` → `&ref(Pair)["whole"]`) — zero op changes, both backends green,
  leak-gated (434-pln87-scalar-reference, 28-references, 87-store-leaks).
- **Residual sliver (recorded under [D-own-1](#d-own-1)):** a scalar-place ref
  (`c = &v[0]`, `r = &s.x`) holds a DbRef into the source's store, but a scalar inner
  carries no `Deps` slot (`depending()` is the identity), so the link is not a readable
  fact — vacuous for FREE placement (the binder owns no store) but unavailable to any
  future lifetime check until `Deps` is carried type-wide (the D-own-1/D-own-2
  completion).

### loft#1491 — CLOSED (2026-09-09): an arm that BINDS the buffer from a call kept two

The `arm_bind` table above answers what an arm's tail IS.  This is the statement BEFORE that
tail, and it had no answer.  An arm whose tail is a bare call is handed the function's own
return buffer and there is one store; the same arm written `{ buf = head(n); buf }` gave the
call its own `__ref_N` and rebound `buf` to it, so the callee filled one store, the caller
handed in another, and the one the caller never adopts was orphaned — **one leaked store per
call, both backends, scaling exactly with the call count.**

The value was never wrong, which is why only a leak channel could see it.  `cbor::encode`
writes the bound spelling in its `CText` / `CBytes` / `CArray` / `CMap` arms and the direct
one in `CBool` / `CInt`, so a CBOR MAP leaked one record per KEY (1, 2, 4, 8 entries → 1, 2,
4, 8 records) while its bytes round-tripped correctly.

**Closed by asking an existing question one block down, not by a new rule.**
`nrvo_collapse_tail_set` already redirects a `Set(cv, Call(…))` whose callee takes a hidden
return buffer to deliver into `cv` itself, and its own comment names this failure —
*"else its `__ref_N` buffer is allocated, orphaned, and leaks one store per call"*.  It was
reached only at the FUNCTION tail.  `materialize_vector_arms_collect` now asks it of each
block it walks; that function's step (1) declines every block whose tail is not `Var(w)`, so
passing `[w]` gates it to exactly this arm and nothing else.

⚠ **The cheap way to have found this earlier was the top-level twin.** `fn f(n) { buf =
head(n); buf }` — the same three lines with no `match` around them — already emitted the
one-buffer form.  The working bytecode was one file away the whole time, which is what the
codegen gate asks for before touching a generator and is what made the fix a translation.

Blast radius, measured: `introspect_diff.sh` over the corpus reads **DIFFERENT 1 of 1437**,
and the one file is `437-nrvo-return-aliasing.loft`, whose single changed line is the same
redirect with values and exit unchanged on both backends.

Three controls say it is a DELIVERY change and not a value one, all answering exactly what
they answered before: a literal-built arm (no call to redirect), a twice-assigned local (the
redirect declines — the first call's store is not what the tail yields), and a bind the arm
then appends to (the redirect fires and the appends still land).

Guard: `tests/scripts/1491-an-arm-that-binds-the-buffer-from-a-call-delivers-into-it.loft`,
falsified at `656caffcc` — 18 leaked records → clean on both backends.

⚠ **Its probe file surfaced a separate defect this does NOT fix, loft#1493:** a match arm
yielding a field of a struct it just built (`{ b = B { items: head(n) }; b.items }`) answers
an EMPTY collection, silently, on both backends, with no diagnostic.  Measured identical
before and after.  The materialiser has a PROJECTION arm for that family (loft#1345), and
this shape reaches it as a field of a LOCAL the arm itself built rather than of an argument.

## Carried by ownership.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [ownership.md](ownership.md) now states only what is open.

### D-own-8 / D-own-26 / D-own-16 closure summaries

**D-own-8 CLOSED 2026-09-03** (loft#1320, loft#1321, loft#1323): a Join's ownership fact was
true on one path only because ONE binding carried two paths.  The close is structural, not
N-ary: every arm tail of a bound value branch that a single bind would leave OWNING — a fn-ref
call of any ownership, a named call answering a record the caller must copy, a plain variable
(`(B-Copy)`) — is given a binding of its own, bound by that single bind's own lowering, and
the joined binding borrows the temps.  The two shapes loft#1320 declined take the witness the
base itself could not be: a SNAPSHOT of the store the base named at the bind (`(O-Latest)`,
the way a rebindable parameter's entry stash already witnesses).  And the `??` hoist that
binds a CALL subject now owns what a plain bind of that call would.  Measured on both backends
at the ceiling and in the over-free direction; the full record, including the two regressions
the first cut introduced and what each taught, is in
[ownership-history.md](ownership-history.md).

**D-own-26 CLOSED 2026-09-03**, against the bar its own entry set: *"the honest cure is a way
to fail a build in which a free-deciding site reads the proxy without the veto."* That gate
now exists, is falsified on five separate paths, and passes — 9 sites declare `free` and all
9 consult `O-Override`; the other 15 declare which of the other three facts they read. The
"eleven of seventeen" it opened with was a hand count that could not separate *asking* the
proxy from *freeing* on it. What the close does NOT cover: a site that declares a non-free
question and frees somewhere the gate cannot see. The full record is in
[ownership-history.md](ownership-history.md).

**D-own-16 CLOSED 2026-09-03.** Every cell that should reach zero does, on both backends, with
every value unchanged: a minting call that reads the local, the self-referential join
`c = mk(i) ?? c`, a conditional borrow, and a local bound from a PARAMETER and then minted.
The one shape that still retains a store is a lambda-CAPTURED local, and that is
`(L-CapHeap)` holding rather than a leak — a captured heap value is SHARED, so declining the
free is the right answer and its right answer keeps a store.  Guard:
`tests/scripts/1085b-a-nullable-local-frees-what-it-displaces.loft`; the full record, including
the two mechanisms that were tried and reverted, is in
[ownership-history.md](ownership-history.md).

### the status line formal/README.md's area table carried until 2026-09-04

**0 open** — back at 0 on 2026-09-03 with D-own-8 CLOSED (every path of a bound value branch its own binding, loft#1320/#1321/#1323); it had been RE-OPENED after the 2026-07-04 zero, down to D-own-8 (a Join's ownership fact is true on one path only). D-own-16 and D-own-26 both closed 2026-09-03; D-own-26's gate had been reporting `0 violations` over its own violations for a week, because it searched for a free in a region the free cannot occur in. The ORIGINAL five stay resolved: D-own-1/2/3/4/5 ALL CLOSED; every store-lifetime decision reads the one total `deps` fact. The soundness proof heap.md's free rules rest on

### D-own-27 closure summary (the chapter's own words, 2026-09-04)

**D-own-27 OPENED AND CLOSED 2026-09-04** (loft#1336): a heap-record local bound by COPY
and later rebound to a VIEW released the copy nowhere — `cur: Node? = a; cur = cur.next`
leaked one store on both backends, a call-minted local and a plain nested-field view leaked
identically, and the inverse order (`s = a.next; s = a`) wrote the second copy INTO the
viewed record on `--native` and ALIASED the source on the interpreter, so `b == 1` on one
backend and a write through `s` reaching `a` on the other.  The `?` was never the axis: the
dense twin `x: Pair = a; x = x.other` released the copy at the rebind AND freed `x` at exit,
which landed on `b`'s store and read clean only because nothing read `b` afterwards.  Closed
by `(O-Witness)` above — the native emitter already carried this tracker privately
(`_own_store_<name>`, for a dense deps-empty local); it now lives in the IR for every mixed
local, both backends translate it, and native's private tracker is left to the hidden
temporaries it was built for.  Guard:
`tests/scripts/1336-a-local-with-mixed-ownership-releases-through-its-owner-witness.loft`,
fifteen cells on both backends, falsified at `c25b444c`; the full record, including the
four measurements that shaped the mechanism, is in
[ownership-history.md](ownership-history.md).
