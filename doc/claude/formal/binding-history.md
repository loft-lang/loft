# formal/binding-history.md — the deviation register for [binding.md](binding.md)

> **The rules are next door.**  [binding.md](binding.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0** — D-bind-29 OPENED AND CLOSED 2026-09-08: `B-Ref-Reshape`'s re-key arm was
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

> **D-bind-28, part two — CLOSED (2026-09-07, loft#1433) — a `&` bind to a keyed
> collection is a LINK, not a copy.**  `(B-Ref-Alias)` says the `&` annotation makes **ANY**
> binding a live link to the source instead of a copy, and names `d += …` as writing through.
> The `&`-bind's source set was `matches!(amp_vector_source, Type::Vector(_, _))` — a set of
> ONE — so all five keyed kinds fell to the deep-copy path instead: `a = &h` gave `a` its own
> store (`OpDatabase`) and `OpReplaceKeyed`-copied `h` into it, and from the bind onward the
> two were independent.  Measured: after one write through each, `h` held `{1,3}` and `a` held
> `{1,2}`, *both with `len` 2* — so a length check on either name alone looked healthy.
>
> It was filed as *"a `&` alias to a keyed collection silently drops every append"*, which is
> what the defect looks like from an EMPTY source: `len(h)` stays 0.  The append is not
> dropped — it lands in the alias's own store, and `len(a)` says so.  That distinction decides
> what a guard must assert: reading only the source would pass a cure that made the alias
> unwritable rather than one that made it a link, so every cell reads BOTH names.

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
> ⚠ **One residual, and it is a DIAGNOSTIC defect rather than a rule one.**  The two refusals
> are not the same message: the dense one names the authored type and both cures, while the `&`
> one says *"No matching operator 'Add' on '&hash<Row,[\"k\"]>'"* — the SCHEMA spelling of the
> type, which no source ever wrote, and no cure.  That is loft#1449 (`Type::name` where
> `source_name` is meant), and this is a live instance of it.

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
> `@FR-Col-Remove` walk (QUALITY.md B8f).

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

