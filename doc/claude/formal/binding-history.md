# formal/binding-history.md — the deviation register for [binding.md](binding.md)

> **The rules are next door.**  [binding.md](binding.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0 here** — D-bind-30 OPENED AND CLOSED 2026-09-08: `(B-Ref-Reshape)` reached only the
INLINE keyed kind, so a `&` view of the record a removal FREES was never refused and a later
insert reusing it read the stale write (below); D-bind-29 OPENED AND CLOSED 2026-09-08: `B-Ref-Reshape`'s re-key arm was
enforced for ONE key type and ONE source spelling — a `text` key never reached the guard at
all, and a keyed lookup's `?` hid the `&` marker from all four refusals (below).
D-bind-28 (the COLLECTION half of `(B-Ref-Uniform)`) CLOSED 2026-09-07: three
independent mechanisms broke it and all three are closed — the vector surface, the keyed BIND
(loft#1433), and the keyed PARAMETER (loft#1445), whose first fix was reverted and whose
rework closes it by SPLITTING the predicate rather than widening it (part four, below).  D-bind-27 OPENED AND CLOSED 2026-09-07: a value branch whose arms view
DIFFERENT containers names EVERY place it can read, not none; D-bind-26 OPENED AND CLOSED
2026-09-07: a removal reached through a FIELD is a disturbance of that field's place;
D-bind-25 OPENED AND CLOSED 2026-09-07: a `sorted` removal renumbers, so it ends the
places its container holds (below, all three).  D-bind-24 OPENED AND CLOSED 2026-09-06 (loft#1401: a projection discharged
with `??` was not a view the materialise walk could see, so it kept aliasing an element
position a `remove` had renumbered; below).  D-bind-23 OPENED AND CLOSED 2026-09-06 (loft#1399: a branch arm projecting a COLLECTION got no copy, so `--interpret` read the reassigned container where `--native` was right; below); D-bind-22 OPENED AND CLOSED 2026-09-06 (loft#1396: a binding whose value is a
BRANCH with a projecting arm was named by nothing, so it never materialised — closed by naming
it through the branch and giving the projecting ARM its own temp, below); D-bind-21 OPENED AND CLOSED 2026-09-06 (loft#1394: a view BOUND INSIDE a
branch arm whose container is reassigned in the SAME arm was never materialised, because
the walk read a `Set` with a value-branch right-hand side whole — below); D-bind-20 OPENED AND CLOSED 2026-09-06 (loft#1393: a view OF A VIEW was not shaken when the outer container was disturbed, below); D-bind-19 OPENED AND CLOSED 2026-09-06 (the `@FR-O-Owner` walk: a struct-ENUM PAYLOAD view was invisible to `(B-View)`'s materialise clause, because the chain that names aprojection's container could not see through the variant check the lowering wraps its subject
in — below); D-bind-18 OPENED AND CLOSED 2026-09-06 (loft#1392: a vector link did not follow a
rebind of its SOURCE, below); D-bind-11 and D-bind-16 CLOSED 2026-09-03 (below); D-bind-12, D-bind-13,
D-bind-14 and D-bind-15 each opened and CLOSED the same day.
D-const-2 opened and CLOSED the same day (2026-09-01), found by the Store Locks
reference review.
B-Ref-Reshape is enforced for all three of B-Disturb's events (D-bind-9,
opened and closed 2026-08-05); B-Ref-AnnotationOnly is enforced in every position, not
only the ones a leading `&` reaches (D-bind-10, 2026-08-09).

> **D-bind-30 — OPENED AND CLOSED (2026-09-08) — `(B-Ref-Reshape)` was enforced for the
> INLINE keyed kind only, and the exclusion of the other four was measured on a case it was
> not then used for.**
>
> `reshaped_containers` collected a keyed removal only when the container is `Type::Sorted`,
> with a reasoned comment beside it: *"`hash`, `index`, `spatial` and `trie` give each element
> a record of its own, so removing one leaves every other key reachable AT THE SAME ADDRESS —
> measured, a view of another element reads correctly after the removal and a write through it
> still lands."*
>
> That measurement is **correct and reproduces**. The case it did not cover is a view of the
> REMOVED record:
>
> ```loft
> c = &h[30];  h[30] = null;  h[70] = Elm{key:70, tag:7};  c.tag = 999;
> //  → k70 reads 999 on BOTH backends, silently: the insert reused the freed record
> ```
>
> **The defect is not which KINDS are collected — it is what a removal DISTURBS.** For a
> `sorted` it is the whole container, because every later position shifts. For the
> record-per-element kinds it is exactly ONE place, and the comment is right about every
> other key. The comment was true at the granularity it was measured at and wrong at the
> granularity it was used at.
>
> ⚠ **The obvious fix is wrong and was built before being rejected.** Collecting the other
> four wholesale closes this and materialises four deliberate corpus controls whose NAMES
> state the proposition — `test_a_hash_removal_is_not_a_reshape`,
> `test_an_index_removal_is_not_a_reshape`, `test_the_other_keyed_kinds_are_not_reshaped`.
> A PLAIN view (no `&`, so `(B-Ref-Reshape)` exempts it and `(H-Materialise)` copies it)
> stops aliasing and its write lands on a copy: one silent-wrong traded for another.
> Measured independently on two trees before being dropped.
>
> Closed by comparing KEYS. Both sides read their key through one `OpGetRecord` reader — the
> view records the key it was bound at, the removal carries its own inside the
> `OpHashRemove`'s argument — and a view is spared only when both keys are literal and
> DIFFER. `sorted` is untouched, and so is every computed key on either side.
>
> **The filter may only ever SPARE, and only on proof.** An absent key means *could be the
> same record* and shakes. Getting that direction backwards turns a conservative rule into a
> silent one, which is the defect being closed, so both fallback directions have a cell.
>
> Whole-population cost: **zero**. Every `.loft` file in the tree carrying a keyed removal
> (54 of 2957) was compiled and none is newly refused — the fix refuses strictly fewer
> programs than the wholesale variant, which cost 6.
>
> Lock-ins: `b_ref_reshape_removing_the_very_key_a_view_names_is_error`,
> `…_removing_another_key_leaves_a_view_alone` (the control that says this is not the
> wholesale version — without it, collecting the container passes the first cell), and
> `…_a_computed_removal_key_is_still_refused`. Falsified: the two defect cells fail on the
> unfixed build and the control passes on both, which is the shape a spare-on-proof filter
> should have.
>
> And the MESSAGE moved with the set. *"A removal renumbers the remaining elements"* is right
> for `sorted` and wrong for the four that just joined, so the reason is split by kind.  Worth
> naming WHY that was easy to miss: the wording was CORRECT for the one kind that reached the
> site, so it read as settled prose rather than as an assumption with a scope — which is the
> exclusion comment's own shape one layer out.  A sentence that is true of everything it can
> currently be said about carries no marker saying how far it reaches.
> [loft#1458](https://github.com/loft-lang/loft/issues/1458) was filed on that wording BEFORE
> the set widened and closed as invalid — correctly, and the same wording is wrong in the
> other direction now. Which way it is wrong depends on which kinds reach the site, so the
> two have to move together.

> **D-bind-29 — OPENED AND CLOSED (2026-09-08) — `B-Ref-Reshape`'s re-key arm was enforced
> for ONE key type and ONE source spelling, and D-bind-9's own closing sentence says why.**
>
> (Numbered 29, not 28: D-bind-28 is the `(B-Ref-Uniform)` collection deviation on the sibling
> branch — four parts, loft#1433 / loft#1445 — and this register does not carry its entries even
> though `binding.md` here carries its header.  A register whose numbers are handed out on two
> trees needs the number checked against BOTH, which is a cheap grep and was not done.)
>
> D-bind-9 closed with *"a rule with more than one producer needs a sweep, not a cell"*, and
> its sweep was 14 shapes of `&` — whole struct, whole vector, element, field, nested, keyed
> non-key, keyed key, and so on. Every one of those axes is a shape of the REFERENCE. None is
> a shape of the KEY, and none is a shape of the type the lookup ANSWERS. Two holes sat in the
> axes the sweep did not have:
>
> The set now comes from `vectors::is_collection` — the `is_keyed` set plus `Vector`, which is
> exactly `(Col-Store)`'s store-backed set — and the keyed deep-copy branch is skipped for a
> `&` bind, leaving the plain handle share whose dep names the source (non-owning, as the
> vector twin already was).  Fixed for the LOCAL and struct-FIELD provenances at every keyed
> kind including `spatial`, which the keyed-field copy path treats apart and which is
> therefore asserted on its own rather than assumed to follow the other four.
>
> ⚠ **This is the third instance of one class**: a type set written as a `matches!` LIST that
> is missing a kind.  `Type::is_amp_rebindable_heap` sits next to the defective line carrying
> the full heap set, and its own doc records being written *"one home rather than two
> `matches!` arms, which is how the keyed kinds came to be missing from both (loft#1291)"*.
> Guard: `tests/scripts/1433-a-keyed-alias-is-a-link-not-a-copy.loft`, both backends, with
> `a_plain_keyed_bind_still_copies` as the control a share-everything cure fails.

> **D-bind-28, part three — REOPENED (2026-09-07, loft#1445) — the `&hash<τ[k]>`
> PARAMETER spelling.**  Closed and reverted the same day: the fix peeled the link in the
> SHARED `is_keyed` / `is_collection` predicates, which are asked at 78 sites and answer two
> questions — *which collection kind* (peel) and *does this variable own a store* (do not
> peel, a `&` parameter aliases the caller's).  `Set(v, Null)` on a `&hash` parameter then
> routed into `gen_keyed_null`, which allocates a keyed LOCAL's own store and resolves with the
> unpeeled `base()`: `unreachable!("gen_keyed_null on non-keyed type")`, and
> `tests/scripts/1291-a-keyed-write-back-does-not-release-the-callers-store.loft` went 8/8 to
> 8/8 ICE.  The narrow form — peel at the `+=` ROUTE, leave the shared predicates on `base()` —
> is the one to land; two of the three original sites (`keyed_known_type`, the route's `dest`)
> are already narrow and are expected to survive.  The account below of what the fault IS
> remains correct and is what the rework implements.  `(B-Ref-Uniform)` says a `&τ`
> variable is used exactly like a `τ` variable with no operation special-cased, and
> `c += [rec]` on a `&hash<Row[id]>` parameter was refused instead — *"Variable 'c' cannot
> change type from `&hash<Row,["id"]>` to `vector<Row>"*, because the append routes did not
> claim the statement (`is_keyed` / `is_collection` read `tp.base()`, which peels `Optional`
> and not the link) and it fell into the VECTOR route.
>
> ⚠ **This entry carried a WRONG reason while it was open, and the correction is the useful
> part.**  It read: *"making those predicates peel the link WITHOUT the keyed emission path
> resolving its store through the parameter's double indirection turns the refusal into a
> SILENT DROP … a refusal is the better of the two states."*  The MEASUREMENT was right — the
> surface peel alone does produce `len` 0 with no diagnostic.  The ATTRIBUTION was invented.
> The emission path resolves a keyed store through a `&` parameter and always did: a keyed
> INSERT one operator over (`c[7] = Row{…}`) reaches the caller's store on both backends,
> with the pre-existing key still readable afterwards.
>
> The real cause was a THIRD site of the same miss: `keyed_known_type` also opens with
> `base()`, so a `&`-wrapped keyed type answers `None`, `new_record`'s fallback hands
> `OpNewRecord` the `vector<τ>` id, and `record_finish` dispatches through `Parts::Vector`.
> It presents as a wrong type NUMBER (`parent_tp` reads the `&vector` twin's id), not as a
> reachability failure — which is why a mechanism sentence could not tell the two apart and a
> `parent_tp` comparison could.  The peel was never the wrong move; it was half a move.
>
> Closed by peeling all three sites with `peel_link` — `is_keyed`, `is_collection`,
> `keyed_known_type` — plus the `+=` route's DESTINATION.  That fourth is not optional:
> peeling the predicates without it hands `append_source` a `RefVar` that matches no arm and
> breaks the `&vector<Row>` twin that always worked, so the failure lands in the CONTROL
> rather than in the cell under test.  Guard:
> `tests/scripts/1445-a-keyed-parameter-appends-through-its-link.loft`, all five keyed kinds
> plus the vector control, both backends, every destination PRE-POPULATED and every cell
> reading the old key as well as the new one — an empty destination cannot tell an append that
> reached the caller from one that built a fresh collection and counted itself.

> **D-bind-28, part four — CLOSED (2026-09-07, loft#1445 reworked) — one predicate, two
> names.**  The reverted fix widened `is_keyed` / `is_collection`, which are asked at 78 sites
> and answer two different questions.  The rework gives each its own name: `keyed_kind(tp)`
> peels the link (which kind, which type id, which insert) and `owns_keyed_store(tp)` does not
> (what may be allocated, minted or replaced in place), leaving `is_keyed` to the sites that
> never had to tell them apart.  `1291` 8/8, `1445` 8/8, `1445b` 2/2 on this tree; two full
> gates on the authoring tree (979s and 1097s).
>
> **What closes the entry is a control, not the appends.**  `(B-Ref-Uniform)` says a `&τ` is
> used EXACTLY like a `τ`, so the question is agreement between the two spellings and not
> whether any particular one is accepted.  `a += b` where both are `hash<Row[k]>` is REFUSED —
> *"cannot append a whole `hash<Row[k]>` to another — a keyed collection's `+=` takes one `Row`
> element written `[…]`, or a `vector<Row>` of them"* — and the `&` spelling is refused too.
> Uniform, so the rule holds; the whole-collection append is a decided surface limit and not
> this entry's business.
>
> ⚠ **The removal cell's receiver is a `sorted`, and only a `sorted` reaches that refusal at
> all.** `reshaped_containers` collects a keyed removal ONLY when the container variable is
> `Type::Sorted` — deliberately, with the reason written beside it: `sorted` is the INLINE keyed
> kind, its elements sit in key order in one dense array, so a removal shifts every later
> POSITION exactly as a vector's does, while `hash` / `index` / `spatial` / `trie` give each
> element its own record and leave every other key at the same address.
>
> So the message ("a removal renumbers the remaining elements") is CORRECT there, and reading
> `(Col-RemoveKeyed)` as contradicting it is a category error this register made and is
> correcting: that rule is about which KEYS stay reachable, and a `&` view holds a POSITION.
> Both are true at once. [loft#1458](https://github.com/loft-lang/loft/issues/1458) was filed on
> that misreading and is closed as invalid.
>
> What the same reading DID turn up is real and is [loft#1460](https://github.com/loft-lang/loft/issues/1460):
> the deliberate exclusion was measured on a view of ANOTHER element, which is safe, and not on a
> view of the REMOVED one, which is not — `c = &h[30]; h[30] = null; h[70] = …; c.tag = 999`
> corrupts the record that reused the freed slot, silently, on both backends.

> **D-bind-25, D-bind-26, D-bind-27 — OPENED AND CLOSED (2026-09-07) — three places
> `(B-Disturb)` names that the walk could not see.**  All three came out of loft#1401's
> boundary matrix, and all three failed IDENTICALLY in the plain spelling and the `??` one —
> which is what said they were not that issue's discharge defect but its neighbours.  Each is
> a silent wrong answer on both backends, and each has its own root.
>
> **D-bind-25 — a `sorted` removal is a reshape.**  Measured, one shape per keyed kind:
> `hash` and `index` answer the right element after another key is removed and a write through
> the view still lands; `sorted` reads the element that shifted in.  `(Col-RemoveKeyed)` is why
> — the four own-record kinds leave every other key reachable AT THE SAME ADDRESS, while a
> `sorted` is the INLINE keyed kind whose elements sit in key order in one dense array, so
> `(Col-RemoveDense)` covers it beside the vector.  `reshaped_containers` listed
> `OpRemove`/`OpRemoveVector` and a keyed removal emits `OpHashRemove`, which serves all five
> kinds — so the fix is keyed on the KIND, not on the op.  Adding the op flat would have
> materialised the four that are correct today, which is the harmful direction.  **This is the
> same boundary loft#1402 crossed from the other side**: there a `sorted` LEAKED through
> `#remove` while `[key] = null` stayed flat, here it goes STALE through `[key] = null` while
> `hash` does not.  One split, two symptoms, and `remove_vector_at`'s `is_linked` gate is the
> third place it shows.
>
> **D-bind-26 — a removal reached through a FIELD is a disturbance.**  `p.va.remove(0)` emits
> `OpRemoveVector(OpGetField(p, off_va, …), …)`, and the collector took the op's argument only
> when it was a plain `Var`.  The VIEW side was already precise — `value_view_place` resolves a
> projection chain to `(p, off_va)` — so only this half was short and the two never met, while
> the same code with `va` in a LOCAL materialises and says so.  Closed by answering PLACES: a
> whole variable ends everything inside it, a field ends that field only.
>
> ⚠ **And that precision is the fix, not a refinement of it.**  `grown_containers` carries the
> measurement on its own half: collecting the PARENT for a field-qualified growth shook every
> view rooted at the same variable, and `moros_editor`'s `undo_pop` read each entry out of a
> copy — the undo stack silently stopped recording, `undo_depth` answering 0 where 3 was due.
> A variable-granular version of this fix is that bug.  Two controls pin it: a sibling member
> REMOVED and a sibling member GROWN must both leave the view aliasing.
>
> ⚠ **And it cost a third rule to land: a binding the loop ITERATES is not materialised.**
> With a field-reached removal counting as a disturbance,
> `for e in d.items { e#remove; }` shook the loop's OWN source temp — a view of
> `(d, off_items)` by every test this walk applies — so the loop walked a COPY while the body
> emptied the original, and never terminated.  `903-loop-remove` went from 0.06s to a 300s
> corpus TIMEOUT, taking both corpus binaries with it.  The iteration depends on that temp's
> IDENTITY, so giving it a store of its own is not a copy of a value but a different loop.
>
> ⚠⚠ **The obvious wider rule was measured WRONG, and that is the entry's second lesson.**
> *"A view the AUTHOR cannot name is not a binding `(B-View)` is about"* reads well, matches
> `resolve_view_root`'s stop at a compiler-generated CONTAINER, and is false: the parser renames
> author bindings too, and a `match` PAYLOAD (`_mv_<field>_N`) is a view the author very much
> wrote and must still materialise.  Shipped as a name test it made
> `a-payload-binding-warns-when-its-subject-is-given-another-variant` read its subject's NEW
> variant — trading a hang for a silent wrong answer.  So the fact is a MARKER on the variable
> the lowering created (`Function::is_iteration_source`, set at the two sites that build the
> temp), not a property of how its name is spelled.
>
> The bisect is worth the sentence too, because the symptom pointed away from the cause: the
> hang was in `State::execute_argv` — the INTERPRETER — while the defect was a compile-time
> analysis deciding to copy something the interpreter's loop depended on, with no compile-time
> symptom at all.  `gdb -p` had nothing to attach to; `perf record -p <pid> -g` named the loop
> in seconds, and three build-and-time steps named which of the three changes owned it.
>
> **D-bind-27 — a branch over two containers names both.**  `c = if k { w[0] } else { v[1] }`
> is a view of `w` on one path and of `v` on the other, and the `If` arm answered `None`
> whenever the arms disagreed — the documented rule, and the right answer for a walk that
> records one container per view.  It is now recorded once per PLACE, which the open-view frame
> already held as one `(view, container, field)` entry per pair, so nothing downstream learned
> a new shape and `shake_places` matches either.  Two arms naming one container at DIFFERENT
> fields stay two places rather than collapsing to `ANY_FIELD`: a disturbance of a third field
> ends neither.
>
> With more than one place per view, the ADVICE could no longer re-derive its container from
> the right-hand side — that names both and only one was disturbed.  `views_to_materialise` now
> carries the whole `Disturbance`, which already held the container it was observed at, so the
> sentence names what actually moved.  The re-derivation was a restatement before it was a
> wrong answer; this is the same one-home correction QUALITY.md's register keeps recording.
>
> Guarded by `a-disturbance-ends-every-place-a-view-can-name` (11 cells, both backends,
> falsified at f4403e62).  SIX of the eleven are controls — the four own-record keyed kinds, a
> sibling member removed, a sibling member grown, and an undisturbed branch — because widening
> a disturbance is the direction that silently loses a write.  Each control has a FIRING twin
> in the same file, so none of them can read green by never being reached: `LOFT_DEBUG_F8=1`
> names exactly the four positive cells and none of the controls.

> **D-bind-24 — OPENED AND CLOSED (2026-09-06, loft#1401) — a projection discharged with
> `??` was not a view the materialise walk could see.**  `(B-View)` and [heap.md](heap.md)
> `(H-Materialise)` say a projection live across a disturbance of its container MATERIALISES —
> a fresh store, the `(H-Copy)` step, "and the author is told".  The plain spelling did exactly
> that; the `??` spelling did neither, so `c = v[1] ?? Box{n:0}; v.remove(0)` left `c` aliasing
> POSITION 1, which `(Col-RemoveDense)` had just renumbered: it read the value that shifted in
> (3 where its element held 2) and a write through it still reached the container.  Both
> backends, in silence.  Not an exotic spelling — `(N-Index)` types `v[i]` as `τ?`, so `??` is
> the discharge the language REQUIRES for a non-null binding.
>
> **Filed as one hole; it was four, and all four had to close together.**  That is the entry's
> lesson: each one alone reads as the whole cause, and each one alone leaves the defect
> standing.
>
>   1. NAMING.  A `??` lowers to a value block that hoists its subject into a temp and hands
>      that temp back from the tail, so the walk saw a bare `Var` naming nothing.  It now
>      resolves a tail through the block's OWN bindings — the notion (a name standing for a
>      value computed here) rather than any one lowering's spelling of it, so `??`,
>      `?? return` and a `match` subject are one step.
>   2. COPY.  Supplied per ARM, as loft#1396/#1399 supply it for a branch-valued binding:
>      `arm_bind` gives the discharge hoist its own `__lift_N`, gated on the walk having named
>      the binding.  The issue recorded naming-without-copy as measured WORSE, and it is —
>      the advice then asserts a guarantee the emitters do not deliver.
>   3. `?? return`.  Its absent path leaves by an early return, so the block's tail is an
>      unconditional `Var` and not an `if`.  `is_value_branch` read that as "not a branch" and
>      the arm never reached the copy at all.  One arm is still a path.
>   4. SPELLING.  A discharged `v[i]` arrives as `OpGetVectorNullable`, which is a projection
>      by every structural test and is deliberately OFF `is_projection_op` because the deps
>      PROXY strands a store on it (that function's own doc says so at length).  The hazard
>      belongs to the proxy, not to the notion, so `view_source_place` is the reading for a
>      walk that only NAMES a place; the strict list is unchanged for the readers that trigger
>      the proxy.
>
> A binding assigned MORE THAN ONCE was the fifth cell and needed the `multi_assigned` bail
> lifted for a NAMED binding only: `@FR-O-Latest` is about the type-level dep list, which
> `lift_join_arm_tails` already declines to rewrite for such a binding, while the copy itself
> is a fact about this assignment.  The PROJECTION arm never asked the question, so the two
> arms disagreed about the same binding.
>
> ⚠ **The first cut regressed loft#1399, and the mechanism is worth keeping.**  Letting a block
> tail resolve through its own bindings also let a `[]` MINT arm name the hidden `__vdb_N` it
> reads its own store out of — a projection by every structural test — and two arms naming
> DIFFERENT containers name none, so the whole binding stopped being a view.  A place inside a
> compiler-generated container is not a place any disturbance can name; `resolve_view_root`
> already stopped at one for that reason, and the namer now does too.  The guard caught it, no
> targeted suite did.
>
> Guarded by `a-discharged-projection-materialises-like-its-plain-twin` (15 cells, falsified at
> 2f471b15 on both backends).  Three of its cells are ALIAS controls — an undisturbed
> discharge, a use before the disturbance, and a disturbance of another member — because
> over-materialising is not the smaller error: it silently loses a write that lands today.
> Unblocks [collections.md](collections.md) `D-col-3` (loft#1402), which could not release a
> removed element's children while such a binding still viewed it.  Found in the
> `@FR-Col-Remove` walk (QUALITY-history.md B8f).

> **D-bind-23 — OPENED AND CLOSED (2026-09-06, loft#1399) — a branch arm projecting a
> COLLECTION got no copy.**  loft#1396 gave a projecting arm its own temp and matched
> `Reference` and struct-`Enum`, answering `None` for a `Vector`; the deps-strip route that
> would otherwise reach a collection is (correctly) not taken for a branch-valued binding, so
> nothing copied.  `--interpret` read the NEW container, `--native` answered correctly off
> empty deps — a split `(O-NoDiverge)` forbids, with `(B-View-Base)` settling the value:
> *a projection is a VIEW at every element type, not only a struct-typed one*.  Closed by the
> same buffer-and-refill a whole-vector copy already uses (`ArmBind::CopyVector`).
>
> **The root was TREE-DEPENDENT, and that is the entry's lesson.**  Filed against the sibling
> branch it read *"the walk never names a collection view"*, which is true there and false on
> the tree that merges: the union of the two `leaf` gates at the loft#1396 pick — that branch's
> `value_view_container` over this one's `.base()` + `Vector` type list — is what made naming
> and copy land together, so here the binding IS named (`LOFT_DEBUG_F8` prints it) and only the
> copy was missing.  The same patch measured INERT on the branch where nothing is named, was
> reverted there as dead, and is live here.  An experiment that measures inert is not thereby
> wrong; recording WHY it was inert is what let this one be recognised rather than re-derived.
>
> **The boundary is inverted between the two kinds, and reading it the record way is how this
> fix looks over-wide when it is not.**  For a RECORD tail an undisturbed projection ALIASES
> and must keep doing so.  For a COLLECTION tail it does not: `(B-Copy)` makes a whole-value
> vector bind a copy, so a write through it reaching nothing is the documented answer in the
> branch spelling exactly as in the plain one.
>
> That is a LANGUAGE fact and not a property of this tree, which matters here because the ROOT
> above was tree-dependent and the two could be confused.  It was measured independently on the
> sibling branch, which carries neither half of the collection work: a record tail aliases in
> both spellings there too, a collection tail copies in both.  The BRANCH was never the axis
> for either kind — what the shipped 2026.9.0 shows is that release being inconsistent between
> the two spellings for a collection, aliasing the branch form while copying the plain one.
>
> **The general form, worth more than this cell.**  The control for *"did this fix copy too
> much?"* is not the same cell for every element type, because what a plain bind already does
> differs BY type.  The comparison that settles it is the PLAIN spelling of the SAME tail on
> the SAME build — not the same shape on an older build, which is what made the release look
> like the authority when it was the thing under suspicion.
>
> Guard `a-collection-projection-arm-of-a-branch-materialises` (9 cells), falsified at
> c22c318f: interpret one assertion failure -> 0, native INERT — which is the right verdict for
> a backend divergence, since only one side can move.
>
> ⚠ **The pairing that makes all of this work was never designed, and nothing but a comment
> names it.**  `ViewWalk::leaf`'s gate now asks two questions that arrived from different
> branches for different defects: a TYPE LIST — which bindings can be views at all, widened to
> `Vector` (through `.base()`) for loft#1377 — and `value_view_container` — which container a
> value views, widened through a branch for loft#1396.  They met at a cherry-pick, and the
> union is what lets a COLLECTION view be both named and copied.  Narrowing either silently
> un-fixes a shipped defect, and both directions were MEASURED rather than reasoned — each
> narrowing made, built, and the guards run:
>
> | narrowing | 1377 | 1396 | 1399 |
> |---|---|---|---|
> | drop `Type::Vector` from the type list | FAIL | ok | FAIL |
> | drop the namer to `base_container_var` | ok | FAIL | FAIL |
>
> loft#1399 is lost either way, and by two different routes — without the type list it is not a
> view at all, without the namer it is not a branch it can see through.  Each issue has a guard,
> so the three together are the line's regression net; no guard names the PAIRING, which is why
> the site itself now does.

> **D-bind-22 — OPENED AND CLOSED (2026-09-06, loft#1396) — a binding whose value is a BRANCH
> with a projecting arm was outside the materialise clause.**  `x = if k > 0 { h.inner } else
> { mk(0) }` followed by `h = Hold{…}` read the NEW container on both backends as a first bind,
> and on `--native` alone as a reassignment — the wrong answer with an `(O-NoDiverge)` split on
> top of it.  The walk that NAMES views asked which container the value projects from, and asked
> it of the whole `if`, which projects from nothing; the binding was therefore never named, and
> the machinery downstream had nothing to act on.
>
> The naming now looks THROUGH a branch (`scopes::value_view_container`): a container named by
> any arm, none where two arms name different ones.  **The copy is per ARM, not per statement**,
> and that split is the entry's substance.  `(O-Complete)` asks for the fact per binding and per
> PATH; only the projecting arm needs a store of its own, so `Scopes::arm_bind` — the arm-lift
> machinery that already binds a call or a variable tail into a `__lift_N` temp — gains the
> projection case, gated on the walk having named the binding.  An arm whose container is never
> disturbed keeps aliasing, which is what `(B-View)` is for.
>
> ⚠ **The whole-statement cure was built first and is measured WRONG.**  Stripping a
> branch-valued binding's deps makes it the OWNER of a store it only views, and the emitters
> have no whole-statement copy to pair with that — `container_element_base` answers `None` for
> an `If` — so its scope-exit free names the CONTAINER's store, which is loft#778's class.  It
> was backed out whole before the per-arm form replaced it, and the naming helper carries that
> in its own doc so the next reader does not re-derive it.
>
> Guard `tests/scripts/a-projection-arm-of-a-branch-materialises-when-its-container-is-reassigned.loft`
> (6 cells: the first bind, the reassignment, the minting arm, the unbranched control, an
> UNDISTURBED arm that must still alias, and a loop whose every pass reads the container as it
> then stands).  The chained spelling — `t = dv.tiles; prev = if … { t.proto } … ; dv = …` — is
> NOT closed here and is not this deviation: the same chain without a branch is equally wrong,
> which is loft#1393's view-of-a-view.

> **D-bind-21 — OPENED AND CLOSED (2026-09-06, loft#1394) — a view bound INSIDE a branch arm
> did not see its container's reassignment in the same arm.**  `(B-Disturb)` × `(B-View)` are
> the same sentence wherever the two statements sit, and the walk that names views to
> materialise reads statements in source order — but it handled a `Set` whose right-hand side
> is a value branch WHOLE, through the `leaf` arm its own doc calls *"deliberately coarse in
> both directions"*.  So the bind and the disturbance were one indivisible step:
> `got = match sh { Holder{inner} => { sh = Empty{z: 0}; inner.a }, _ => -1 }` answered 0 on
> `--interpret` and 1 on `--native`, and `got = if c { x = h.inner; h = Hold{…}; x.a }` — a
> plain STRUCT view, no enum in sight — answered the NEW container's value on BOTH backends.
> The coarseness is right for a form whose internal order is unknown; a `Set`'s is not, so one
> is walked in the order it runs: the value, then the target's own establishment at the point
> the slot is written.
>
> Two facts had to travel with the deps.  A materialised view stops being a view, so the
> never-free mark it was given as one is LIFTED (`Function::clear_skip_free`) — a `match`/`is`
> payload binding is marked by the parser, and stripping only the deps left it owning a store
> nothing released.  And a binding carrying NO deps is admitted beside one that names the
> container: the `is` spelling of a payload capture carries none where its `match` twin does,
> and there is nothing to strip but still a mark to lift and emitters to steer.
>
> Guard `tests/scripts/a-view-bound-inside-a-branch-arm-sees-its-containers-reassignment.loft`
> (9 cells, four controls — including a LOOP, whose second pass must read the first pass's
> value rather than the frozen materialised one).  Two shapes filed rather than folded in:
> **loft#1396**, a view that IS an arm's value (`x = if k > 0 { h.inner } else { … }`), where
> both halves of the naming can be made to see it but the emitters have no per-arm copy — the
> widening was measured, found unsound alone (a binding made owner of a store it only views,
> whose scope-exit free names the container's) and backed out; and **loft#1397**, the
> OVERWRITE cousin (`w.st = Empty{…}`), which this doc's own clause settles as *not* a
> disturbance — the value is right and what is missing is loft#980's warning, whose exemption
> for a per-arm binding assumes the variant cannot change under it.

> **D-bind-19 — OPENED AND CLOSED (2026-09-06, the `@FR-O-Owner` walk) — a struct-ENUM
> PAYLOAD view was outside the materialise clause.**  `(B-Disturb)` names reassigning the
> container as one of the three events that end a place, and `(B-View)` materialises a view
> live across one — a copy taken at the bind, and the author told.  `x = tw.inner` on a struct
> does both since @PLN130 F8; `x = sh.inner` on a struct-ENUM did neither: the interpreter read
> the REASSIGNED subject's bytes at the payload's offset (`Empty { z: 0 }` answered as the old
> `Pay`, so `x.a` was `0`), `--native` answered the old payload by another route, and nothing
> was said on either backend.
>
> One derivation was short of one shape, in four readers.  A payload projection does not name
> its subject directly — `sh.inner` lowers to `OpGetField(if <tag == Holder> { sh } else
> { OpNullRefSentinel() }, …)` — so every chain that peels a projection to its container
> variable bottomed out at `None`.  Two of those chains were byte-identical copies documented
> as "mirroring" each other (`scopes::base_container_var`, which the walk that NAMES a view to
> materialise reads; `generation::container_element_base`, which the emitters that COPY it
> read), so the peel had to be written twice to work once — and a walk that names a container
> the emitter cannot is a copy the compiler reports and does not make.  Both now call
> `use_analysis::projection_container_var`, beside the `is_projection_op` list whose own doc
> already named them as readers of one shape.
>
> Three sites more, found by the same peel:
> * the establishment test (`scopes::established_stores`) did not count a struct-enum
>   reassignment at all — the literal builds into a `__ref_p2_N` work-ref and hands the
>   variable that BLOCK, and only the block's own `OpDatabase` was read, which names the
>   compiler work-ref and is filtered out.  A block whose tail is a heap `Var` establishes,
>   the same fact the bare-`Var` arm beside it already carried;
> * the materialise arm made the binding an OWNER without consulting `@FR-O-Override`, the
>   veto its var-copy sibling three hundred lines below already asks.  A `match`/`is` payload
>   binding is marked never-free by the parser precisely because it is a view, so materialising
>   one leaked a record per call on `--native` while answering no differently — measured as a
>   regression of this walk's own first cut, and the guard now holds it;
> * `@FR-O-Oracle` itself classified a payload view `Owned` (its `If` arm resolved the subject
>   through the subject's OWN definitions, found a mint and answered `None`), which is the
>   over-free direction its own caveat names.  Its user-visible face was the `lost-write`
>   WARNING telling an author a write that lands was lost — the tier that gates a library's CI
>   (loft#1395).  The same peel, in the oracle's borrow-base walk; Check A stays clean over the
>   1247-file corpus and the fuzz corpus.
>
> Guard `tests/scripts/a-payload-view-materialises-when-its-subject-is-reassigned.loft`
> (falsified at 5f4ac074: interpret 2 assertion failures → 0; native INERT, which is what a
> backend-divergence guard can be).  ⚠ The peel tests BOTH arms, and the sentinel one by the
> OP IT CALLS: a first cut that read "one arm is a `Var`" also claimed `a?` discharging a
> nullable parameter (`if a.rec != 0 { a } else { <mint> }`), which turned a generic's return
> delivery from an adopt into a copy nobody freed — one leaked record per call, caught by
> `1026-generic-discharged-null-return` under the wrap harness's leak gate and by nothing
> else.  Same class as loft#1379: recognise a lowering by what it BUILDS, never by node shape.  RESIDUAL, filed as loft#1394: a payload binding written
> INSIDE a `match`/`is` arm whose subject is reassigned in the same arm is still invisible —
> the walk handles a `Set` whose right-hand side is a value branch whole, through the `leaf`
> arm its own doc calls "deliberately coarse in both directions", so the bind and the
> disturbance are never separated in time.  Reaching it needs the walk's ordering model, not
> another classifier.

> **D-bind-16 — CLOSED 2026-09-03 (opened the same day, loft#1321) — `(B-Copy)` did not hold
> when the right-hand side is a branch JOIN.**
>
> `b = if c { a } else { [0, 0] }` ALIASES `a`, and so does the struct spelling, while the
> plain bind of the identical value one line away copies. Present on the shipped 2026.8.0,
> so long-standing rather than a regression, and identical on both backends.
>
> **It was filed as part of D-bind-15 and is not.** loft#1319's matrix carried a `??` column
> described as *"the same defect discharged"*; the control above has no nullability in it at
> all and aliases the same way, because `a ?? d` lowers to `if !isnull(a) { a } else { d }`.
> So the axis is the JOIN and the `?` was a passenger — and D-bind-15's fix leaves these
> cells exactly where they were, which is the other half of that measurement. Two defects
> sharing a symptom read as one until a cell moves only one of them.
>
> The destination ends up DEPENDING on the source (`b: vec<int> deps=[a]`) rather than
> owning: `classify_vec_bind` recognises a bare `Var` and an owned field read and nothing
> else, and the record side keys on `Value::Var` in both backends. That dep is what closes
> every copy path downstream — `owned_ref` requires `depend().is_empty()`, so the
> `OpBindOrCopy` arm written for exactly this shape is unreachable. The ownership ORACLE
> reports the joined binding `Owned` throughout, so the analysis has the right answer and the
> shipped fact does not.
>
> **An attempt was made, measured, and REVERTED (2026-09-03), and what it established is the
> useful part.** The rule it implemented — a join read ARM BY ARM, copying where every arm
> would have copied on its own — is right, and three facts the arm walk needs were not
> obvious:
>
> * reading the JOIN rather than its arms is wrong. `v[i]` is `τ?`, so the ordinary element
>   read is `c = vv[0] ?? [0]` — a branch. Copying it makes `(B-View-Depth)` unreachable for
>   its own documented spelling, and `bind-copies-or-views-the-whole-boundary.loft` goes red
>   on the cell that exists to say so.
> * a `??` HOISTS its subject into a temp (`__ncc_N = vv[0]`), so the arm the walk reaches is
>   a bare `Var` and the projection is one statement up. The walk has to resolve a temp
>   against what its block bound.
> * `use_analysis::is_projection_op` does not name `OpGetVectorNullable`, while
>   `generation::hoist::ELEMENT_ADDRESS_OPS` pairs it with `OpGetVector` for the same
>   question. Two one-homes, one notion, and reading only the first answers "not a place" for
>   every nullable element read.
>
> The third is why it was reverted: **a CALL arm may return a BORROW.** `it = get(b) ?? d`,
> where `get` answers a view of its parameter, has no syntactic projection in any arm, so the
> walk said "copy" and the caller then freed at scope exit what it had borrowed — loft#974's
> guarded behaviour, caught by `accessor_borrow::an_accessors_returned_view_names_its_parameter`.
> Trading a silent alias for a wrong free is not an improvement.
>
> **CLOSED 2026-09-03, with the copy face and the free face as one change.**  The rule the
> attempt implemented stands; what changed is WHERE it is applied.  Instead of deciding the
> join's copy and then re-deriving each arm's copy in both backends, each qualifying arm tail
> is rewritten into the bound spelling on a temp — `{ __lift_N = a; __lift_N }` — and the temp
> is bound by the SINGLE bind's own lowering (`scopes.rs::lift_join_arm_tails`, ownership.md's
> D-own-8 record has the whole arm-kind table).  The three facts above are then honoured for
> free: a `??` hoist is a variable the join hands back and is left alone unless its subject
> was a call the caller must copy; the join is never read whole; and a call arm that returns a
> borrow is bound by the same codegen arm that copies `it = get(b)` — no syntactic walk decides
> anything.  A record arm copies at the bind on its first and every later execution; a vector
> arm is refilled into a function-scoped buffer by `OpReplaceVector`, whose element type the
> scope pass now reads from the store registry it was handed for the purpose.
>
> Measured on both backends: the filed matrix (vector and struct, `if` and `match`, the `??`
> spelling), a parameter arm and a loop-element arm against their plain binds, the accessor and
> closure view arms (which also closes an interpreter/native split: the interpreter viewed and
> native copied), a null nullable subject binding its default, and the return position.
> `(B-View-Depth)`'s `c = vv[0] ?? [0]` stays a view.  Guard
> `1321-a-joined-binding-copies-what-a-plain-bind-copies.loft`, falsified at 26d17f4b.
>
> So the predicate the next attempt wants is not the syntactic walk but *"does this arm's
> VALUE borrow?"*, which the return TYPE already answers — loft#974 is the change that put
> the dep there (`-> Y974?["b"]`).

> **D-bind-15 — CLOSED (2026-09-03, loft#1319) — `(B-Copy)` did not hold for a NULLABLE
> heap local: a whole-value bind ALIASED its source.**
>
> `b = a` with `a: vector<integer>?` aliased `a`, and so did a nullable struct, while the
> keyed kinds copied — which is what said the axis was the `?` and not "heap". None of the
> rule's three exceptions reaches it: this is a whole value off an OWNED local, not a struct
> projection (B-View), not a borrowed base (B-View-Base), not an index or nested read
> (B-View-Depth).
>
> **One cause, and it is the spelling again.** `τ?` is `Optional(τ)` — the same storage
> behind a nullability marker (`@FR-L-Null`) — and FOUR sites decided the lowering by
> matching the `Type` variant BARE, so the wrapped shape reached none of them and the
> default (alias) stood: the vector-bind selector and its consumer, the interpreter's
> first-set dispatch, and the native generator's whole-record bind. D-bind-13 is the same
> sentence one construct over, and the keyed kinds took this peel in loft#1143 — so this is
> the third time the register records it, and the sites were siblings of ones already fixed.
>
> **The second half is that a copy must not turn ABSENCE into EMPTINESS.** A null source has
> to leave the destination null, not holding the store the copy allocated for it. Both
> mechanisms already existed and are reused rather than restated: `Stores::vector_replace`
> gains the guard `replace_keyed` has carried since loft#1150, and the record bind routes
> through `OpBindOrCopy`, whose borrow arm materialises and whose other arm adopts — which
> for the null sentinel is exactly "stay null". `OpCopyRefOrNull` was tried first and is
> wrong here: it binds `Stores::null()`, whose `store_nr` is a REAL slot with `rec == 0`,
> while `x == null` on a record lowers to `OpRefIsNull` and tests `store_nr == u16::MAX`.
> The two spellings of absence agree for the element read it was written for and not for a
> bound local.
>
> **Why eleven cells and both backends read green over it.**
> `tests/scripts/bind-copies-or-views-the-whole-boundary.loft` is the one place the
> copy-vs-view boundary is pinned, and every one of its eleven subjects was declared
> non-null — `??` appeared in it only inside element-read assertions. The axis it never
> moved is the one that broke. It has the nullable-subject axis now, and the four added
> cells fail on the pre-fix build.
>
> Guard: `tests/scripts/1319-a-nullable-whole-value-bind-copies-like-its-dense-twin.loft`,
> whose controls are `&` (must still alias), the struct projection and index read (must still
> view), the collection projection off an owned base (must still copy) and every keyed kind
> (must not move).

> **D-bind-14 — CLOSED (2026-09-03, loft#1316) — `(B-Ref-StoredRef)` admitted its one
> position only when the field came LAST, and not at all once the field was nullable.**
>
> The rule names exactly one place a prefix `&` is legal outside a `&τ` binding — a
> struct-literal field whose declared type is `reference<τ>` — and it conditions that on the
> FIELD'S TYPE and on nothing else. Two things were read instead.
>
> **The terminator.** The gate accepted the `&` only when the next token was `;` or `}`, which
> is what ends an ASSIGNMENT right-hand side. A field value also ends at the `,` before the
> next field, so `Trail { l: &pool[0], n: 4 }` was refused while the identical literal with
> the fields swapped compiled. Field ORDER is not in the rule, and a reader hitting this reads
> it as "`&` does not work here" rather than "`&` does not work last-but-one".
>
> **The type.** The same gate matched `Type::Reference` unpeeled, so a field declared
> `reference<τ>?` — whose bytes `L-Null` makes identical to `reference<τ>`'s — was not a
> `reference<τ>` field as far as the gate was concerned, and the `&` the pointer field exists
> for had no spelling once the `?` was written. That is D-layout-4's mechanism reaching this
> doc: one IR spelling, two source notions, and a site that reads neither the share marker nor
> `base()`.
>
> **Status — CLOSED.** The position is named rather than inferred: `AmpHead` is `No`,
> `AssignRhs` or `StoredRefField`, and the terminator set is read off it, so "which tokens end
> this operand" is answered once per position instead of once per gate. The type test peels.
> Guard: `tests/scripts/1316-a-nullable-reference-field-is-still-a-pointer.loft` (the nullable
> half, both backends) and `tests/scripts/150-amp-head-position.loft` (the position family,
> which already owned the `&`-in-a-field cell and gains the not-last one).

> **D-const-2 — CLOSED (2026-09-01) — `(Const-Value)` went unenforced on two
> append routes, and both mutated the CALLER while the parameter said `const`.**
>
> `fn f(p: & const vector<integer>) { p += [9]; }` compiled, and the caller's vector grew.
> So did `fn add(p: const hash<R[k]>) { p += R { … } }`, and its `sorted` and `index`
> twins. Both backends, exit 0, no diagnostic. `(Const-Value)` says the value behind a
> value-const name is read-only and **every** through-write is rejected, so both are
> deviations and neither was a design question.
>
> **One cause: the guard was attached to the lowering ROUTE rather than to the write.**
> `parse_assign_op_inner` picks among a dozen routes by target shape, and each route that
> could reach a const binding carried its own copy of the check — the vector builder, the
> keyed builder, the two text paths, and a `Value::Insert` bypass added when the struct
> constructor was found to miss the others. A per-route guard is exactly as complete as
> that route's target-shape test, so every shape a route declines falls through unchecked,
> silently. The vector route destructured `f_type.base()` against `Type::Vector`, and
> `base()` peels `Optional` but not `RefVar` — so the `&` spelling of a vector parameter
> was never asked. The keyed routes had no check at all.
>
> **The cure is that the question is not the route's to ask.** Whether a write is allowed
> is a property of the BINDING; the route only decides how it is lowered. One
> `guard_const_write(var_nr, op)` ahead of the dispatch replaces all five copies, which is
> also why the fix DELETES code. Every diagnostic keeps its wording; the one observable
> change is that a `;`-terminated statement now reports the same COLUMN as the same
> statement without one, because the guard no longer runs after the terminator has been
> consumed (`a_diagnostic_names_its_own_line_*` pins all three layouts).
>
> **Why it stayed unfound.** The rules' own oracle — `40-const-fields.loft` plus the
> `pln40_*` negatives — crosses `const` with the four quadrants and with struct-vs-enum,
> and with nothing else: no `&` cell, no keyed collection. An OPEN count is only as strong
> as the crossings under it. `tests/scripts/const-binds-through-every-append-route.loft`
> is that crossing, measured cell by cell against the pre-fix build.

> **D-bind-13 — CLOSED (2026-08-26, loft#1106) — `(B-Copy)` did not reach a bind whose
> destination was NULLABLE, and the same blindness left the callee's minted store
> unowned.**
>
> `r = pick(q, c)` where `pick(a: P?, …) -> P?` ALIASED its argument: `r?.x = 55` set `q`.
> The identical call written `-> P` copies, which is the oracle. `(B-Copy)` is not
> ambiguous here — a call RESULT is a whole value, not a projection, so none of B-View /
> B-View-Base / B-View-Depth reach it — and the same shape leaked one record per call on
> the arm where the callee minted its own store.
>
> **One cause, and it is a spelling.** `P?` is `Optional(Reference(P))`: the same storage
> behind a nullability marker. Every shape question in the heap first-bind dispatch —
> `gen_set_first_at_tos`'s arm list, `generation::dispatch`'s `heap_def_nr`, and
> `scan_set`'s `record_target` — was asked against the BARE type, so the nullable spelling
> reached none of them and the bind stayed a plain adopt of the returned `DbRef`.
>
> **The trap this issue carries, and it is worth stating.** The clean-looking twin
> (`q: P? = …`) was clean BY ACCIDENT: its dep was dropped only because the caller-side
> dep resolver has no `Optional` arm either, so the local read as an owner and earned a
> free. Classifying `Optional` "properly" without the runtime guard turns that clean case
> INTO the leaking one. So the deps strip and both backends' guard read ONE predicate,
> `use_analysis::nullable_join_first_bind`: a strip always has a guard under it, and a
> guard always has the free it was emitted for.
>
> **The fifth spelling was the one that hid.** `Function::make_independent` — the REMOVE
> half of the dep list — had its own inline arm list, and it too had no `Optional`. So a
> nullable local's dep could be READ (`Type::depend` peels) and SET (`Type::with_deps`
> peels) but never CLEARED: no `S?` could be made an owner by any caller, and the strip
> above was a silent no-op until this was folded onto `Type::deps_mut`. A dep list reached
> through three faces has to peel on all three.
>
> Enforced at: `use_analysis::nullable_join_first_bind` (the question), `scopes::scan_set`
> (the strip), `state/codegen::gen_set_first_at_tos` and `generation/dispatch` (the guard),
> `Type::deps_mut` (the one home for the mutable dep list).
> Guard: `tests/scripts/1106-a-nullable-heap-local-owns-its-bind.loft` — 15 cells including
> the null-answer arm, the struct-enum spelling, the nullable COLLECTION return (a
> different delivery, untouched) and an argument that OUTLIVES the call.

> **D-bind-12 — CLOSED (2026-08-23) — the struct write-back was a real defect; the
> collection alias was the RULE being under-stated.** Filed as two halves; measuring the
> second one properly split them apart.
>
> **Half one — FIXED.** Writing back a value BOUND from a sibling element vanished from the
> IR entirely and leaked the record with it: `for p in w { hs = p.0; p.1 = hs; }` left
> `w[0].1` unchanged, on both backends. `move_elidable_source`'s last gate is *"owns a
> transferable store"*, read off `Uses::def_vdb` — whose own doc says *`v = OpGetField(vdb,
> 0, _)` where vdb is OpDatabase'd* and whose walk never checked the second half. So `hs =
> p.0`, a read of an EXISTING element through a borrow, counted as owning one, and
> `move_rewrite` dropped the `OpCopyRecord`. That drop is sound only when the source is
> CONSTRUCTED — its build ops are retargeted onto the destination — and `hs` has no build
> ops, so the copy WAS the write. `collect_uses` now enforces the documented condition once
> the whole body has been walked (the `OpDatabase` may not have been visited at insertion
> time). Precisely scoped: emitted IR is **byte-identical on 120 of 120 scripts**, and
> `857`'s own allocation count is unchanged at 27, so the pointer-bind it protects is
> untouched.
>
> **Half two — RESOLVED (2026-08-24): `B-View` was under-stated, and the missing clauses are
> now written as `B-View-Base` and `B-View-Depth`.** It is NOT the owner question this entry
> first called it: `OWNERSHIP_MODEL § The law` and #426's RESOLUTION had already decided it,
> and #426 records that its own filed premise (*"an index / nested read must COPY"*) was the
> wrong read. So the code was right and the rules doc was incomplete. The whole boundary — 11
> cells, both backends — is pinned by
> `tests/scripts/bind-copies-or-views-the-whole-boundary.loft`.
>
> **Original reading, kept because the mistake is the useful part:** `hv = p.0` on a
> COLLECTION element aliases, and the first reading scored that against `B-Copy`. Measured
> across the 2×2 off a BORROWED base, three of four projection cells are views:
>
> | construct | element type | behaviour |
> |---|---|---|
> | struct field | vector-typed | view |
> | struct field | struct-typed | view |
> | tuple element | vector-typed | **view** — the cell that was filed |
> | tuple element | struct-typed | copy |
>
> The implemented model is *a projection off a borrowed base is a VIEW; off an OWNED base it
> COPIES* — gated explicitly by `classify_vec_bind`'s `depend().is_empty()`, deliberate
> (`cells = sc.v; cells[i] = h` writing through is @PLN25 p379's point), and with its
> alternative measured to CORRUPT (#426, `185-nested-boolean-vector`). Verified in both
> directions: an owned base copies (`a = h.items` ⇒ `[1,2]`), a borrowed one views
> (`b = s.vecf` ⇒ `[9,9]`), and the p379 write-through reaches the source.
>
> `B-View` above states the view for a **struct-typed** projection only, so the rules cannot
> express a model the language depends on. Per [README](README.md) that means the RULE wants
> extending, not the code changing — **a rules question for the owner, deliberately not
> decided here**, since widening `B-Copy` instead would delete p379's idiom and re-enter
> #426.
>
> ⚠ **The fourth cell was the one deviation, and `B-View` already settled its direction —
> FIXED the same day.** A STRUCT-typed tuple element copied while its three siblings viewed,
> and `B-View` says a struct-typed projection IS a view, so there was no decision to make:
> the code had to move. The stored-tuple element read took the synthetic struct's attribute
> type VERBATIM, carrying neither the base's deps nor the base variable — so the bind typed
> as an OWNER while holding a handle into someone else's record, and was handed an
> `OpFreeRef` to match. Its two siblings already did it right and one says why: the
> plain-tuple site's P197 comment (*"without this, `a.v.0` returns a `Str` whose ptr points
> into a freed host"*), and `fields.rs`'s struct-field read, which carries the base deps AND
> `depending(base_var)`. All four projection cells are now views, the bind carries `["p"]`,
> and the spurious free is gone. Precisely scoped: emitted IR is unchanged on **80 of 80**
> tuple-bearing scripts (the only file that differs is the guard's own).
>
> **The consequence is pinned rather than left to be discovered:** a three-step swap through
> a bound element does NOT swap (`held` names the place), which is what its three siblings
> already did. `test_swap_through_a_view_does_not_swap` asserts that, and
> `test_swap_by_holding_the_value` shows the cure — hold the VALUE (a scalar/text local) and
> rebuild after the write.
>
> Guards: `tests/scripts/reference-tuple-heap-element-through-a-record.loft` — 8 cells, the
> two write-back ones proven to fail on a pristine worktree at `c3d18a5f` while the shapes
> that always worked (`p.1 = p.0`, a fresh literal) pass there, which is what made this read
> as *"writes to `p.1` are fine"*.

> **D-bind-11 — CLOSED (2026-09-03, loft#1006): a `&(τ, …)` holds any element a struct field
> can, and the two options the entry below named were never a choice.**  The stack form cannot
> OWN a `text` element — a 16-byte `Str` borrow has no owner of its own in a frame — while the
> record form already performed this exact swap correctly on both backends through the loop
> variable over a `vector<(text, text)>`.  So a `&(…)` whose elements are not all scalars is a
> reference to the synthesized `__tuple<…>` RECORD — precisely what a `&S` is — and the one
> refusal site, `Parser::ref_var_type`, builds that type instead of refusing.
>
> **The whole distance was the LITERAL local.**  A tuple local bound from a heap-tuple RETURN
> was already that record; a loop variable was already that record; only `a = ("a1", "b1")` was
> stack-built.  Making EVERY heap-tuple literal local a record was measured and rejected: 17
> corpus scripts changed exit code, two of them crashes — nullable elements, fn-ref elements,
> nested and forward-referenced tuples, destructure ownership, none of which a `&` ever touches.
> The representation moves only for a tuple local that is the SOURCE OF A LINK: recorded in
> pass 1 at the link (`b = &a`, `c: &(…) = a`) and at the call argument, applied at the bind in
> pass 2 (`Parser::ref_linked_tuple_locals`, the `adopted_ret_defs` pattern), built by the
> return path's own `rewrite_tail_tuple_with_work_ref`.  With that scope the corpus answers
> identically with and without the change — measured script by script against the pre-change
> binary.
>
> ⚠ **Three cuts the matrix caught.**  Pass 1 typed the linked local as the stack tuple, and
> the record RHS was then UNBOXED back to it (`unboxes_stored_tuple`), so the link saw a stack
> tuple on pass 2; the unbox is declined for a linked local.  An annotated `c: &(text, text) =
> a` failed on pass 1, where `a` was still a stack tuple and the link's derived type disagreed
> with its own annotation; the link derives the record type as soon as the fact is recorded.
> And `slow-reference-parameter` fired for `&(text, text)` — advice whose premise (a by-value
> parameter already propagates writes) is FALSE for a tuple, whose by-value form is a copy
> (`1278-…`); suppressed for a `__tuple<…>` reference.
>
> **What stays refused, and each is a cell in `102-…`:** a NULLABLE element (its `?` does not
> survive the synthetic name), a fn-ref element, a nested tuple — the record cannot spell or
> lay them out as a field; a by-value tuple PARAMETER or a FIELD as the source (no local to
> build the record for, `T-Ref-Src`); and a `&(…)` containing a type declared LATER in the
> file, refused the way a tuple return is (`refuse_forward_ref_tuple_params`, which has to
> match the RESOLVED type — `resolve_adopted_stubs` has already replaced the stub by then).
>
> The rule moved with it: [tuples.md](tuples.md) gains `(T-Ref-Rep)` (which representation a
> `&(…)` names) and `(T-Ref-El)` now admits what a struct field can.  Measured on both
> backends, `LOFT_POISON` clean, no native leak: a literal local, a return-bound local, a loop
> variable, a local link in both spellings, `text` / vector / struct / keyed / three mixed
> elements, a linked local re-bound, copied, destructured, appended and passed to two callees
> 500 times.  Guard: `tests/scripts/reference-tuple-heap-elements-link.loft`, seven cells,
> falsified at 29743c5c.
>
> **D-bind-11 — OPEN (2026-08-19) — `&(τ, …)` admits only SCALAR elements, against
> B-Ref-Alias and B-Ref-Uniform.** `B-Ref-Alias` says the `&τ` annotation makes **ANY**
> binding — scalar OR heap — a live link, and `B-Ref-Uniform` says a `&τ` variable is used
> exactly like a `τ` one. A reference TUPLE obeys neither once an element is not a scalar:
>
> ```loft
> fn sw(p: &(text, text)) { t = p.0; p.0 = p.1; p.1 = t; }   // refused at the signature
> fn sw(p: &(integer, integer)) { … }                        // fine, both backends
> ```
>
> Since 2026-08-24 the admitted set also contains a VALUE ENUM, which has a `boolean`'s exact
> 1-byte layout and was excluded only because two spellings of "is this a scalar" had drifted
> (`data::is_scalar` is now the one home). The refusal for a heap element stands and names the
> element type.
>
> ⚠ **Re-measured 2026-08-23, and the SECOND of the two named options is already running.**
> This entry says closing it needs *"either an op family that writes the STACK form through a
> DbRef, or backing a `&(…)` carrying heap elements with a real record"* — and the
> record-backed path is not hypothetical. A `for p in v` over `vector<(text, text)>` performs
> the EXACT swap `fn sw(p: &(text, text))` is refused for, correctly, on BOTH backends
> (`[("a1","b1")] → b1|a1`). So the open question is not *"can a reference tuple carry heap
> elements"* — it demonstrably can — but the narrower *"can a `&(…)` PARAMETER or LOCAL be
> given the record backing the loop path already uses"*, which D-tup-2 made pointed by
> deliberately making tuple locals STACK-backed so `&` works for scalars.
>
> The SIGSEGV below still reproduces (re-measured the same day with the `text` arms re-added),
> and its cause is now stated one level down: a `text` element on the STACK is a 16-byte `Str`
> — `{ ptr, len }`, a raw BORROW — while the record form is a 4-byte handle, so the record ops
> read a `Str` as a handle and get a corrupt record number. That is also why `fn f(s: &text)`
> WORKS while `&(text, text)` cannot: the `&text` parameter writes into the caller's 24-byte
> owned `String` via `OpClearStackText`/`OpAppendStackText` and the owner never changes,
> whereas a tuple's text element has no owner of its own on the stack.
>
> ⚠ **That working record-backed path had NO guard** — no script in the corpus wrote a `text`
> tuple element through it — so the evidence this entry now rests on was one refactor away
> from vanishing silently.
>
> **The remaining half is a REPRESENTATION choice, and the ops are what force it.** With the
> offset corrected, adding the `text` arms still SIGSEGVs — measured — because the two element
> paths speak different op families:
>
> | | addresses | representation of a `text` element |
> |---|---|---|
> | plain tuple (`OpPut*` + frame position) | a slot in the CURRENT frame | 16-byte inline stack form |
> | reference tuple (`OpSet*`/`OpGet*` + DbRef + offset) | any frame, via the link | 4-byte record handle |
>
> A callee must write the CALLER's frame, so only the DbRef family can reach it — and that
> family speaks the record form. Scalars are immune because an `i64` is 8 bytes in both. Closing
> this needs either an op family that writes the STACK form through a DbRef, or backing a
> `&(…)` carrying heap elements with a real record. That is the decision `D-tup-1` records as
> missing, and it is why the refusal stands meanwhile.
>
> `tuples.md` states no rule for `&(…)` at all, which is how a composition of two specified
> features went unspecified; see its Deviations note.
>
> ⚠ **Narrowed 2026-08-23 — a SECOND B-Ref-Alias violation was sitting behind this one, and
> it was not about element types at all.** This entry reads as *"`&(…)` works for scalars,
> and the open half is heap elements"*. Measured across POSITIONS instead of element types,
> the scalar half only worked at a PARAMETER: at a local, neither `b = &a` nor
> `b: &(integer, integer) = a` linked anything, at any element type. The first dropped the
> `&` and bound a copy — silently, on both backends — and the second typed a reference over a
> value, which the interpreter read as a store index and `--native` refused with a raw rustc
> `E0308` handed to the user. A `&(boolean, boolean)` local answered the un-swapped tuple with
> exit code 0. Fixed the same day (tuples.md D-tup-2, guard
> `tests/scripts/reference-tuple-local-binding.loft`): a tuple local is stack-backed, so it
> joins the scalars at `OpCreateStack` and B-Ref-Alias holds at every position for every
> admitted element type.
>
> **What stays open here is exactly the heap-element half**, and the table above is why. The
> entry's own framing — element types — is what hid a whole axis: a rule quantified over "ANY
> binding" is falsified by a POSITION as readily as by a type, and only one of those two was
> being swept.

> **D-bind-10 — CLOSED (2026-08-09) — the ⚑ VITAL rule was enforced for HALF of each
> expression.** The rule named `x + &y` as a parse error and grammar.md's D-gram-4
> declared the positional rule "total". Measured, four shapes compiled on both backends:
>
> ```loft
> b = 1 + &a;                         // an operand — the rule's OWN named example
> b += &a;                            // a compound-assignment RHS: not a bind site
> fn g(a: integer) -> integer { 1 + &a }   // a block-final tail value
> s = S { x: &a };                    // a struct-literal field of a NON-reference type
> ```
>
> **The mechanism, and why the sweep had to be over positions.** The guard
> (`operators.rs::parse_operators`, deepest precedence level) decided by peeking the token
> AFTER the `&`-operand: a `;` or `}` there meant "the `&` was the whole RHS". That proves
> nothing FOLLOWS the `&` — never that nothing PRECEDED it — so every shape where the `&`
> is the LAST operand of a larger expression passed. The one sub-expression test,
> `pln87_amp_in_subexpr_is_error`, puts the `&` at the HEAD (`b = &a + 1`), which is the
> single sub-expression position that peek did catch. One cell, one direction.
>
> **The fix supplies the other half.** The first primary of a binding RHS consumes an
> `amp_head` marker; a `&` reached after any operator, or inside a nested construct, sees
> it gone. The accept condition is now `terminates AND at head` — the pair is total. The
> head is opened in exactly three places: a plain `=` RHS (`parse_assign_op`), a statement
> start (so a bare `&a;` still reaches D-bind-7's own message), and a `reference<τ>` field
> value (`B-Ref-StoredRef`). Emitted IR + native Rust are byte-identical over the
> eight-shape accept corpus.
>
> **What the position sweep still missed, and the axis it held fixed.** The first fix
> rejected `S { x: &a }` for every field type — and broke `store_compact_b2.loft`, where
> `Linked { link: &pool[i] }` fills a `reference<Leaf>` field. Legality there is decided by
> the field's TYPE, not by the `&`'s position, and a sweep that varies position while
> pinning the type reads as complete and is not. That is `B-Ref-StoredRef`, previously
> unstated anywhere in this doc.
>
> A pre-freeze error-add (`manifest::CONTRACT_VERSION == 0`; [COMPATIBILITY.md § The error
> surface is one-directional](../COMPATIBILITY.md)) — every program it rejects was already
> silently dropping the `&` and binding a copy. Lock-ins: `pln87_amp_as_tail_operand_*`,
> `pln87_amp_as_compound_assign_rhs_*`, `pln87_amp_in_block_final_expression_*`,
> `pln87_amp_in_struct_literal_field_*`, `pln87_amp_in_return_statement_*` in
> `tests/parse_errors.rs`, with the ACCEPT half — every legal `&` position, each asserting
> the write reaches the source — in `tests/scripts/150-amp-head-position.loft`. The @PLN87 ladder (L1–L6), the model + doc reconciliation (PR#436), the residual

D-bind-7 and D-bind-8 (closed below) are all verified; @PLN40's Const-Bind / Const-Value /
Const-ScalarCollapse / Const-Compose are shipped and enforced for struct fields, parameters,
and locals — and, since @PLN102 K1, for **enum-variant fields** too (their one former residual
gap, D-const-1, now closed).

> **D-bind-9 — CLOSED same day (2026-08-05).** B-Ref-Reshape landed from the maker's sentence,
> which named REMOVAL, so the other two of `B-Disturb`'s events kept silently downgrading a `&`
> to a copy — measured on both backends, each with a *"copied out of"* advice line:
>
> ```loft
> c = &s[30];  c.key = 5;                                     // RE-KEY: s[5] was ABSENT
> c = &bx.inner;  bx = Mid { inner: Box { n: 22 } };  c.n = 99; // REASSIGN: bx.inner.n was 22
> ```
>
> Closing D-bind-8 while these held was an accounting error: the deviation named all three
> mechanisms of one rule and the sign-off covered one. Both now refuse, under the C79 principle
> (*decline what we cannot implement safely*) rather than as a second special case. The
> reassignment arm is the same liveness walk with the cause filter dropped; the re-key arm
> refuses at `note_key_field_write` where the base `is_amp_link`, and needs no liveness question
> because the key write IS the use. Lock-ins `b_ref_reshape_rekey_through_amp_link_is_error` and
> `b_ref_reshape_container_reassign_under_amp_link_is_error`, with their positive twins (a
> NON-key field still writes through; a reference dead before the reassignment still does).
>
> **The sweep that found them is the reusable part:** 14 shapes of `&` — whole struct, whole
> vector, element, field, nested, keyed non-key, keyed key, local reassign, callee reassign,
> `&` param mutate, `&` param rebind, loop, branch, overwrite-in-place — each asserting the one
> thing `&` promises, that the write reaches the source. Twelve honoured it; these two did not.
> A rule with more than one producer needs a sweep, not a cell.

> **D-bind-8 — CLOSED by adding B-Ref-Reshape (@PLN130 F9, [loft#779](https://github.com/loft-lang/loft/issues/779)).**
>
> B-Ref-Alias is unconditional — *"the `&τ` annotation makes ANY binding a live LINK to the
> source"* — and the code had one exception: three shapes where the write was silently
> DISCARDED, on both backends.
>
> ```loft
> // (a) the element does not even MOVE: `remove(2)` drops the last element.
> c = &v[0];  v.remove(2);  c.n = 99;      // v[0].n was 11 — the write was discarded.
> // (b) the element moves (index 2 -> 1) and the link does not follow it.
> c = &v[2];  v.remove(0);  c.n = 99;      // v[1].n was 33.
> // (c) the reshape is in the CALLEE, through a `&` parameter — and here with NO diagnostic.
> fn shift(target: &Box, all: &vector<Box>) { all.remove(0); target.n = 99; }
> shift(v[2], v);
> ```
>
> **The resolution is REFUSAL, not repair** (maker, 2026-08-05: *"The removal of anything from
> a structure (vector for example) that has an open `&` relation (for us an edge case) should
> be forbidden on compile time"*), so all three are now compile-time errors under the new rule
> **B-Ref-Reshape** above. That makes the pair total with no runtime machinery: a `&` always
> writes through, because the one shape where it could not is rejected before it runs. It is
> the rustc bargain in loft's spelling — rustc refuses the mutation while a borrow lives, loft
> refuses the removal — and it is affordable precisely because the maker classes it an edge
> case. Following the link instead (`if link.pos > removed.pos { link.pos -= size }`, which a
> dense vector makes arithmetic rather than a lookup) was feasible and was declined: not worth
> per-link runtime arithmetic for an edge case.
>
> A pre-freeze error-add. `manifest::CONTRACT_VERSION == 0`, and
> [COMPATIBILITY.md § The error surface is one-directional](../COMPATIBILITY.md) says loft may
> always DROP an error and after the freeze may never ADD one — so every place loft is too
> permissive is a last-chance-to-add, and every program this rejects was already silently
> wrong.
>
> **Two things measurement changed about the shape of the fix**, both recorded in
> `probes/40-reshape-refusal/README.md`:
>
> - the cross-frame half does **not** key on the `&` token. A plain struct PARAMETER aliases
>   the caller's element exactly as a `&` one does (cell X9: `fn w(t: Box) { t.n = 99 }` called
>   as `w(v[2])` writes 99 into the caller's `v`), and loft's own `warn_redundant_amp` advice
>   tells authors so — refusing only the `&` spelling would mean taking that advice trades a
>   compile error for a silent lost write. loft#779's own table asserted the opposite (*"plain
>   param copies (C86), so nothing to lose"*); that row is measurement-contradicted;
> - a plain LOCAL bind stays exempt for the opposite and equally measured reason: it does not
>   alias across a reshape, because @PLN130 F2 materialises it and says so.
>
> **Why D-bind-4 did not catch it.** Its lock-in is `c=&v[0]; c=9; v[0]==9` — no reshape. The
> rule was stated unconditionally and verified only in the simple shape, so a later change could
> narrow it without flipping any cell. The conformance lock-ins are now
> `b_ref_reshape_*` in `tests/parse_errors.rs` (six refused shapes and three positive ones) plus
> `tests/scripts/149-reference-survives-callee-reshape.loft`; `tests/scripts/145-…` and `774-…`
> pin the PLAIN-bind behaviour, which is unchanged.
>
> **What it did NOT close:** the other two disturbances. The maker's sentence named removal, so
> the RE-KEY and REASSIGNMENT causes were scoped out and still downgrade a `&` to a copy — now
> tracked as **D-bind-9** above, under the widened C79 principle rather than as an open question.

> **Landed via @PLN102 K1 (verified, closed):**
> - **D-const-1 — enum-variant `const` / value-const fields are now enforced identically
>   to struct fields.**  `enum Shape { Circle { const radius: integer }, … }`; after
>   `if s is Circle { … }`, the direct write `s.radius = 9` is now REJECTED at parse time
>   (backend-independent, so no interp/native split). Root cause was that the field-write
>   guard resolved the field table via `Parts::Struct(fields)` only; the fix extends BOTH
>   the leaf-field block (`validate_write`) and the value-const chain-walk
>   (`lhs_frozen_through`) to also walk `Parts::EnumValue(_, fields)` — the variant def's
>   `attributes()[f_nr]` aligns with its `EnumValue` field order, so the const_field /
>   value_const checks apply unchanged (verified: the positive cells stay accepted, no
>   over-reach into a pattern-bound local copy). Diagnostics now name the owner as a
>   "variant". A pre-freeze error-add (`CONTRACT_VERSION` was 0). Regression: the boundary
>   matrix graduated to `pln40_enum_variant_*` in `tests/issues.rs` (negatives + the
>   over-reach guard) and the positive cells in `tests/scripts/40-const-fields.loft`, both
>   backends. The remaining laundering-via-local / -return / -generic scopes stay deferred
>   (Phase 3, post-1.0; see
>   [../plans/40-const-fields/const-model-phase2.md § Phase 3](../plans/40-const-fields/const-model-phase2.md)).

> **Landed via @PLN87 / PR#436 (verified, closed):**
> - **D-bind-0** — `&τ` is now `Type::RefVar` (a reference type the variable carries); `&` is
>   no longer a general operator (a dedicated diagnostic rejects it elsewhere). Reads/writes
>   dispatch on the variable's RefVar type, not a per-expression flag.
> - **D-bind-1 / D-bind-2 (NORTH STAR)** — scalar live read + write-through: `a=3; b=&a; b=4;
>   a==4` → verified on interp **and** native.
> - **D-bind-3** — struct-field reference write-through: `b=&s.x; b=4; s.x==4` (the #415 gate
>   no longer blocks it).
> - **D-bind-4** — vector-element reference: `c=&v[0]; c=9; v[0]==9`.
> - **D-bind-6** — `&`-parameter link: `fn f(b:&integer){b=4}; f(a); a==4` → both backends.
> - **D-bind-doc** — `OWNERSHIP_MODEL § The law` rewritten to "heap aliases by default; `&`
>   binds a live REFERENCE"; the write-back framing is gone.
> - **D-bind-7 (the last residual ⚑ vital position)** — a bare `&a;` statement (and a
>   block-final `{ &a }`, the same leak) is now parse-rejected. The fix sits at the statement
>   chokepoint, `parser/expressions.rs::parse_assign`: a statement that BEGAN with a prefix
>   `&` whose `&` was not consumed by an assignment is the non-binding use the rule forbids.
>   The `operators.rs` guard clears `amp_pending` whenever it has already reported the `&`
>   (sub-expression / non-place), so the flag is still set at the chokepoint only in the
>   unreported bare/block-final case; a `started_with_amp` gate keeps a leaked flag from a
>   nested `&(…)` parse from mis-firing. Verified on interp **and** native; `pln87_d_bind_7_*`
>   in `tests/parse_errors.rs` (bare statement · bare field statement · block-final). The
>   caret points at the `&`.
>
> The former deferred case has **landed**: `&`-write-back from a CALL/var RHS
> (`fn f(o: &Obj){ o = mk() }`) now routes the RHS through a transferable owned temp, so the
> write-back reaches the caller (`a.x == 9`) — verified on interp **and** native. The parse
> rejection is gone; `tests/issues.rs::pln87_amp_writeback_from_call_writes_back` is an active,
> passing test (no longer `#[ignore]`d).

## Carried by binding.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [binding.md](binding.md) now states only what is open.

### D-bind-18 — OPENED AND CLOSED (2026-09-06, loft#1392): a vector link followed the link's own whole-value write but not the source's

`(B-Ref-Alias)` makes a `&`-annotated binding *a live LINK to the source instead of a copy*,
and `(B-Ref-Uniform)` says a `&τ` variable is used exactly like a `τ` variable.  A vector link
is not a stack deref: it SHARES the source's `DbRef` — the lowering a `&vector` parameter takes
— so a fresh backing on EITHER side re-points one of the two and the link stops being live.
loft#1371 gave the link side its cure (`@FR-B-Ref-Write`): a whole-value write THROUGH the link
clears the shared store and refills it in place.  The source's own whole-value write is the
same question one step out, and had none:

```loft
v = [1, 2];
q = &v;
v = [7, 8, 9];
println("q={len(q)} v={len(v)}");     // q=2 v=3 — q is still on the store v held at the bind
```

Both backends, silently.  The struct and text spellings follow a rebind (`OpCreateStack`,
loft#1371) and a link to a struct FIELD follows one because a field rebuild is in place
already, so the vector local was the one shape left.

**Closed by registering the SOURCE in the same set as the link**, so its rebuild clears and
refills the shared store.  Two things that fall out of it, and both are the rule rather than
the repair:

* The registration is gated to PASS 2.  The set outlives a pass, so a registration made when
  the link is parsed reaches the source's own DECLARATION when pass 2 re-reads the body from
  the top — dropping the allocation that gives the source a backing store at all.  Measured:
  `v = []` after a link compiled to a clear of a variable `--native` had never declared.
  Pass 2 reads in order, so the declaration is parsed before the link registers.
* `(I-Comp)` is stated over the PLACE, not the spelling.  Once the source rebuilds in place,
  the clear reaches the link too — so a build that reads the destination THROUGH the link
  (`a = [for i in 0..r.len() { (r[i] ?? 0) * 2 }]` under `r = &a`) reads what it is emptying.
  It used to work by accident, because the fresh backing left the link on the old store.  The
  link's partner is now part of both halves of `snapshot_read_destination`: the read TEST
  (`parts_read_a_vector_link`) and the RENAME, which redirects reads spelled through the link
  onto the snapshot with the destination's own.  The corpus caught this — the alias CONTROL in
  `1194-a-comprehension-reads-its-destination` answered `[]`.

Guard `a-vector-link-follows-a-rebind-of-its-source` (14 cells: the rebind in both spellings,
twice, and to empty; the three writes through the link and the three through the source; a
comprehension and a literal reading through the link; and the struct-field, text and no-link
controls), falsified at 3360fb93 — exit 1 -> 0 on both backends.

### D-bind-17 — CLOSED (2026-09-06, loft#1372, @PLN153 phase 4): a `&` link carries a nullable slot

`(B-Ref-Intro)` admits `&τ` for every τ; `(B-Ref-Write)` and `(B-Ref-Uniform)` make a `&τ`
variable a τ variable that writes the source; `(F-ParamRef)` makes a `&` parameter the
write-back channel.  Nothing restricts τ, so `&integer?` should link a nullable slot.  It does
not: a link's inner type is asked BARE at every read and write site — `??` sees `&integer?`
and refuses its default, `+` and a copy-out see a non-null slot and warn, the interpreter's
write dispatch has no arm for the wrapper, native's parameter type does not match the write —
and the `&` of a nullable LOCAL fell past the scalar and record arms of the lowering and bound
a silent COPY: `q = &x; q = 7` left `x: integer?` at 5 on both backends, with nothing said.
The parameter's body was refused on its write and on its read with retype messages that named
neither the link nor a cure.

Found by @PLN153 phase 4's `optional` screen: the lowering's `is_scalar` closure and record
test were two of the 353 bare shape tests, and the cell built to reach them was the finding.
The entry stood OPEN for a day, with both spellings declined where the link type is built
(`@FR-B-Ref-Reshape`: a link loft cannot honour is refused, never downgraded).

**CLOSED 2026-09-06.**  The answer was the shape the entry named — one spelling of "the slot
behind a link" that every site asks through — and it is `Type::base()`, because `Optional(τ)`
shares `τ`'s storage exactly: a `&τ?` has the SAME representation as its `&τ` twin, and the
absence rides the slot's own sentinel.  There was no new mechanism to build, only nine sites
each asking the link's inner type BARE and so matching no arm for the wrapper:

- the interpreter's `RefVar` READ and WRITE dispatch (`state/codegen.rs`), which panicked
  *"Unknown reference variable type"*;
- native's local-link bind and read arms (`generation/dispatch.rs`, `generation/emit.rs`),
  which emitted a binding with no right-hand side at all;
- native's `&`-parameter write-back — the displacement test, the text coercion and the
  boolean one — where a `&text?` wrote `*var_p = "z"` with no `.to_string()`;
- the `??` subject (`parser/operators.rs`), which peeled `Optional` but not the LINK, typed
  its own result `&integer?`, and reported every default as the author's error;
- the retype check (`variables/mod.rs`), which refused `&integer? = 7`;
- the argument match (`parser/mod.rs`), which compared the parameter's referent against an
  argument reading as plain `τ`, failed, and passed the argument BY VALUE — the callee then
  deref'd an integer as a stack ref;
- the bare-`null` conversion, which found no `OpConv…FromNull` returning a link type and
  DROPPED the store in silence, so `q = null` left the source at its old value.

Guard: `tests/scripts/1372-a-reference-links-a-nullable-slot.loft`, plus
`tests/scripts/153-a-link-to-a-nullable-slot-carries-its-slot.loft` — this entry's own decline
guard, whose cells stayed and whose expectation flipped from the refusal to the answer the
rules always gave.  The whole-value write through a `&` link to a text, a struct or a vector —
the NON-nullable controls of the same matrix — is loft#1371, closed apart.

### D-bind-16 / D-bind-11 closure summaries

**D-bind-16 CLOSED 2026-09-03** (loft#1321): `(B-Copy)` is read ARM BY ARM at a branch join.
Every arm tail a plain bind would copy — a variable, a parameter, a loop element, a call whose
record return the caller must copy — is rewritten into the bound spelling on a temp
(`scopes.rs::lift_join_arm_tails`), bound by that plain bind's own lowering, and the join
borrows the temps; an arm a plain bind would view stays a view (`(B-View-Depth)`'s `vv[0] ??
[0]` is the control).  It is the copy face of ownership.md's D-own-8, closed by the same
change; the reverted attempt's three facts (a `??` hoists its subject; the join is not to be
read whole; a call arm may return a borrow) are all honoured by binding each arm through the
lowering that already knows them.  Guard:
`tests/scripts/1321-a-joined-binding-copies-what-a-plain-bind-copies.loft`, falsified at
26d17f4b on both backends.  ⚠ A branch consumed as a call ARGUMENT is NOT copied: an argument
aliases (calls.md F-ParamHeap).  ⚠ And a binding the join is not the one assignment of keeps
the runtime join bind it had (`r = x; for … { r = v[i] ?? x }` copies through
`OpBindOrCopy`): one variable carries one type-level fact for all its Sets.

**D-bind-11 CLOSED 2026-09-03** (loft#1006): a `&(τ, …)` holds any element a struct field can.
The two representations the register named were never a choice — the stack form cannot own a
`text` element, and the record form already did this exact swap through the loop variable —
so a `&(…)` whose elements are not all scalars is a reference to the `__tuple<…>` record, a
linked tuple LOCAL is built as that record, and the rule is written down as `(T-Ref-Rep)` in
[tuples.md](tuples.md).  A nullable, fn-ref or nested-tuple element stays refused and the
refusal names it.

- **D-bind-18 — a view was stale once its container GREW, and the rule said it survived**
  (2026-09-06, loft#1373).  `(B-Disturb)` listed three place-ending events and an append was
  not among them, so the materialise walk never fired: `d: S = v[0]` followed by two hundred
  appends read `4294967296` on both backends with strict stores silent — while the SAME code
  with two appends read `1`, because nothing had reallocated yet.  One shape, two answers,
  decided by an allocator fact the author cannot see.

  **What made it look settled, and was the finding.**  `(B-View-Depth)` said in as many words
  that a view *"survives a source realloc"* and named
  `85-store-lifetime-reference-default-views.loft` for it.  Measured: that guard's cell A
  appends to the INNER vector of a `vector<vector<integer>>`, so its view — which names the
  OUTER element SLOT — reads the repointed handle and survives.  The realloc of the container
  the view NAMES was never measured, and at that level the guard's own shape answers
  `len(b) == 0`.  A rule sentence resting on a cell that measures one level in is the
  `(N-Idem)` "OPEN: 0" shape again: the claim was re-measurable and had not been re-measured.

  **Status — CLOSED.**  `(B-Disturb)` gains GROWING as its fourth event; the existing answer
  applies unchanged (the view MATERIALISES and the author is told, `(B-View)`), and
  `(B-Ref-Reshape)` refuses a `&` INTO a container that grows while the link is live.  ANY
  growth disturbs rather than only one that provably crosses the capacity, because the
  alternative gives one program two meanings.  `scopes.rs::grown_containers` is the fourth
  event's home, apart from `reshaped_containers` only so the ADVICE can name the right
  statement — a reader told "removing an element renumbers the others" goes looking for a
  `remove` that is not in the function.  The five growth spellings all name their container
  at arg 0 and are read through one shared walk (`containers_named_by`).

  The in-versus-to distinction `(B-Ref-Alias)` needs was already in one place and needed no
  new test: `base_container_var` answers `None` unless the right-hand side is a PROJECTION, so
  `pe = &e; e += [4]` and `pv = &o.v; pv += [3]` keep compiling while `c = &v[0]; v += [x]`
  is refused.  Measured against the sibling checkout's `&` cells and `503-vector-reference-alias`.
  Guard: `tests/scripts/1373-growing-a-container-ends-the-places-inside-it.loft`, eight cells
  on both backends with four controls (the inner-realloc shape 85 measures, a nested field
  read, a view dead at the growth, and a view with no disturbance at all — the last is the
  boundary a fix that materialised everything would cross).  ⚠ **It shipped for one commit with a spurious materialise, and the fix's own guard could not
  see it.**  `OpNewRecord(parent, tp, fld)` names its container in TWO parts, and reading the
  parent alone shook every view rooted at that variable whichever field it named:
  `moros_editor`'s `undo_pop` reads `e = s.us_entries[idx]` and appends to `s.us_redo`, so each
  undo entry came out of a copy, the undo stack silently stopped recording, and `undo_depth`
  answered 0 where 3 was due — a program's meaning changed, with only an advice.  A
  field-qualified growth was left UNCOLLECTED for one commit (`fld == u16::MAX` is the
  whole-variable append), which is the honest direction while the question cannot be answered:
  a missed disturbance costs a materialise, a spurious one costs a program its meaning.  It is
  answered now (loft#1384): a struct FIELD is a container of its own, so the walk compares
  PLACES rather than variables — `base_container_place` gives the view its `(var, byte offset)`,
  `Stores::field_position` converts the growth's field NUMBER into that same offset, and
  `same_place` matches them with `u32::MAX` on either side meaning the whole variable, so
  reassigning the parent still ends every place inside it.  The `&`-refusal path has no store
  to convert with and keeps the conservative answer, which for a REFUSAL is the safe direction.  What
  caught it was `make ci`'s `moros_editor` html smoke, the only thing in the tree exercising
  that shape, and it is NOT in the corpus the emission diff walks (`tests/fixtures/libs` is
  outside it) — so a four-file diff read as a small blast radius while a library was broken.
  The COLLECTION-typed twin was filed rather than
  bundled and closed one commit later (loft#1377): it needed the copy EMITTED rather than the
  dep stripped, because a collection bind decides copy-vs-view at PARSE time
  (`classify_vec_bind`) and cannot hear a scope-pass strip.  The emitted shape is the one a
  whole-vector copy already takes — a `__lift_N` buffer refilled by `OpReplaceVector` — and the
  local NAMES it rather than owning it, which a loop makes load-bearing: left owning, the
  local's scope-exit free released the buffer and the next iteration refilled a freed store
  (`rec=3735928559` under the arena poison).  `Contract: strained` — the rule
  gained an event.

### the status line formal/README.md's area table carried until 2026-09-04

**0 open** — D-bind-11 CLOSED 2026-09-03 (a `&(τ, …)` with a heap element is a reference to the `__tuple<…>` record; it had read: `&(τ, …)` admits only SCALAR elements, against B-Ref-Alias/B-Ref-Uniform) and D-bind-16 CLOSED the same day (a join binds every arm a plain bind would copy through its own temp, loft#1321); (the D-bind-11 entry read: — the two backends represent a reference tuple differently and `text` is the first element where that shows; loft#1006) — **B-Ref-AnnotationOnly is now total** (D-bind-10, 2026-08-09): a `&` that was the LAST operand of an expression (`b = 1 + &a`, `b += &a`, a block-final `1 + &a`, `S { x: &a }`) used to compile, because the guard peeked only the token AFTER the operand; `B-Ref-StoredRef` records the one legal non-binding position, a `reference<τ>` field. **B-Ref-Reshape** landed (@PLN130 F9, loft#779): disturbing a container while a `&` reference into it is LIVE is a compile error, for all three of `B-Disturb`'s events (removal, re-key, container reassignment). It is the first application of C79's 2026-08-05 *decline-what-we-cannot-implement-safely* revisit, whose reason is forward compatibility: an error can be dropped later, a silently different semantics cannot. Also closed: `&` is a TYPE ANNOTATION (`&τ` = `Type::RefVar`), @PLN87 ladder L1–L6 + D-bind-7 closed; the @PLN40 two-level `const` model (Const-Bind/Value/…) shipped, and D-const-1 (enum-variant const) closed via @PLN102 K1 — enforced identically to struct fields, both backends

## Deviations carried by binding.md until 2026-09-29

Closed entries moved here from the rules chapter's register (RELEASE.md § 5b), as written.

* **D-bind-68** *(opened 2026-09-28, CLOSED 2026-09-28; loft#1717)* — `(B-Scope)`: a `for f in
  x#fields` walk is unrolled at compile time and created its variable BY NAME, so it missed
  loft#915's per-loop binding twice over.  Pass 1 leaves the name naming the LAST walk's
  variable, so pass 2's first of two same-name walks took the second's slot; the body's pass-1
  deps then named a binding pass 2 never set, and `--native` declared it inside one field's
  block and freed it outside (`cannot find value var_f`) while `--interpret` ran.  And the walk
  never registered its variable as a loop's (it ends its loop before the body, which has no
  run-time loop for a `break`), so a later `for f` over anything was refused as shadowing "a
  local named 'f'".  **Fix.**  `Parser::create_loop_var`, keyed per loop, and
  `Variables::served_as_loop_var`, which records the fact without making a loop current.  Guards
  `tests/scripts/1717-two-fields-walks-spelling-one-name-are-two-bindings.loft` (native),
  `tests/scripts/1717b-a-loop-beside-a-fields-walk-may-reuse-its-name.loft`.

* **D-bind-67** *(opened 2026-09-27, CLOSED 2026-09-27; loft#1700)* — `(B-Scope)`: a bind after
  the block that bound its name had ended was refused when it had another type (`if c { w = 1 }
  w = "x"` — *"cannot change type from integer to text"*), because the parser keeps one
  variable per name and the new binding inherited the ended one's type.  The rule already
  called it a new binding; whether it may retype was left open until the owner ruled it may.
  **Fix.**  Where the ended binding would refuse the new type (loft#1145's
  `retype_would_be_refused`, so a program that compiled is unchanged), the bind gets a
  variable of its own — named by the statement's position, which both passes agree on — and
  the spelling names it until the enclosing block ends.  Deciding "ended" on pass 1 as well
  needed the binding's block recorded on both passes (the innermost block ordinal, which
  allocates nothing); a body local rebound in ANOTHER loop stays loft#1145's split, asked by
  loop ordinal because loop numbers differ between the passes; a `&` link bind keeps its own
  refusals.  `retype_would_be_refused` now also answers a vector against a scalar, a text or a
  record, which every arm of `change_var_type` refuses.  Guard
  `tests/scripts/1700-a-bind-after-its-block-ended-is-a-new-binding-at-any-type.loft`.

* **D-bind-66** *(opened 2026-09-26, CLOSED 2026-09-26)* — `(B-Scope)`, the silent half of
  D-bind-65: a variant's field bound inside a SUB-pattern — a slice element (`[Vn { rs }]`), a
  slice tail, a tuple element, a struct field (`S { w: Vn { rs } }`) — pointed the name at its
  binding, and the list recording what the name meant before was dropped at all five sites.  So
  after the `match` an outer `rs` read the capture (`4` where the program holds `100`), on both
  backends with no diagnostic, and a later local of that name was refused naming `_mv_rs_1`.
  Only the top-level enum arm restored its names.  **Fix.**  Each site hands its list to the
  arm's frame, which `end_pattern_arm` restores.  Alongside it, `pattern_variant_enum` maps a
  value typed as one VARIANT to its enum, as the top-level dispatch does: `w = Vn { rs: 4 }`
  in a tuple was refused as "Vn, which has no variants".  Guard
  `tests/scripts/a-sub-pattern-capture-leaves-the-outer-name-alone.loft`.

* **D-bind-65** *(opened 2026-09-26, CLOSED 2026-09-26)* — `(B-Scope)`: a pattern's names were
  bound BY NAME (`add_variable`) at every site but the struct-enum field and the `is` capture —
  slice elements (`[e, ..]`), `..rest`, repetitions, bare names, `v @ pat`, tuple elements and
  plain-struct fields.  So a later `match` binding the same name reused the first arm's
  variable, and a different type was refused as a type change (`match a { [e, ..] => … }` then
  `match b { [e, ..] => … }` over `vector<Pt>` and `vector<text>`); a local of that name after
  the `match` was refused the same way; and a capture spelled like a PARAMETER bound into the
  parameter.  **Fix.**  One home, `Parser::pattern_binding`: a new binding per occurrence, as
  a `for` loop's variable is (loft#915) — own name `e#N`, reported as `e` — with the spelling
  pointed at it for the arm.  The arrow seals the arm's names and the end of its body
  restores what they named before (`end_pattern_arm`), so frames nest with nested `match`es.
  The `never-read` lint now reports a `name#N` binding under its spelling, which also covers a
  second `for i` loop it used to skip.  Guard
  `tests/scripts/a-patterns-names-end-with-its-arm.loft`.

* **D-bind-64** *(opened 2026-09-26, CLOSED 2026-09-26; loft#1690)* — `(B-Copy)`: a vector bind
  written into the arms of a `??` copies through `lower_vec_copy_bind`, which minted the element
  temp `_elm_N` on the SECOND parser pass only for a plain-variable copy (its `in_arm` leg forces
  the allocation there).  Every later element temp in the function shifted between the passes,
  and a valid program was refused naming one: *"Variable '_elm_3' cannot change type …"*.
  **Fix.**  The `in_arm` copy mints on pass 1 too — the one leg of the allocation test that
  answers the same on both passes.  Guard
  `tests/scripts/1690-a-bind-written-into-the-arms-of-a-coalesce-is-the-same-on-both-passes.loft`.

* **D-bind-63** *(opened 2026-09-26, CLOSED 2026-09-26; loft#1686)* — `(B-Copy)` / heap.md
  `(H-Copy)`: a local bound from a top-level VECTOR constant did not copy.  The constant is
  pre-built once in the write-locked constant store and its use site answers a view of it
  (`OpConstRef`); the bind kept the view, so the first write through the local reached the
  lock — an internal panic, "Write to read-only store … locked by: Store::lock", on both
  backends and in every spelling (`v = NAMES; v += […]`, an index write, a `??` or `if` arm,
  a typed local).  The corpus never wrote through such a bind (the `x ?? GLOBAL` crossing:
  two files).  **Fix.**  `Parser::classify_vec_bind` gives a constant read the bare-variable
  verdict, `CopyVar`, so the bind takes `lower_vec_copy_bind`'s copy on every route (an arm
  reaches the same classifier); read positions — the index, the length, the iteration —
  keep the view.  The one route that is not a bind, a callee writing through a PARAMETER
  handed the constant, stays a fault (a parameter aliases its argument, and a copy there
  would be a silent lost write) and is now the defined one heap.md `(H-WriteLocked)` asks
  for: the constant store is locked as a user lock (`Store::lock_constant`, both backends)
  and the refusal names a constant and the cure.  Guard
  `tests/scripts/1686-a-local-bound-from-a-vector-constant-copies-it.loft`; the parameter
  route `tests/exit_codes.rs` `a_write_through_a_parameter_to_a_constant_is_a_defined_fault`.

* **D-bind-62** *(opened 2026-09-25, CLOSED 2026-09-25; loft#1679)* — `(B-Scope)` at the CALL
  spelling, both halves of it.  The rule says a bind after the `}` starts a NEW binding and a
  read of the ended one is refused; a fn-ref local got neither.  With a rebind,
  `for f in fs { … } f = two; f(1)` called the ENDED binding — `one` where that binding was
  still live (an `if` arm: a silently wrong FUNCTION, answering 101 where the program spells
  201) and the exhausted sentinel, printed `null`, where it was a loop variable, on
  `--interpret` only.  With no rebind, `for f in fs { … } f(1)` was not refused at all, on both
  backends, where `for i in 0..3 { } i` is.  **Where (measured).**  One question — is a fn-ref
  callee name a READ of that variable? — answered at two places, and each missed it.  The scopes
  pass gives the second binding its own slot (`scan_set`'s `copy_variable`) and remaps every
  read through `var_mapping`: `Var`, `TupleGet`, `TuplePut`, `FnRef`, `FnRefDnr` — @PLAN53
  cluster 2 extended that list once and stopped one member short of `CallRef`, whose callee
  index `scan` copied through verbatim.  And the parser's two indirect-call sites (one per
  ARITY) resolve the name with `self.vars.var(name)` instead of through the bare-name read that
  asks `check_block_scope`, so `(B-Scope)`'s refusal had a hole exactly at the one kind of local
  a program can only use by calling.  `--native` ran the first half because it names its locals
  `var_<name>`: both bindings are one Rust local there and the rebind shadows it — right by
  accident, which is why one backend answered and the other did not.  **Closed** by remapping
  the `CallRef` arm and by asking `check_block_scope` at both call sites (it is `pub(crate)`
  now, and its doc names itself the one home for the question).  Guards:
  `tests/scripts/1679-a-call-through-a-rebound-name-reaches-the-new-binding.loft` (11 cells: what
  ended the binding, the later value, arity 0 and 1, integer and text return, the call in a
  nested block / a later loop / a comprehension, plus the reads that were never wrong) and
  `tests/scripts/1679b-a-call-through-a-name-whose-block-has-ended-is-refused.loft` (the refusal
  half; `1600b`/`1600c` remain the homes of the READ spelling, and neither called the name).

* **D-bind-61** *(opened 2026-09-24, CLOSED 2026-09-24; loft#1664)* — `(B-View)` for a KEYED
  payload binding.  `match k { Ky { k_look } => { k = Ky { … }; k_look += [r]; len(k_look) } }`
  answered a wrong length on the interpreter and panicked `--native` with *"Store access out of
  bounds"*, in a linked group or out of one: the binding kept naming a store its subject no
  longer owned.  **Where (measured).**  The disturbance walk opened a view only for a binding
  typed `Reference | Enum | Vector`, so a keyed binding was never condemned, and the materialise
  arm had no keyed copy.  **Closed** for the PAYLOAD binding (`scopes::keyed_payload_view`: a
  keyed `_mv_` binding the parser marked never-free) — the walk opens it, and the materialise
  copies it at the bind through `OpReplaceKeyed` into a buffer, the keyed twin of the vector
  arm's `OpReplaceVector`.  Asked of the payload binding only, because widening the walk's type
  list alone was measured unsound: a keyed projection bound off an owned base already copies
  (`(B-View-Base)`).  Guard: cells a6 and b2 of
  `tests/scripts/1664-a-group-member-payload-binding-materialises-like-any-view.loft`.

* **D-bind-60** *(opened 2026-09-24, CLOSED 2026-09-24; loft#1664)* — `(B-Ref-Reshape)` for a
  `&` link to a whole COLLECTION.  `p = &o.v; o = S { … }; p += [7]` was MATERIALISED with an
  advice, where the record spelling (`p = &o.r`) and the scalar one (`p = &o.r.n`, D-bind-56 the
  day before) of the same program are refused — and this rule is explicit that the copy is the
  one answer a `&` may not be given: *"loft will not quietly downgrade the reference to a
  copy"*.  **Where (measured).**  A FOURTH spelling of *"did the author write `&`?"*.  The
  refusal gate asked `is_amp_link` (the struct projection the parser leaves unlowered) and
  `is_place_link` (the `RefVar` local a scalar or text place lowers to); a collection link is
  neither and carries `is_amp_container_link`, which the MATERIALISE walk already read — to
  spare the link from its own container's growth — while the REFUSAL walk beside it did not.
  The parser's own note beside `amp_container_link` had written the question down as open and
  named this rule's answer to it.  **Closed** by the third disjunct at the refusal gate.  A
  PLAIN collection bind off a borrowed base is deliberately NOT in the set: `(B-View)` says that
  one materialises, which is what the marker exists to distinguish.  Radius, measured: TWO
  cells tree-wide and ZERO published libraries (42/42 against a current index).  One is a
  control that pinned the materialise, moved to
  `parse_errors::b_ref_reshape_reassignment_of_a_container_link_base_is_error` exactly as the
  reference-INTO cell in the same file was; the other is a `@PLN157` bytecode-comparison cell
  under `doc/claude/plans/`, found by a sibling's gate after the first measurement said "one"
  having walked `tests/scripts` alone.  A refusal's radius is every `.loft` in the tree, and
  `doc/` holds 1376 of them.  The three events that do NOT disturb a reference TO a container are
  unchanged.  It is also what makes [collections.md](collections.md)'s `D-col-6` answerable for
  this spelling: a link that can never become a copy still names its origin field at every
  write.

* **D-bind-59** *(opened 2026-09-24, CLOSED 2026-09-24; loft#1665)* — `(B-View)` for a `text`
  PAYLOAD binding.  A `text` binding holds a copy of the characters, and #673 makes a write
  through it mean the field write by MIRRORING the copy back into the subject after each write.
  The mirror was unconditional, so `match e { Ei { v } => { e = Ei { v: "zz" }; v += "x" } }`
  wrote `"abx"` into the NEW `e`, on both backends and with no advice line, where a record
  payload binding in the same program materialises and says so.  **Where (measured).**  The
  disturbance walk opened a view only for a binding typed `Reference | Enum | Vector` (or a
  place link), and the mirror was a parse-time statement the scope pass could not tell from an
  author's own `e.v = v`.  **Closed** at the walk, which is the home of the question: a mirrored
  binding is recorded on the `Function` (`text_payload_views`), the walk opens it as a view of
  the place its `OpGetText` read names (`scopes::text_payload_place`), and the mirror is emitted
  inside a `text_mirror` block, which the scan drops for a binding the walk condemned.  The
  binding keeps the copy taken at the bind, which is what materialising a text view means, and
  the advice is the record view's.  A per-binding verdict, loops included — a parser-linear
  *"reassigned since the bind"* flag would have answered wrongly through a back edge (the m5
  cell).  The same change makes every materialise advice name a payload binding as the author
  wrote it (`v`, not `_mv_v_1`), through one helper, `variables::author_spelling`.  Guard
  `tests/scripts/1665-a-text-payload-binding-stops-writing-into-a-reassigned-subject.loft`, 7
  cells, falsified at `a75a4d1ed` on both backends.

* **D-bind-56** *(opened 2026-09-24, CLOSED 2026-09-24; @PLN167 C2)* — `(B-Ref-Reshape)` for a
  `&` link to a SCALAR or TEXT place.  `c = &v[1]; v += [x]; c = 99` compiled and lost the write
  (`v[1]` stayed 22), `t = &v[1].s` read `null` after a growth, and `c = &v[1].n` crashed
  `--native` on a misaligned store address — on both backends, where the same program with a
  record link is refused.  **Where (measured).**  The view walk opened a view only for a binding
  typed `Reference | Enum | Vector`, and the refusal read `is_amp_link`, a marker set only where
  the parser leaves a struct projection unlowered.  A scalar or text place is LOWERED to a
  `RefVar` local (@PLN167 A3, B1, C1), so it reached neither.  And one step further in: the walk
  took every `Set(c, …)` for a re-binding, while on a scalar link `c = 99` writes the place — so
  a write after the disturbance cleared the shake it should have reported.  **Closed** by
  `scopes::is_place_link` (a non-argument `RefVar` local is a `&` link by construction) at the
  two readers, and by `scopes::link_set_repoints`, the one re-point-or-write test, now asked by
  both the walk and the interpreter's `set_var` (it was that function's private arm, D-bind-36).
  A link to a LOCAL is unchanged: `OpCreateStack` names no container.  Measured: twelve refused
  cells over integer, `u8`, float and text links, element / element field / record field, and
  all four events plus a callee's growth, both backends; the controls (dead at the event,
  another container, re-pointed after it, a link to a local) unchanged.  Guards
  `tests/scripts/167-a-scalar-or-text-link-refuses-a-disturbance-of-its-container.loft`,
  `tests/scripts/167-a-scalar-or-text-link-survives-what-does-not-disturb-it.loft`.
  **R4 of the plan asked whether overwriting a linked text while a borrowed read of it is live
  dangles** (`for c in t { o.a = … }`): measured no — a text walk binds its source once
  (`(I-Text)`), both backends under `LOFT_POISON` and `LOFT_STRICT_STORES` — so no clause is
  owed; the `loop-source-written` warning now reaches a write through the link as it reaches a
  write to the field.

* **D-bind-52** *(opened 2026-09-23, CLOSED 2026-09-23; loft#1631)* — `(B-Copy)` for a RECORD
  read through a link.  `y = e` with `e = &z`, and `y = p` with `p: &P` a parameter, bound `y`
  with a borrow dep on the link, so `y` VIEWED the record and `y.n = 9` wrote `z` — where a
  whole-value bind copies and a plain local source (`y = z`) does.  Both backends agreed.
  **Where (measured), three places.**  The parser's link peel gave a record read a borrow dep
  (the `&`-parameter half on purpose, loft#772: without it `w` owned the caller's store — a copy
  settles that the other way, since `w` then owns the copy and nothing else).  Both emitters'
  whole-record copy arms asked the SOURCE's shape through `base()`, which does not see through a
  `&`, so a link source took the plain alias; native also spelled the source `var_<src>`, the
  pointer rather than the record behind it.  And a NULLABLE destination (`y: S?`) pre-inits its
  slot before the value runs, but `intervals.rs` gave only the bare `Reference` an early
  `first_def`, so `y` was handed the slot of the `&S?` link it was about to read — a link frees
  nothing, so its range ended at that read — and the pre-init zeroed it (interpreter panic).
  **Closed** by peeling a record link read to an OWNED value, asking `peel_link()` at every copy
  arm (and rendering native's source through the Var emitter, one `OpBindOrCopy` witness helper
  on the interpreter), and asking the early-`first_def` test through `base()`.  A program that
  wrote a `&` record parameter through a plain alias (`w = p; w.n = 9`) no longer writes it, and
  is told so: the `&` is then unused.  Guard
  `tests/scripts/a-record-read-through-a-link-is-copied.loft`.  Found while closing D-bind-51.

* **D-bind-51** *(opened 2026-09-23, CLOSED 2026-09-23)* — `(B-Copy)` with `(C-Ref)` for a plain
  bind from a LOCAL link.  `y = e` with `e = &z` kept the link's `&τ` type into the bind, so `y`
  became a second link: `z = 9` afterwards read 9 through `y`, and `y = 4` wrote `z`, on both
  backends; the annotated `y: integer = e` was refused as "cannot change type from integer to
  &integer"; and a record there panicked the interpreter's allocator and emitted Rust that did not
  compile.  **Where (measured).**  `parse_assign_op_inner` peels a bare link read to its value
  type, but asked only of a `&` PARAMETER — its comment took every other `RefVar` source for an
  explicit `&` bind, which by then has already been lowered to `OpCreateStack` / `OpVarRef` and is
  no bare `Var`.  **Closed** by peeling a local link's read the same way.  Guard
  `tests/scripts/a-plain-bind-from-a-link-copies-the-value-it-reads.loft`.  Found while fixing
  loft#1614.

* **D-bind-50** *(opened 2026-09-22, CLOSED 2026-09-22; loft#1612)* — `(B-Copy)` for the
  destination of a `??` CHAIN of three or more operands.  A plain bind copies a heap whole
  value, and the two-operand spelling does: it is lowered per arm, and an arm's bind is that
  copy.  `??` is left-associative, so a longer chain hoisted its subject into a `__ncc_N` temp
  and bound the destination to THAT — and the temp views the operand it chose, by the rule that
  makes it a borrow (loft#723: freeing it would be a use-after-free).  So the destination shared
  the chosen operand's store: on the interpreter for a record, on BOTH backends for a vector and
  for a chain whose first operand is a call.  Silent — the shared store reads right until the
  operand is rebound, and then the destination reads the new value, or freed memory once both
  release it.  `(O-NoDiverge)` was broken with it, since `--native` copies a record's temp.
  **Closed** in the PARSER, by making `??` RIGHT-associative (`grammar.md` (G-Assoc): its RHS
  parses at its own level, the one place associativity is decided).  `??` is associative as a
  value — either grouping answers the first present operand, in the same order, short-circuiting
  the same way — so no program's meaning moves; what moves is the SHAPE.  Right-associated there
  is no chain subject to hoist: every operand is an ARM of a plain `if`, and an arm's bind is the
  copy `(B-Copy)` describes, which is exactly why the two-operand spelling was always right.
  `LOFT_COALESCE_LEFT_ASSOC=1` parses the left-associated form again.
  A chain the AUTHOR parenthesised — `(x ?? y) ?? d` — still hands the parser a subject that is
  not a variable, so that one is right-associated in the scopes pass instead
  (`reassociate_coalesce_chains`, loft#1591's rewrite with its destination gate lifted;
  `LOFT_NO_COALESCE_REASSOC=1` keeps the gate).  Without it the parenthesised spelling kept the
  hoisted form and the destination VIEWED the operand — on the interpreter only, so it was a
  `(O-NoDiverge)` divergence as well (guard cell `c27`).
  A SECOND defect was in the way, pre-existing and reachable by hand as `b = x ?? (c() ?? d())`:
  a chain BLOCK as an arm is not a tail a plain bind may take (its tail is the compiler temp),
  so `sink_set_into_arms` declined the whole branch — and then the chosen VARIABLE's arm beside
  it recorded no hand-off, so at a droppable type `x`'s record was released from the destination
  AND from `x`, the second release on freed memory (`M8,R8,D8,D8`; loft2-21 measured this shape
  on the first cure, and a value cell cannot see it — which is why the guard's droppable cells
  exist).  Closed with it: the chain block is sunk as a UNIT, the arm binding the join exactly as
  a statement of its own would, and a chain block handed to the sink as the VALUE declines, since
  that statement is already what a sunk arm writes.  Guard
  `tests/scripts/1612-a-coalesce-chain-copies-the-operand-it-chooses.loft`.

* **D-bind-49** *(opened 2026-09-21, CLOSED 2026-09-21; loft#1554)* — the CALL-SITE half of
  `(B-Ref-Reshape)` read one spelling of one event.  A call handed both a container and a
  reference into it (`shift(v[2], v)`) was refused only where the callee REMOVES through a bare
  `&vector` parameter — `removed_params_map`, built from `OpRemoveVector`/`OpRemove` over a `Var`
  typed `RefVar`.  The rule exempts no spelling (*"A plain PARAMETER is NOT exempt"*), `(B-Disturb)`
  names four events *"wherever they happen"*, and `calls.md` `F-ParamHeap` states the call-site
  reach in so many words — so every other shape was a deviation, and a silent one: the program
  compiled and read or wrote the element that moved, identically on both backends.
  **Measured**, 14 hazard cells and 9 controls, on the build before the cure: ONE hazard refused
  (`t = &v[2]; shift(t, v)`, and that by the FRAME half).  The other 13 compiled — a plain
  `vector` container (a lost write: `33` for `99`), a struct FIELD as the container, the field
  itself handed in (`shift(b.items[2], b.items)`), a GROWTH of either (`11` for `99`; the issue's
  `0` once the vector reallocates), a growth two frames down, a `&vector` growth, a `&` element
  parameter, an element bound earlier from a field, a reference INTO an element
  (`shift(v[1].inner, v)`), a struct-enum element, a `?`-discharged element, and the `&vector`
  removal itself once the call sat inside a FORMAT STRING, where the element argument is the
  nullable read `OpGetVectorNullable` — a second spelling the element test did not know.
  **Cure:** the call-site half reads `disturbed_params_map` (@PLN164 C3's fact, the one the frame
  half and the materialise already read), unioned with `removed` so `LOFT_NO_CALLEE_DISTURB=1`
  keeps the narrower refusal; the argument's place is composed with the callee's
  (`call_arg_place` · `compose_param_place`), and the element test names places through
  `value_view_places` (`arg_references_element_in`, `names_element_in`) — a local bound earlier
  from a field is resolved by its own bindings, an unresolved one is taken to name the container.
  No new fact and no new predicate.  The message gained the growth form (*"`stash` grows `bag`,
  and a container that outgrows its allocation moves every element…"*); the removal form is
  byte-identical.
  **What keeps it from over-reaching, each a compiling control with a hand-computed answer on
  both backends:** a SIBLING field's growth (`100`), a callee disturbing ANOTHER container
  parameter (`94`), a SCALAR read out of an element (`35`), an element of another container
  (`123`), a callee that only reads, scalar elements, and the index workaround the message names.
  A call through a FN-REF is the one edge the refusal cannot follow; it compiles and loses the
  write (`33`), as before.
  **Blast radius (measured, not predicted):** `loft --check` over the 1628-file corpus names
  **0** files (the harness proven able to fail on a hazard cell first); over the consumer sources
  (crawler, dryopea, moros, zero-trust-shared-files, loft-libs-*) every file that parses names 0,
  and a syntactic scan of all 325 for a call handed an indexed element beside its container or a
  prefix of it finds ONE site, whose element argument is an integer (`hud.itex[i] ?? 0`).
  ⚠ **How it stayed open while reading CLOSED.**  This was cured once, on 2026-09-17, and the
  cure was lost in a JOIN: two streams had each numbered a D-bind-47 the same day, the join kept
  the other stream's `scopes.rs` and merged the PROSE — so D-bind-48's *"Two shapes the call-site
  half also missed"* paragraph below and `calls.md` both described a call-site half that read
  `value_view_places` while the code read `removed`, the six tests were gone, and the issue
  closed on a `Fixes` trailer with `main` still answering `2 2 0` on its own repro.  Nothing
  gated the difference: a register entry is prose, and the merged tree's guards were the other
  stream's.  The check that found it is the cheap one — run the issue's repro on the joined
  tree — and it belongs to every join that squashes a stream carrying a `Fixes`.

* **D-bind-48** *(opened 2026-09-17, CLOSED 2026-09-17)* — `(B-Ref-Reshape)`'s CALLEE clause was
  never implemented for a GROWTH, so a disturbance one frame down did not refuse.  The rule states
  the reach outright — *"The disturbance may be in this frame or in anything the frame CALLS"* —
  and `(B-Disturb)` states it for all four events: *"an event disturbs WHEREVER IT HAPPENS … at any
  depth."*  @PLN164 C3 gave that reach to the MATERIALISE walk and not to the refusal.
  **Measured**, 17 cells, both backends byte-identical.  The purest is all-`&`, with the callee
  disturbing the very parameter it was handed: `fn vgrow(v: &vector<H>, n) { v += [mk(n)] }` under
  a live `e = &v[0]` compiled and released one resource TWICE (`M1 M2 R1 D1 D1 D2`).  The droppable
  population `(H-View-Drop)` owns the same shape without the `&`, and a callee's REMOVAL from a
  FIELD of a parameter was not refused either — `removed_ref_params` keys on
  `OpRemoveVector(arg0)` / `OpRemove(arg1)` over a bare `Var` typed `RefVar`, so producer 1
  reached only a parameter named DIRECTLY (`fn drop_last(all: &vector<Box>) { all.remove(2) }`,
  pinned by `b_ref_reshape_callee_removal_under_local_amp_link_is_error`).  Producer 2 — a call
  handed BOTH a container and a reference into it, `shift(v[2], v)` — is a separate route with its
  own message and was never in question here.
  **Where:** `def_reshape_refusals` ran `ViewWalk::run(..., Some(removed), None, …)` — `removed`
  passed, `disturbed` withheld.
  **Two shapes the call-site half also missed, and the blast radius, measured before landing.**
  Its element test did not count the nullable element read a FORMAT string passes
  (`print("{shift(v[2], v)}")` printed the moved element's stale bytes) nor a `?`-discharged one;
  the test reads the place through `value_view_places` now, and a SIBLING field's growth still
  compiles.  Over the 1629-file corpus and the consumer sources (crawler, dryopea, moros,
  zero-trust-shared-files, loft-libs-*) the wider refusal names ONE program —
  `164-element-place`'s `g27`, written to hand a container and its element in, now called
  through a fn-ref, which is the one edge the refusal cannot follow.
  ⚠ **The lesson, and it cost the first reading of this defect a much larger cure.**
  `ViewWalk::shake_plain_places`'s doc SAID it works *"over PLAIN views only, leaving every `&`
  link alone"*, which described its INTENT for the materialise consumer and not what it does.  The
  code has no `&` test anywhere: `shake_places_keyed` builds its hit list from `same_place &&
  !spared && !names_container_itself`.  So a link INTO a disturbed container was already shaken and
  already MATERIALISED — the silent `&`-to-copy downgrade this rule exists to forbid, emitted with
  the copy-out advice — and what was missing was only a CONSUMER reading that answer.  A comment
  that states intent where a reader will take it for behaviour is worth more care than a wrong one,
  because it is believed.  That doc now states what the function does: the shake is followed by a
  restore keyed by VIEW over `whole_container`, which is `(B-Ref-Alias)`'s in-versus-to distinction
  and not a `&`-versus-plain one.  The quotation is kept in the PAST tense deliberately — nothing
  gates a doc that quotes a code comment, since `check_doc_drift.sh` reads plan links, time
  projections and retired-feature claims, so the next edit of that comment cannot report this line.
  **Cure:** `reshape_refusals` builds `disturbed_params_map` and threads it into
  `def_reshape_refusals` → `ViewWalk::run`'s fifth argument.  Two hunks; no new fact and no new
  predicate.
  **What keeps it from over-reaching, both measured as controls:** `names_container_itself` still
  spares a link TO a container, so `157-view-header`'s `grown_between` keeps reading 11 — D-bind-46's
  in-versus-to distinction does the work — and the gate is unchanged (`amp || drops`), so a
  NON-droppable plain view across a callee's growth still compiles, still materialises and still
  says so.  That last cell is the one that decides the unit, and it holds.
  **Consequence worth stating:** the refusal reads the same `callee_disturb_enabled` switch as the
  scope pass, so `LOFT_NO_CALLEE_DISTURB=1` restores pre-C3 blindness on BOTH sides at once and is
  no longer a clean A/B for the materialise alone.  That is the switch's honest meaning — one
  rule's reach, two consumers — and the halves stay tellable apart at the symptom, a refusal being
  loud where a materialise is quiet.
  **Blast radius (measured, not predicted):** corpus A/B over 1591 files, **0 changed** — no
  program in the tree writes the shape, which is why the corpus could not have caught it and is
  also why it is not evidence of safety for real code.
  The message gained a population-dependent joiner: the callee form names the callee's act before
  the reason, and for a droppable view the growth CAUSES the copy (`so`) where for a `&` link the
  two are parallel facts (`and`).  With one joiner both read *"would grow `b`, and … and …"*.
  Found while re-measuring `heap.md` D-heap-11, whose own **Boundary** paragraph described this
  asymmetry incorrectly and is corrected there.

* **D-bind-47** *(opened 2026-09-17, CLOSED 2026-09-17)* — `(B-Ref-Reshape)`'s refusal could not
  see a GROWTH of a container held in a FIELD, so the rule's answer depended on where the
  container was stored.  Measured on both backends, four cells varying only the container's home
  and the disturbance: `v = [...]; c = &v[0]; v.remove(1)` refused, `v += [x]` refused,
  `b.v.remove(1)` refused — and `c = &b.v[0]; b.v += [x]` **compiled**, materialising the link
  into a copy with the copy-out advice.  So a `&` was silently downgraded exactly where this rule
  says REFUSE: a write through the link is lost, and where the element owns a droppable the
  resource is released TWICE (`M1 M2 R1 D1 D1 D2`, both backends).  `is_amp_link` is not the axis
  — the identical binding refuses under `remove`.
  **Where (measured).**  A growth names its container by field NUMBER (`OpNewRecord(b, tp, 1)`)
  while a view carries a byte OFFSET; `Stores::field_position` is the only converter, and
  `grown_containers` returns early without a store, leaving every field-qualified growth
  UNCOLLECTED.  `def_reshape_refusals` ran `ViewWalk::run` with `database: None`, which
  `binding-history.md` records as deliberate — *"the `&`-refusal path has no store to convert with
  and keeps the conservative answer, which for a REFUSAL is the safe direction"*.  That is an
  AVAILABILITY premise, and it no longer holds: the parser owns `pub database: Stores` at
  `check_reshape_under_reference`, the layouts are registered when each struct is declared, and
  `field_position` answers `u16::MAX` — *cannot say* — for anything it does not know.  A missed
  disturbance is still the safe direction; it was not a reason to leave one whole class invisible.
  **Cure:** the refusal walk is handed the same store the materialise walk has always had.  The
  callee reach (`disturbed`) stays `None` deliberately — that is @PLN164 C3's separable widening.
  **Blast radius (measured, not predicted):** one corpus file, this rule's own guard
  `a-link-to-a-whole-container-survives-that-containers-growth.loft`, whose control cell
  `a_link_into_the_container_is_unchanged` asserted the unrefused answer in so many words
  (*"keeps today's answer"*).  It pinned the compiler, not the rule, and is now
  `parse_errors::b_ref_reshape_growth_of_a_field_container_under_amp_link_is_error` — a cell that
  refuses belongs in the refusal harness, because it takes a whole `.loft` file with it.
  Found while measuring `heap.md` D-heap-11, whose cure reuses this gate.

* **D-bind-46** *(opened 2026-09-16, CLOSED 2026-09-16)* — `(B-Ref-Alias)`'s in-versus-to
  distinction at a container held in a FIELD.  `d = &cv.data; cv.data += [7]; cv.data += [8];
  (d[2] ?? -1) + len(d)` read **0** where 11 is right, on both backends, with the copy-out advice
  rather than silence.  The SAME body with the two appends moved into a CALLEE read 11 — and that
  half is pinned by `157-view-header`'s `grown_between` — so one program had two meanings
  depending on which side of a call the append sat on.  A `remove` under the live link behaved
  the same way, and a write through the link after a growth was lost (`d[0] = 99` left `cv.data[0]`
  at 1).  **Where (measured).**  `record_target` already draws the distinction the rule needs —
  *"`pe = &e` names no container while `pw = &w[0]` names `w`"* — but through `base_container_var`,
  which reaches only a whole VARIABLE.  A link to a container held in a field IS a projection, so
  it named `(cv, off_data)`: the same place `cv.data[0]` names, the place model carrying one
  variable and one field OFFSET.  `(B-Disturb)`'s growth ends only the second of those — it moves
  every ELEMENT, while it merely repoints the field SLOT that a reference TO the container
  re-reads.  **Closed** by reporting, off the walk that already answers the place
  (`projection_place_of`, exposed as `use_analysis::view_source_place_indexed`), whether the chain
  read an element; `ViewWalk` records the place such a binding names DIRECTLY and
  `names_container_itself` spares it from `Grown` and `Reshaped`.  `Reassigned` still shakes it —
  that is the one event which leaves the slot itself with nothing to point at, and sparing it
  would hand out a link to a store the reassignment released.  The DIRECT place is matched, never
  `resolve_view_root`'s: a binding whose own container is a view resolves to the OUTER container,
  and growing THAT does move the record holding this binding's slot.
  Measured on both backends, byte-identical across a 19-cell matrix, clean under `LOFT_POISON`,
  the interpreter leak gate and the native leak check, and identical under `LOFT_HOIST_VERIFY=1`
  and `LOFT_NO_VIEW_HOIST=1` — the native header hoist being the one place the cure could have
  gone wrong on a single backend.
  ⚠ **The `&` has to be asked for, and the first cut did not ask.**  A plain whole-collection bind
  copies at PARSE time into its own `__vdb_N` backing — identically off an owned base and off a
  borrowed PARAMETER — which read as licence to key the mark on the collection TYPE alone.  Off a
  LOOP VARIABLE it does not copy: `for b in bv { c = b.vecf; b.vecf += [9] }` aliases and
  materialises today, and `(B-View)` says it must keep doing so, because a plain bind already meant
  value semantics.  Hence `amp_container_link`, set where `amp_collection_bind` is decided, and NOT
  a widening of `is_amp_link`, whose readers are struct-shaped (the whole-record write route gates
  on `Reference`/`Enum`; the re-key refusal and `(B-Ref-Reshape)`'s refusal would decline different
  programs).
  Guard `tests/scripts/a-link-to-a-whole-container-survives-that-containers-growth.loft`.
  ⚠ Two of its cells passed BEFORE the fix and are controls rather than coverage, which matters
  before either is cited as evidence: a keyed `a = &s.h` across an add, and a nested
  `d = &o.inner.data` across a growth.  The nested one passes only because `grown_containers`
  cannot name a nested place at all (its `OpGetField` arm requires the container to be a bare
  `Var`), so it is a MISSED disturbance rather than a considered answer — it would pass the same
  way with the cure reverted.
  **Left open, and it is the same rule's other half:** a collection link is still quietly
  downgraded where the rules say REFUSE.  `c = &s.h; s = Host{…}` materialises in silence, which
  `(B-Ref-Reshape)` calls a compile-time error, because that refusal reads `is_amp_link` and a
  collection bind is not in its population.  Closing it widens which programs the refusal declines
  and needs its own measurement over the corpus and the published libraries.
  Found via @PLN164 C3's matrix (loft#1543).  ⚠ That issue's body names
  `ViewWalk::shake_plain_places` and `use_analysis::view_source_place_indexed` as code C3 landed.
  Both are live code now — C3 merged in `46aaf2967` — but neither existed when the issue was
  filed: the body describes a design sketch as though it had shipped.  Date what it claims against
  that merge rather than reading it as a record of the tree.

* **D-bind-45** *(opened 2026-09-15, CLOSED 2026-09-15)* — `(Const-Value)` for a
  value-const value handed to a PLAIN heap parameter.  A plain struct or vector parameter names the
  caller's record (`calls.md` F-ParamHeap), so `fn bump(a: Account) { a.balance = 999 }` called as
  `bump(acct)` with `acct: const Account` wrote the caller's balance on both backends with no
  diagnostic.  **Decided (owner, C124): by SIGNATURE.**  A value-const value reaches only a parameter
  declared `const`; whether the callee's body writes it is not asked — a line's meaning is judged by
  the line and the signatures it names (C121), and a proof about a body stays an optimisation's
  (C122).  **Built, as a WARNING first — an error since 2026-09-28, when every shipped library had declared its read-only parameters** (`const-to-plain-parameter`, the owner's rollout: 102 call
  sites in consumer libraries reach 17 read-only helpers not yet declared `const` — DESIGN_DECISIONS.md
  C124 § Rollout lists them; it becomes an error once they are): the call gate beside D-bind-44's `&`
  gate, reading the parameter's `const` from
  the definition's attribute (`Attribute.value_const`, now serialised in the IR store so a cached
  stdlib or library keeps it); the standard library declares its read-only heap parameters `const`;
  a generic instance, an interface stub, a bound-method stub, a default-value function and an
  overload dispatcher carry the `const` of what they copy; an `Op*` primitive is exempt, since only
  the standard library's own bodies can call one and `const` there means an immediate operand.
  Guards `tests/scripts/a-const-value-reaches-only-a-const-parameter.loft` and
  `…-passed-to-a-const-parameter-is-legal.loft`.  **Closed with the function-reference half:** a function type spells `const` (`fn(const T)`,
  carried as `ConstParams` beside the parameter types rather than as a wrapper on them), so a call
  through a reference, a builtin's callback (`map` / `filter` / `any` / `all` / `count_if` /
  `reduce`) and a lambda's own `const` parameter are judged by the same signature rule; a
  plain-parameter function is refused where `fn(const T)` is expected, a join of functions keeps the
  `const` every arm declares, and a closure's capture of a value-const value is a value-const field
  of its record.  Guards `tests/scripts/a-const-value-reaches-a-function-reference-only-through-const.loft`,
  `…-is-not-written-through-a-const-lambda-or-a-closure.loft` and
  `…-through-a-const-function-type-is-legal.loft`.  loft#1540.

* **D-bind-44** *(opened 2026-09-15, CLOSED 2026-09-15)* — `(Const-Value)` through a VIEW: a value-const
  value was written, in silence and on both backends, through a loop variable over its elements
  (`for f in ps { f.x = 5 }`), an element or field bound to a local (`p = ps[0]`, `q = w.p`,
  `r = &ps[0]`), a loop over such a view, a loop over a value-const FIELD, and `f#remove`; and it could
  be handed to a `&` parameter, whose callee wrote it.  `ps[0].x = 5` was refused, which made the rest
  look enforced.  **Where (measured).**  The guards resolve a write to its ROOT variable and ask that
  variable's flag; a view is a variable of its own, bound by a projection, and nothing gave it the flag.
  **Closed** by marking the view at its bind (`Parser::mark_const_view`): a projection, a `&` link or a
  loop over a value-const value — the binds `(B-View)` and `(B-Ref-Alias)` make aliases — gives the view
  the value's read-only flag, so every existing guard refuses a write through it, and the refusal names
  the view and what it views.  A bare-variable bind (`(B-Copy)`) and a call's result (`(O-Move)`) are
  not views and stay writable, as does a scalar or `text` copied out.  A value-const value or view
  handed to a `&` parameter is refused at the call (plan 40 rule 4).  Measured on both backends over
  25 cells, and by `--check` over 8566 `.loft` files (this repository and the consumer checkouts):
  no file gained or lost a refusal, 461 of them never reaching pass 2; one over-approximation remains — the mark follows the bind, so a view local rebound to a
  fresh value is still refused.  Guards `tests/scripts/a-view-of-a-const-value-is-read-only.loft` and
  `…-leaves-its-copies-writable.loft`.  loft#1540.

* **D-bind-43** *(opened 2026-09-14, CLOSED 2026-09-15)* — `(B-Ref-Write)` for a VECTOR written through a
  local `&` link from a named vector: `a: vector<integer> = [1]; n: vector<integer> = [7, 8]; c = &n; c = a`
  must make `n` a copy of `a`, and left `n` at `[7, 8]` on both backends while `c` read a copy of `a` — the
  write was lost and the link named a store of its own.  The same from another vector link and in a loop.
  Silent; the same on 2cff47dfc.  A literal build or an append through the link was right.  **Where
  (measured).**  The link is typed a plain vector sharing `n`'s store and registered in
  `amp_vector_locals`; `c = a` lowered to a fresh store filled from `a` (`OpDatabase`, `c = OpGetField(…)`,
  `OpAppendVector(c, a, 0)`).  The clear-and-refill it needs lives in `Parser::assign_refvar_vector`, which
  took only a `&vector` PARAMETER or annotated local (`RefVar(Vector)`).  **Closed** by letting that
  handler also take a plain vector local registered in `amp_vector_locals`, for `=` only, and never for the
  statement that makes the link: that statement registers the local a moment earlier, and claiming it
  left the local without a slot (a compile error on every vector link, caught by the first build).
  Measured on both backends, plain, under LOFT_POISON and with the native leak check; a literal, an append
  and a write-through onto the vector the link names are unchanged, and no existing corpus program's
  emission moved.  Guard `tests/scripts/a-vector-written-through-a-local-link-refills-what-it-names.loft`.
  Found while sizing D-bind-42 over the heap kinds.

* **D-bind-42** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-Ref-Write)` and `(B-Copy)` for a RECORD
  written through a LOCAL `&` link from a named record: `a = S{v: 1}; n = S{v: 7}; c = &n; c = a; c.v = 9`
  must leave `a` at 1 and `n` at 9, and left both at 9 on both backends; `…; c = a; a.v = 5` then read 5
  through `n`.  The write-through made `n` and `a` one record.  The same held from another link, in a
  loop, for a struct-enum link, and in the shape `157-link-repoint.loft`'s struct cell uses — that cell
  only reads after the write-through, so it passed on the alias.  The `&S` PARAMETER write-back copied
  and was right.  Silent; the same on 2cff47dfc.  **Where (measured).**  `Parser::assign_refvar_reference`
  materialises the copy (`OpDatabase`, `OpCopyRecord`) for a bare variable only when the target is a
  PARAMETER, because the same function once also saw a `&` local bind that must link.  A local link's
  `c = a` stayed `c = a`, and the interpreter installed `a`'s record reference (`SetStackRef`).
  **Closed** by dropping the parameter restriction: a `&` bind that must link reaches that function
  already lowered (`OpCreateStack`, or `OpVarRef` since D-bind-41), so a bare variable there is always the
  write-through.  Measured on both backends, plain, under LOFT_POISON and with the native leak check,
  across every shape above; a `&` re-point, a text write-through and the parameter write-back are
  unchanged, and `c = n` onto the record `c` already names copies that record onto itself, which gives the
  same values.  The copy is real now, so `advice[avoidable-copy]` reports it where the source is still
  used.  Guard `tests/scripts/a-record-written-through-a-local-link-is-copied-into-it.loft`.  Found while
  measuring D-bind-41's struct cells.

* **D-bind-41** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-Ref-Repoint)` against `(B-Ref-Write)` when
  the SOURCE is itself a link.  `c = &b` must re-point `c` to what `b` links, and `c = b` must write `b`'s
  value through `c`; both lowered to the same IR, `c = b`, so each backend gave one answer to both
  spellings, silently.  With `a = 1; n = 7; b = &a; c = &n;`, `c = &b; c = 9` wrote `n` on `--interpret`
  (`A1 N9`) and `a` on `--native`, and `c = b` copied on `--interpret` and re-pointed on `--native`
  (`N7`).  The same split held in a loop, in one arm, for a local link re-pointed to a `&` parameter,
  and for `text` links; a `&` parameter re-pointed to another link wrote through on both backends,
  and a `&text` or `&S` parameter re-pointed to a callee-local link wrote through the same way.  A
  first bind was right on both.  **Where (measured).**  The parser's `&` lowering had no arm for a
  source whose type is a link, so the `&` was dropped; the interpreter's link write-through and
  native's local link-to-link arm each read the resulting `c = b` one way.  **Closed** by spelling
  the re-point: the parser lowers `c = &b` to `c = OpVarRef(b)`, the raw link cell `b` holds — the op
  the interpreter's first-bind link copy already emits.  The interpreter routes it to the link's
  slot like the install op, for every kind of link; native takes `b`'s pointer in the local, record
  and parameter arms, and its local link-to-link arm now serves only a FIRST bind, so a reassignment
  `c = b` writes through.  Measured on both backends, plain, under LOFT_POISON and with the native
  leak check, across every shape above.  The one corpus program with a first bind from a link
  (`434-pln87-scalar-reference.loft`) changes its IR spelling and native's record-link representation
  and passes on both backends.  Guard
  `tests/scripts/a-link-re-pointed-to-another-link-takes-what-that-link-names.loft`.  Found while
  measuring D-bind-37's repoint of one `&` parameter to another.

* **D-bind-40** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-Ref-Alias)`, `(B-Ref-Write)` and
  `(F-ParamRef)` for a value ENUM: a `&` to an enum local, element or field was silently a copy, and
  a write through a `&` enum parameter crashed the interpreter.
  - `e = Col.Green; c = &e; c = Col.Blue` left `e` Green on both backends, and so did a link to an
    element (`e = &es[1]`), a field (`e = &o.k`) and a nullable local.
  - `fn f(c: &Col) { c = Col.Blue; }` overflowed the interpreter's call stack; native answered right.
  **Where (measured).**  Four sites, each on one path the enum takes:
  - The parser lowers a `&` only for a scalar, and its own scalar list left the value enum out while
    `data::is_scalar` counts it, so the bind copied.  The same list made a tuple with an enum member
    take the record-backed link (`tuples.md` D-tup-14).
  - On the first pass an enum element read is the bare `OpGetVector` place, before its enum getter
    wraps it, and the lowering knew only the wrapped spelling.  The first pass typed the local as
    the enum and the second as its link: "cannot change type from Col to &Col".
  - The interpreter read and wrote an enum link with `OpGetByte` / `OpSetByte`, which take a range
    minimum the site never wrote.  The next instruction was read as that minimum, the code stream
    lost its alignment, and control fell back into the caller's code, which called again.
  - Native's local-link arms had no enum, so the bind emitted no right-hand side; and a bare variant
    written through the link did not resolve, because the enum context read the type without
    peeling the link.
  Closed at each: the lowering asks `data::is_scalar` and accepts the bare element op, the link ops
  are `OpGetEnum` / `OpSetEnum`, native links an enum as a `*mut u8`, and `enum_context` and the
  variant resolver read through `peel_link`.  Guard
  `tests/scripts/an-enum-link-reads-and-writes-through-its-own-op.loft`.  Found while measuring
  D-bind-39's enum element face.

* **D-bind-39** *(opened 2026-09-14, loft#1567; CLOSED 2026-09-23)* —
  `(B-Ref-Lvalue)` for an integer STORE place stored in fewer than 8 bytes — an element or a field
  of `u8`, `i8`, `u16`, `i32`, or a narrow range: `c = &u[1]`, `c = &o.a`.  The rule says such a
  place links; it was refused on both backends with one error per `&`, at the `&` bind and at a
  `&` parameter, naming two ways out (copy into a local and write it back, or declare the place
  `integer`).  **The refusal is lifted and every such place now links**, measured on both backends
  across 17 cells: each narrow kind read and written through a field link and an element link, a
  nested field, a nullable element and field (absence survives both ways), two links naming one
  place, a re-point between two elements and between two fields, and a bare `&u8` parameter taking
  a field and an element — with the neighbouring field, the neighbouring element and the length
  read back untouched in every cell.
  **What the entry said had to happen first, and what actually did.**  It said the deviation
  *"closes when a link carries its target's width — a representation decision"*, on the premise
  that *"a link to a `u8` local (an 8-byte frame slot) and a link to a `u8` element (a 1-byte store
  slot) have the same type"*.  `@PLN167` decision 1 dissolved that premise — a linked narrow local
  holds its type's FIELD encoding, so the two are one representation — and `state/codegen.rs`
  already read and wrote through `NarrowSlot::of_type` for *"a linked local, a field, an element"*.
  So no representation work remained.  What remained were **two gates that had been written to the
  old premise**, and finding them is the whole of this closure:
  1. `Parser::scalar_place_ref`, which turns a place expression into the link's target, matched an
     `OpGet*` with exactly TWO arguments (`else if let [base, fld] = gargs.as_slice()`).  A narrow
     read carries a THIRD — the `min` its encoding is biased by, `OpGetByte(v1, fld, min)` — so it
     fell to `None`, the `&` was silently dropped, and the local was typed a plain `int`
     (`LOFT_VAR_TABLE` reads `pa int` beside `pn &int`).  That is why writes were "lost": there was
     no link to write through, only a copy.  The `min` is decoding information and not part of the
     address, so the place is the same `OpGetField(base, fld)` every other field read yields.
  2. The re-point branch's `scalar_link` test read `spec.byte_width(false) == 8`, excluding narrow
     integers on purpose — *"a link does not yet honour a narrow place's width (D-bind-39)"*.  With
     the first gate fixed, a re-point fell through to the write-through path and wrote the 12-byte
     stack CELL through the kind's own `set_op`: the interpreter panicked in the store for an
     element (`rec=1748762626`, a corrupt reference) and wrote the read-only CONST store for a
     field, while **native was already correct**, because it routes a re-point by the value's shape
     rather than by the target's width.  A re-point is not a value write at any width.
  **Both gates cited this entry by number, and that is the lesson.**  Each was a correct guard
  against a defect that no longer existed, and each read as settled because it named the deviation
  it was protecting.  A deviation's closure has to sweep its own citations rather than only its
  headline behaviour — the second gate was reachable only through the first, so fixing the
  first is what made the interpreter crash where it had merely refused.
  **Uniformity is what the guard scores**, not just success: an unfitting compound step answers the
  type's default through the link exactly as it does at the place (C127), drawing the same
  `narrow-fallback` advice, and a direct write beside two links reaches both.  Guard
  `tests/scripts/1567-a-link-to-a-narrow-integer-store-place-reads-and-writes-it.loft`, which
  replaces `a-link-to-a-narrow-integer-store-place-is-refused.loft` and keeps its seven shapes,
  scored by value instead of by diagnostic.  `a-link-to-a-narrow-integer-local-reads-and-writes-it.loft`
  still pins the frame half.  D-bind-38, the TEXT face of the same rule, is closed too (loft#1566).

* **D-bind-54** *(opened 2026-09-23, CLOSED 2026-09-23; found by loft3-ca on the wide spelling,
  where it was recorded as D-bind-53 — a number this tree had already given loft#1639's entry, so
  the two were reconciled on contact rather than on the join)* — `(B-Ref-Lvalue)` for a field of a
  STRUCT ELEMENT: `p = &v[0].f`.  The place a link is given is built by `Parser::scalar_place_ref`,
  whose first arm answered the ELEMENT for any read through an element accessor — **the field
  operand was not mis-offset, it was dropped**.  The introspect is the whole story, two different
  fields of one element yielding byte-identical places:
  `p1(1):&integer(0, 255) = OpGetVector(v(1), 10i32, 0i32);` beside
  `q1(1):&integer = OpGetVector(v(1), 10i32, 0i32);`, and natively
  `addr_mut::<u8>(__ed.rec, __ed.pos)` with no offset added.
  **Measured**, both backends identical, with `struct E { a: u8, b: u8, c: integer }` — whose
  layout puts `c` at offset 0, `a` at 8 and `b` at 9: `&v[0].a` and `&v[0].b` each wrote `c`, and
  `&v[0].c` was **right by accident**, its offset being zero.  With
  `struct W { a: integer, b: u8, c: integer }`, `&x[0].b` wrote `a`.  loft3-ca reached it from the
  text side (`&v[0].s` with `struct O { s: text, n: integer }` lands on `n`) and reported `49 2 3`
  for `1 42 9` on a wide field — the same defect seen through a layout whose wide field was not at
  offset zero.
  **Closed** by naming the field unless its operand is literally zero, where the element's base
  address and the field's ARE the same address — which is exactly what lets `vector<integer>`'s
  `&v[0]` and a struct element's `&v[0].f` share one IR shape without ambiguity.  Written once, for
  reads of ANY arity, so a NARROW read's third operand (the `min` its encoding is biased by) is
  covered by the same code rather than by a second arm that would have to agree with the first
  forever — that reconciliation was the join hazard loft3-ca flagged, and removing the second arm
  removes it.
  **Why it surfaced now.**  The narrow spelling was unreachable while D-bind-39 refused a link to a
  narrow store place; lifting that refusal in the same commit admitted it onto this broken path.  So
  a refusal was hiding a defect one layer down, and the lift is what made it reachable — the reason
  the lift's own guard scores neighbours rather than targets, and the reason it is closed here
  rather than filed: shipping the lift without it would have been a silent-wrong introduced by a
  fix, which is `(B-Ref-Reshape)`'s own objection to a link that cannot be honoured.
  Guard `tests/scripts/1567-a-link-to-a-narrow-integer-store-place-reads-and-writes-it.loft`, six
  cells linking every field of both layouts in turn and reading all three fields back each time, so
  a write landing on a neighbour is caught BY the neighbour.  loft3-ca's `#1566` guard keeps the
  integer and text cells on the same rule.

* **D-bind-53** *(opened 2026-09-23, CLOSED 2026-09-23; loft#1639)* — ⚠ opened as `D-bind-44`,
  a number already taken by a CLOSED entry of 2026-09-15 that lived on a branch this tree had
  not yet joined.  Renumbered on the join: a deviation number is repo-WIDE, and the check has
  to span every in-flight branch rather than the tree in hand. — `(B-Ref-Intro)` says a `&`-annotated binding
  gives the variable type `&(typeof a)`, so the link's type comes from the TARGET and an explicit
  annotation naming a different type is a mismatch `(C-Ref)` has no conversion for.  It is not
  refused: the link reads and writes the target's slot at the ANNOTATION's width and bias, handing
  the stored code back as a value.  `b: i8 = -1; pb: &u16 = &b` reads `127` (`enc_byte(-1, -128)`,
  the raw byte); `d: integer limit(1000, 1100) = 1050; pd: &u8 = &d` reads `50`; `h: u16 = 65535;
  ph: &u8 = &h` reads `255`.  The writes corrupt the target — `pf: &u16 = &(f: i8 = -1); pf = 5`
  leaves `f == -123`, `pg: &u8 = &(g: Lim = 1050); pg = 200` leaves `g == 1200`, and
  `pi: &u8 = &(i: u16 = 65535); pi = 200` leaves `i == 65480`, one byte written over two.  The
  annotation IS consulted for the value check, against the wrong type: `pg = 1060` is refused as
  not fitting `u8` over a slot that holds `1000..=1100`.  `--native` does not compile at all
  (`*mut u16 = addr_of_mut!(var_a)` over a `u8`, rustc E0308 per link), so the backends diverge
  too.  **Where (measured).**  `u8` is the one row that reads correctly, because its bias is zero
  and its encoding is the identity — the covered spelling is the one that cannot fail, which is why
  nothing caught it.  Same class as the frame readers @PLN167 A0–A2 closed (#1632): a reader taking
  the stored code for a value, here with the wrong width supplied by the annotation rather than by
  the reader.  Belongs to A3 with D-bind-38 and D-bind-39.  Found when a peer's rules-side read
  predicted the spelling was already refused.  **Closed** the same day: an annotation naming a
  different integer type than the target is now REFUSED, which is what `(B-Ref-Intro)`'s
  `b : &(typeof a)` and `(C-Ref)`'s single τ already required.  One home,
  `Parser::amp_annotation_mismatch`, asked at the `&` site.  It compares (min, max,
  `forced_size`) and deliberately NOT `IntegerSpec` whole, because `not_null` is a claim about
  the SLOT that an annotation and a declaration can disagree on without naming different types
  — the same flag that caught `non_null_reads_null` and `Data::integer_alias` out the same day.
  An UNANNOTATED `p = &x` arrives with its type already inferred from the target, compares
  equal, and is never refused.  Scoped to integers, which is what was measured.  The message
  names both types through loft#1641's `int_type_name`, so the cure it offers can be typed back
  in.  Guards `tests/scripts/1639-a-link-takes-its-targets-type-not-its-annotations.loft` for
  what must still link, and two `@EXPECT_ERROR` cells in `102-expected-errors.loft` — the
  narrower direction pinning a two-message cascade rather than hiding it.  **D-bind-39**
  (loft#1567) CLOSED the same week: a link to a narrow STORE place now links, and the
  representation decision its entry asked for turned out to have been made already.

* **D-bind-38** *(opened 2026-09-14, CLOSED 2026-09-23; loft#1566)* — `(B-Ref-Lvalue)`: a link to a TEXT place is refused.  `a:
  vector<text> = ["aa"]; t = &a[0]` and `o = O{s: "aa"}; t = &o.s` stop with "`&` requires an
  addressable operand — a variable, struct field, or vector element", on both backends and already
  on the first bind, while the same spellings over an integer, float, enum or struct place link.  The
  rule names a field and an element as lvalues without an exception for text.  **Where (measured).**
  A text place reads through its own op — `OpGetText(OpGetVector(a, 4, 0), 0)`, `OpGetText(o, 8)` —
  and `Parser::is_amp_place` does not list it.  Lifting the refusal is not the whole cure: a local
  `&text` link is a `*mut String` on `--native`, and a store's text slot is not a `String`, so the
  link needs a representation for a text that lives in a store.  (The `u16` and `i32` places this
  entry first carried are narrow integer places and left with D-bind-39, which closed 2026-09-23;
  text is the only face of this rule still refused.)  Found while probing D-bind-36's repoint over every element kind.
  **Closed** by @PLN167 C1: a text field or element is a place, and a `&` bind to one makes the
  STORE kind of a text link — a link holding the slot's `DbRef` (the place `scalar_place_ref`
  already gives a scalar), whose every mention is parsed as that field
  (`OpGetText(OpVarRef(t), 0)`), so its read is the field read and its write the field's setter on
  both backends.  The kind is a fact of the VARIABLE (`Variable::store_text_link`), carried
  through the IR snapshot, and a bind of the other kind is refused — in either order and across
  the arms of an `if` — because `(B-Ref-Repoint)` keeps a link's type.  A link to a text VARIABLE
  is unchanged and its emission byte-identical.  Guards:
  `tests/scripts/167-a-text-link-into-a-store.loft`,
  `tests/scripts/167-a-text-link-keeps-its-kind.loft`.  The PARAMETER face is D-bind-55.

* **D-bind-55** *(opened 2026-09-23, CLOSED 2026-09-24; loft#1566, loft#1602, @PLN167 C3)* —
  `(B-Ref-Lvalue)` with `(F-ParamRef)`: a text field or element — or a store-kind text link —
  handed to a `&text` PARAMETER was refused (loft#1602's stopgap; before it the call copied and
  dropped the callee's write), where the rules say the parameter links to the place as the `&`
  bind does.  **Closed** by instantiating the function per text-link KIND, decision 2's route:
  the call hands the slot's `DbRef` (`scalar_place_ref`, the place the bind takes) and is pointed
  at the function's STORE instance, minted after pass 2 (`parser/store_text.rs`) as a clone whose
  parameter carries `store_text_link` and whose body is rewritten to C1's spelling — a read
  `OpGetText(OpVarRef(t), 0)`, a write the field's setter.  The rewrite is total over a measured
  closed set: across the 206 functions with a `&text` parameter in the stdlib and the corpus the
  parameter is written only by `Set` and the `Op…Stack…` write ops, each of which has a twin on a
  text variable.  The stack instance is the function as written, byte-identical
  (`scripts/introspect_diff.sh`).  Instances close transitively (forwarding, recursion, a local
  link bound from the parameter).  `(B-Ref-Reshape)`'s callee clause now reaches the text
  spelling (`grow(h.v[0], h)` is refused; it crashed on both backends while admitted).  An
  instance's key is `n_f@st<mask>` and every name decoder cuts at `@`, so messages, traces and
  profiles name `f`.  Guards `tests/scripts/1602-a-ref-text-parameter-links-a-text-field-or-element.loft`,
  `tests/scripts/1602-a-ref-text-parameter-refuses-what-it-cannot-link.loft`, and the warm-cache
  cell `a_store_text_instance_reads_the_same_warm` (`tests/arc_e_program_cache.rs`).  The
  FUNCTION-VALUE spelling is D-bind-57.

* **D-bind-57** *(opened 2026-09-24, CLOSED 2026-09-24; loft#1656, @PLN167 R5a)* —
  `(B-Ref-Lvalue)` with `(F-ParamRef)` through a FUNCTION VALUE: `g = app; g(o.a)` with
  `fn app(t: &text)`.  On `main` both backends compiled it and answered `alpha`: the conversion
  copied the field and the callee's write was lost with nothing said.  C3 first refused it,
  because a store instance is picked per call site and a function value is fixed before the
  call.  **Closed** by letting the CALL pick, since only the call knows the argument's kind: the
  argument is lowered to the place as a direct call's is, the call's store mask
  (`Data::store_text_mask`, the one test both backends ask) selects the STORE instance of
  whichever function the value holds, and after pass 2 an instance is minted for every
  candidate of the value's type (`fnref::dispatch_arms`, the candidate set's one home).  The
  interpreter dispatches through `OpCallRefStore`, `OpCallRef` with the function swapped for its
  instance for that one call (the slot is left as it was).  Native's arms call each candidate's
  instance, and an instance is reachable with the function it copies.  **Found on the way, and
  closed with it:** a `&text` parameter through a function value did not compile on `--native`
  in EITHER kind.  `dispatch_arms` and the synthetic-argument loop took every `RefVar(Text)`
  attribute for a text-return work buffer, so `fn(&text)` matched no function (an empty
  `match`) and the argument was spelled empty.  The rule is now positional
  (`fnref::visible_fnref_attrs`): the buffers follow every user parameter.  An instance is never
  a candidate itself (`Definition::is_store_text_instance`), or `rec`'s instance acquired an
  instance of its own.  Guards: `test_through_a_function_value` in
  `tests/scripts/1602-a-ref-text-parameter-links-a-text-field-or-element.loft` (a named function
  re-pointed between two, the stack kind through the same value, a capturing lambda, a
  text-returning one, an element of a vector of functions and an absent one), and the
  warm-cache cell.  Not expressible yet, and not a deviation of this rule: a function TYPE
  spelled with a `&` parameter (`fn(&text)`), so such a value arises by inference only.

* **D-bind-58** *(opened 2026-09-24, CLOSED 2026-09-24; found by @PLN167 C3's K9 cell)* —
  `(O-NoDiverge)` for a `&text?` PARAMETER on `--native`: any read of it (`t == null`,
  `t == "x"`) emitted `*var_t`, a MOVE of the `String` behind the `&mut`, and rustc refused the
  program (E0507) while the interpreter ran it.  The parameter arm tested `**inner` for `Text`
  exactly, where the local-link arm beside it reads `inner.base()`.  **Closed** by asking the same
  question as that arm.  A loud divergence, never a wrong value.  Guard: K9 in
  `tests/scripts/1602-a-ref-text-parameter-links-a-text-field-or-element.loft`
  (`test_a_nullable_text_field`, the variable half).

* **D-bind-37** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(O-NoDiverge)` for `(B-Ref-Repoint)` on a
  `&τ` PARAMETER: `fn f(c: &integer, v: vector<integer>) { c = &v[1]; … }` re-pointed the parameter's link
  on `--interpret` (after D-bind-36) and did not compile on `--native` — rustc E0308 for an element or
  a field, and an empty right-hand side (`*var_c = ;`, or `u8::from()` for a boolean) for a callee
  local.  A loud divergence, never a wrong value.  **Where (measured).**  The native generator treats
  every assignment to a `&` parameter as the write-back, so a place wrote its reference into the
  caller's variable.  **Closed** by a repoint arm at the top of that branch: for a scalar parameter,
  a place op on the right binds a borrow of the store slot, and `OpCreateStack(m)` a borrow of the
  local; the slot pointer is built by the same helper a local link uses.  Measured on both backends,
  plain, under LOFT_POISON and with the native leak check: an element, an element then a write, a
  field, a loop, a callee local, read-repoint-read-write, and float, enum, boolean and character
  parameters; the write-through control is unchanged.  A `&` parameter re-pointed to ANOTHER link is
  D-bind-41, which this arm does not reach.  Guard
  `tests/scripts/a-reference-parameter-re-points-to-an-element-a-field-or-a-local.loft`.

* **D-bind-36** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-Ref-Repoint)` on `--interpret` for
  a link to a SCALAR whose new source is an element or a field: `c = &w[0]; c = &v[0]` and `f =
  &o.x; f = &o.y` panicked (a store access out of bounds; in the allocator for a float element),
  while native re-pointed.  The parser lowers the place to its own op (`OpGetVector`, `OpGetField`),
  not to the install op D-bind-32 routed to the link's slot, so `set_var` took the write-through
  path.  For a link to a scalar the right-hand side's TYPE separates
  the two spellings: a place op is declared to return a reference, a value read out of the place is
  the scalar.  `set_var` routes the former to the link's slot as well, at every integer width since
  D-bind-39 closed — the exclusion of a narrow place was that deviation's, and outlived it by one
  commit, during which a narrow re-point wrote a stack cell through the kind's own `set_op`.  A link to a record or a
  collection reads an element's VALUE as a reference too, so there the type cannot tell them apart
  and only the install op is routed.  (A struct element bound with `&` is typed as a view of its
  vector, `ref(P)`, rather than as a `&P` link; rebinding it already left the first element alone,
  and a cell pins that.)  Guard `tests/scripts/a-scalar-link-re-points-to-an-element-or-a-field.loft`.

* **D-bind-35** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-Disturb)` did not hold for a view
  bound inside ONE ARM of an `if` whose other arm assigns the same local: `x = mk(4); if k > 0 { x =
  h.inner } else { x = mk(0) }; h = Hold{…}` left `x` reading the new container on both backends,
  where the unbranched bind and the value join materialise.  The materialisation walk (`ViewWalk`)
  walked the two arms one after the other, so the second arm's assignment ended the view the first
  arm had bound — the same program with the arms swapped materialised.  The arms are now walked as
  alternatives: each from the views open before the `if`, keeping what either leaves open or
  disturbed, and a literal key only where both agree.  Found by D-bind-34's closure, which lowers the
  value join to exactly that statement form.  The walk's imprecision in the other direction is
  unchanged and deliberate: a view bound before the `if` and disturbed on one path only is
  materialised on both, and the author is told.  On `--native` the newly materialised copy then
  LEAKED: a displacement free emptied the slot first, so the copy landed in a fresh store, and the
  generator's owner tracker records a copy only on a first declaration — the reassignment branch
  left it unnamed, and nothing released it.  Falsifying the guard caught it in the leak column; that
  branch now asks `materialises_element` too, the question the first declaration already asked.
  Guard
  `tests/scripts/a-projection-assigned-in-an-arm-views-until-its-container-is-reassigned.loft`.

* **D-bind-34** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-View)` and `(O-NoDiverge)`: a REASSIGNMENT from a
  value join whose taken arm is a struct PROJECTION copies on `--interpret` and views on `--native`.
  `h = Hold{inner: mk(1), …}; x = mk(4); x = if k > 0 { h.inner } else { mk(0) }; x.a = 9` leaves
  `h.inner.a` at 1 on the interpreter and at 9 natively, while the unbranched `x = h.inner` and the
  author's statement form `if k > 0 { x = h.inner } else { x = mk(0) }` view on both backends.  The
  container is not disturbed, so `(B-View)` gives the view and the interpreter is the deviating
  side.  Found while closing D-bind-33: that cure, first built wider, wrote this join out as
  statements too, and native then copied as well — the rule's answer lost on the one side that had
  it.  Narrowed to local arms, the split is back to what it was, and it is recorded here.
  **Where it happens (measured).**  The author's statement form carries an owner witness
  (`(O-Witness)`: `__own_x`, released by store identity at the projection's assignment), so the
  projection arm is a view on both backends.  The value join gets none: `owner_witness_locals` runs
  before the scan and does not read the join's projection arm as a view assignment, so `x` stays an
  owning slot, and the interpreter lowers an owning slot's reassignment from an `Own::Join` value to
  `OpBindOrCopy`, which copies the borrowed arm.  The boundary agrees — a binding whose previous
  assignment was itself a view views on the interpreter too.  It is the third fact a pass before the
  scan reads off the join where the author's spelling has it (family 7's per-path flags and
  D-bind-33's call-arm owner were the other two, `heap.md` D-heap-7).
  **Closed by carrying the scan's own decision.**  The first scan records, by the address of each
  `Set`'s value node, the reassignments it writes out per arm; the caller rewrites exactly those in
  the original code, strips the arm locals' deps the parser gave the binding, and scans again — so
  every analysis before the scan, the owner witness among them, reads the per-arm form.  A structural
  "is this a reassignment" was measured first and rejected: it disagreed with the scan at 52 of 899
  corpus sites.  The lowered family-7 cells now emit exactly what their author-written twins emit,
  and the value join views on both backends.  Closing it exposed D-bind-35.

* **D-bind-33** *(opened 2026-09-14, CLOSED 2026-09-14)* — `(B-Copy)` did not hold for a
  REASSIGNMENT from a join whose other arm is an owning CALL.  `a = mk(1); x = mk(9); x = if c { a }
  else { mk(2) }; x.id = 77` changed `a` on the path that took it, and a later write to `a` showed
  through `x`, on both backends; the first bind from the same join copied.  The releases went wrong
  with it (`heap.md` D-heap-7, `p_j1`/`p_j2`): the displaced `mk(9)` was never released, and neither
  was a record assigned to `x` after the join, because the binding stayed typed as a view of `a`.
  **Between two mechanisms.**  A first bind lifts each arm into a temp of its own (D-bind-16,
  loft#1321); a reassignment cannot borrow those temps (`(O-Latest)`), so it is written out per arm
  instead (`scopes::sink_set_into_arms`).  The parser had already given the call arm an owner for the
  value form's view-typed join (`materialise_owned_call`'s `join-arm-owner` block), whose tail is a
  compiler temp, and the write-out declined every compiler-temp tail — so this reassignment kept the
  value form, which binds the chosen arm's STORE.  The write-out now accepts that block and writes the
  arm out as its call, wherever every other arm is a local or `null`.  Beside a projection arm it
  still declines: written out, a projection arm lost the dep that keeps it a `(B-View)` view, which
  was measured and is D-bind-34.  Guard
  `tests/scripts/a-reassignment-from-a-join-with-a-call-arm-copies-its-local-arm.loft`.

* **D-bind-32** *(opened 2026-09-09, CLOSED 2026-09-09; numbered 30 on its own branch, where
  `D-bind-30` was already spent by the `(B-Ref-Reshape)` entry closed a day earlier — the join is
  what showed the collision)* — `(B-Ref-Repoint)` on `--interpret`,
  at every τ but a vector.  The rule was not written: B-Ref-Write said what a heap write
  through a link does ("it does not re-point the link") and nothing said what `p = &q` on an
  existing link does, while the parser had long lowered it to the link's install op
  (`Set(p, OpCreateStack(q))`) and native ran it as a re-point (`var_p = addr_of_mut!(var_q)`).
  The interpreter's `set_var` link branch recognised the install value only to keep a fn-ref
  off the write-through, and every other kind fell into the write-through: the cell went
  THROUGH the link into the old source's slot and the displaced-store free ran on a stack ref
  (`BUG (#306)`), so a struct read a garbage word and lost the second record's field, a text
  was cleared through the link and read empty, an integer kept its first source.  Found by
  @PLN157 § V-p's cell c18 (a plain-record `&` view rebound in a loop), whose interpreter
  oracle answered 34359738373 for 13.  Closed with the rule written and the install routed
  to the link's own slot FIRST, before any kind's write-through — `state/codegen.rs::set_var`
  cites it; `tests/scripts/157-link-repoint.loft` carries the six kinds and the control.

* **D-bind-31** *(opened 2026-09-09, CLOSED 2026-09-09, loft#1489)* — `(B-Copy)` did not hold for
  a bind out of a CLOSURE CAPTURE.  `e = q` inside a lambda ALIASED the captured collection or
  record: `e += […]` grew the outer `q`, `e[0].a = 99` and `e.a = 99` reached it, on both
  backends, with nothing saying so.  The same bind out of a PARAMETER — the identical shared heap
  value, `(F-ParamHeap)` and `(L-CapHeap)` being one sentence — copied throughout, which is what
  named the difference.
  **One op, two notions.**  A closure reaches its capture through the closure record, so the
  source arrives as `OpGetDbRef(__closure, off)`: a PROJECTION's spelling for a whole value the
  author bound by name.  Every reader that had to tell `(B-Copy)`'s whole value from
  `(B-View)`'s interior place tested for a bare `Var` or an `OpGetField` and answered "not a
  bind" — the FIFTH time this family's selector has been the narrow part while its lowering was
  already right (P261, loft#917, loft#1279, loft#1326).  `reads_a_capture_whole` is the one home
  now, and it asks about the BASE, because the loose spelling also admits an auto-`Reference`
  POINTER FIELD read (`h.link`), which really is the projection this is not.
  The two passes did not agree either: on pass 1 a capture is still a placeholder `Var`, so the
  vector selector answered `CopyVar` there and `NotABind` on pass 2 for one body.
  Guard `1489-a-capture-is-bound-and-returned-as-a-whole-value.loft`, whose `(B-View)` and
  `(B-Ref-Alias)` cells are what say the copy did not swallow the aliasing that is meant to
  stay.

* **D-bind-29** *(opened 2026-09-08, CLOSED 2026-09-08, loft#1463)* — the FUNCTION half of
  `(B-Ref-Uniform)`, on `--native` only.  A write through a `&fn(…) -> τ` link did not land when
  the caller's slot already held a CAPTURING closure: the interpreter wrote it, native left the
  old value in place and said nothing.
  **The special case was in the DISPATCH, not in the write.**  A fn-ref call passes the closure
  environment beside the tag, and the emitter took it from the caller's own `___clos_N` local
  whenever one existed — re-deriving *which environment does this slot hold* from the MINT SITE,
  which is right only while nothing else can write the slot.  A `&fn(…)` link is what can.  The
  environment now always comes from the slot's own `.1`: where the mint put it, and where a
  rebind puts the next one.
  The entry as opened guessed the 20-byte stack form was the axis — 8 B `d_nr` plus a 12 B
  closure `DbRef` against a `d_nr` alone — and the LAYOUT was a symptom rather than the cause.
  Capturing is what makes a `___clos_N` local EXIST for the emitter to prefer; the widths never
  entered the decision.  Worth keeping, because the layout reading is the one a reader arrives
  at from the report and it costs a session: the two `loft introspect` dumps differ in the
  dispatch line, not in any width.
  Guarded by `1463-a-closure-written-through-a-link-lands-on-a-capturing-slot.loft`, whose
  CONTROLS are loft#1443's non-capturing cells — the shape that always worked — so the file says
  both are closed rather than one traded for the other.
