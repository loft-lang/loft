---
render_with_liquid: false
---
# Changelog

All notable changes to the loft language and interpreter.

---

## [Unreleased]

### `(B-Disturb)` ends every place a view can name (2026-09-07, D-bind-25/26/27)

Three neighbours of loft#1401, found by its boundary matrix and each failing IDENTICALLY in the
plain spelling and the `??` one — which is what said they were not that issue's discharge defect.
Each is a silent wrong answer on both backends.

**A `sorted` removal is a reshape.**  Measured per keyed kind: `hash` and `index` answer the
right element after another key is removed and a write through the view still lands; `sorted`
reads the element that shifted in.  The four own-record kinds leave every other key reachable at
the same address (`(Col-RemoveKeyed)`), while a `sorted` is the INLINE keyed kind whose elements
sit in key order in one dense array — `(Col-RemoveDense)`, beside the vector.  All five remove
through `OpHashRemove`, so the reshape is keyed on the KIND; adding the op flat would have
materialised the four that are correct today.  Same boundary loft#1402 crossed from the other
side, where a `sorted` LEAKED through `#remove` while `[key] = null` stayed flat.

**A removal reached through a FIELD is a disturbance.**  `p.va.remove(0)` names its container as
a projection and only a plain `Var` was collected, so `c = p.va[1]` kept reading the element that
shifted in — while the same code with `va` in a local materialises and says so.  The collector
now answers PLACES: a whole variable ends everything inside it, a field ends that field only.
The precision IS the fix — `grown_containers` records what a variable-granular version costs
(`moros_editor`'s undo stack silently stopped recording), and two controls pin it.

**A branch over two containers names both.**  `c = if k { w[0] } else { v[1] }` is a view of each
on the path that takes it, and the walk answered `None` whenever the arms disagreed.  A view is
now recorded once per PLACE, which the open-view frame already held as one entry per pair, so
nothing downstream learned a new shape.  With more than one place, the ADVICE could no longer
re-derive its container from the right-hand side — that names both and only one was disturbed —
so `views_to_materialise` carries the whole `Disturbance`, which already held the container it
was observed at.

Landing the field half needed a third rule: **a binding the loop ITERATES is not materialised.**
The iteration depends on the temp's IDENTITY, so `for e in d.items { e#remove; }` shook the
loop's own source, walked a COPY while the body emptied the original, and never terminated —
`903-loop-remove` went from 0.06s to a 300s corpus timeout.  The obvious wider rule (*a view the
author cannot name*) was measured WRONG: the parser renames author bindings too, and a `match`
payload must still materialise, so a name test traded the hang for a silent wrong answer.  It is
a MARKER on the variable the lowering creates (`Function::is_iteration_source`).

Guard: `a-disturbance-ends-every-place-a-view-can-name` (12 cells, both backends, falsified at
f4403e62).  SEVEN are controls, because widening a disturbance is the direction that silently
loses a write; each has a FIRING twin in the same file, so none can read green by never being
reached.

### A `??`-discharged projection is the view its plain spelling is (2026-09-06, loft#1401, D-bind-24)

`c = v[1] ?? Box{n:0}; v.remove(0)` left `c` aliasing POSITION 1, which `(Col-RemoveDense)` had
just renumbered: it read the value that shifted in and a write through it still reached the
container — both backends, in silence, where the plain `c = v[1]` materialises and says so.
`(N-Index)` types `v[i]` as `τ?`, so `??` is the discharge the language REQUIRES for a non-null
binding.

Filed as one hole; it was four, and each alone leaves the defect standing.  NAMING — a `??`
lowers to a value block that hoists its subject into a temp and hands that temp back, so the
walk saw a bare `Var`; a tail is now resolved through the block's OWN bindings, which covers
`??`, `?? return` and a `match` subject in one step, and the walk answers a PLACE so container
and field come from one derivation.  COPY — per ARM, as loft#1396/#1399 supply it for a
branch-valued binding, gated on the walk having named the binding.  `?? return` — its absent
path leaves by an early return, so the tail is an unconditional `Var` and `is_value_branch` read
that as "not a branch".  SPELLING — a discharged `v[i]` arrives as `OpGetVectorNullable`, a
projection by every structural test that is deliberately off `is_projection_op` because the deps
PROXY strands a store on it; that hazard belongs to the proxy, so `view_source_place` is the
reading for a walk that only NAMES a place.

The report now follows the COPY rather than the strip, so `(H-Materialise)`'s "and the author is
told" holds whichever mechanism supplied the store.  The first cut regressed loft#1399 — a `[]`
MINT arm named the hidden `__vdb_N` it reads its own store out of, and two arms naming different
containers name none — closed by treating a compiler-generated container as no place at all,
which `resolve_view_root` already did.  Guard:
`a-discharged-projection-materialises-like-its-plain-twin` (15 cells, three of them ALIAS
controls, both backends).  `Fixes #1401`.

### A vector removal releases what the element owned (2026-09-06, loft#1402, D-col-3)

`v.remove(i)` and the in-loop `e#remove` retained one record per removal: 2000 add+remove cycles
at a population of ZERO held 2004 records where the keyed baseline holds 4.  `remove_vector_at`'s
UNLINKED branch shifted the bytes and released nothing, so one `sorted` leaked through `#remove`
and not through `[key] = null`.  Its own doc said why it thought it needn't — "there is no
separate record to free" — true of the element's own record and false of its CLAIMS.  Walked now
through `get_vector`, the same index→element map `remove_vector` walks, and released BEFORE the
shift because an inline element IS the slot.

It could not land alone: while loft#1401 left a `??`-discharged binding viewing the removed
element, releasing that element's children emptied a value the program was still reading, and
`445-generic-tree-walk` failed — rightly.  Guard:
`a-vector-removal-releases-what-the-element-owned` (10 cells, both backends); its oracle is
FLATNESS rather than a count, because the absolute record count differs between the backends and
`collect_store_leaks` cannot see a record retained inside a LIVE store.  `Fixes #1402`.

### `(N-Store)` is asked at the assignment target (2026-09-06, loft#1404, D-Null-Assign)

The fifth position loft#1313's heap half did not reach.  `s.rec = null` did not happen —
`s.rec.n` still read 5 where the literal `S{rec: null}` reads 0 — and `v[i] = null` was a no-op,
both silent on both backends.  The filed scope was wrong: `x = null` at a heap target spells FIVE
things and three are not stores, so widening the ask to `is_dbref` would have reported correct
code.  Measured at the lowering, a keyed `c[key] = null` is `OpHashRemove` (`(Col-Remove)`'s
delete), `s.coll = null` is `OpClearVector`/`OpClearKeyed` (that field's clear), and a
`reference<T>` POINTER field is `OpSetDbRef(sentinel)` — a store that LANDS.  Only the two
dropped writes build `OpCopyRecord(null, …)`, so the ask went to `copy_ref` and needs no
container, keyed or pointer-marker test.  A gate at the parse site needed all three and still got
the pointer field wrong, because #328's share marker is not on the resolved target type by then.

The VALUE is unchanged and that is settled: a dense field has no discriminant to spend on
absence, so the cure the message already named — declare it `τ?` — is the real one.  The
CONSEQUENCE clause is now the reporting position's, because "the slot holds null" is measured
true for a scalar and for a record travelling as a HANDLE and false here; the four shipped
positions keep their wording to the byte.  Six cells in `tests/heap_nstore.rs`, four of them
negative controls.  `Fixes #1404`.

### The native cache publishes a whole binary or none of it (2026-09-06)

`--native` compiles into a per-process scratch dir and publishes to a SHARED, content-keyed
`<script dir>/.loft/cache/<stem>-<hash>`.  That publish was a plain `fs::copy`, which truncates
the destination in place and then streams ~11 MB into it; for that window a concurrent run of
the same source sees a path that exists and passes `cache_safe_to_execute` — symlink, owner and
mode, but never SIZE — and execs a 0-byte-and-growing ELF, dying with no stdout and no stderr.
Once an entry exists at 0700 a later copy truncates it while the mode stays 0700, so the P254
mode check never covered this.  The stale-entry sweep made it worse by removing every `<stem>-*`
entry BEFORE the copy, including a binary a concurrent run had already accepted.

Now staged under a private `.<leaf>.<pid>.tmp` in the same directory, tightened while still
invisible under its final name, then `rename`d; the sweep runs after and spares the entry just
written.  One home: `native_utils::publish_cached_binary`.  Found gating a branch — `make ci`
red in two runs of three on `alias_link_baseline::baseline_leak_clean_native` (its two native
cells compile one source concurrently), with an empty output block as the only evidence.
Guarded on the destination's INODE changing across a publish, which fails on the pre-fix form;
the end-to-end racing test does NOT falsify and its header says so.

### A nullable collection is refused with its discharge, not with a list of kinds (2026-09-06)

`for x in v` where `v: vector<integer>?` reported *"Unknown in expression type
`vector<integer>?`"* twice and *"cannot iterate over `vector<integer>?`; expected vector,
sorted, index, hash, text, or range"* once — a kind list that recites the kind the author had
already picked correctly, when the only thing in the way is the `?`.  `Parser::iterator` now
recognises a nullable whose inner type IS iterable and names the cure instead: add `?` (the
type's default, an empty collection) or `?? []`, either of which gives an absent collection
zero iterations.  Both spellings already worked; nothing said so.

The duplicate line is gone with it: `Parser::for_type` peels `τ?` before resolving the element
type — the element type of a nullable collection is its element type, the `?` being a fact about
the collection rather than about what it holds.  @PLN25's dn1 audit carries a NEEDS-FIX row for
that site (*"`for x in nullable` misses Text/Integer arms → peel in_type"*) AND, further down,
the verdict that dropped it: peeling `for_type`/`iterator` routed `text?` into a
text-char-iteration path that PANICS.  Only the first half is taken here — `iterator` does not
peel, it refuses — so that path is unreachable; verified on `text?` null and present.  Five
errors become three, informative one first, and the optional audit gains a peeling site
(729 · 365 · 5 · 359).  The cure named is the inner type's own default, so `text?` is told
`?? ""` rather than `?? []`.

The refusal is the null model being consistent rather than an omission: iteration is the same
`(N-Coal)`/`(N-Default)` discharge `v[i]` needs, and a loop that silently accepted a null source
would be the implicit unwrap `types.md` rules out everywhere else.

### A hash iterates in KEY order, and the rules doc now says so (2026-09-06)

`collections.md`'s `(Col-Order)` read *"hash → UNSORTED bucket walk (no key order) — the C-Order
decided edge"*, which is the opposite of the edge `concurrency.md (C-Order)` decides: the
SEQUENTIAL walk is key-ordered and only the `par` walk gives that up.  Measured on both
backends — a `hash<E[id]>` filled 49 down to 0 iterates 0..49, a text-keyed one alphabetically,
and the same collection under `par(…, 4)` comes out scrambled — and the parser's `hash_scratch`
sort is what makes it so, as LOFT.md and STDLIB.md already described.  So the code, `C-Order`
and the user-facing docs agreed and one line dissented; the line is corrected and
`@FR-Col-Order` gains its first citation, at the snapshot builder that implements it.  New guard
`a-collection-iterates-in-the-order-its-kind-defines.loft` pins all six kinds on BOTH backends —
the half of the rule nothing had asserted — including the `spatial` Morton interleave, whose
convention is read off four unit-square points rather than assumed (the second axis takes the
low bit of each pair, so the first-declared axis is major).  QUALITY.md B8g.

### A `#remove` refusal names the collection the author wrote (2026-09-06, loft#1403)

`hash`, `trie` and `spatial` all iterate a pre-sorted SNAPSHOT of their records, so `#remove`
would remove from the snapshot and is refused for all three — through one scratch variable, and
so through one message spelled for the hash: a `trie` author was told their loop was "hash
iteration" and prescribed `hash[key] = null` for a collection they never wrote.  The refusal now
names the kind, recovered from the scratch's own deps where the source is a local and from the
struct's one snapshot-walked field for `for e in b.data`; with two such fields the loop's own is
not decidable from there, so the wording stays kind-neutral rather than guessing.  A `spatial`
is keyed by coordinate axes and gets `spatial[x, y] = null` as its cure.  The message is a
pinned surface — `tests/issues.rs`, `the-reference-quotes-its-refusals-word-for-word.loft` and
CAVEATS.md quote it — and all three moved with it.  The kind is read WITHOUT peeling `τ?`: a
nullable collection cannot be iterated at all, so one never reaches the question, and a nullable
SIBLING field is not a candidate for which field the loop is over — peeling counted it and made
a decidable case answer vaguely.  One more `Type`-discriminating site, on the OPAQUE side of the
optional audit (729 · 364 · 5 · 360).
`@FR-Col-Remove` gained its first code citations in the same pass (`remove_vector_at`,
`remove_owned`, `State::remove`, `vector::remove_vector`), from the walk that found this
(QUALITY.md B8f), which also filed
loft#1401 (a `??`-discharged projection escapes `(H-Materialise)` and reads a renumbered
position after a removal) and loft#1402 (a by-index removal keeps what the element owned — one
record per removal, and it cannot be closed before #1401).

### A branch's projecting arm gets its own temp, so a binding chosen by an `if` materialises (2026-09-06, loft#1396)

`scopes::value_view_container` names the container a `Set`'s value views THROUGH a branch — any
arm's, none where two arms name different ones — and is read only by the walk that names views
to materialise, never by the deps strip.  The copy is supplied per ARM instead: `Scopes::arm_bind`
gains a projection case, gated on that naming, so the arm-lift machinery binds `h.inner` into an
owned `__lift_N` whose plain projection bind the F1 materialise already copies on both backends,
while a minting arm keeps its own store.  `x = if k > 0 { h.inner } else { mk(0) }` then survives
a later `h = …` as a first bind and as a reassignment, where before it read the new container on
both backends and on `--native` respectively.  The whole-statement alternative (strip the
binding's deps) was built and measured wrong — no emitter has a copy for a branch-valued
right-hand side, so the binding owned a store it only viewed — and the helper's doc carries that.
Guard `a-projection-arm-of-a-branch-materialises-when-its-container-is-reassigned` (6 cells, two
controls), falsified at bd629983.  formal: binding-history D-bind-19.  The chained spelling stays
open as loft#1393's view-of-a-view, which is wrong without a branch too.

### A `Set` whose value is a branch is walked in the order it runs, so a view bound in an arm sees its container's reassignment (2026-09-06, loft#1394)

`ViewWalk::walk_stmt` gains an arm for a `Set` whose right-hand side is an `If`, `Block` or
`Insert`: the value's statements are walked first, then the target's own establishment
(`disturb`, read off the whole statement because a struct-enum literal's mint names a work-ref
and only the `Set` says which variable took it), then the target is recorded — `leaf`'s third
step, split out as `record_target`.  Read whole before, a bind inside an arm and the
reassignment of its container in the same arm were one step, so nothing materialised:
`got = match sh { Holder{inner} => { sh = Empty{…}; inner.a }, _ => -1 }` answered 0 on
`--interpret` and 1 on `--native`, and the plain-struct `got = if c { x = h.inner; h = Hold{…};
x.a }` answered the new container on both.  The materialise arm now also lifts the never-free
mark it is retiring (`Function::clear_skip_free`) and admits an EMPTY dep list beside one
naming the container, which is what the `is` spelling of a payload capture carries.  Guard
`a-view-bound-inside-a-branch-arm-sees-its-containers-reassignment` (9 cells, four controls),
falsified at 6c09de23.  Filed: loft#1396 (a view that IS an arm's value — the naming can see it,
the emitters have no per-arm copy, and the widening alone is unsound), loft#1397 (the overwrite
cousin, which `(B-Disturb)` settles as not a disturbance; what is missing is loft#980's
warning).  formal: binding-history D-bind-18.

### A projection's container has one name, so a struct-enum payload view materialises, and the oracle stops calling it owned (2026-09-06, the `@FR-O-Owner` walk)

`use_analysis::projection_container_var` is the one derivation of *which container did this
view come out of?*, and it peels the variant check a payload projection wraps its subject in
(`sh.inner` is `OpGetField(if <tag> { sh } else { sentinel }, …)`).  It replaces two
byte-identical loops that each documented themselves as mirroring the other —
`scopes::base_container_var` (read by the view-materialise walk) and
`generation::container_element_base` (read by the emitters) — and is now read by the ownership
oracle's `borrow_base_guarded` as well, where a payload view classified `Owned`: the
over-free direction, whose visible face was `lost-write` warning about a write that lands
(loft#1395).  `scopes::established_stores` counts a `Set` whose right-hand side is a BLOCK
tailing in a heap `Var` — how a struct-enum literal hands over its work-ref — so a payload
view now sees its subject's reassignment; and the materialise arm consults `@FR-O-Override`
before making a binding an owner, the guard its var-copy sibling already carried, without
which a never-free `_mv_<field>_N` binding leaked a record per call on `--native`.  Guard
`a-payload-view-materialises-when-its-subject-is-reassigned` (6 cells, three controls),
falsified at 5f4ac074.  Oracle Check A clean over the 1247-file corpus and the fuzz corpus.
Filed: loft#1394 (a payload binding written inside the arm, which needs the walk's ordering
model), loft#1392, loft#1395.  formal: binding-history D-bind-19, IMPLEMENTATIONS.md's
`@FR-O-Oracle` row.

### An `is` payload binding borrows its subject like its `match` twin (2026-09-06, loft#1398)

`(O-Proxy)` reads an empty dep list as OWNED.  #429 gave a HEAP struct-enum payload binding a
frame dep on its subject for that reason, closing an interp-vs-native divergence — at
`parse_match_enum_field_bindings` only.  The `is` spelling is the same bind at the sibling site
and had `set_skip_free` without the dep, so its type read OWNED and `--native` deep-COPIED the
payload where the interpreter aliased it, then leaked the copy: `if w.st is Holder { inner } {
w.st = Empty{z: 0}; g = inner.a; }` answered 1 natively and 0 on the interpreter, `kt=82 Pay×1`
not freed.  `(O-NoDiverge)` forbids the split; `(B-Disturb)` says 0 is right, and the `match`
twin has answered 0 on both backends throughout.

Fixed at the `is` site with #429's own shape test — `Reference` / `Vector` / `Enum` take the dep,
a scalar takes none, and a `text` payload is untouched (an owned copy with its own write-back).
The two sites stay separate on purpose, because they bind through different subject expressions;
what they share is `Parser::match_borrow_source`, so the next divergence of this class is a grep
for that call.  Guard `an-is-payload-binding-borrows-its-subject-like-its-match-twin` (8 cells),
falsified at cb2dca92.  formal: ownership-history.md D-own-39.

### A payload binding warns when its subject's place is given another variant (2026-09-06, loft#1397)

`(B-Disturb)` is explicit that overwriting a place is NOT disturbing it, so a `match` / `is`
payload binding still names the payload slot and reads what is there now — both backends, and
the VALUE is what the rules give.  The issue offered two ways out and only one is admissible:
the other rewrites that clause for one type former, and the rules do not change to match the
code.  So the cure is the DIAGNOSTIC.

loft#980's `variant-field-unchecked` exists for this hazard and is deliberately quiet for
`match` / `is` bindings, because those are per-arm and are the cure it names — an exemption
whose premise is that the variant cannot change under the binding.
`use_analysis::warn_variant_overwritten` reports where it does, keyed on the ARM's OWN tag test
rather than a discriminant re-derived in the lint, so it cannot drift from the numbering the
parser emits: the condition names the place and the tag in one node, and the overwrite is an
`OpSetEnum` on that same place with a different one.  Both spellings reach it — `match` hands
the test bare, `is` yields it as the tail of an `Insert`.  The binding is recognised by its
`_mv_` name rather than through `mv_field_origin`, which is cleared when the two passes are
joined.

A LOCAL subject never arrives: `sh = Empty{…}` builds into a work-ref and repoints the local,
so its `OpSetEnum` names a different place, and that shape is a reassignment `(B-View)` already
materialises.  Guard `a-payload-binding-warns-when-its-subject-is-given-another-variant`
(both spellings plus four controls — the same variant, an unrelated field, the copy-out cure,
and the local subject), falsified at 26c5609b.  Zero reports over `tests/scripts`, `tests/docs`
and `default/`.  `LOFT_NO_VARIANT_OVERWRITTEN` turns it off; DIAGNOSTICS.md carries its row.

### A branch arm projecting a collection gets its copy (2026-09-06, loft#1399)

loft#1396 gave a projecting arm its own temp, matching `Reference` and struct-`Enum` and
answering `None` for a `Vector`; the deps strip that would otherwise reach a collection is
correctly not taken for a branch-valued binding, so nothing copied — `--interpret` read the
reassigned container while `--native` was right off empty deps.  `(B-View-Base)` settles the
value at the old one; closed with `ArmBind::CopyVector`, the buffer-and-refill a whole-vector
copy already uses.

Two things worth more than the fix.  The ROOT was tree-dependent: filed against the sibling
branch as *"the walk never names a collection view"*, true there and false here, because the
union of the two `leaf` gates at the loft#1396 pick made naming and copy land together — so
the same patch measured INERT there and is live here.  And the aliasing BOUNDARY is inverted
between the kinds: a record projection aliases when undisturbed, a collection one copies
(`(B-Copy)`), so an "undisturbed arms must still alias" control is right for a record and
wrong for a collection — which is how this fix looks over-wide when read the record way.
Guard `a-collection-projection-arm-of-a-branch-materialises` (9 cells), falsified at c22c318f
with native INERT, the correct verdict for a backend divergence.  formal: binding-history.md
D-bind-23.

### A view of a view is a place inside the outer container (2026-09-06, loft#1393)

`(B-Disturb)` ends a view's place when its CONTAINER is disturbed, and a chain names ONE place
however many statements it is spelled over.  `base_container_place` resolves a chain inside one
EXPRESSION; split through a local it did not, so `t = dv.tiles; prev = t.proto` recorded `prev`
as a view of `t` while the disturbance named `dv`, and `prev` was never shaken — it read the
CURRENT pass's data, both backends, no advice.  Reported by `planets`, where the loop compared
consecutive simulation steps and therefore answered *nothing changed* on every step.

`ViewWalk::resolve_view_root` follows the container through the views already open and keeps the
OUTERMOST field, which is `base_container_place`'s own rule one level out; bounded, stopping at
the first container that is not an open view.  Guard
`a-view-of-a-view-is-a-place-inside-the-outer-container` (6 cells), falsified at 00ff5bb5.
Residual and wrong for a different reason: the same chain whose tail is a value BRANCH, which
`leaf` takes whole — the ordering half, loft#1394's, measured unmoved by this fix.  formal:
binding-history.md D-bind-20.

### A build reads its destination through an element or a capture (2026-09-06, loft#1391)

`(I-Comp)` is *whichever destination*; D-iter-4's snapshot reached the ones
`Parser::field_place` could name — a variable and a chain of `OpGetField`.  The two it could
not answered the EMPTIED result on both backends, silently: a field reached through an ELEMENT
(`xs[0].items = [xs[0].items[1]?, …]`) and a collection a CLOSURE captured.

Closed in the PLACE.  `field_place` gained an ELEMENT step — the index must be one a single
statement cannot change (a constant or a variable), and both element spellings normalise to one
step, since a nullable read of an element is that element — and a CAPTURE step,
`OpGetDbRef(__closure, <offset>)`, which is @PLN93's build-into-target.  Two gates beside it had
hidden the second destination: the place was computed only when the caller SAID the destination
was a field, so a capture had no read test at all; and `literal_builds_into_dest` ran its
`OpClearVector` ahead of the whole block, so the snapshot copied an already-emptied destination
— the clear sits inside the block after the snapshot now, where the field spelling has always
put it.

Guard `a-build-reads-its-destination-through-an-element-or-a-capture` (14 cells: both
destinations by literal, comprehension and `+=`, a variable index, a surrounding loop, an
if-arm, and five controls — a sibling field, another element's field, a nested plain field, the
plain local, and a capture that reads only `len`), falsified at 00ff5bb5.  `matrix_axes.py` is
what named the loop and comprehension axes the first cut had held fixed.  formal:
iteration-history.md D-iter-5.

### A vector link follows a rebind of its source (2026-09-06, loft#1392)

`(B-Ref-Alias)` / `(B-Ref-Uniform)`: a `&` binding is a live link.  A vector link SHARES the
source's `DbRef` rather than dereferencing a stack slot, so a fresh backing on either side
re-points one of the two.  loft#1371 cured the link side (`@FR-B-Ref-Write`: a whole-value
write through the link clears the shared store and refills in place); the source's own
whole-value write had no cure, so `q = &v; v = [7, 8, 9]` left `q` on the old store, on both
backends, silently.  Registering the SOURCE in `amp_vector_locals` is the fix.

Two consequences, both load-bearing.  The registration is PASS-2 only: the set outlives a
pass, so a registration made at the link would reach the source's DECLARATION when pass 2
re-reads the body from the top and drop the allocation that gives it a backing store —
measured, `v = []` after a link compiled to a clear of a variable native never declared.  And
`(I-Comp)` is over the PLACE: with the source rebuilding in place, a build that reads its
destination through the LINK reads what it is emptying, so `Parser::snapshot_read_destination`
now takes the link's partner into both its read test (`parts_read_a_vector_link`) and its
rename.  `1194-a-comprehension-reads-its-destination`'s alias control is what caught it.

Guard `a-vector-link-follows-a-rebind-of-its-source` (14 cells), falsified at 3360fb93.
formal: binding-history.md D-bind-18.

### A captured local assigned again frees the store it ends up holding (2026-09-06, loft#1388)

`(O-Latest)`: a closure record adopts the store its capture named AT THE BUILD, and the frame's
scope-exit free is suppressed so the record's cascade is the sole owner (#323).  loft#1324
established that the suppression must name the same STORE the adoption does, and closed it for
the COLLECTION half via `capture_build_backings`.  The DIRECT half asked `is_captured(v)` — a
fact about the BINDING — and kept suppressing the free of whatever the local named LAST, so
every reassignment after a build orphaned a store.

`capture_build_backings` now returns `CaptureBuilds`: the backing map it always had, plus the
set of captured locals ASSIGNED AGAIN after their build, read off the same walk.
`capture_adoption_owns_free` — the one home `get_free_vars`, `check_ref_leaks` and
`ownership_cfg`'s leak oracle all read — declines the suppression for those.  The two build
spellings differ in ORDER: a stored closure is built by an earlier statement, an inline argument
inside the very right-hand side that reassigns the local, and `Value::walk` is pre-order, so the
inline build is reached AFTER its own assignment.  `captures_built_in` reads those builds off
the right-hand side against the map as it stood before the assignment, and the walk skips them
when it arrives — both spellings of "the capture reaches `v`" (the local outright, and a view
whose root is the local, minted inside that same right-hand side).

Measured over 9 cells on both backends: 7 clean where 6 leaked, values unchanged.  Guard
`a-captured-local-reassigned-after-the-build-frees-its-own-store`, falsified at ac412a96.
Both residual cells were CLOSED the same day by `(O-Witness)` — release by store identity, with
the hand-off at the closure build ahead of it and the record's own release gated on every
capture in the build having a witness — and the COLLECTION half by the same suppression clause
that declines the direct capture's, since a capture assigned again after its build no longer
names the backing the record adopted.  Eleven cells, both backends, all clean.  See
`formal/ownership-history.md` D-own-38 for what the gate protects against, and for the
vector-typed witness that was built and measured wrong on the way.

### A variant arm and an enum arm join, and the `match` join widens like the `if` join (2026-09-06, loft#1390 + loft#1389)

`formal/types.md` `(C-Var)` licenses `Reference(S) ⤳ Enum(E)` and nothing between two variants,
so once a branch's arms have settled on ONE variant, `convert` is being asked the wrong question
for both kinds of arm that JOIN to the enum instead: another variant (loft#1117, closed 2026-08)
and the enum itself.  The second was still asked and answered *"expected Circle, got Sh"* — for
`match`, for the `if` twin, through a wildcard and through a named arm.  `_ => e` hands back the
binding the statement assigns, which is the ordinary "replace it when …" shape over an enum.

The acceptance was half of it.  The `match` join (`join_arm_into`) kept the FIRST arm's type
however many arms widened it, so `v: Circle = match e { Circle{r} => …, Square{s} => Square{…} }`
was ACCEPTED and read a `Square` at `Circle`'s offsets — loft#980's class, silent, where the `if`
twin has refused it throughout because `parse_if` widens before the destination is checked.

One home: `Parser::joins_to_enum`.  `arm_joins_to_enum` asks it at the two acceptance sites
(`block_result`'s sibling arm, `parse_match_arm_body`), `join_arm_into` at the six `match` arm
sites, `parse_if` at its two join sites, and `match_arms_unify` at the cross-arm gate — so an arm
the acceptance waves past is exactly an arm the join widens for.  The predicate gained the check
its new callers need: a `Reference` arm must be a variant of THIS enum, since an acceptance site
sees pairs a join site only saw after `convert` had refused them.  Guards
`a-variant-arm-joins-with-its-enum` (7 cells) and
`a-variant-typed-destination-refuses-a-widened-match`, both falsified at 32e36462.
formal: types-history.md D-Var-Enum.

Beside it, loft#1389: `Parser::change_var` strips the degenerate #328 self-dep — *"x borrows from
x"*, which no ownership rule defines — and read `Type::Reference` alone.  A struct-enum is the
other RECORD kind (`data::is_dbref` names the two together), so `e: Sh = Circle{r: 1}` kept
`deps=[e]`, `owns_displaced_store` read it as BORROWED (`@FR-O-Proxy`), and a join reassignment
never freed the store it displaced — one per execution on both backends, values right throughout.
Stated over `Reference | Enum(_, true, _)` and read through the generic dep accessors, so the
stripped list keeps its dep SPACE.  The COLLECTION kinds stay out by measurement, not by
assumption: an env-gated probe over the corpus counted 6418 self-deps reaching this site on a
collection kind (3760 `Vector`, 2658 `Text`, 42 keyed/optional) — every one the @P302
re-init-in-place ownership marker `ownership.md` (g) reads as `Owned` — and ZERO on `Enum`.
Guard `an-annotated-struct-enum-local-owns-what-it-mints` (7 cells), falsified at 32e36462
(`kt=82 Circle×6` -> clean, both backends); it carries a `main` because `--tests` does not
leak-check.  formal: ownership-history.md D-own-37.

### A collection literal reads what its destination held, and a native join reassignment frees what it displaces and compiles (2026-09-06, the `@FR-O-Detach` walk)

`Parser::snapshot_read_destination` (`src/parser/vectors.rs`) is the one home for *a build
reads its destination*: it copies the destination into a `build_src` temp before the first
write and renames every read in the parts to the copy — the comprehension's deferred route
(loft#1194) now calls it, and the vector LITERAL asks it for the first time, on a local (`=`
and `+=`), a parameter and a struct field (`Parser::field_place` compares the place).  The two
sites that insert the destination's detach at the head of the build's ops — `create_vector`'s
`=` repoint and `parse_assign_op_inner`'s `clear_vector_field` — insert after the snapshot,
whose length `parse_vector` records in `Parser::build_snapshot_len` and `parse_assign_op_inner`
hands them (flat ops, not a block: a block scoped the temp's `let` away from its readers on
`--native`).  Sixteen spellings measured wrong before (`[0, 0]`, `len` reading `0` then `1`, a
struct element's `?? default`), all right after, both backends.  Native's `owned_ref_reassign`
(`src/generation/dispatch.rs`) now counts a `Value::If` as a right-hand side that produces a
store, so `s = if c { mk(7) } else { s }` frees the displaced store on both backends
(`(O-NoDiverge)`); `output_if_inner` (`src/generation/emit.rs`) closes an arm's brace on the
same peeled test it opened it with, so a span-wrapped `join-arm-owner` arm — the `match`
spelling — no longer emits an unbalanced `}`.  Guards
`a-vector-literal-reads-what-its-destination-held` and
`a-join-reassignment-whose-other-arm-is-the-binding-frees-and-compiles`, both falsified at
6f9c0886.  Filed: loft#1388, loft#1389, loft#1390, loft#1391.  formal: iteration.md `I-Comp`
names the literal (D-iter-4), ownership.md D-own-36, IMPLEMENTATIONS.md gains the
`@FR-O-Detach` row.

### A match arm converts to the type its siblings answer in — loft#1380's twin (2026-09-06)

`@FR-N-Decl` — a declared slot checks `e ⇐ τ`, and a construct's type is what its destination
checks against.  loft#1380 closed that for an `else if` CHAIN by parsing the chain's then block
with the enclosing then arm's type (`parse_if_expecting`).  Every `match` arm site still passed
`&Type::Unknown(0)`, so `parse_block`'s tail conversion had nothing to convert TO and a bare arm
was never converted at all:

- `f: float = match k { 1 => 1.5, _ => n }` read the integer's bits as a float (`1e-323`);
- `x: integer = match k { 1 => 1, _ => 2.5 }` read the float's bits as an integer;
- `c: u8 = match k { 1 => a, 2 => a + b, _ => b }` put **260** in a `u8`.

Silent on `--interpret`; on `--native` the first two handed rustc an `i64` in an `f64` position
and leaked a raw `error[E0308]`.  The third was silent on both.  The `if` twin of each is
refused, and loft#1380's own body recorded the `match` spelling as a VERIFIED workaround — it
was not one.

Present for every subject kind.  The enum and struct-enum paths do carry a cross-arm
`match_arm_types_unify` gate, but it asks `Type::is_same`, which collapses every `Integer(_)` to
one type — so the narrowing cell was silent there too, and the scalar, vector and tuple paths
carry no gate at all.

**The fix.** One home: `parse_match_arm_body` parses an arm with `match_arm_expected(&result_type)`
— what the arms have agreed on so far, `Unknown` while nothing is settled — and all seven arm
sites route through it.  `"match_arm"` joins `arm_of_sibling` in `block_result`, so a `{ … }` arm
converts at its tail with the carve-outs already applied to `else` (the literal-fit exemption,
the sibling-variant join, the honest nullability); a BARE arm is converted through the same
`convert_admitting` / `validate_convert` pair, with those carve-outs restated in the same order.

Two carve-outs are load-bearing and were found by measurement, not by reading:

- **A `&τ` on either side is passed through.** A struct-enum pattern binding yields a borrow
  (`Ship { carrier } => carrier` is `&text`) while a sibling arm yields the owned twin
  (`_ => "none"`); `match_arm_types_unify` strips the wrapper for exactly this reason. Asking
  `convert` for it re-pointed the wildcard arm's value — `35-nested-match.loft`'s extractor
  stopped answering its own literal.
- **The cross-arm gate stays silent for an arm the conversion already named** (`arm_convert_reported`,
  cleared after the body is parsed so a nested `match` cannot leave its inner arm's answer
  behind). The two ask the same question and the conversion asks it more precisely; without the
  flag a kind mismatch reported twice.

`validate_convert` renders the `match_arm` context as *"a match arm"*: the string doubles as the
internal name `parse_block` and `block_result` key on, so the keying is untouched and the reader
gets *"expected float, got text on a match arm"* beside the twin's *"on else"*.

Guards: `tests/scripts/1380c-a-match-arm-answers-in-its-siblings-type.loft` (the cells that run —
subject kind, arm kind, arm taken, destination) and `1380d-a-match-arm-of-another-type-is-refused.loft`
(ten refusals, one diagnostic each). Both falsified at `14b50b1f` on both backends.

### The seam between the store-lifetime and group-walk streams: two rules that had never met (2026-09-06)

Neither defect below exists in either stream alone.  `@FR-Col-Group`'s element-level write
route was measured against a store where an absent read answered the SLOT spelling and a
nullable element was read as a plain projection; `@FR-L-Null` then made an absent read answer
`nullref` (loft#1374) and a tagged element read as `if <present> { payload } else { nullref }`
(loft#1367).  Two rules, each right on its own tree, whose implementations met for the first
time when the streams were joined.

* **The group walk could not reach through a nullable element.**  `keyed_field_site` walks the
  `OpGetField` chain to find a field's holder; a `vector<R?>` element arrives as that `if`,
  which is not a vector read, so `rooms[0].items.remove(0)` resolved no holder and the sibling
  unlinks were never emitted — the removed record stayed findable under its key with no
  diagnostic, on both backends.  The three walk entries (`keyed_field_site`, `holder_type`,
  `vector_element_type`) peel the read through `use_analysis::through_null_arm`, which is that
  question's one home.  The DENSE twin was never broken, which is what says the tag is the axis.
* **A record copy had no null DESTINATION guard.**  `w.es[9] = e` on a shorter vector names no
  element, so the write is dropped — `(L-Null)` read from the destination end.  With the absent
  read answering `nullref` the destination store is `u16::MAX`, and the copy indexed
  `allocations[65535]` and killed the process on a program the compiler accepts.
  `do_copy_record` guarded a null SOURCE for @PLN25 and not a null destination; it now guards
  both, and so does its native twin `OpCopyRecord`.

`(L-Null)` in `formal/layout.md` now states the destination half beside the read half, and
`(Col-Group)`'s prose names the peel.  Guard:
`a-group-walk-reaches-through-a-nullable-element-and-drops-a-null-write` (7 rows incl. the
dense control and a negative index, which names a REAL element and must still be written),
`@falsified-at: 96ef2467` — the joined tree with all six picks and neither fix.  Branch-internal:
neither shape reproduces on `main`, so neither is filed.

### loft#1378: a generic at a self-referential struct parses (2026-09-06)

Picked from the quality stream (`667983e5`).  A generic instantiated at a struct that holds a
collection of its own type, with a nullable return, recursed without a base case while resolving
the instance and took the process down on both backends.  The same commit writes and reads a
generic's NARROW vector elements at their declared width rather than at `integer`'s.

Guard: `1378-a-generic-at-a-self-referential-struct-parses`.  `Fixes #1378`.

### A linked group's members must share one element layout (2026-09-06, loft#1385, D-col-2, C117)

A struct holding BOTH a dense `vector<E>` and a `vector<E?>` beside a keyed member split into
two groups — the dense member fell out of its own group, silently, `len` of the member that
never received the record being a legal `0`.

It is a conflict between two rules, not a gap in one.  `(Col-Group)` said membership is "not
about whether the element is dense or nullable"; `(N-Dense)` says a `vector<E>`'s elements are
non-null unless the author wrote `vector<E?>`.  One record set that may hold absence cannot be
read through a non-null element type, and the records are not the same shape — a nullable
element is the tagged `__nullable<E>`, a dense one is `E`.

Both silent answers were measured.  The obvious fix — comparing the element through
`Stores::key_owner`, so the rewritten keyed member and the dense vector compare equal — DOES
form the group both ways; the dense member then receives a record and MISREADS it (`a[0].n`
answered 7, `a[0].k` answered 2, the `Some` discriminant), which is loft#1134's misread and
worse than the zero it replaces.  So the declaration is declined where the group would form
(`Parser::refuse_mixed_nullability_group`, in the parser before either membership derivation
runs, leaving the B7x agreement census untouched), with a message naming both cures.
`(Col-Group)` now states the layout condition, and the declined alternative — the group
adopting the tagged layout with tag-aware dense reads — is C117 in DESIGN_DECISIONS.md.

Fires in all four declaration orders.  Guards: the `@EXPECT_ERROR` cell and a controls twin
whose four shapes each differ from the refused one by exactly one thing — no keyed member (so
no group forms), all-nullable, all-dense, and a member's own `?`.  `Fixes #1385`.

### loft#1386: a value-position `match` needs a value from every arm (2026-09-06)

`@FR-F-Block` discards a block's value *"only where the BLOCK itself is a statement — a
`;`-terminated one"*.  A `match` used as a VALUE is not that, so every arm has to produce one.

A void arm is exempt from the arm-type unification — right in STATEMENT position, which is
loft#1382's half — and in VALUE position that exemption let the whole `match` take the OTHER
arms' type and answer `null`: a value the program never wrote and the type never declared, with
no diagnostic on `--interpret` and a raw rustc E0308 naming a generated `.rs` file on
`--native`.  The `if` twin was refused in the parser all along, so one construct answered where
the other refused, for the same program.

It refuses now, in the parser, so the rustc error is no longer reachable.  Of the two available
answers the accepting one is silent-wrong, which is what the freeze axis picks.

Two things the fix needed beyond the rule:

* **Statement position, from loft#1382's flag.**  `parse_match` takes and clears
  `stmt_if_pending` exactly as `parse_if` does, so the same one-token peek in `parse_block`'s
  loop serves both constructs.
* **"An arm was void" is not "the running result is void".**  The result starts `Void` before
  any arm and is promoted by the first one, so the fact is carried in its own scoped flag,
  set as the arms are joined and read once at the end of the construct.  There are SIX arm-join
  sites — the enum arms, the wildcard, and the scalar, vector and tuple forms — and a fix at one
  is a fix for one form.

Eight cells on both backends: both arm orders in value position, a three-arm case with the void
one in the middle, the statement cells in both orders, all-void, all-value, and the `if` twin.


### loft#1382: a statement `if` discards EITHER arm's value (2026-09-06)

`@FR-F-Block` discards a block's value *"only where the BLOCK itself is a statement — a
`;`-terminated one"*, and `@FR-F-Drop` adds that the work still runs.  Neither says which SIDE
of an `if` the discarded value sits on, so both orders are statements.  Only one compiled: a
void THEN arm makes the expected type `void`, which accepts any else arm, while a void ELSE
arm reached the arm-agreement gate as `void ⤳ integer`, licensed by nothing.

Position is knowable only by the caller — `parse_block`'s statement loop is what sees the `;`
— so it is handed down: a one-token peek marks a statement that BEGINS with `if`/`match`,
`parse_if` takes and clears it (an `else if` chain recurses through `parse_if_expecting` and
keeps it; a value-`if` nested in a statement one does not inherit it), and the gate reads it.

Three things constrain it, each found by the suite rather than by reasoning:

* A leading `if` does not prove statement position — a function TAIL begins its statement too,
  and there the arms must still agree (`parse_errors::wrong_if`).  Looking AHEAD for the `;`
  is the obvious answer and is wrong: scanning to the end of the construct re-lexes it, and
  reverting left the parser mis-positioned on 250 tests.  So the gate RECORDS the mismatch and
  the statement loop reports it unless a `;` followed.  Recording TYPES rather than a rendered
  message keeps `validate_convert`'s two-defs-one-name case (loft#1094) intact.
* Only a VOID arm.  The corpus pins `if c { 2 } else { "a" };` as a refusal — two VALUES of
  different types is a mistake wherever it sits.
* Position AND a void arm together.  Keying on the arm type ALONE breaks twenty tests:
  `Type::Void` on an arm is also what a block reports when its value travels through a BUFFER.

The arm keeps its OWN type in statement position, which the native side needs: loft#1381's
discard gate fires on exactly one arm being void, and handing it the then arm's type made both
read non-void.  `arm_result` also learned to type a nested chain-`if` — `infer_type` does not
answer for one, so the outer gate declined while the inner had already discarded, leaving a
value arm beside a `()` one.

Nine cells on both backends.  It also restores the statement `else if` chain that
loft#1379/#1380's arm conversion had narrowed.


### loft#1368: a return that may borrow one of two sources is FRESH (2026-09-06)

`@FR-F-Ret` — a returned whole heap value is FRESH, never a view of a parameter; the only
borrow a caller may get back is an explicit `&T`.  A return whose tail is a value BRANCH hands
back each arm's own borrow, so the callee's declared return names TWO sources
(`fn pick(p, q, …) -> Node["p", "q"]?`).  The caller's guarded bind can witness only ONE, so
the arm that borrowed the other compared unequal, read as owned, and was ADOPTED: a write
through the result reached the caller's argument, on both backends and in silence.

The cure is the workaround written into the compiler.  Binding the branch to a local first
copies each arm through its own temp (`@FR-B-Copy`, the join-arm lift of loft#1321), so every
arm hands back a value of its own and no witness is needed — which is exactly why the
documented workaround (`r: Node = if first { p } else { q }; r`) already worked.  Every
`-> S` / `-> S?` return now gets it, not only the ones whose author knew to write it.

Three cures were measured and rejected first, and they bound the answer:

* `Own::join(Borrowed{a}, Borrowed{b})` → `Borrowed{a}` breaks `pick(a, a, …)` and makes the
  workaround leak — `Own::join` serves EVERY branch join, so the callee's own `r = if …` is
  re-classified and loses its copy.  **No lattice-side cure can work**: the lattice does not
  know it is looking at a return.
* → `Borrowed{u16::MAX}` makes BOTH arms alias: every witness-gated consumer reads an
  unnameable base as "no witness" and declines, and declining means adopt.
* The lift at `scopes`' `Value::Return` arm never fires — instrumented, that arm sees the
  function tail zero times.

A correction to the filed scope: the DENSE `-> S` return was documented as the clean
workaround and aliases identically, so the axis is a value branch over parameters, not the
`?`.  The issue's `wa:` label is now `wa:partial`.

**The GENERIC instance is closed beside it (loft#1387).**  The rewrite cannot run in a
TEMPLATE — its body is cloned into every monomorph, and a local minted there reaches codegen
in the clone with no slot (`generic-monomorph-null-and-element` catches exactly that) — and
the monomorph does not re-parse its block, so it took neither.  `bind_monomorph_join_return`
applies the same rewrite to the MONOMORPH's body, in its own frame, where the local gets that
function's slot; it joins the `promote_monomorph_*` family that already runs there for the
text, tuple and vector returns.  The tail arrives either wrapped in a `Return` or as the bare
branch, because a monomorph's body is the substituted template's and its delivery has not run,
so both shapes are handled.

Eight cells on both backends; guard `tests/scripts/1368-…loft`, `@falsified-at: 964bab93`.


### loft#1383: a generic at two integer widths is two monomorphs (2026-09-06)

`@FR-G-Mono` says a call with concrete argument types produces ONE specialised copy per
DISTINCT instantiation.  Two integer WIDTHS are two instantiations and collided into one:
every `Integer(_)` resolves to the single `integer` type DEF, so `T = u8` and `T = u16` both
mangled to `t_7integer_id` and the second call was checked against the FIRST instantiation.

The consequence was order-dependence in the program's MEANING: narrow first REFUSED the wider
call (*"cannot implicitly narrow u16 to u8"*, naming the innocent second call site), wide first
admitted the narrower one by widening.  Collapsing the width is right for CONVERSION —
`(C-Int)` admits a widening — and wrong for IDENTITY, which is why one predicate could not
serve both.

The key is now the concrete type's own name, which carries the range, exactly as loft#1024 did
for a collection whose type def erases its element.  The mangled name feeds a Rust identifier,
so the range spelling's parentheses join the 1:1 replacement set and the LEN prefix
`original_name` / `find_method_receivers` parse back stays correct.

Measured on the emitted definitions: the same width twice yields ONE monomorph
(`t_15integer_0__255__id`), two widths yield TWO — it distinguishes without multiplying.
Eight cells on both backends, including the `vector<T>`-returning shape whose collision typed
an inferred local `vector<u8>` on pass 1 and `vector<integer>` on pass 2.  Guard:
`tests/scripts/1383-a-generic-at-two-integer-widths-is-two-monomorphs.loft`,
`@falsified-at: 964bab93`.


### A struct field is a container of its own: (B-Disturb) asks which field grew (2026-09-06, loft#1384)

loft#1373 could not tell `w.a` from `w.b` and gave the case up rather than get it wrong:
`OpNewRecord(parent, tp, fld)` names its container in TWO parts, so reading the parent alone
shook every view rooted at it whichever field it named (which silently emptied `moros_editor`'s
undo stack), and reading it not at all left `d = w.a[0]` stale when `w.a` itself grew.

The two sides name the field in different SPACES, which is what the middle answer needed: a
view carries a byte OFFSET (`OpGetVector(OpGetField(w, 16, …), …)`) and a growth a field NUMBER
(`OpNewRecord(w, tp, 1)`).  `Stores::field_position` is the conversion — the by-INDEX twin of
`Stores::position`'s by-name — `base_container_place` answers the view's `(var, offset)`, and
`same_place` matches them, with `u32::MAX` on either side meaning the whole variable so a
REASSIGNMENT of the parent still ends every place inside it.  The store is threaded into
`ViewWalk` for the materialise path only; the `&`-refusal path runs without it and keeps the
conservative answer, which for a refusal is the safe direction — refusing less, never more.

A field APPEND and a field REBUILD emit the SAME `OpNewRecord(w, tp, fld)`, and only a
preceding `OpClearVector` on that place tells them apart — in a SEPARATE statement, so a
per-statement disturbance walk cannot pair them.  `(B-Disturb)` says overwriting a place is not
disturbing it, so reading a rebuild as a growth materialised a view the rule says survives, and
`bind-copies-or-views-the-whole-boundary` — the seventeen-cell guard `formal/binding.md` names
as pinning the copy-vs-view line — went red on its `(B-View-Base)` cell.  The cleared places
are therefore accumulated across the WALK rather than per statement, which errs the safe way: a
place cleared once and genuinely grown later is a MISSED disturbance, costing a materialise,
where the other direction costs a program its meaning.

Guard: seven cells, four of them controls — a sibling field's growth, two siblings growing, a
view dead at the growth, and the whole-variable case loft#1373 already fixed.  The `&W`
parameter is a cell too, since a field reached through one is the same place.  Verified on both
backends with the fixture libraries, the eleven-guard view family, and scopes 258/258, parser
619/619, store 262/262.  `Fixes #1384`.

### A collection-typed element view materialises like its record twin (2026-09-06, loft#1377)

`(B-View-Depth)` makes a vector INDEX read a VIEW "whatever the element type", and
`(B-Disturb)`'s fourth event makes a growth a place-ending one — so `b = w[0]` on a
`vector<vector<integer>>` whose container then grows must take its own copy, as `d = v[0]` on a
`vector<S>` does.  It read `len(b) == 0`, silent on both backends.

The record fix did not carry, and admitting the type to the two gates was measured and backed
out at the time: the walk then recorded the view and the advice fired over a copy that was
never made.  Stripping the container dep is enough for a RECORD, whose bind reads the deps at
emit time; a COLLECTION bind decides copy-vs-view at PARSE time (`classify_vec_bind`'s
`depend().is_empty()`, the `(B-View-Base)` citation), which runs before the scope pass.  So the
copy is EMITTED instead, in the shape a whole-vector copy already takes (`ArmBind::CopyVector`):
a `__lift_N` buffer owning its store for the function's life, refilled in place by
`OpReplaceVector`, so a materialise inside a loop costs one store rather than one per iteration.

The local NAMES the buffer rather than owning it (`set_skip_free`), and a LOOP is what makes
that load-bearing: left owning, its scope-exit free released the buffer's store and the next
iteration refilled a freed one — the guard's loop cell read `rec=3735928559` under the arena
poison.  Guard: `1377-a-collection-typed-element-view-materialises-too.loft`, with three
controls — the inner-realloc shape `85` measures, a view with no disturbance at all (which must
still alias and write through), and a sibling field's growth.  Falsified at 8624960f on both
backends.  `Fixes #1377`.

### A linked group is the SET of collections over one element type, not a hub (2026-09-06, loft#1375, D-col-1)

`{ a: vector<E>, b: vector<E>, h: hash<E[k]> }` made the keyed member a HUB rather than the
group a set: a write through `h` reached both vectors, a write through either vector reached
only `h`, and each vector held its own entries plus whatever arrived through the hash — silent
on both backends, `len` of the short member being a legal `0`.

Filed as a design call and settled by the rule instead.  `(Col-Group)` reads *"provided at least
one of THEM is keyed"*, where `them` is every collection over that element type in the struct,
and its second sentence gives the rest by being applied twice.  `Stores::field` asks the keyed
question of the STRUCT now; and because it runs once per field as the struct is built, a keyed
member arriving LAST joins the members that were skipped while it was absent — without that
half `{h, a, b}` and `{a, h, b}` formed the group and `{a, b, h}` did not, the declaration-order
dependence loft#843 and loft#1158 removed for the pairwise case reappearing one level up.  The
rule's last sentence is qualified rather than changed: two non-keyed members are independent
exactly when the struct has no keyed collection over their element type.

Guard: `1375-a-linked-group-is-a-set-not-a-hub.loft`, with the two controls that keep the
boundary — two plain vectors and NO keyed member stay independent, and a collection over another
element type is not a member.  Residual filed apart: a dense `vector<E>` beside a nullable
`vector<E?>` splits into two groups, which is loft#1204's rewrite mechanism rather than this one
(loft#1385, D-col-2 OPEN).  `Fixes #1375`.

### (B-Disturb)'s growth event names a whole variable, not a variable's field (2026-09-06, loft#1384)

loft#1373 shipped for one commit with a spurious materialise.  `OpNewRecord(parent, tp, fld)`
names its container in TWO parts, and `grown_containers` read the parent alone, so a growth of
`s.us_redo` shook a view of `s.us_entries` — `moros_editor`'s `undo_pop` then read every undo
entry out of a copy, the stack silently stopped recording, and `undo_depth` answered `0` where
`3` was due, on both backends, with only an advice.  A field-qualified growth is now left
UNCOLLECTED (`fld == u16::MAX` is the whole-variable append), which is the honest direction: a
missed disturbance costs a materialise, a spurious one costs a program its meaning.  The
residual — a view of a field's element while THAT field grows — is loft#1384, and matching
field-wise needs the type table, since the view carries a byte OFFSET and the growth a field
NUMBER.

What caught it was `make ci`'s `moros_editor` html smoke, the only thing in the tree that
exercises the shape.  It is not in the corpus `introspect_diff.sh` walks — `tests/fixtures/libs`
is outside it — so a four-file diff, five green subject suites and a green falsify all read
clean while a library was broken.  Control added to the guard:
`a_sibling_fields_growth_does_not_disturb_this_view`.  `Refs #1384`.

### A statement `if` discards what its arms yield, on `--native` too (2026-09-06, loft#1381)

`(F-Block)` says a block's value is discarded "where the BLOCK itself is a statement — a
`;`-terminated one", and `(F-Drop)` adds that the work still runs.  The interpreter has always
done that; the native emitter rendered each arm as a Rust EXPRESSION, so
`if c { println("x") } else { 5 };` handed rustc a `()` arm beside an `i64` one and the author
got a raw `error[E0308]: `if` and `else` have incompatible types` for a program loft accepts.
The `else if` chain failed the same way.

`emit.rs::output_if_inner` gives that shape `{ <arm>; }` on both sides, which is what a
statement means.  The discriminator is a VOID arm beside a non-void one and is exact rather
than a proxy: an `if` read as a VALUE has arms the parser already made agree, and a void one
could not be read.  Both arms must be POSITIVELY typed — the first cut read an arm the emitter
cannot type as void, which fired on six TUPLE files and cost each the value its `if` was there
to produce; the corpus diff named all six.  Emission: DIFFERENT 17 of 1292 against 50f87d36,
every one the intended `{ … ; }` wrap, all 17 re-run green on `--native`.

Measured beside it and filed apart: the MIRROR statement `if c { 5 } else { println("x") };` is
REFUSED at parse time ("expected integer, got void on else") while the arms swapped compile —
one order of a statement `if` accepted and the other not, where `(F-Block)` discards either
(loft#1382).  `Fixes #1381`.

### Growing a container ends the places inside it — (B-Disturb)'s fourth event (2026-09-06, loft#1373, D-bind-18)

`(B-Disturb)` listed three place-ending events and an append was not among them, so the
materialise walk never fired for one: `d: S = v[0]` then two hundred appends read
`4294967296` on both backends with strict stores silent, while the same code with TWO appends
read `1` — `Store::resize` had not yet had to move the record.  `(B-View-Depth)` appeared to
settle it (*"the view survives a source realloc"*), and the guard it named appends to the
INNER vector of a `vector<vector<integer>>`, whose view names the OUTER slot and reads the
repointed handle; the realloc of the container the view NAMES was never measured, and at that
level the same shape answers `len == 0`.

`scopes.rs::grown_containers` is the fourth event's home — apart from `reshaped_containers`
only so the advice names the right statement, and sharing one argument walk
(`containers_named_by`) so the two op lists cannot drift in how they read a container.  Five
spellings, each naming its container at arg 0: `OpNewRecord`, `OpPreAllocVector`,
`OpAppendVector`, `OpInsertVector`, `OpHashAdd`.  The existing answer applies unchanged — the
view materialises and the author is told (`ViewCause::Grown` with its own advice) — and
`(B-Ref-Reshape)` refuses a `&` INTO a container grown while the link is live.  A `&` naming
the container itself is untouched, and needed no new test: `base_container_var` answers `None`
unless the right-hand side is a projection.  The two view gates now read the local's type
through `base()`, so a nullable `S?` view materialises like its dense twin.  Guard:
`1373-growing-a-container-ends-the-places-inside-it.loft`, eight cells, four of them controls.
Residual filed apart: a COLLECTION-typed element view is still stale (loft#1377) — admitting
the type makes the walk record it and the advice fire, but the materialise arm is
record-shaped, so the author would be told about a copy they did not get.  `Fixes #1373`.

### @PLN153 phase 5, batch 1: a read that names no record answers `nullref` (2026-09-06, loft#1374, D-layout-5 / D-layout-6)

`(L-Null)` gives a reference that has left its slot ONE spelling of absence, `nullref`; an
element read past the end, a keyed miss and a zero child pointer answered the container's live
`store_nr` with `rec == 0` instead, and only the `rec`-testing readers (`OpEqRef`,
`OpConvBoolFromRef`, `is_absent_collection`) saw it — the handle test (`OpRefIsNull`), a `S?`
parameter, a `-> S?` return and the nullable call-result bind all read it present, the bind
copying garbage.  `DbRef::or_null` is the one predicate, called where a read mints a value:
`vector::get_vector` (the hoisted read falls back to it), `State::vec_get_or_raise` and
`Stores::vec_get_or_raise_runtime`, `Stores::get_ref`, and the exits of `State::get_record`
and `codegen_runtime::OpGetRecord`.  A runtime change on its own; with the two parser
halves below, `scripts/introspect_diff.sh` against the pre-batch compiler reads
`DIFFERENT 18 of 1289` — the two new guards (one compiler refuses them), fourteen files
where a record bind into a NULLABLE local (`t: S? = s`) now takes the null-aware bind
(`OpBindOrCopy`; the `rec`-tested native bind) in place of `OpDatabase` + `OpCopyRecord`,
and two where a tagged element's field read now consults the discriminant first.
The `vec_get_or_raise` comment that kept the container's `store_nr` "so wrapping ops that call
`stores.store(&db)` directly don't panic" was measured false: no op in `fill.rs` resolves a
store before testing `rec` — but the interpreter's `State::iterate` and `State::step` did,
where their native twins (`codegen_runtime::OpIterate` / `OpStep`) test the holder's record
first; the two had drifted, and a `for` over a collection FIELD of a holder reached through
`nullref` (`for t in tokens[i].subs` past the end) read the store's header word as the
collection record before and indexed `allocations[u16::MAX]` after.  Both now test the record
first and iterate nothing, which is what C80 asks of a read through null; the six iteration
scratch builders (`build_hash_sorted_vec` and its unsorted, radix, index, trie-prefix and
radix-range siblings) answer `nullref` for an absent holder for the same reason, since a
`for` over a `hash` field collects its records into a scratch before the iterate.  Beside it, both
backends' record binds chose the null-aware form
by the SOURCE's type alone (`gen_set_first_ref_var_copy`; `dispatch.rs`'s record bind), so a
bare view holding `nullref` into a `S?` local copied nothing into an allocated record;
`Variables::bind_admits_absence` asks both sides for both.  Two readers of a `vector<S?>`
element: `parse_index` wrapped the tagged `__nullable<S>` element `Optional` for an unfit index
(`τ??`, refused between passes as a type change; `null_census` now counts an `Optional` over
the synthetic), and E2's field/method receiver projected the payload without the discriminant
— it now goes through `read_through_tag`, and a dense-`self` method reports `(N-Store)` for
the undischarged receiver exactly as for a `S?` local.  Guards:
`1374-an-absent-pointer-leaves-its-slot-as-nullref.loft`,
`153-a-tagged-element-read-by-a-variable-index-is-one-null.loft`,
`153-a-method-call-on-a-tagged-element-reports-the-undischarged-receiver.loft`.
`Fixes #1374`.
### loft#1372: a `&` reference links a nullable slot — D-bind-17 CLOSED (2026-09-06)

`@FR-B-Ref-Intro` admits `&τ` for EVERY τ, `@FR-B-Ref-Uniform` makes a `&τ` variable a τ
variable, `@FR-F-ParamRef` makes a `&` parameter the write-back channel, and none of them
restricts τ.  `&τ?` was declined anyway — the deviation `D-bind-17`, opened the day before —
because "the read and write lowerings do not carry the wrapper".

They needed no wrapper.  `Optional(τ)` shares `τ`'s storage EXACTLY, so a `&τ?` has the same
representation as its `&τ` twin and the absence rides the slot's own sentinel.  What was
missing is the thing the deviation entry named: one spelling of *the slot behind a link*,
asked wherever a link's inner type is read.  That spelling is `Type::base()`, and nine sites
were asking bare:

* the interpreter's `RefVar` READ and WRITE dispatch — `Optional` matched no arm and it
  panicked *"Unknown reference variable type"*;
* native's local-link bind and read arms — the bind emitted no right-hand side at all
  (`let mut var_q: … =  as …;`) and rustc reported it;
* native's `&`-parameter write-back — the displacement test, the text coercion and the
  boolean one, so a `&text?` wrote `*var_p = "z"` with no `.to_string()`;
* the `??` subject, which peeled `Optional` but not the LINK, typed its own result
  `&integer?`, and reported every default as the author's error;
* the retype check, which refused `&integer? = 7` as *"cannot change type"*;
* the argument match, which compared the parameter's referent against an argument reading as
  plain `τ`, answered no, and passed the argument BY VALUE — the callee then dereferenced an
  integer as a stack ref and the store accessor went out of bounds;
* the bare-`null` conversion, which found no `OpConv…FromNull` returning a link type and
  DROPPED the whole store in silence — `q = null` through a link left the source at its old
  value on both backends.

Thirteen cells on both backends: the issue's own repro, the local and parameter spellings,
the annotated bind, absent and present sources, a null written through the link, a live read
through it, `text?` / `S?` / `vector<T>?` inners, and the two non-null controls that say what
the answer should be.  `refuse_nullable_link` is deleted; its decline guard's cells stayed and
its expectation flipped, from `153-a-link-to-a-nullable-slot-is-declined` to
`…-carries-its-slot`.  Both guards falsified at 964bab93, which refuses every cell at parse
time.  binding.md's deviation list reads **OPEN: 0**.


### loft#1376: a whole-value write through a `&` link to a PLACE reaches the place (2026-09-06)

The sibling of loft#1371, and `@FR-B-Ref-Write` settles it the same way: at a heap τ the
write REPLACES the source's contents.  `pi = &o.i` and `pe = &v[0]` reach no `&` lowering at
all — a struct projection is already a VIEW under `@FR-B-View`, so both spellings emit the
same ops and only `is_amp_link` records that the `&` was written.  The variable holds the
field's or element's own `DbRef`, so an INTERIOR write through it landed, which is what made
the shape look like it worked; a WHOLE-VALUE write emitted a plain `Set` that re-pointed the
variable while the place kept its value, on both backends and in silence.

The copy-INTO-the-place branch that serves `o.i = S { … }` already existed; it was gated on
the target NOT being a bare `Var`, which is exactly what a `&`-linked local is.  It now also
admits a bare `Var` that `is_amp_link` names, so the link reaches the lowering writing the
place directly has always had — one home, not a second copy.  `@FR-B-Disturb` is why the
answer is a copy rather than a refusal: overwriting a place is not disturbing it, so a view
of it survives.

The discriminator is the VALUE, through the new `produces_whole_record` — the same thing
native's own link arm dispatches on, and it survives an IR snapshot where a per-statement flag
would not.  It HAS to be the value: `pi = &o.j` and `pi = o.j` emit identical ops (@PLN130 F9
— a `&` at a struct projection is invisible in the IR), so a place READ cannot be told apart
from a re-point of the link and keeps its binding meaning.  What is unambiguous is a value
that MINTS a record — an object literal or a call — because there is no place behind it to
link to; that is the one case this rule claims.  An allow-list of accessor NAMES was the wrong
shape and is what a keyed element read (`c = &s[30]`) fell through: routed into the copy, the
bind defined nothing and every later read reached codegen at slot 65535.  A PLAIN view is not
marked either way, so `c = o.i; c = S { … }` keeps `@FR-B-Copy`'s meaning.

Nine cells on both backends (place kind, write kind, nesting depth, direction, and the
plain-view and local-link controls); `tests/scripts/1376-…loft`, `@falsified-at: 964bab93`.
It also closes the two cells loft#1371's matrix left red.


### loft#1369: a nullable rebind from an ABSENT source releases the record it displaces (2026-09-06)

`@FR-O-Proxy` — the destination owns the store it holds, so a rebind that displaces it must
release it — with `@FR-O-Borrow` bounding it from the other side: a local that only VIEWS a
parameter owns nothing and frees nothing.  Native's nullable record bind emits two arms,
`if src.rec == 0 { dest = DbRef::NULL } else { dest = OpDatabase(dest); OpCopyRecord(…) }`.
The else arm needs no release — `OpDatabase` recycles the slot's existing store — while the
null arm displaces it, and the release was emitted at a FIRST bind only.

At a REASSIGNMENT the two native sites that could emit it each named the OTHER as the one
that frees: the nullable arm's comment said a reassignment "is already wrapped by
`output_set`'s `_old_*` stash", and the stash's own gate excludes a bare `Var` right-hand
side because it "is a copy whose own arm frees what it displaces".  Both comments were
internally consistent and the behaviour was not — for `x = <nullable Var>` at a reassignment
neither site emitted a free, and the record orphaned once per call whose source was ABSENT AT
RUN TIME.  The interpreter's twin was clean throughout, so `--native` alone lost the store.

The release moves to the one site that knows which arm it is on, gated on the same
`owns_displaced_store` predicate the stash asks, so a view of a parameter still frees
nothing.  Both comments now state which site owns the free rather than pointing at each
other.

The boundary is not the one the issue described: it is not the rebind between two parameters
but the RUNTIME absence of the source — two absent calls leak two records, and the same
function with the argument present is clean.  Seventeen cells on both backends (source
nullability, rebind count, declared vs inferred local, source kind, read shape, runtime
null, call count); the interpreter was green on every one before and after.  Guards:
`tests/scripts/1369-a-nullable-rebind-from-an-absent-source-releases-what-it-displaces.loft`
(`@falsified-at: 964bab93` — native leaked `kt=81 S1369×2`, interpret INERT, which is what a
backend-divergence guard should read) and
`tests/leak_cross_mode.rs::issue1369_…`, which arms the native leak check.


### loft#1371: a `&` link to a text or vector LOCAL is a link, and a whole-value write through one reaches the source (2026-09-05)

`@FR-B-Ref-Write` is the `&` ladder's north star and `@FR-B-Ref-Uniform` says a `&τ`
variable is used exactly like a τ variable, read and write alike, for a scalar or a heap
value.  Neither narrows τ, and the `&` PARAMETER channel already kept both promises for
every heap kind (`calls.md F-ParamRef`) — the LOCAL bind kept them for a scalar, a record
and a tuple and special-cased the rest, which is what `B-Ref-Uniform` says not to do.  The
parameter cells are what made the answer decidable rather than a design call: they are the
same rule at the same τ, and they were right throughout.

Four lowerings, one per shape the special-casing produced:

* **text** — `parse_assign_op`'s `stack_src` now admits a `Type::Text(_)` source, so the
  bind takes the same `RefVar(τ)` + `OpCreateStack(src)` form the parameter has.  Before,
  a text source matched no arm at all: the `&` was dropped and the bind COPIED, in both
  directions (`pc = &c; pc = "z"` left `c` at "a", and `c = "z"` afterwards was not visible
  through `pc`).  Native represents the link as `*mut String` rather than the parameter's
  `&mut String` — a raw pointer for the same reason the scalar link uses one, so the source
  local stays readable while the link is live — and the `Stack`-variant text ops take one
  `unsafe` wrapper at the op dispatch rather than one per op.
* **vector** — the `&` bind's `DbRef` share aliases element writes and appends already, but
  left the variable plain-typed, so `create_vector` gave a whole-value write a FRESH backing
  and re-pointed the local at it (`pe = [2, 2]` left `len(e)` at 1).  A `&`-linked vector
  local now reaches the `OpClearVector` branch a `&vector` parameter already takes, so the
  write clears the SHARED store and refills it in place.
* **the annotated spellings** — `pc: &text = c` and `pe: &vector<T> = e` typed the variable
  as a link over a value; the interpreter read the buffer as a stack ref and panicked
  (`store.rs` out-of-bounds; `keys.rs` store_nr out of range).  The vector annotation now
  yields the plain vector type its prefix twin gives, and `amp_vector_bind` asks through the
  `RefVar` wrapper the annotation coerces the source to.  `vector<T>?` is deliberately not
  matched, so it still reaches loft#1372's refusal.
* **struct** — the displacement release was gated on `is_argument` on the interpreter and
  absent from native's representation entirely.  The gate goes (the question is about the
  LINK, not how it was introduced — native's twin already asked only the inner type and the
  ownership), and native's local `&struct` link becomes `*mut DbRef` into the source's slot
  instead of the source's DbRef by value, which is what lets a whole-value write reach the
  source there at all.  Both backends now stash / install / `OpFreeRefIfDistinct`.

Measured as a 30-cell boundary matrix on both backends over five axes — source type kind,
bind spelling, source place, write kind, and the `&`-parameter control — each cell's value
hand-computed from the rules before the first run, asserting value and leak.  Guard:
`tests/scripts/1371-a-whole-value-write-through-an-amp-link-reaches-the-source.loft`
(`@falsified-at: 964bab93`).  `B-Ref-Write` now states its heap clause, which is what the
issue asked the binding chapter for.

**Still open, filed apart:** a `&` to a struct PLACE — `pi = &o.i`, `pe = &v[0]` — carries a
whole-value write on neither backend.  That is a link to a projection rather than to a
local, and it is a different lowering.


### @PLN153 phase 4, batch 1: the opaque verbs' bare callers, and a `&` link to a nullable slot declined (2026-09-05)

`is_dbref`, `heap_dep`, `heap_def_nr` and `is_scalar` are opaque to `Optional` themselves, so
each bare caller answers wrong for a `τ?` at once; the walk went caller by caller with a cell
each (`pln153-scratch/stage4/`).  The finding: the `&` lowering's `is_scalar` closure and
record test let a `&` of a nullable LOCAL fall past both arms and bind a silent COPY (`q = &x;
q = 7` left `x: integer?` unchanged on both backends), and lifting the parameter side's
retype refusals showed every read and write site asks a link's inner type bare — `??`, `+`,
the copy-out, the interpreter's write dispatch, native's parameter type.  A feature the rules
promise and the lowerings do not carry, so `Parser::ref_var_type` (and the field-link site)
now DECLINES a `&τ?` with one message naming the cure, both spellings, once per link, and the
binding keeps its plain type so nothing cascades — `D-bind-17` (OPEN, loft#1372); the two
source-kind tests read through `base()` so a nullable source reaches that decline.  The
matrix's non-nullable controls found loft#1371 (a whole-value write through a `&` link to a
text, struct or vector does not reach the source; the struct leaks).  Guard:
`153-a-link-to-a-nullable-slot-is-declined` (five spellings).  **Superseded 2026-09-06**
by loft#1372, which closed `D-bind-17` and flipped that guard to
`153-a-link-to-a-nullable-slot-carries-its-slot`.

### @PLN153 phase 4, batch 2: the CFG ownership oracle sees a nullable heap local (2026-09-05)

`ownership_cfg.rs` asked `heap_dep()` of a local's type bare at its four heap filters, so a
nullable heap local was never a leak or over-free candidate: the over-free positive control
with its view declared `vector<integer>?` went unflagged under the injected free the dense
control is flagged for.  Peeled through `base()` at all four; cell
`oracle_over_free_check_sees_a_nullable_view_local` + probe `08b-overfree-positive-control-nullable`.

### @PLN153 phase 3c: a tagged projection reaching a local is read through its tag (2026-09-05)

`(L-Null-Tag)` reserves the tagged `__nullable<S>` for INLINE storage and `(L-Null)` gives
everything else the pointer; the code left a LOCAL's spelling to whichever assignment parsed
last, so `x = y; x = o.opt` read the pointer `y` as a tagged record and the owner witness freed
its store, `x = o.opt ?? y` was refused naming the synthetic, `x = o.opt; if c { x = null }`
was refused, and `d: S = o.opt` was silent and read zeroes for an absent slot (loft#1367).
`Parser::read_through_tag` is the one home — a tagged value reaching a non-slot position is
read through its tag there and becomes the pointer on both passes — with four callers: the
assignment seam's plain-local target, the tuple destructure (`tagged_pointer_type` for the
type half), the `??` subject and the postfix `?` subject.  The read is an `if` (payload
projection | nullref) that three ownership predicates each misread in turn; one predicate
now answers for all, `use_analysis::through_null_arm`, with `holds_no_store` the join's
identity in the var-level oracle as well.  The reverse order (`x = o.opt; x = y`) had been
right all along under `@FR-B-Copy` (a bind off a parameter copies), against loft#1367's own
expectation.  Corpus: 15 files moved, all emission, each green on both backends under strict
stores; guard `1367-a-tagged-projection-bound-to-a-local-is-the-pointer` (21 cells);
`D-Null-Local` opened and closed in `layout-history.md`.  `Fixes #1367`, `Contract: settled`.

### @PLN153 phase 3b: the declared local takes `(N-Store)`'s severity, and the inferred one widens (2026-09-05)

One question — what is a LOCAL's type after a `τ?` is written to it — with two answers in
the rules and one refusal in the code.  `(N-Decl)`: a declared `x: τ` keeps τ and the write is
`(N-Store)`'s, a WARNING at full width with the store proceeding, an ERROR at a narrow width.
`(N-Join)`: an inferred local widens to `τ?`, silently.  `change_var_type` refused both with
"cannot change type from `τ` to `τ?`" — the twelfth home of the refusal and the only one that
erred where the eleven others warned (Stage A's `local` row read ERROR for every full-width
kind while `element`, `argument` and `return` read WARN), and the inferred half hid behind a
vacuous guard: the phase-1 hold cell indexed with a CONSTANT, which @PLN102 D1 trusts by
contract and types non-null, so its assert read the in-band sentinel and nothing was ever
widened; with a variable index the program was refused.

Each half now has its home.  A declared local — a parameter too, and a write-back `&τ`
parameter, which is the caller's slot one link away and was never asked (the `RefVar` peel
in `change_var_type` carried the null in silence) — is asked through the one store face
(`convert_store`, "the local `x`" / "the parameter `x`") BEFORE the retype, at the assignment
seam and at the tuple-destructure site; the retype then sees the peeled type.  An inferred
local takes a `(N-Join)` arm beside DN6's null-start arm (which now reads its source through
`base()`, so `a = null; a = v[i]` joins too): widen to `Optional(the wider width)` — `u8 ⊔
integer? = integer?` — and say nothing.  The split reads one predicate,
`Function::is_declared` (`argument || annotated`, what `retype_would_be_refused` already
read), refined at the parser by `author_declared`: a local the compiler PROMOTES to a hidden
out-parameter (the `text_return` buffer hoist) carries `argument` under the author's name,
and without the refinement five corpus files warned spuriously — `got = maybe(i); return got
?? "<none>"` read as a parameter store.  Read off the definition's `hidden` attribute, not
declared per hoist site.

Measured: Stage A moved the five full-width `local` cells ERROR → WARN and nothing else
(66 silent / 32 warn / 4 error); a 25-cell list written before the first edit, every value
hand-computed, both backends under strict stores and poison; `scripts/introspect_diff.sh`
over the corpus differs only on the re-pinned guards and loft#859's file, which now compiles
(its inferred `g` widens and the return warns).  `D-Decl-Sev` opened and closed in
`types-history.md`; the worked table under the rules corrected.  Found on the way and filed:
loft#1369, a nullable struct local rebound from a non-null parameter to a nullable one leaks
one record on `--native` (pre-existing on the declared spelling; a neighbour of loft#1367).
A documented refusal became a warning — `Contract: strained`.

Three Rust tests had used that refusal as their measurement channel and were re-pinned to the
rule's warning: `untrusted_arith_index_stays_nullable` (an untrusted index into a declared
accumulator), the two `qq_null_typing` cells (a nullable `??` fallback into a declared local,
with `LOFT_NO_QQ_NULL` restoring the pre-fix silence rather than the pre-fix accept), and
`pln102_stdlib_reachable_null_returns_are_typed_nullable`.  The fix VERIFIER (`loft fix`,
`fix_apply::verify_fix`) had read "nothing new may appear" as "no new ERROR": `x: integer =
"5" as integer` was offered `as integer?`, which now compiles into that slot with `(N-Store)`'s
warning and stores a null on a bad parse — the rewrite changed the program's meaning and the
verifier would have written it.  It now counts a new WARNING as `Breaks` too (advice stays
below the line), which is the two-tier doctrine applied to a rewrite: a warning is the tier
where ignoring it can produce a wrong result.

### @PLN153 phase 3a: `(N-Store)` is asked at the one arm every `τ? ⤳ τ` peel passes, and eight silent stores now report (2026-09-05)

`Parser::convert`'s Optional-SOURCE arm is where every nullable value is peeled to its base,
and a census over the whole corpus (`LOFT_TRACE_UNWRAP=1`, kept as an instrument: 6014 peels
in 1268 files, each with its caller) showed no peel happens anywhere else.  So the rule's
τ? half is asked THERE — `nstore_unwrap_report`, one body — and its bare-`null` half at
`convert`'s entry (`nstore_null_report`), instead of at eleven store sites that each had to
remember to ask first and a twelfth that did not.  A store names its slot through
`convert_store(…, what, at)` (a context stack the arm reads for its wording; the tuple arm
prefixes `element i of` as it recurses), a read that is not a store admits the peel through
`convert_admitting` — a null test, a condition, `&&`/`||`, an overload trial, a
null-transparent callee, an `if` arm meeting its sibling — and a bare `convert` is asked with
generic wording: a site that forgets degrades the message, never the rule, and a face that
forgets to admit warns spuriously, which the corpus shows.  The lowerings that store without
converting (the if-join accumulator, the append routes, a struct literal's vector-field deep
copy, a `null` return's sentinel, a rewritten tuple return) keep `n_store_violation`, now a
thin caller of the same two bodies.

What it found: the seven cells of loft#1366 (a `τ?` into a tuple literal's member, all six
kinds; a `vector<τ>?` into a non-null struct field) and an eighth the census named — a
nullable INDEX (`v[j]`, `s[at + 1..]` after a `find`, six corpus files, every one behind an
`if at < 0` guard that a null never takes).  Each reports as a WARNING at every width, the
loft#1232 doctrine for a seam first covered: reporting where there was silence is the gain,
refusing what compiled yesterday is the break the freeze forbids.  The 102-cell Stage A
matrix moved on exactly those seven cells and nowhere else; `scripts/introspect_diff.sh` over
the corpus (stderr included) reads `DIFFERENT 16 of 1272` — every one a stderr line and none an emission: the thirteen corpus files that gained a warning (nine nullable indexes after a `find`, three `null` keys, one `null` into a vector field), reviewed one by one, and the three new guards, which differ by construction; the admitting-faces guard is silent on both
builds with every value pinned.  `(N-Store)` in `formal/types.md` now names its slots and
its non-stores.  `Fixes #1366`.

### A null element of a linked group leaves nothing, and a hash remove of an absent record is a no-op (2026-09-06, the B7x gate flake)

`make ci` on ffae9ce6 passed with one flaky: the B7x guard's r4 cell (`w.es[0] = E{…}` into a
NULL slot of a `vector<E?>` group member) read `len(by_k) == 1`, one seed in twenty.  The
element-level unlink loop B7x emits (`group_sibling_unlinks`) handed the OLD element to
`Stores::remove` whether or not it was a record; a null slot arrives as a record whose key
reads as zero, and two views answered two wrong ways: `hash::hash_rec_pos` probed for a record
the table did not hold, WRAPPED to the home bucket and answered it, so `remove` zeroed a live
sibling's entry under every seed whose zero-key bucket was occupied; `tree::remove` asserted
`Item not found` on every seed (the `index` member, deterministic).  Two homes fixed:
`Stores::absent_nullable_record` is the one null test both halves of `@FR-Col-Group` ask
(`link_siblings` on ENTER — it already skipped a non-`Some` — and `Stores::remove` on LEAVE,
for the keyed kinds), and `hash_rec_pos` answers `Option`, stopping at the first empty slot,
so a remove of an absent record is a no-op, never a hole.  Guard
`a-null-element-of-a-linked-group-leaves-nothing` (hash / index / trie / all three; first,
middle, last slot; refill, null-over-null, `remove(i)`; nested), 0 failures over 60 seeds
where 2 in 40 failed; falsified by hand on ffae9ce6 (exit 101 → 0 both backends).

### A generic at a self-referential struct no longer dies in the parser (2026-09-06, loft#1378)

`fill_monomorph_body` → `rewrite_vector_write_triplets` sized a `vector<T>` element for EVERY
generic body — eagerly, before any `out += [v]` triplet was looked for — through
`Parser::type_element_size`, a type-alone re-derivation that summed a struct's fields and
descended into a field of the struct's own type without end.  `fn id<T>(v: T) -> T? { v }` at
`struct Node { value: integer, next: reference<Node>? }` was a bare SIGSEGV on both backends
(and under `introspect`); the release refused it earlier at the layout, and the loft#1316 layout
fix made the crash reachable.  The element id now comes from `Data::vector_element_type` — the
one home the concrete `+=` append asks — and the stride from `Stores::size`, as the concrete
`OpPreAllocVector` is sized; `type_element_size` is deleted.  Behind the crash sat the same
disagreement one level down: the rewritten WRITE (`primitive_setter_call`) keyed its width on
the alias def's `forced_size`, which `type_elm` never carries for a monomorph's concrete type,
so every narrow element of a generic `vector<T>` was written eight bytes wide — `200 0` for two
`u8`s at 2b992851, `29768 32767` for `-3000, 3000` as `i16` — and with the stride corrected that
became an eight-byte write into a two-byte slot; the in-body READ (`wrap_vector_get_val`) took
eight bytes back.  Both now go through the concrete build sites' own home,
`vectors::narrow_elm_write` / `narrow_elm_read` (the `NarrowIntKind` derivation
`Parser::narrow_elm_set` carried, now a free function it wraps).  Guard `1378` (write and
in-body read at `u8`, `i16`, `i32`; the self-referential, mutual and nested cells; one generic
per integer width because of loft#1383, filed: two widths collide into one monomorph).  Named
residual: `par_elem_size` (collections.rs) and `data::element_stack_size` are two more
derivations of a vector element's stride, unwalked.

### A value-`if` is not a coalesce, and an `else if` chain converts at its arms (2026-09-06, `@FR-E-Uncomp-NN` walk, loft#1379 / loft#1380)

`Parser::range_guard_inside_discharge` (@PLN152) matched the bare-variable `??` lowering — a
plain `Value::If` — by node shape, so every author's value-`if` stored into a narrow slot was
"discharged": the then arm wrapped in a `dn4cast` (null in a `u8`; the `limit(…)` default
lost), the condition's first operand range-cast (`(k as u8?) == 1000`), and the narrowing
refusal skipped.  `Parser::bare_variable_discharge` now asks `coalesce_not_null` to rebuild the
condition for the then arm's variable and accepts only an exact match; `null_discharge_subject`
keeps its looser `If` arm for the left-hand side, where no author's `if` can stand.
`Parser::parse_if` became a wrapper over `parse_if_expecting(code, expected)`: an `else if`
chain's then block is parsed with the enclosing then arm's type, so `parse_block`'s tail
conversion covers it (literal-fit exemption included); the three `context == "else"`
carve-outs there — the loft#1350 tuple boxing, the sibling-variant carve-out, the
loft#978/#1103 honest deps — read `arm_of_sibling` (`"else"`, or `"if"` with a known expected
type).  Guards `1379`/`1379b`/`1380`/`1380b`; `optional` audit row 714/355.  Filed loft#1381
(`--native`: a statement `if` with a value-bearing else arm, E0308).

### An element-level write through a group's vector member acts on the group (2026-09-05, `@FR-Col-Group` walk, D-col-1 opened)

`w.es[i] = e` copied into the element record IN PLACE, `w.es[i] = null` cleared its payload,
`w.es.remove(i)` unlinked the vector slot — none reached `Stores::record_finish` (which only
adds) or the unlink loop the keyed removal carries, so every keyed sibling kept the record
under its old key: `by_k[11]` null with `len(by_k)` still 2, a nulled record counted, a
removed key findable and re-added twice.  Silent, both backends, nested too.  Now
`Parser::group_elem_write` (`src/parser/collections.rs`) binds the element once
(`hoist_index_arg` keeps the index single-evaluation), emits `Parser::group_sibling_unlinks`
— the loop `keyed_group_remove` and `loop_group_remove` each spelled, now the one home — then
the write, then for a replace `OpLinkRecord` → `Stores::link_record_siblings`, the sibling
half of `record_finish` factored out (the primary already holds the record).  The temporary
is typed as the element place resolves, deps included: without them the native emitter reads
the bind as owning and deep-copies (`@FR-B-Copy` / `@FR-O-NoDiverge`).  `holder_type` reads
the `OpGetField` type annotation and resolves a vector-element base, so a nested group
(`w.rooms[0].items`, and under `vector<R?>`) is found.  `v.remove(i)` now types `boolean`
(its op always did).  Rule text extended with the LEAVING clause and `(Col-Group-Dup)`;
`(D-col-1)` opened for a group with two plain vectors (loft#1375).  Guard:
`a-group-element-written-through-the-vector-member-reaches-every-member.loft`, falsified at
2b992851 on both backends.  `index/target_surface.json` regenerated (one builtin more).

### Scratch hygiene: a native compile sweeps dead-process artefacts, and the scratch families each have a pruning rule (2026-09-05)

One box held 434 GB of loft scratch under `~/.cache/tmp` (16,326 `loft_native_bin_<pid>`
binaries from runs killed from outside, 364 GB of `make falsify` control builds, 170 GB of
agent-session scratch) and `make ci` died in the native corpus with `FAIL unknown-mode`.
Now: `platform::native_compile_space_ok` sweeps dead-process artefacts on EVERY compile
(`reclaim_dead_native_scratch`), keeping the aged fallback for the low-space path so the test
runner's per-file cache survives; `scripts/sweep_scratch.sh` carries one rule per family and
`make ci` runs it on its own scratch (was a seven-day `find`); `make sweep-scratch` /
`make sweep-target` are the by-hand sweeps; `scripts/falsify.sh` keeps `LOFT_FALSIFY_KEEP`
(4) controls.  TESTING.md § Scratch hygiene is the table.  Guard:
`tests/native_scratch_hygiene.rs`.

### A vector local bound from a value branch copies at the parser's selector, and a vector parameter rebinds locally (2026-09-05, loft#1370, D-own-35 / D-call-14)

The vector twin of B7v's record fix, closed at the vector copy's own home (QUALITY.md B7w):

- **`Parser::sink_vec_bind_into_arms`** — a vector local bound from a value branch (`if`,
  `else if`, `match`, `??`) is written out per arm at `classify_vec_bind`, so each arm gets the
  lowering a single bind of its tail has: a whole variable and an owned projection copy, a `??`
  hoist is judged by what it was bound from, a call's buffer / a literal / an index read keep the
  value they have.  A copy inside an arm always mints (`vec_copy_needs_db`: the local carries the
  join's deps there, so `@FR-O-Proxy` cannot say what it holds); a block that yields its own
  buffer is bound whole; a first bind through a wrapper block is declared at the statement by a
  null `Set` the post-parse scan elides on a reassignment; a promoted return buffer
  (`is_hidden_param`) keeps the value form; and a returned local the rewrite sank stays
  `Bind` in the return-promotion ladder (`Parser::branch_sunk_vectors` carries the
  bound-to-a-branch fact the removed `Set(v, If)` used to show).  Both backends; every
  spelling and element kind.
- **A vector parameter's first rebind from a variable mints a store of its own**
  (`@FR-F-ParamRebind`); it refilled the caller's store in place, statement form included.
- **The Tier-0 elision asks that its destination be a LOCAL** (`v_is_local`,
  `src/use_analysis.rs`): a parameter is defined at entry, and rewriting its reads onto the
  source answered the source on a loop's first turn, ahead of the rebind.

Guards: `a-vector-local-bound-from-a-value-branch-copies-the-chosen-arm.loft`,
`a-vector-parameter-reassigned-from-a-variable-rebinds-locally.loft`, both falsified at
faa38979.  Corpus IR census: 20 of 1260 files moved, all green on both backends.

### The owner witness survives the cache, and the caller side of a nullable bind copies (2026-09-05, `@FR-O-Witness` walk, D-own-34)

Four store-lifetime defects, one shape — a nullable heap local not treated as the heap local it
is — found by the caller-side and startup-cache matrices of QUALITY.md B7v, both backends:

- **`owner_witness` now survives the startup cache.**  It was maintained in the IR and restored
  by no snapshot field, so a warm program-cache run served the pre-witness copy arm and wrote a
  copy into the record a witnessed local was viewing (loft#1336's sharp cell read `b == 7` warm,
  `b == 2` cold).  `__own_<name>` is the tenth stored `Variable` field (`VAR_OWNER_WITNESS`),
  written and read through the JSON codec, the store codec, the schema source `ir.loft` and the
  baked layout constants; `CACHE_FORMAT_VERSION` → 5 so a v4 bundle is not read.
- **A nullable local bound from a call that answers a BORROW of its argument copies it**, like
  its dense twin.  `nullable_join_first_bind` admits a single-witness `Borrowed` (not only a
  `Join`); the reassignment strip and the var-copy strip read `base()`.  A two-source return
  keeps the plain adopt (loft#1368).
- **A `-> S?` callee no longer frees a PARAMETER on its null path** (F-ParamHeap: the caller owns
  it; a rebound parameter keeps its entry-stash release).
- **A record reassigned from a value branch is lowered to the statement form** (`if c { x = a }
  else { x = b }`) so each arm copies; the vector/keyed twin is loft#1370.

Guards: `a-nullable-local-bound-from-a-borrow-returning-call-copies-it.loft`,
`a-null-answer-does-not-free-the-argument-the-other-arm-hands-up.loft`,
`a-record-reassigned-from-a-value-branch-copies-the-chosen-arm.loft`,
`tests/arc_e_program_cache.rs::a_warm_run_keeps_the_owner_witness`.


### The per-path fact reaches the nullable spellings (2026-09-05, `@FR-O-Complete` walk, D-own-33)

Four defects, one shape — a nullable local not treated as the heap local it is — found by
the statement-form matrix of QUALITY.md B7u (a local assigned on two paths with different
ownership, every cell called twice) on both backends:

- **A literal's work-ref adopted inside a loop body** (`y: S? = S { … }`, a struct-enum
  literal, an `if`/`match`-arm literal) had two owners: the binding's per-iteration free
  returned the store, the function-scoped `__ref_p2_N` kept the number, and the next pass's
  `OpDatabase` reused it in place after another record had taken it.  loft#1317's pairing
  declined the inner-scoped case.  The literal buffer now takes the pairing a CALL buffer has
  (@P378(a) `witness_buffer`, extended to a list so a two-arm literal branch declines against
  both), reached through `scopes::adopted_work_refs`.  A move at the adopt was tried and
  reverted (the owner witness, loft#1200's flag and the `??` lift all read the buffer as the
  owner; four leaks).
- **`scopes::needs_pre_init` peels `Optional`**: a nullable local first assigned inside a
  branch gets its `Set(x, null)` and one first assigned inside a loop body is hoisted, as the
  dense twin always was.  A nullable VECTOR's null-init is the sentinel
  (`state/codegen.rs::gen_set_first_nullable_collection_null`); a keyed one keeps the
  allocating init its `OpReplaceKeyed` assignment needs.
- **`Parser::join_arms` reaches a value block's tail**, so `join_source_frees` licenses the
  free-source bit for every `match` arm as it did for an `if` chain; a keyed local bound
  through a `match` freed nothing before.
- `scripts/falsify.sh` passes `LOFT_POISON` / `LOFT_STRICT_STORES` through when the caller
  arms them — the loop-literal defect is observable only under the arena poison, and the
  guard's `@falsified-at` line says so.

Filed, owned by @PLN153 phase 3: loft#1367 (two spellings of `S?` in one local).

### A generic's keyed member reaches the concrete key-field refusal (2026-09-05)

A consequence of the entry below: a generic's tuple or keyed member now takes the concrete
twin's lowering, so an instance whose hash is keyed on the field its own code writes meets
the `Cannot write to key field` refusal the concrete function always met.  A program that
compiled only because the instance skipped that path is refused now, with the same message.
### @PLN153 phase 2: the null-flow flag is read through one face per rule, and the fold changed nothing (2026-09-05)

Ten sites read `nullflow_enabled()`, and the plan expected them to split by what they do —
propagate, gate, warn.  They split by RULE instead: three decide `N-Store`'s warn/error split,
three `N-Prop`, three `N-Domain`, one `N-Cast`, and a site reading the bare flag said nothing
about which, so the sites drifted — both `N-Store` branches re-spelled the narrow-width test
by hand.  `keys.rs` gains the flag's four faces (`nprop_enabled`, `ndomain_enabled`,
`ncast_asserts`, `nstore_softens(narrow)`), each cited `@FR-N-*` and each the flag and nothing
more, and `Parser::nstore_narrow` is the one home for `byte_width < 8`.  `heap_target ||`
stays per-site, being the site's fact and not the rule's.

Verified the way a refactor that claims to change nothing has to be:
`scripts/introspect_diff.sh` (the B7r/B7s method as a script — `introspect` of two compilers
over the corpus, stderr included) reads `IDENTICAL 1268/1268` under the default and `IDENTICAL 1268/1268` under
`LOFT_NO_NULLFLOW=1`.  What phase 3 inherits in numbers: the refusal itself still has five
homes, and one of them (`change_var_type` for a declared local) is an ERROR where this split
would warn.

### `loft introspect` always parses — a warm cache hit rendered every variable as 65535 (2026-09-05)

Found by the byte-identity run above: a binary living outside `target/` (cached) read as a
DIFFERENT compiler from the same source built inside it (never cached), on one file — and
the difference was the cache, not the code.  A warm bundle carries no variable table, so the
dump printed `n#index(65535)` and `-` in the slot table's number and span columns; two runs
of the released 2026.8.0 on `examples/fizzbuzz.loft` emit differently.  `introspect` reports
what the parser emits, so it now parses whatever the cache holds (`main.rs`, beside the
script and sandbox exemptions).  Guard: `arc_e_program_cache::introspect_parses_fresh_under_a_warm_program_cache`,
red with the gate inverted.

### A generic's instance returns what its concrete twin returns (2026-09-05)

`@FR-F-Ret` says a returned whole heap value is fresh.  A monomorph kept the RECORD lowering
its template parsed `T` as, because the declaration defers a generic's return promotion to
instantiation and nothing there received it: a `-> T { x }` bound to a struct, vector or
keyed collection handed the ARGUMENT up (a write through the result wrote the argument, both
backends), a `-> (T, integer)` stayed a stack tuple aliasing the argument, a vector `s = x`
aliased and the frame freed the caller's vector.  Four cures at instantiation, each what the
concrete twin does: return deps from the oracle where every return leaf is the parameter
itself; the deferred tuple boxed by `tuple_return_rewrite` (shared by the pass-1 prediction
and the pass-2 signature) with `promote_monomorph_tuple_return` rewriting the body; vector
binds and a borrowed vector return copied by `promote_monomorph_vector_return`; and, in the
tuple return leg every declaration uses, a keyed member copied instead of written as a 4-byte
header, a nullable record member tag-written instead of landing on the discriminant, and a
nullable vector member's `null` written as the absent id instead of an empty vector — the
last two pre-existing on concrete code.  Guard: 52 cells with concrete twins as the oracle,
falsified at `babf9e64` on both backends; five corpus files' IR moved, all green under
`LOFT_STRICT_STORES`.

### The displacement free reads one predicate on both backends (2026-09-05)

`@FR-O-NoDiverge` says both backends translate the same `deps` facts.  The one store-lifetime
decision still made in each code generator — freeing the store a heap local displaces at a
reassignment — was two predicates, `state/codegen.rs`'s `owned_ref` and
`generation/dispatch.rs`'s `owned_ref_reassign`, kept "the interpreter's verbatim" by hand and
drifted four times (the keyed kinds, the vector destination, the override veto, the detach),
each found by a leak or an abort on one backend alone; the one-argument borrow test they share
had a third spelling in the scope-exit sweep.  The fact-reading half now has one home,
`Function::owns_displaced_store` with `Function::borrows_one_argument` beneath it; both
backends and the sweep read it, and only what is genuinely per backend stays at the site.
`introspect` output (IR, bytecode, Rust) is byte-identical across all 1247 corpus files.
`Function::has_borrow_arm`'s doc claimed both backends' displacement frees read it; neither
does — its one reader is the fn-ref delivery strip, and the dep that strip keeps is what the
frees read — so the receipt now says so.

### The ownership oracle joins a minted variable's other definitions, and its shadow reads the one base translation (2026-09-05)

`@FR-O-Oracle`'s two derivations — `use_analysis::ownership_of` and the @PLN94 flow-sensitive
shadow in `ownership_cfg` — disagreed in 14 places over the 1247-file corpus (Check A's zero
had been measured on nine files).  Two shapes were the oracle's: its first arm called any
`OpDatabase`-minted variable `Owned` regardless of its other definitions, so a local minted once
and rebound by a call that may return its argument, or by a capture read inside a closure, got
the verdict that licenses a free (masked at run time by the distinctness guard and by the
loft#1331 detach).  The arm now joins the mint with the variable's definitions — a bare-`Var`
right-hand side is a copy and so `Owned`, a call or projection is what the oracle says — and a
minted variable with no `Set` (the retbuf) stays `Owned`.  One shape was the shadow's: its
private copy of the callee-to-caller base translation lacked loft#1318's fixes.  The
translation now has one home, `use_analysis::structural_arg_base`, read by both.  Check A
14 → 0; `introspect` output (IR, bytecode, Rust) byte-identical on all 1247 files.  The A1b
gate asserts the runtime failure of the known-wrong plan and Check A clean on both plans;
`LOFT_OWN_INJECT_FACT_OWNED=<var>` is Check A's injected true positive.  `LOFT_OWN_ORACLE=own`
prints a `RETSUM` line per heap-returning function comparing `return_adopts_fresh_store` with
the oracle's return class: 277 of 1244 differ, 32 in the risky direction, all generic
monomorphs or closures whose declared return dep reads "fresh" for a borrowed return — no
free decider reads that proxy without a delivery buffer, so recorded, not fixed.

### The never-free veto is stated over the free NOTION, and its one admissible free is named (2026-09-05)

`@FR-O-Override`'s contract read *"no `OpFreeRef` is ever emitted for this binding"* — one
spelling of five, and both backends intercept the never-free flag downstream for only two
(`OpFreeRef`, `OpFreeRefTag`, a bare variable).  Measured over the 1247-file corpus with a new
oracle check (Check D under `LOFT_OWN_ORACLE=check`: every free op whose first argument is a
never-free binding, RED in a live spelling, NOTE in a dropped one): 217 function–binding pairs freed a
never-free binding by `OpFreeText`, all of them the `??` text subject or the text return stage
the staging pass itself frees after the value is copied out, and nothing else.  The RULE was
extended to say so — every ownership-derived free in any spelling is forbidden; the release
the marking pass places on a consumption fact is admissible — and the shape got a name,
`Function::is_staged_text_temp`, which the ncc-orphan pass and Check D both read.
`LOFT_OWN_INJECT_FREE_SKIPFREE=<var>` is the check's true-positive control.

"Which ops free their first argument?" was a hand-spelled name list in nine places, no two
agreeing (three blind to `OpFreeRefOrHandUp`, five to `OpFreeRefTag`).  `OpSets` now carries
`frees` / `unconditional_ref_frees` / `conditional_ref_frees` / `text_free` and all nine read
it; IR and emitted Rust byte-identical before and after on every corpus file whose IR carries
the conditional spelling.  A local that mixed ownership took BOTH the loft#1200 displacement
flag and the loft#1336 owner witness, the witness's never-free mark dropping the flag's free
at codegen — the witness now runs first, so a witnessed local carries one release mechanism
and no dead free (172 IR lines gone from the 1200 guard).  Check B consults the veto (a
dropped free is not an over-free) and its "0 FP" claim is restated as the measurement: ten
hits over 1247 files, one shape, each clean under `LOFT_STRICT_STORES`.
`LOFT_SKIPFREE_TRACE=*` traces every never-free mark with its writer.

### A nullable local holding a projection view does not free the store it displaces (2026-09-05)

The D-own-16 `borrows_one_argument` residual reads a nullable heap local's single-ARGUMENT
dep as ownership and frees the store it displaces at a reassignment — sound for a WHOLE-value
argument borrow (`d: S? = p`, free-protected on the borrow path), a silent over-free for a
PROJECTION.  `d: In? = q.inner` aliases q's NESTED store (no free-protection), so the
reassignment released the caller's record; a view of a local's field (`o.inner`) or a vector
element (`vs[i]`) failed the same way, the dep naming its base.  Silent-wrong: the freed store
read correct until a later allocation reused its slot, then returned the filler's value (`777`
for `71`, both backends); the vector-element shape crashed out of bounds under `LOFT_POISON`.

A view owns no store (`@FR-O-Owner`), so the empty/argument-dep proxy is wrong for a
view-holder and `@FR-O-Override` vetoes it.  `scopes::nullable_view_locals` names the nullable
heap locals that hold a projection view (the oracle calls it `Borrowed` and it is not a bare
`Var` — a whole-value bind COPIES) and marks them never-free before the scan; the three
free-site twins (`state/codegen.rs`, `scopes.rs` scope-exit, `generation/dispatch.rs`) already
consult `is_skip_free`.  Excluded, and kept on their own machinery: a solely-owned minting
call (the loft#1200 runtime flag) and a view+mint mix (the loft#1336 owner witness).  Found on
the `@FR-O-Latest` rule-led walk (QUALITY.md B7p, `formal/ownership-history.md` D-own-30),
guarded by `tests/scripts/a-nullable-view-local-does-not-free-what-it-displaces.loft`,
falsified at `51646648` on both backends.

### The @FR-F-Ret join: a monomorph mint that reused a template name, and a collapsed member that owned what it viewed (2026-09-05)

Picking loft2's `64437246` (a generic's instance returns what its concrete twin returns) onto
this branch turned its own 52-cell guard red on both backends — the guard passes on the
branch it was written on, and every guard of this branch passed too, so the defects were in
the PRODUCT of the two streams, the shape DEBUG.md § the seam names.  Two mechanisms, both on
this branch's side:

**A work-ref minted in the monomorph frame re-claimed a template name.**  `Function::copy`
started every work counter at 0, and `work_refs` reuses an existing `__ref_N` by name when
the counter is behind it — the pass-1/pass-2 same-site rule.  On this branch the TEMPLATE of
`fn g<T>(x: T) -> (T, integer) { s = x; t = (s, 7); return t; }` already mints `__ref_1` for
its tuple-member copy (loft#1361); the picked `promote_monomorph_tuple_return` then asked for
a work-ref in the instance and was handed the same `v3 __ref_1`, retyping the member's backing
into the boxed return record while `t` still depended on it.  The caller read a zeroed record.
Carrying the template's stored counter was measured NOT to be enough (`LOFT_TRACE_WORKREF`
still showed the reuse; the stored number is not trustworthy at every instantiation), so
`copy` now DERIVES all eight counters from the names the cloned table carries —
`<prefix><digits>` exactly, so `__ref_` cannot read `__ref_p2_1`.  A monomorph-time mint is
always a new name; the reuse arm keeps the case it was written for.

**A member unwrapped to a local owned it.**  For a collection-bound `T`, @PLN153 phase 1's
`collapse_parametric_tuple_member_copies` unwraps the template's record copy to the bare local
and stripped the element's dep on the backing — which made the instance's `t` read as OWNING
its member, so the IR freed `t.0` at the callee's exit: the caller's hash.  A one-call probe
read the freed store back intact, even under `LOFT_STRICT_STORES`; the guard's second call is
what observed it.  The dep now moves to the local the member is a view of
(`Variables::retarget_tuple_member_deps`, `(B-View)`), and the member is neither freed nor
copied — the aliasing D-tup-9's collection half already records.

Verified: the picked guard 52/52 on both backends, every guard of this branch, the template
matrix, the parser and scopes subjects.  `LOFT_VAR_TABLE` and `LOFT_TRACE_WORKREF` are what
found both; the guard alone said "0".

### @PLN153 phase 1: every N rule has one home, and a measured pair behind its citation (2026-09-05)

Eighteen `@FR-N-*` rules in `types.md`, three of them cited when the plan opened (7 sites)
and fifteen with no code representation at all — a rule with zero sites cannot be walked, only
located.  This phase located them, and the census is the plan README's § Phase 1 table: one
predicate or emitter per rule, with the line it lives on.  Four rules turned out to share a
home with another and are cited there (`N-Idem` with `N-Opt` at `Type::optional`, `N-Parse`
folded into `N-Cast` at the assertion cast, `N-Domain` beside `N-Div` at the `/`/`%` typing,
the checked cast beside `N-Cast` at its DN4 lowering).

The order was the plan's: the PAIR first, the citation second.  `153-n-rules-hold.loft` is
the HOLD half — every rule's own example asserting the value `types.md` promises, green on
both backends — and ten `153-n-*-refused*.loft` files are the REFUSE halves, each pinning the
compiler's actual diagnostic as an `@EXPECT_ERROR` / `@EXPECT_WARNING` substring.  Two
negations were measured rather than assumed: `e ?? d` with a `τ?` default is itself `τ?`
(exactly `Γ ⊢ d ⇐ τ`), and a `match` on `τ?` with NO null arm is silent — `x` binds `τ?` and
propagates — so that is a phase-3 matrix cell, not a deviation.  `rule_tags.py check` reads
18/18 N rules cited.

What the pairs measured for the phases after this: `N-Store`'s refusal already has at least
five homes and a SITE-DEPENDENT severity — a declared local is an ERROR from
`change_var_type` (*"cannot change type from integer to integer?"*), an element is a WARNING
from the `mod.rs` split, a constant index into a literal is silent because it is provably in
range, and the narrow refusal spells its alias `integer(0, 255)` rather than `u8` — which is
phase 3's starting census in numbers.  And the "ten `nullflow_enabled()` sites" split by RULE
(three N-Store, two N-Prop, two N-Domain, one N-Cast, two the min/max/clamp shape), so phase
2's fold is one predicate per rule, not one for all ten.

### @PLN153 phase 0: `τ??` is not constructible, and a forward alias behind a `?` resolves (2026-09-05)

`types.md (N-Idem)` says `τ?? ≡ τ?`.  `Type::optional` is the idempotent former, the type
parser builds every `?` through it, and the plan's first question was whether any of the
thirteen direct `Type::Optional(Box::new(…))` constructions could nest one anyway.  Answered by
measuring rather than reading: **three could**, and they are one shape — a rewrite that
substitutes a resolved target under a wrapper and re-wraps with a bare `Optional`
(`typedef.rs::set_attr_type_keeping_optional`, `Data::rewrite_type_opt`,
`Function::rewrite_unknown`).  With `type Maybe = integer?` declared AFTER `struct S { f:
Maybe? }`, the field was `integer??` and its first write was refused with that spelling in the
message.  All three re-wrap through the former now, each cited `@FR-N-Idem`.

The instrument is `src/null_census.rs` — an observer beside `ownership_cfg::oracle` in
`scopes::check`, gated on `LOFT_NULL_CENSUS` because `[profile.dev.package.loft]` compiles a
`debug_assert` OUT of the library — and `scripts/null_census_sweep.sh` over the corpus:
`TOTAL nested=0 files=1258 failed=0`, with a hand-built `Optional(Optional)` reading 1 in the
module's own tests so the zero is a measurement.  ⚠ A refused program prints the STDLIB's census
lines and nothing of its own; the sweep counts it as `failed`, never as a 0, and the per-probe
reads that "passed" before the fix were exactly that vacuity.

The guard that pinned the route then found three resolution defects on it, each one fact held
in two places.  `Type::is_unknown` sees through `Vector` and nothing else, so pass 1's operator
deferral judged `Optional(Unknown(alias))` settled and refused *"No matching operator '==' on
'unknown?'"* for a program pass 2 resolves — cured with `Type::has_unknown`, a walker over the
`Type` keystone, at the deferral only (`is_unknown` has 91 callers and keeps its settledness
meaning).  An annotated local `x: Maybe? = 5` lost its `?` when the assignment overwrote the
`Optional(Unknown)` placeholder (`LOFT_LOG=type_timeline:x` showed the three writes) — cured
with the mirror of `change_var_type`'s loft#1073 arm: a declared type carrying a stub is not a
baseline.  And a forward nullable-REFERENCE alias field lints `redundant-null-check` while its
storage and values are right, because the lint reads the stored spelling where the
declared-before case carries the source one — the `(L-Null-Which)` two-spelling question, so it
is phase 5's first cell, not a spot fix.  Guard
`tests/scripts/153-a-forward-alias-field-keeps-one-optional.loft` (c1–c4, both backends).

Phase 0's second question — is `(N-Intro)` the only null-direction edge in the code's `⤳`? —
is answered by `Parser::convert` itself: it carries BOTH edges, and the implicit `τ? ⤳ τ`
unwrap the rules say does not exist stands because `convert` services comparisons too, so the
`(N-Store)` teeth live at the store sites instead.  That is the scattered shape phase 3 folds,
stated by the compiler; recorded in the plan README with `implicit_checked_narrow`'s
null-producing `Integer ⤳ u8?` beside it as a cell of its own.

### A tuple member typed by a generic's type variable is copied for the type each instantiation bound (2026-09-05)

loft#1365 / D-tup-9.  `(T-Cons)` copies a heap member into a tuple literal and leaves a scalar
one alone, and neither rule has a clause for generics — a monomorph is an ordinary program, so
`(s, 1)` must behave the same whether `s` is written `Ctr` or reached through a `T` bound to
`Ctr`.  The template cannot decide it: a type variable is spelled `Type::Reference` to its
placeholder and so looks exactly like a record, while what the member IS exists only per
instantiation.  Both one-sided answers were measured and both are wrong — DECLINING the copy
left a struct-bound `T` aliasing (`s.bump()` then read through `t.0` answered 1 where the
concrete twin answered 0), and emitting it UNCONDITIONALLY allocated a record with the type
variable's own row, the layout escape loft#1070's guard refuses, an ICE on both backends.

So the template emits the record copy and `Parser::collapse_parametric_tuple_member_copies`
removes it again in each monomorph whose bound type it does not fit.  The test is the block's
own contents against that type (`tuple_member_copy_shape_fits`), never a "this was a type
variable" flag: a generic body also builds tuples from CONCRETE members, whose copies are
right and must not be touched.  Unwrapping the value is only half of undoing the guess — the
template also gave the tuple ELEMENT the backing's dep, and an element naming a backing whose
copy is gone is owned by a variable nothing fills, so the store the member holds is freed by
nobody.  The dep goes with the copy, through `Variables::make_tuple_members_independent` —
`make_independent` one level down, because a tuple carries no deps of its own and `deps_mut`
on a `Type::Tuple` is `None`, so the existing verb would have quietly done nothing.

Guard `tests/scripts/1365-…` — nine record cells each against a CONCRETE twin rather than a
literal, plus four scalar cells as the control, since keeping those green is exactly what the
declining version bought by making the record cells wrong.  `matrix_axes.py` is what found the
last two: the loop and `if`-arm cells were wrong too, and the container-kind axis it reported
missing turned out to be an ICE.  A COLLECTION binding still aliases and D-tup-9 stays open for
it, with the cause now measured rather than guessed: building the collection copy at monomorph
time works on all four cells and leaks one store in the CALLER, because a generic's monomorph
returns a STACK tuple where the concrete twin returns the boxed `__tuple<…>` record — so the
caller binds the member by bare projection, with no backing to adopt the callee's store.

### A copy of a tuple releases a droppable member once, and heap.md opens its first deviation (2026-09-05)

The seam between the two 2026-09-05 streams, found by probing it rather than by either
side's guards: loft#1361 made a whole-tuple bind COPY its heap member, and loft#1362 made a
whole-value copy MOVE the drop — but `scopes::drop_bearing_source` had no answer for a copy
whose SOURCE is a tuple member, so `t = (s, 5); u = t` built two records that each ran the
author's `OpDrop`.  Silent, both backends, identical values.  Neither guard could see it:
the tuple guard carries no `OpDrop` and the drop guard carries no tuple.

Measured across four builds rather than reasoned about.  2026.8.0 releases once everywhere;
`tuxedo-1361-tuple-copy` alone was worst (four shapes doubled, one tripled);
`tuxedo-quality-2026-09` alone is clean, because without the member copy one record wears
both names; the JOIN heals three of the four.  So the family is loft#1361's regression that
the H-Drop work absorbed most of, not something the picked range introduced.

`drop_bearing_source` gained a `Value::TupleGet` arm resolving the member to its backing
work-ref, and the `__ref` buffer arm of `drop_handoff_node` now reads its source through
that one home instead of matching a bare `Var`.  Reading the pairing is the part with a
trap in it: the dep lists are UNIONED across a tuple's heap members, so every heap element
carries the same list and the k-th dep backs the k-th HEAP member — with one droppable
member `first()` is right by accident and with two it names the wrong work-ref, which is
the cell the guard pins.  The helper declines unless the dep count equals the heap-member
count, and declines outright for a PARAMETER, whose deps are the caller's in another dep
space; declining costs the hand-off, where guessing would suppress an unrelated local's
release.

Guard `tests/scripts/a-copy-of-a-tuple-with-a-droppable-member-releases-once.loft` (9 cells
incl. a two-resource control, falsified at `7aaab369` on both channels).  Three shapes stay
open as `heap.md` D-heap-1 — a nested tuple, a member declared nullable, and a copy off a
tuple parameter — each because the element type carries no backing dep to read, or the deps
are not this frame's; none reproduces on `main`, so they are branch-internal by the bug
policy.  `heap.md` moves from `OPEN: 0` to `OPEN: 1`, which is what that line is for.

### A reassignment releases the droppable it displaces, and the hand-off fact follows the assignment (2026-09-05)

loft#1362.  `scopes::displaced_drop` — at a `Set` of an owned droppable-typed local already
in scope, and at a statement-level `OpDatabase(v, tp)` that rebuilds one in place — copies
the record into a null-safe snapshot temp (`__disp_N`) at the head of the scan's prefix
(before the transition free and the right-hand side), runs the type's hook on the temp
after the Set, frees it and resets it to the true sentinel (a freed reference still reads
`rec != 0`, and the sweep's second visit ran the hook on the recycled slot until it did).
The owner predicate is the transition free's, and inside a loop the outer latest-assignment
fact is trusted only when this assignment owns too (`ref_rhs_ownership`), so a view assigned
in a body is never copied and released as if owned.  `owned_refs` is kept through `base()`
so a nullable local has an entry.  `drop_transferred` is now flow-sensitive: re-armed by
every statement's hand-offs in scan order (`drop_handoff_node`, the body the up-front
collector runs too) and retired by an unconditional reassignment — which is what let
`copy_moves_drop_from` drop its per-variable single-assignment guards (`t = s; s = …`
releases through the copy only; `t = s; t = …` releases what it displaces).
`copy_hands_off` peels nested field and element reads to the root variable
(`type_owns_droppable_anywhere`): `o.s = S {…}` into the nested `o.s.h` and `v[0] = S {…}`
into an element were not hand-offs, and the literal's work-ref released the resource a
second time beside the container's cascade.  Rule: `formal/heap.md (H-Drop)` — new; the
chapter had no drop clause.  Guard `1362-a-rebind-releases-the-droppable-it-displaces.loft`
(13 cells), both backends, with both leak checks armed.

### A tuple with a heap member copies like any other value, and the eager release leg finds its checkout (2026-09-05)

loft#1361: three places kept a tuple's stack WORDS where `binding.md (B-Copy)` and
`tuples.md (T-Cons)` say copy — a heap member's word is its handle.  The whole-tuple bind
(`u = t`) is lowered at the assignment onto the literal's per-member copy
(`Parser::tuple_member_owned_copy`, which now takes a `TupleGet` source and gained the struct,
struct-enum, nullable and nested-tuple branches); the destructure's `T1.4` temp reads a
collection member through the same copy when the base is an owned tuple local; a projection
`a = t.0` of a vector member joins `classify_vec_bind`'s `CopyOwnedField` (`L-Tuple` makes the
tuple a struct).  A struct member read out stays the view `B-View` makes it, and
`ownership_of` now says so for a `TupleGet` (it fell to `Owned`, and `warn_dead_stores`
warned that a landed write was lost).  The backing for a keyed or struct member is a
function-scope `__ref_N` work ref — the temp an inline record literal mints — because a
block-scoped local was the block's own value and nothing freed it once the tuple sat in an
argument (`show((s, 5))` leaked one store per member; the keyed branch had since
loft#1230).  The native tuple emitter renders an `Insert` member as a block expression.
Guard `tests/scripts/1361-…` (15 cells, falsified on both channels); D-tup-8 opened and
closed in `tuples-history.md`; QUALITY rows 388/364/24 and 703/346/5/352.

Release mechanics found by the first `make release-gate`: `registry-validation.yml`'s
`discover` job read `scripts/revalidate_matrix.py` with no checkout (loft#1334 moved the
not-a-library set there) — it checks the repo out now; `release-gate.sh`'s run lookup passed
`--arg` to `gh --jq`, which takes a bare expression; and the `x86_64-apple-darwin` leg leaves
`macos-14` (deprecated, brownouts through October, gone November 2nd) for `macos-15`.

### A struct-enum whole-value bind copies on both backends, and a nullable struct-enum's payload field resolves (2026-09-04)

`@FR-B-Copy` walked as a lens (QUALITY.md § B7n).  Three of its nineteen sites spelled
`Type::Reference` bare where the question is *"is this a heap RECORD?"* — the interpreter's
first-bind and rebind copy arms (`state/codegen.rs`) and the parser's dep-strip
(`parser/objects.rs`) — while their siblings (the null-init arm, the call-source arm, both
native arms) read `Type::heap_def_nr`, which names a struct-enum too.  So `e: Sh = Ci {…};
c = e; e.n = 9` read 9 through `c` on `--interpret` and 1 on `--native` (D-op-1 with the
interpreter wrong), the branch-arm lift (`scopes.rs::arm_bind`) declined a struct-enum arm
because codegen had no copy for it (so the join aliased on BOTH backends), and a NULLABLE
struct-enum destination kept its source dep while both emitters copied — a copy nobody
freed.  All three read `heap_def_nr` now, and **`Data::copies_as`** is the one home for which
record pairs copy: the same def, or a variant into its own enum (`types.md (C-Var)`), which
native already admitted and the interpreter refused (`c: Sh = ci` aliased).
`parser/fields.rs`'s struct-enum field branch matched the receiver bare as well, so `e.n` on
an `e: Sh?` was "Unknown field" on a read and "cannot change type from Sh? to integer" on a
write (pass 1 re-typed the receiver); it reads through `base()` like `type_elm` beside it.
Guards: `tests/scripts/a-struct-enum-whole-value-bind-copies-like-a-struct.loft` (10 cells),
`a-nullable-struct-enum-payload-field-resolves-like-a-struct-field.loft` (5), and two cells
in `bind-copies-or-views-the-whole-boundary.loft` — all falsified at `12e58a4d` on both
backends.  The walk's 45 cells also measured 30 negative results (parameters, loop
variables, field and element destinations, `??` subjects, deep copies, rebinds, text,
sorted / index, `if`-arm and loop-body contexts, the destination-write direction) and one
family filed apart: a tuple's heap member is shared by a whole-tuple bind, a destructure and
a projection, and by the literal for a STRUCT member (loft#1361, `tuples.md` D-tup-8).

The struct-enum copy exposed a second family: a whole-value copy of a DROPPABLE released it
once per record (`h2 = h`, `t = s`, `t: S? = s`, a struct-enum arm now that it copies, and
`t = s; return t` — three times, two of them before the caller read its copy).
`scopes::copy_moves_drop_from` is the one home for the move C111 chose for containers,
extended to the plain copy: read by `collect_drop_transferred` (the parser's `Set(v,
Var(src))` and the `OpCopyRecord` into a `__ref*` buffer — a materialised branch arm or a
return buffer), by `lift_join_arm_tails` for its `__lift_N = a` (built after the collector
ran), and by the `double-move` lint, which now reports `t = s; u = s`.  The move needs a
singly-assigned source and destination (a compiler buffer is exempt: its null placeholder is
its second `Set`, and a return buffer is an argument), and a copy off a parameter suppresses
the COPY's drop instead — the caller owns.  A rebind still never releases what it displaces
(loft#1362, pre-existing, filed with cells).  Guard
`tests/scripts/a-whole-value-copy-of-a-droppable-releases-once.loft` (12 cells, two
controls); `139-drop-cascade.loft` c8 keeps its `alive,30` with the struct twin c8s beside it.

### The warm cache carries its import tables, an eager generator owns its snapshots and runs its tail, and every ignore says how it runs instead (2026-09-04)

loft#1359: the IR root (`tools/ir_schema/ir.loft` `Data`, cache format **4**) now carries
`imports` (the retained `AppliedImport`s `rebuild_indices` replays) and `use_names`; a warm
load of a multi-source program used to lose every `use lib::*` binding and the module map,
because neither is derivable from the definitions.  `DATA_ROOT_WORDS` / `BUNDLE_ROOT_WORDS`
size the root claims from the schema — `Store::claim` counts 8-byte words INCLUDING the
record header, and a claim sized from the stride alone put the last field in the next block
(the hang that cost an hour).  The JSON snapshot carries both tables too, and the guard
`multi_source_round_trip_preserves_derived_indices` is un-ignored; `g2_m6_warm_store` gains
a real multi-source warm run.  loft#1356: the native eager factory pushes every handle as a
SNAPSHOT (`coroutine_snapshot`, one store per generator, freed at exhaustion and from
`drop_stores`) instead of refusing struct/vector loop-body yields, and it now emits the
generator's TAIL — it had dropped every statement after the last yield, which leaked each
persistent heap local of an eager generator and lost a `print` after the loop.  The
`yield from` desugar binds `__yf_item` as a borrow of `__yf_sub` (the loft#481 dep), so an
enclosing `if` arm no longer frees the sub-generator's frame through it (BUG #306 on the
interpreter).  loft#1358: closed as C116 — one refusal home
(`Parser::refuse_capturing_closure_in_collection`), the canary converted to its guard.
`tests/ignored_tests.baseline` shrinks to 33 and `doc_hygiene::every_ignore_reason_says_how_it_runs`
requires each reason to name the run it rides (`--ignored` by hand, a nightly job, a platform);
the two `web/http.loft` skips were inert (the file lives in `tests-network/`, which no suite
walks) and are gone.
### Every text buffer a frame mints is released, and the release valgrind sweep runs nightly

loft#1357 — the residue of loft#1338 under `scripts/valgrind-sweep.sh`: 11 corpus files losing
one `String` per call on the interpreter, values right on both backends.  Eight shapes, each
closed where its own question is asked (calls-history D-call-12): a lambda that already holds
its one hidden `&text` buffer now accumulates / binds into a local and MOVES it into that
buffer (`parse_block`'s `do_if_acc` / `do_tret_bind`; the pass-2 gate reads
`lambda_text_buffer_var` because such a lambda never minted the `__acc`/`__tret` attribute the
gate keyed on); a returned bare text local is delivered through the buffer and freed
(`free_vars`, and `free_copied_text_sources` now frees the returned local it copied);
`text_return_orphan_risk` flags a returned text LOCAL whatever bound it (a text `Set` copies,
so the oracle's borrow-of-argument answer was true of the store and false of the `String`);
`rewrite_text_returns_into` reaches a `return` inside `Loop` / `Iter` / `Drop`;
`try_generic_instantiation` re-asks the promotion once `instantiate_nested_generics` has
retargeted the nested call (the first ask saw the template's `-> S?`); a tail that READS its
buffer is staged into `__ret_N`, moved, freed (`free_vars` and `insert_free`,
`any_text_return_buffer`); a `??` temp consumed by a SCALAR tail is freed after the scalar is
hoisted, and one consumed by an `if` CONDITION is freed after the condition is evaluated into
a boolean (the statement scan in `convert`); a `par` loop's text binding of the element is
not marked never-free (a text binding copies); a `parallel` arm frees the `__work_N` texts it
wrote, on the worker; and `run_tests` resumes a frame-yielding `main` until it finishes, as
the CLI does (it scored the first frame as the whole test and abandoned the frame's texts —
the sweep's one remaining red file).  The text ledger (`LOFT_TEXT_TIMELINE`) is one `Mutex`
for the process and reports under `--tests`.  Guard `tests/scripts/1357-…loft` + `tests/text_buffer_ledger.rs`
(99 orphans → 0 by hand; inert on the six `make falsify` channels).  `miri.yml` gains the
nightly `valgrind` job (the sweep on the release binary, both backends) and
`release-gate-sweeps` (the two ignored tests whose reason names the release gate).

### A lambda's lifetime tuple is boxed like a named function's, a tuple result joins a tuple literal, and a nullable record from a fn-ref copies on the interpreter (2026-09-04)

loft#1349: a lambda declared `-> (vector<integer>, text)` stored its annotation verbatim,
so its tail was handed up as the bare tuple its arms yield and the vector element aliased the
argument's field; both lambda forms now take `Parser::boxed_tuple_return`, the rule the named
declaration already applied (`has_lifetime_concern` → `tuple_def`), at the same point on both
passes (closures-history D-clo-21).  loft#1350: an `else` arm yielding a stack tuple whose
element types spell the expected synthetic `__tuple<…>` name is boxed into its own work-ref
in `block_result` and retyped as the record, so `parse_if` joins two records; a different
shape keeps the refusal (tuples-history D-tup-7).  loft#1353: `use_analysis::callee_of`
admits the nullable fn-ref spelling when the return borrows a visible ARGUMENT — the reassign
copy brackets every ref argument — and still declines a return that borrows the closure
(`fnref_return_borrows_closure`), the shape the 1114 guard pins (closures-history D-clo-22).
loft#1351 is the sibling checkout's fix, carried here so the harness measures what a real
build emits: `tests/native.rs` calls `namespace_colliding_native_fns` after its parse, as
every `--native` path does; its guard lives on that branch.  Guards `1349`/`1350` (one
file), `1353`.  Filed apart: loft#1354 (a tuple local yielded by an arm is moved on
`--native`).

### A `--lib` directory that lost to a project-local `lib/` says so (2026-09-04)

loft#1352.  `use` resolution is first-wins, and a project-local `lib/`, a declared dependency
and the script's own directory are probed before `--lib`, so the flag never reaches a name
one of those provides — three measurements of a patched library copy passed through `--lib
<copy>` from the repository root scored the unmodified tree, in silence.  The precedence is
REPORTED, not moved: `lib-flag-outranked` (advice) names the file that answered and the file
the flag provides, once per id, quiet without a flag, when the winner lies inside the flag's
directory, and under `LOFT_NO_LIB_OUTRANKED`.  Whether the precedence itself should change is
an owner call and is left where it is.  `tests/lib_flag_outranked.rs` pins the reported /
honoured pair with a copy that cannot parse behind the flag, so a clean run is positive
evidence.

### Four returns settle against the rules: a boolean match is exhaustive, a nullable vector return copies its projection, a nullable record reassigned from a call copies on the interpreter, and a nullable lambda keeps its `?` (2026-09-04)

loft#1343: a `match` on a boolean spelling `true` and `false` is exhaustive — the scalar match
parser makes the last arm the fallback instead of carrying a typed-null fall-through whose
leaf the join read as nullable (`formal/matching.md` `(M-Bool)`, D-match-1).  loft#1345: a
`-> vector<T>?` function's branching tail reaches the vector materialiser, which now has a
PROJECTION leaf (`is_projection_op` → clear + append into the buffer), so `q.items` under a
null arm is copied instead of handed up as the view (calls-history D-call-10).  loft#1346: the
interpreter's set lowering skipped its borrowed-view copy when "the call reads the
destination", asked through the flag that routes the post-free — also raised for a nullable
local — so every nullable record reassigned from a borrowed-view call, the `if` join's lifted
arm temp included, kept the raw pointer while native copied; it asks `rhs_reads_v` now, and
`OpCopyRefOrNull` writes `DbRef::NULL` for a null result instead of `Stores::null()`, which
allocates (ownership-history D-own-29).  loft#1347: the borrow-copy and forwarder vector
delivery legs re-set the returned type without the declared `?`, so a lambda `-> vector<T>?`
published two types across the passes and was refused; one `set_delivered_vector_return`
keeps the `?` (D-call-11).  Guards `1343-…`, `1345-…`, `1346-…`, `1347-…` +
`tests/boolean_match_exhaustive.rs`.  Filed apart: loft#1349 (a lambda's lifetime tuple),
loft#1350 (a tuple result refuses to join a tuple literal), loft#1353 (the fn-ref nullable
record join).

### The nightly gate: a fn-ref return of any shape maps its deps into the caller, and the branches doc no longer links a file that is never committed (2026-09-04)

loft#1335, both legs.  **Debug-assertions gate:** `fnref_result_type` bridged a fn-ref
call's return deps from the callee's attribute space to the caller's frame for four listed
shapes and handed every other shape back untouched, so a keyed collection, a `?` return or a
tuple reached the caller with attribute indices in place — the `if` join then unioned
attribute 0 into a frame list ("dep-space violation", `Deps::union`, on every nightly since
the loft#1245 guard).  The mapper now asks `Type::borrow_deps` / `rewrap_deps` and lists
nothing; `Deps::renumber_frame` exempts an EMPTY attribute-tagged list (a `&text`
parameter's declared type carries the tag into the variable table).  The whole gate is green
on this tree.  `formal/ownership-history.md` D-own-28; DEPS_INVENTORY.md crossing site 6.
Guard `tests/scripts/1335-…loft`.  **Doc index hygiene:** the generated
`LIBRARY_BRANCHES.md` linked `LIBRARIES.md`, which is built on demand and never committed,
so the link was broken on every CI checkout; `scripts/lib-branch-audit.sh` now names the
file and the command instead.  Held fixed and filed apart: a nullable VECTOR return, a
vector in a returned tuple, and a nullable record chosen by an `if` alias the field.

### An early text return is delivered through the caller's buffer, and a user `&text` parameter is never that buffer (2026-09-04)

loft#1338.  A text function's block tail already delivered through the hidden `&text`
parameter `text_return` promotes; an EARLY `return <call>` / `return a ?? b` / `return
t[0][0]` reaching the scope pass with frees to run was copied into a frame-local `__ret_N`
String marked `skip_free` — orphaned, one per call, on the interpreter only (native collapses
the temp), with every value right.  Both hoist sites (`scopes::free_vars` and the block-tail
leg of `insert_free`) now write the value per arm into the buffer the function already holds
(`push_text_arms_into`), free the `__work_N` / `__ncc_N` temps the copy drained, then run the
frees and return the buffer; the copy stays only for a function with no buffer.  The loft#568
orphan predicate classifies every `return` site, not the tail alone (`early_return_ownerships`;
a null arm is a sentinel and excluded), and the targeted promotion (@PLN104) no longer defers
the view-of-local / join-of-local classes — `553 textslice` is green on both backends.  The
buffer question has one home, `Definition::text_work_buffers` (a HIDDEN `RefVar(Text)`
attribute): six sites restated it as any `RefVar(Text)`, which a user `&text` parameter also
is, so the predicate left such a function unbuffered and `--native` wrote the returned text
INTO the parameter.  `formal/calls-history.md` D-call-9; the LSan `append_text` suppression's
premise ("fault path only") is corrected in place.  Guard
`tests/scripts/1338-an-early-text-return-is-delivered-through-the-caller-buffer.loft` +
`tests/early_text_return.rs` (the text ledger under `LOFT_TEXT_TIMELINE=1`).  Side finding
loft#1343.

### A view of a local returned through a nullable return is copied before it escapes (2026-09-04)

loft#1337.  A `-> S?` return has no delivery buffer, so what is handed up is what the
reference-delivery selector decides — and two shapes got past it on both backends: a local
rebound through its own reference field (`cur = cur.next; cur`, a SELF-dep the view walk
skipped) handed up a sibling local's record after its exit free, and a projection inside an
`if` arm (`if take { t.l } else { null }`) was never seen, so the tail was demoted to
`return null`.  `(F-Ret)` already says a whole heap value is handed out owned, never a view
of a local.  Closed at the selector: a self-dep on a user local reads as a view, an `if` arm
is a tail where there is no buffer, and the buffer-less materialise is made per arm — only
the viewing arms are copied, a null arm stays null, a nullable local source copies only
where present.  The dense route is byte-identical.  Guard: `tests/scripts/1337-…`, both
backends, falsified at `c0a09c95`.  `D-call-8` opened and closed.

### A local whose assignments mix ownership releases through an owner witness (2026-09-04)

loft#1336.  `cur: Node? = a; while cur != null { cur = cur.next }` leaked the copy `cur` took
at the bind on both backends, and `s: Node? = a; s = a.next; s = a` wrote the second copy
INTO the viewed record on `--native` while the interpreter aliased the source.  Neither the
`reference` field nor the `?` nor the copy-bind was the axis: a call-minted local, a nested
field view and the dense twin all misbehaved the same way, because a binding carries one dep
list and it records whichever assignment parsed last.  `formal/ownership.md` gains
`(O-Witness)`: such a local is given a hidden `__own_<name>` that names the store it minted
while it still holds it, maintained in the IR at every assignment by store identity
(`OpDistinctStore`, `OpRefAlias` are the two new ops) and released at scope exit; the local
is never-free.  The native emitter's private `_own_store_` tracker was the reference route
and now applies only to hidden temporaries; both emitters copy into such a local FRESH and
decline the materialise arms for it.  `LOFT_NO_OWNER_WITNESS=1` is the A/B opt-out, and the
`LOFT_NO_JOIN_OWN` positive controls set it too.  Guard: `tests/scripts/1336-…`, fifteen
cells, falsified at `c25b444c`.  D-own-27 opened and closed; the record is in
`ownership-history.md`.
### A fn-ref call's return records what it borrows, for every kind of return (2026-09-04)

`Parser::fnref_result_type` maps a lambda's declared return deps — attribute indices in the
callee's space — through the actual arguments into the caller's frame.  It did so for text,
vector, struct and enum returns and handed every other kind back verbatim, so a fn-ref
returning a keyed field view (`fn(q: Bag) -> hash<K[k]> { q.m }`) reached the caller still
naming attribute 0.  In the caller's frame that is whichever variable holds number 0: with a
scalar parameter first, the enclosing function's own return then recorded the scalar and read
as an OWNED store, and a branch join over such an arm unioned attribute-space with frame-space
deps — the debug-assertions gate's `dep-space violation` on `1245b` (loft#1335).  The shape
list is gone: any type that borrows is asked through `Type::deps_ref` and rewrapped, a tuple
element-wise, which is what `(O-Move)` states and what `call_dependencies` already learned for
`Optional` (loft#938).  Guard: `frame_vars::a_fn_ref_return_borrows_the_argument_in_the_callers_space_for_every_kind`
asserts the FACT — the enclosing function's return names the parameter — in a build that always
runs; falsified: the keyed cell answers `[0]` on the four-shape list.  Values were right in
every cell before and after, because the lift and the ownership oracle re-derive what the dep
mis-stated; the script corpus is clean under `-C debug-assertions=on` again.  Noted on the way:
a named function cannot hand a fn-ref call's TUPLE back as its tail (`expected __tuple<…>, got
(…) on return from block`), a standing refusal that kept a tuple cell out of the guard.

The gate's other red, `issue_328_reference_self_recursive_walk`'s `Database 5 not correctly
freed`, is loft#1336: a nullable struct local bound by copy leaks its store when rebound to a
`reference` field view, and a later copy-bind writes into the viewed record on `--native`.
Filed with its eleven-cell matrix; the cure needs a per-variable ownership witness.

### Path spelling has one home, and the Windows nightly's two real failures close (2026-09-04)

`portable_path` now answers every path-spelling question the tree used to answer per site:
`plain_canonical` / `try_plain_canonical` / `plain_canonical_str` (canonicalise in the plain
spelling every other path uses — on Windows `fs::canonicalize` answers a verbatim `\\?\C:\…`
that never equals or prefix-matches its plain twin), `strip_verbatim` (disk and UNC),
`is_stdlib_source` (however the stdlib was loaded — the eight readers that checked only
`default/` treated an installed stdlib as user code), `is_under` (by component, so `pkg` does
not claim `pkg2/`) and `is_under_canonical` / `same_file`.  Fifty `canonicalize` sites, thirteen
stdlib checks and the two string-prefix package tests route through them.  That is what closed
the Windows leg's two real reds: the logger's project root came back verbatim and matched no
record's plain path, so a `[levels]` key ending in `/` could not raise a file's level
(`log_source_path`); and the check-line contract test built its expected path from the 8.3
`temp_dir()` and a bare `canonicalize`, neither the driver's spelling
(`check_line_audiences`).  Validated on Windows through `windows-probe.yml` dispatched on the
branch with the named tests.  The macOS reds in the same nightly were already closed by #1330
and a retry-green cold-cache flake in `native_scripts`; the `doc index hygiene` red was the
generated branch report linking the uncommitted library catalogue.

### The release gate: every nightly against one commit, one verdict (2026-09-04)

`release-gate.yml` calls the six nightlies — `ci.yml` (full matrix incl. Windows, the
stdlib round-trip, the differential oracle), `miri.yml`, `registry-validation`,
`revalidate-libs`, `browser-threads`, `repro-build` — as reusable workflows against ONE
commit, and a `verdict` job is red on any leg that is not `success`, advisory PR jobs
included.  `make release-gate` dispatches it on the pushed branch and waits; the release
checklist's six hand-dispatched `M-nightly-*` items become one measured `A-release-gate`,
keyed by HEAD's commit.  Measured cause: the 03:00 daily started between 03:34 and 14:45
UTC over three weeks, on whatever `main` was, and push-to-main's full matrix was red on
each of the last eight merges — the nightly-only legs are where a merge is found red, so
a release needs them run deliberately on the candidate.  Each nightly gains a
`workflow_call` trigger; `miri.yml`'s `from_gate` keeps its issue filer and digest with
the schedule, and three concurrency groups now carry the caller's workflow name.
CI_BUDGET.md § The schedule is not a clock; RELEASE.md § The nightlies.

### A fn-ref that FORWARDS is witnessed by the dep its callee declares (2026-09-04)

Branch-internal, never on `main`: loft#1329 made a captured fn-ref resolvable, which first
made this shape reachable.  A forwarding lambda (`fwd = fn(q) { inner(q) }`) reaches its own
return through `__closure`, so `use_analysis`'s summary cannot name the base and answers
`Own::Owned` — the arm's FALLBACK, which its own doc says is not a verdict.  D-own-8's arm
lift read it as one and freed the caller's collection, one per evaluation, with the value
still right for the first iterations.

Declining the lift there was measured and rejected: it closes the over-free and leaks worse
than 2026.8.0 (70 000 forwarding mint arms exhaust the store table where the release is
flat).  The fact the summary lost is the one the callee DECLARES — `-> vector<T>["q"]` names
the parameter the return borrows — so `callref_declared_borrow_base` maps it to the caller's
argument through the same `caller_arg_base` a resolved base uses, and
`callref_collection_join_base` consults it exactly where the oracle answered the fallback.
One identity free serves both answers.  Guard: the forwarding cells in
`1323-every-arm-of-a-value-branch-has-its-own-binding`, falsified at `1bee39aa` on both
backends — a second control, because at the first the target does not resolve and the cell
would pass for the wrong reason.

### A `-> text?` lambda called through a fn-ref compiles on `--native` (2026-09-04)

Found while sweeping the nullable-return spellings beside loft#1329, and it is the same
mistake on the TEXT axis: one question read with two spellings, one of which forgot the
wrapper.

`Parser::text_return` peels `Optional` before it converts a body, so a `-> text?` call site
appends exactly the same hidden `&text` work buffers a `-> text` one does.  The native fn-ref
dispatch's candidate filter asked whether this was a text return of the RAW type, so for
`Optional(Text)` it counted those buffers as user-visible arguments.  No candidate matched the
arity, the dispatch collapsed to `_ => unreachable!()`, and a `match` with no value-producing
arm has no type — so rustc answered `error[E0282]: type annotations needed` pointing at
generated Rust.  A whole class of loft program did not compile, reported as a rustc diagnostic
naming nothing about loft.

The filter's own doc comment already records this exact failure for the arity it fixed
earlier (loft#1116, where a call appending two buffers matched nothing).  This is the same one
a wrapper out, which is why both reads are now the single `is_text_return` and peel together
rather than being two `matches!` that agree by habit.

⚠ **The interpreter is not a control for it** — it answered correctly on every cell
throughout, so the whole defect is in what `--native` emits and a cell scored on the
interpreter reads green on the broken build.  What scores it is the native run's EXIT.

⚠ **And `null` was the cell that could have been traded away.**  Treating a `text?` return as
a text return also wraps each dispatch arm in `.to_string()`, which is the shape that turns an
absent text into `""`.  Measured: it does not — a null arm still reads `== null` and an EMPTY
string still reads `!= null`, identically to the named-function twin on both backends.

Guard `tests/scripts/fnref-text-optional-return-dispatches.loft`, 5 cells; falsified at
`f437c1f2` (native exit 1 → 0, interpret INERT).

### A vector local rebound from a fn-ref call frees the store it displaces (2026-09-04)

loft#1329, and it is loft#1328's sibling: the same question asked of a VECTOR destination
instead of a reference, an enum or a keyed collection.  `x = m(i)` in a loop over a fn-ref
held one store per ITERATION and aborted at the 65 535-store table.  Four readers of one
fact were short, and each had to be closed before the next was measurable.

**1 — the native gate had no `Vector` arm (`@FR-O-NoDiverge`).**  `generation/dispatch.rs`'s
displaced-free gate tested `Reference | Enum | keyed`, while the interpreter's twin
(`state/codegen.rs`'s `owned_ref`) has carried `Vector` all along.  One kind, present on one
side and absent on the other — the asymmetry the rule exists to forbid.

**2 — a capture is a payload, not a candidate.**  With that closed, a fn-ref FORWARDING to
another fn-ref still grew the peak on BOTH backends, which is the rule holding: they read the
same fact and it was short.  A capturing lambda's assignment is a block that mints the closure
record, WRITES each capture into it and then yields the `FnRef`; a capture that is itself a
fn-ref is written as an `FnRefDnr` argument of that write.  `use_analysis::fnref_target_in`
walked the whole tree, saw two definition numbers and answered `Some(u32::MAX)` — *"this
variable names TWO targets"*, the answer reserved for a slot two different lambdas were
assigned to.  The callee was then unresolvable, `callref_delivers_collection` returned false,
the dep survived, and nothing freed anything.  It now reads the marker the right-hand side
YIELDS, which is what the function's own doc comment already described; every branch of a
value branch is still yielded, so `f = if c { a } else { b }` reports the ambiguity that
reading is for.

**3 — one question asked with two spellings, so the sweep contradicted itself.**  The nullable
destination could not be measured at all: a lambda declaring `-> vector<τ>?` aborted the
compiler on the H5 two-pass contract.  The between-passes buffer sweep (`parser/mod.rs`) asks
*"does this deliver a collection?"* through `ret_promo_base`, which peels `Optional(Vector)` —
and then rejected the same definition on the RAW return type.  So the peel admitted the lambda
to the promotion pass while the raw read denied it a buffer, and pass 2 GREW the attribute.
Both reads are now `ret_promo_base`, and the buffer's type is the BASE, matching the
signature-time reservation in `definitions.rs`: the buffer is storage, and storage is never
absent — the `?` belongs to the value the caller reads.

**4 — and its native twin.**  Reserving the buffer is half; the fn-ref dispatch decides whether
to MINT one, and `generation/emit.rs` read the raw return type there too.  A nullable-collection
candidate was handed `DbRef::NULL` and its delivery wrote through it — `vector_add` carries no
`.rec != 0` test, so the arm that returned a collection faulted with *"a NULL DbRef reached a
store accessor"* while the arm that returned `null` was quietly fine.

**A carve-out's own safety claim is a measurement, not a premise.**  `owned_ref`'s bare
`Vector` arm said a nullable vector *"already releases through its own path"*, so widening it
*"would free twice"*.  Re-measured, that is false: `x: vector<τ>? = null` grew the peak 1:1
with the iteration count on both backends while its dense twin stayed flat.  The peel is safe
because an `Optional` destination routes to the runtime-GUARDED post-free, which
`free_displaced` no-ops on a same-store, free-protected or stack-record ref — the double free
the bare spelling was avoiding is not reachable from there.

⚠ **The exit-leak gate cannot see any of this.**  The frame frees everything, so at 200
iterations every shape reports no leak on both backends and answers correctly; what grows is
the PEAK.  `LOFT_STORES=timeline`'s `peak` is the cheap instrument (203 → 5 at 200 iterations
for the nullable cell), and the store table is what turns the watermark into an accept/reject
split a guard can score.

**A second cure moved an existing A/B onto a different channel.**
`nullable_ret_buffer::default_path_is_unchanged` pinned that `LOFT_NO_NULLABLE_RETBUF=1`
restores the pre-fix path, measured as *"the filed leak returns"* — and it stopped holding,
because the work-ref that program leaks through is an `Optional(Vector)` local re-Set every
turn, so `owned_ref`'s new peel releases it whatever the switch says. Its own failure message
named the two explanations (*"either the opt-out broke or the leak has another cure"*) and the
second is the true one. The control now measures the switch on `MIXED` and on the VALUE
channel: with the buffer off, `pick`'s aliasing arm hands back the caller's `base`, the result
reads as owned, and the program prints `base=2 base0=39`. That is measured identically on
`a8c0b74d` — the switch's own effect, not something the peel introduced — and a value channel
is the stronger one, since a leak gate is monotone and cannot score an over-free.

Guard `tests/scripts/1329-a-fn-ref-vector-rebind-frees-the-store-it-displaces.loft`, 8 cells,
5 of which fail on `a8c0b74d` across 10 of the 16 cell x backend runs; the other 3 are its
controls, and they pass on the control build.  `make ci` green.

### A `??` default-arm mint is released once (2026-09-03)

loft#1322.  `_vec_N` and `__vdb_N` name ONE store — the view is `OpGetField(__vdb_N, 0)` — and
both were freed: the return-delivery materializer through the view after the append, the record
at scope exit.  `@FR-O-Borrow` allows one owner and one free.

The site was behaving as designed, which is why the obvious readings do not close it.
`parser/operators.rs` picks the view model for an OWNED subject (`skip_free(_vec_N)`, the record
releasing the store) and `mark_inline_ref` only for a BORROWED one — *"do not allocate"* without
*"never free"* — because a borrowed subject's `??` in return-tail position hands the view to the
materializer.  The filed shape takes the second arm honestly: probed, its subject type reads
`Deps { items: [0] }`, naming the parameter.  So neither name was silenced.

**Two cures were measured before the third, and both are recorded on the issue.**  Peeling the
`Optional` on the first gate is INERT — the subject's type is already an unwrapped `Vector`.
Silencing the record unconditionally closes every shape and then exhausts the store table in
`1248b-a-capture-witness-is-the-slot-the-return-reads`.

⚠ **The flags are CUMULATIVE ACROSS PASSES, and that is the whole of the third cure.**  A
capture subject reads empty deps on pass 1 and takes the view model, then reads non-empty deps on
pass 2 and arrives at the other arm — so its `_vec_N` is ALREADY never-freed, and silencing the
record too leaves the store with no owner.  The record goes quiet only where the view's free
actually runs, asked as `is_skip_free(w)`: which arm the variable ENDED in, not which one this
pass took.

⚠ **No value can carry the verdict**, so the guard is in two halves.  A second free of an
already-freed store is a no-op and `LOFT_STRICT_STORES` does not flag it, so
`tests/scripts/1322-a-default-arm-mint-is-freed-once.loft` pins the shapes and their values
while `tests/redundant_free.rs` counts `LOFT_TRACE_DB`'s `already_free=true` lines — the only
channel there is.  It carries a `the_harness_can_see_a_redundant_free` cell, because without one
a trace that stopped reporting would read exactly like a clean tree.

⚠ **A residual under the same rule, one pair of names over:** a closure record is released
through BOTH the fn-ref value and its `___clos_N` local, so its cascade runs twice and the second
finds the capture's store gone.  Pre-existing (measured on `origin/main`), unchanged here, and it
is the shape that cell uses.

### An opaque fn-ref return borrows its arguments (2026-09-03)

loft#1327.  A closure called through a fn-TYPED PARAMETER may hand back its argument, and the
caller freed it anyway: `fn plain(a, g) { u = g(a); u[1] }` released the CALLER's vector on the
borrow arm, on both backends, silently — the next allocation reused the slot and the caller's
vector then read that allocation's contents (`some[1]` answered 299 where 42 was written).
Present on released 2026.8.0.

`@FR-O-Move` puts the obligation on the return TYPE — *"if the return borrows a parameter, the
return type records it"* — and that presumes a type able to carry it.  A DEFINITION carries it,
which is what `fnref_result_type` maps into the caller's space.  A fn TYPE has nowhere to write
it: `fn(vector<integer>?) -> vector<integer>` is the whole of what an author may spell.  So the
deps arrived empty whatever the target did, `u` was typed an owner, and scope exit freed it.

A fn-typed PARAMETER is the case where the target is unknowable from the body being compiled: no
assignment in it resolves one.  So the return now borrows what it MIGHT borrow — every heap
argument, rooted through `view_root_slots`, the same walk the @P290 bracket uses — and a
non-empty dep is what stops the free.  The rule gains a clause for it, `(O-Opaque)`.

⚠ **The mint arm costs nothing, and that was measured rather than assumed.**  Declining a free
normally trades the over-free for a leak; here the fn-ref call's own runtime buffer already owns
what the closure minted, so 70 000 default-arm calls past the 65 535-store table complete on both
backends.

⚠ **A fn-ref LOCAL is untouched.**  A local is resolved from its assignment
(`Scopes::fnref_target`), and the lift and identity-free routes built on that read the empty deps
this leaves alone — the same-frame cell is the control.  A local assigned two DIFFERENT lambdas
is opaque in the same way and is NOT covered: the parser cannot see that, and the same free
reaches it.  That fact lives one pass later.

Guard `tests/scripts/1327-an-opaque-fn-ref-return-may-be-its-argument.loft`, 6 cells; falsified
at `c3da7fce` (interpret 3 assertion failures -> 0, native 1 -> 0).  The VALUE channel is what
moves: a freed-then-reused slot answers wrong rather than faulting, so neither the leak nor the
panic channel sees it.

Found by loft-b9 while measuring D-own-8's parameter-base cells.

### A rebind from a fn-ref call frees the store it displaces (2026-09-03)

loft#1328.  `x: P? = null; for i in 0..N { x = m(i) }` over a fn-ref `m` held one store per
iteration on `--native`, released only at frame exit.  The native gate that emits the displaced
free listed its fresh-store right-hand sides as `Call | Insert | Block` — `CallRef` was missing,
so a rebind from a CLOSURE call escaped it entirely.  The interpreter's twin
(`state/codegen.rs`'s `owned_ref`) keys on the DESTINATION and so never had a spelling to miss,
which is the asymmetry `@FR-O-NoDiverge` exists to forbid.  One `Value::CallRef` arm closes it.

⚠ **The exit-leak gate cannot see this class.**  The frame frees everything, so at 1000
iterations both backends report no leak and answer correctly; what grows is the PEAK.  The
65 535-store table is what turns that into something scorable: at 70 000 iterations `--native`
aborts with `store table exhausted` where the interpreter completes and answers 69 999.  So the
guard counts past the ceiling and `make falsify` reports the move on the PANIC channel, not the
assert channel — no assertion is reached on the broken build at all.

Found by loft-b9 while measuring a lift over a branch arm; the sharper channel (accept/reject at
the ceiling rather than a peak watermark) is what made it a guardable defect.

Guard `tests/scripts/1328-a-fn-ref-rebind-frees-the-store-it-displaces.loft`, 6 cells over the
nullable, dense and keyed destinations with the direct-call spelling and a destination-reading
callee as controls; falsified at `05302d39`.

⚠ **A sibling this does NOT close, measured while checking the kinds:** a VECTOR local rebound
from a fn-ref call aborts at the same ceiling on BOTH backends — the gate's type test names
`Reference`, `Enum` and the keyed kinds, and the interpreter's `owned_ref` does include
`Type::Vector`, so the vector shape fails for a different reason and is its own item.

### A whole-tuple bind COPIES on `--native` too (2026-09-03)

loft#1325.  `u = a` over a local `(text, text)` emitted `let mut var_u: (String, String) =
var_a;`, which MOVES it — so every later read of `var_a` was rustc E0382 and the program did not
build at all, while the interpreter ran it and answered what `@FR-B-Copy` promises: an
INDEPENDENT copy.  An accept/reject split between the backends is the divergence
`formal/operational.md` D-op-1 forbids, and the refusing side was the wrong one — `(B-Copy)`
says the bind is a copy, so `.clone()` is the emission that keeps the promise.

The arm is the sibling of P247's, one level out: that one clones a `TupleGet` source
(`__ref_N = var_t.0`), this one the whole `Var`.  Both share `tuple_has_non_copy_leaf`, so
neither can drift about what a non-Copy leaf is, and both fire only on a LOCAL source — a tuple
PARAMETER arrives borrowed and is re-spelled by `tuple_arg_owned_elems` (loft#840, loft#1005),
which keeps that pair to itself.

Four shapes were refused and each named a different rustc message: the plain bind, a mixed
`(integer, text)` pair and a nested `((integer, text), integer)` as *borrow of moved value*, and
the issue's second shape — append into a vector, then keep writing the source — as *assign to
part of moved value*.  An all-scalar tuple is `Copy` in generated Rust, compiled before the
change and is the control for the axis.

Guard `tests/scripts/1325-a-whole-tuple-bind-copies-on-native-too.loft`, 8 cells; falsified at
`61e6fc62` with native exit 1 -> 0 and the interpreter INERT.  That asymmetry is structural: a
guard for an accept/reject divergence has one movable side, and on the broken build `--native`
produces no program to run, so the exit code is the only channel an assert could never reach.

### `=` on a captured KEYED collection replaces it, like its vector twin (2026-09-03)

loft#1326.  A whole-value rebind of a captured keyed collection EMPTIED it — `m = [Row { k: 9,
v: 9 }]` inside a closure read back `len 0`, both backends, nothing said.  Both axes were
required: the same rebind of a captured VECTOR was correct (loft#1279 taught that branch this
lesson), the same keyed collection reached through a captured STRUCT FIELD was correct, and the
same statement written without a closure was correct.

The cause is the one loft#1279's own commit predicted — *"That selector has now been too narrow
three times, and the lowering was right every time"* (P261, loft#917, loft#1279).  This is the
fourth.  A capture resolves to `OpGetDbRef` of the closure-record field rather than to the
`OpGetField` a struct field gives, so the keyed replace's `self.is_field(to)` answered no and the
whole clear-then-build branch was skipped.

Two halves, because the literal ARRIVES differently through a capture: it is a `Block` whose ops
build straight into the destination (@PLN93's build-into-target) where a struct field gets a
`Value::Insert` of the same ops.  So the gate admits a captured destination, and the clear goes
in FRONT of that block rather than inside it — putting it inside erases what the block just
built, which is the mistake loft#1279's first attempt made in the vector arm.

⚠ **Naming the destination is not writing into it.**  A comprehension over the captured
collection itself (`s = [for x in s { … }]`) names `s` and builds a fresh value; clearing before
it reads its source is what made seven of loft#1195's cells answer empty.  The gate asks
`value_writes_into`, the mutating-op question, which is the same one the vector arm asks — and
the comprehension is a cell in the guard rather than a hope.

Guard `tests/scripts/1326-a-captured-keyed-collection-rebind-replaces.loft`, 10 cells over the
four right-hand-side sources, all four keyed kinds and five controls (the plain local, the
captured struct field, the captured vector, `+=`, and the comprehension); falsified at
`24381205`.  Eight of the thirteen probe-matrix shapes answer wrong on that build.

`formal/closures.md` records what this does NOT settle: a rebind OUTSIDE the closure lets the
closure read the reassigned value at the keyed kinds and the build-time value at vector and
struct, because a keyed rebind refills the existing store.  Store lifetime is correct either
way; which value the closure should see is an open contract question.

### A closure record suppresses the free of the store it actually holds (2026-09-03)

loft#1324.  A closure record takes over the frame-exit free of its capture's store, and
`get_free_vars` decided WHICH local that is by walking `function.tp(v).depend()`.  That list is
one LATEST fact, and `@FR-O-Latest` says so: ownership belongs to the latest assignment *at a
point*, which a type cannot express.  For a capture reassigned after the closure was built the
two name different stores, and the suppression landed on the wrong one — in both directions at
once:

* the store the record ADOPTED kept its frame-exit free, so a closure that ESCAPES its defining
  frame read a released store — `null(oob)`, both backends, nothing said;
* the store the local now names had its free suppressed although nobody adopted it, so it leaked
  one store per reassigned capture — the filed symptom.

The issue asked which of the two stores leaks.  It is the REASSIGNED one; the closure keeps the
build-time store, which the guard pins by value (`h(0)` reads 42 while `e` reads 52).

`Scopes::capture_build_backing` answers positionally: one pre-order walk of the raw body records
each local's most recent backing root, and `OpSetDbRef(___clos_N, off, capture)` — the build —
reads it off.  Computed once per consumer rather than per variable, and all three consumers take
the same map, because the free emitter, `check_ref_leaks` and `ownership_cfg`'s oracle must
answer identically or a suppression one of them does not know about reads as a leak (loft#1308's
lesson, restated here rather than re-derived).

⚠ **Declining the suppression for a reassigned capture also stops the leak and leaves the
use-after-free exactly where it was.**  That is why the cure names the store instead, and why
the escaping cell is in the guard: it is the one a decline cannot pass.

⚠ **The keyed kinds diverge on a question this fix does not settle.**  `e = [Row { k: 1, v: 51 }]`
over a captured `hash` / `sorted` / `index` REFILLS the existing store instead of minting one, so
the closure reads the reassigned value where the vector and struct spellings read the build-time
one.  The store-lifetime half is correct either way — no leak, no premature free, both backends —
so this is a contract question, recorded in `formal/closures.md` under `(L-CapHeap)` and filed.

Guard `tests/scripts/1324-a-reassigned-capture-suppresses-the-store-the-record-holds.loft`,
12 cells over reassignment count, build order, capture kind, spelling, loop depth and escape;
falsified at `392694cd`.  Six of its shapes leaked on that build and none do now; the ASSERT
channel moves only on the escaping cell, because `make falsify` runs a guard through `--tests`,
which does not leak-check.

### Every path of a bound value branch has its own binding — D-own-8 and D-bind-16 closed (2026-09-03)

loft#1320 (residual), loft#1321, loft#1323.  `@FR-O-Complete` asks for the ownership fact per
binding, per path; a binding joined from arms that disagree carried both arms' deps and so
had ONE fact for two paths — a borrow, which left the arm that minted with no owner (leak) or,
read the other way, let the join alias the arm a plain bind would have copied (`@FR-B-Copy`).
The close widens loft#1320's principle to the whole arm-kind table: `Scopes::arm_bind`
rewrites every arm tail a SINGLE bind would leave owning into `{ __lift_N = <tail>;
__lift_N }` — a fn-ref call of any ownership, a named call answering a record the caller must
copy, a plain variable (record: copied at the bind; vector: refilled into a function-scoped
buffer by `OpReplaceVector`, whose element type the scope pass now reads from the `Stores`
registry it is handed — `scopes::check(data, database)`) — and the joined binding's dep list
is rewritten to name the temps.  A view arm stays a view; a branch in call-ARGUMENT position
lifts calls only (an argument aliases).  The two shapes loft#1320 declined take a witness
SNAPSHOT (`__wit_N`, written beside each bind from that bind's base — `@FR-O-Latest`); the
`??` hoist of a call subject owns what a plain bind of it would and releases its previous
store in the IR (so `--native`, which does not release a displaced store on a fn-ref re-bind
of a user local — loft#1328 — stays flat); `gen_set_first_at_tos`'s Reference/Enum null-init
arm is asked through `.base()`; and the loft#1245 witnessed lift no longer frees a fn-ref
return that is a raw keyed or index VIEW (it emptied the caller's hash — a regression since
PR #1268).  Two more proxy sites declare `@FR-O-Proxy asks free` and consult the override.
Guards: `1323-every-arm-of-a-value-branch-has-its-own-binding`,
`1321-a-joined-binding-copies-what-a-plain-bind-copies`,
`1245b-a-witnessed-lift-does-not-free-a-keyed-view`, each falsified at 26d17f4b on both
backends.  Filed beside it: loft#1327 (a fn-ref through a fn-typed PARAMETER reads owned and
frees the caller's collection — present on 2026.8.0) and loft#1328.

### A call-shaped argument names the store a fn-ref may hand back (2026-09-03)

loft#1318.  `@FR-O-Oracle` says a call resolves through the callee's return summary, and a
`Join` is settled at run time by store identity against the base the dep names.  The summary
is written in the CALLEE's parameter space, so delivering it means naming the caller value the
returned store may lie in — and where that translation gave up, a `CallRef` answered `Owned`,
the one verdict that licenses a free.  A fn-ref `??` handing back its argument then released a
container the caller still held: `g(pick(vs, 0))` in a loop answered 42, then `null(oob)` with
`len(vs)` at 0, then the `??` default, with nothing said.  Present on both backends.

Three translations were losing the base, all in `use_analysis::caller_arg_base`:

* an argument that is itself a CALL.  The structural walk stops at one deliberately — a
  callee's returned store may be its argument's or one it minted — on the stated ground that
  the caller lifts it into a temp first.  A BORROW-returning callee is not lifted, so the
  premise did not hold and the walk simply answered "unnameable".  It now asks the oracle,
  which is the thing that decides that split: `g(pick(vs, 0))` roots at `vs` one frame further
  out, and `g(mk())` stays unnameable because a minted store belongs to no caller variable.
* a keyed lookup.  `projection_ops`, the set the oracle roots a borrow through, carried four
  ops and was short of `OpGetRecord`, `OpVectorRef` and `OpVectorRefNullable` — every one
  declared `-> reference[arg0]`.  `m[k].v` therefore read as a MINT, and the caller emptied
  its own collection: quietly at `hash`, with `Store access out of bounds` at `sorted` and
  `index`.
* a hidden `__retbuf`.  Hidden parameters were refused wholesale as return mechanism rather
  than something the author wrote.  The delivery BUFFER is the exception — the caller
  allocates its own `__ref_N` and passes it at that position — so a callee handing back its
  `__retbuf` hands back a store the caller already holds.  `__closure` stays refused, since
  nothing is passed at its position and `closure_capture_base` reads that one out of the
  closure build.

Naming the base rather than declining is what keeps the other half: the mint arm of the same
closure is still freed per call, measured at 70 000 iterations against the 65 535-store
ceiling.

⚠ **Statement context is an axis in this family, and it retires guard cells silently.**
`s += g(vs[0])[1]` and `c = g(pick(vs, 0))[1]` are CORRECT on the broken build while the same
reads inside an interpolation are wrong — the accumulate and bind paths reach a different lift
decision.  Two cells of the guard were vacuous until each was measured against the pre-fix
build.  The filed axis table also had one row backwards: two straight-line calls fail too, and
the loop only makes it easy to see.

⚠ **`is_projection_op` is still short by the two nullable element reads by its own criterion,
and correcting it is blocked.** Adding `OpGetVectorNullable` / `OpVectorRefNullable` strands
three `Cell` records in `tests/scripts/1040-generic-par-worker-in-generic-fn.loft`:
`state/codegen.rs`'s @PLN130 F1 materialise arm fires on the deps PROXY while the free sweep
reads the ORACLE, and a `par` body's element bind sits in the gap — `@FR-O-Proxy`'s named
hazard in the allocate direction.  It does not reproduce on `main`, so it stays a measurement
rather than an issue; the doc comment on `projection_ops` carries it.

Guard `tests/scripts/1318-a-call-shaped-argument-names-the-store-a-fn-ref-may-hand-back.loft`,
14 cells over container kind, argument shape, call spelling, statement context and which arm
runs; falsified at `b1bd3212` (interpret exit 1 -> 0, 6 assertion failures -> 0; native exit
1 -> 0).  Eight cells fail on that build and six are its controls.

### A nullable whole-value bind COPIES, like its dense twin (2026-09-03)

loft#1319.  `@FR-B-Copy` says a plain bind — scalar or heap whole-value — leaves the bound
variable INDEPENDENT.  It did not hold when the source was nullable: `b = a` with
`a: vector<integer>?` aliased `a`, and so did a nullable struct, while the keyed kinds copied.
None of the rule's three exceptions reaches the shape — `B-View` is a struct PROJECTION,
`B-View-Base` a BORROWED base, `B-View-Depth` an INDEX or nested read — and this is a whole
value off an owned local.

One cause, four sites, and it is the spelling again: `τ?` is `Optional(τ)`, the same storage
behind a nullability marker (`@FR-L-Null`), and each site decided the lowering by matching the
`Type` variant BARE, so the wrapped shape reached none of the copy paths and the default stood.

| site | what it decides |
|---|---|
| `Parser::classify_vec_bind` + its consumer | whether a vector bind is a copy at all |
| `codegen::gen_set_first_at_tos` | the interpreter's first-set dispatch |
| `generation::dispatch` whole-record bind (`heap_def_nr`) | native's copy |

The second half is that a copy must not turn ABSENCE into EMPTINESS — a null source has to
leave the destination null, not holding the store the copy allocated for it.  Both mechanisms
already existed:

* `Stores::vector_replace` gains the guard `replace_keyed` has carried since loft#1150 — *an
  absent source copies nothing and marks the destination absent* — and the nullable vector
  bind emits `OpReplaceVector` rather than `OpAppendVector` to reach it.  A dense destination
  cannot be absent and keeps the append, so its IR is unchanged.
* the record bind routes through `OpBindOrCopy` with the SOURCE as its own witness: a present
  source aliases the witness, so the borrow arm materialises a fresh store and deep-copies;
  an absent one fails the `store_nr != u16::MAX` half and is ADOPTED, which lands the true
  sentinel.  Native emits the same decision as a guarded `if var_src.rec == 0`.

⚠ `OpCopyRefOrNull` was tried first and is wrong here.  It binds `Stores::null()`, whose
`store_nr` is a REAL slot with `rec == 0`, while `x == null` on a record lowers to
`OpRefIsNull`, which tests `store_nr == u16::MAX`.  The two spellings of absence agree for the
element read it was written for and not for a bound local.

**The `??` column of the filed matrix is a DIFFERENT defect** — measured, not assumed.  It was
filed as "the same defect discharged"; a JOIN with no nullability anywhere in it
(`b = if c { a } else { [0, 0] }`) aliases identically on both backends and on the shipped
2026.8.0, and this fix leaves those cells where they were.  `a ?? d` lowers to
`if !isnull(a) { a } else { d }`, so the `??` cells were reaching the join lowering.  Filed as
loft#1321, registered as the OPEN deviation D-bind-16.

Guard: `tests/scripts/1319-a-nullable-whole-value-bind-copies-like-its-dense-twin.loft`,
falsified at `dad9b359` on both backends; controls are `&` (must still alias), the struct
projection and index read (must still view), the collection projection off an owned base (must
still copy), and every keyed kind (must not move).
`tests/scripts/bind-copies-or-views-the-whole-boundary.loft` — the one place the copy-vs-view
boundary is pinned — gains the nullable-subject axis it never had: all eleven of its subjects
were declared non-null, which is why eleven cells and both backends read green over this.

### A `reference<T>` field may be nullable, and `?` no longer turns a pointer into a copy (2026-09-03)

loft#1316.  `@FR-L-Null` and `@FR-L-Null-Tag` split on a property of the type: a τ that reserves
a null VALUE keeps its own bytes and spends the reserved pattern on absence; only a struct stored
INLINE needs a discriminant.  A stored reference reserves `nullref`, so `reference<T>?` is the
first case.  `synth_nullable_struct_fields` gave it the second.

Both notions are `Type::Reference` in the IR, told apart by the FIELD's `u16::MAX` share marker
(#328) — the same bit `Data::has_value_cycle` reads to skip pointer edges — and the rewrite
discarded the deps with `_`.  Measured with `LOFT_DUMP_TYPES=1`:

```
HolderN { l: reference<Leaf>  }   ->  HolderN[12/4]  l:dbref[0]
HolderQ { l: reference<Leaf>? }   ->  HolderQ[16/8]  l:__nullable<Leaf>[0]   <- was
HolderS { l: Leaf             }   ->  HolderS[8/8]   l:Leaf[0]
HolderSQ{ l: Leaf?            }   ->  HolderSQ[16/8] l:__nullable<Leaf>[0]
```

`reference<Leaf>?` and `Leaf?` were byte-identical, so the `?` erased the pointer.  Three sites
read the field type without peeling and each produced a different face of it:

* **`typedef.rs::synth_nullable_struct_fields`** tagged the field.  A struct stored inline cannot
  contain itself, so on a reference graph returning to its own struct the field had no finite
  size: `struct Node { next: reference<Node>? }` failed with *"field 'next' has no position
  (u16::MAX)"*.  That is why loft#1313 had to suppress `(N-Store)` for the shape — the cure it
  would have named did not compile.
* **`objects.rs`'s `&` head gate** matched `Type::Reference` unpeeled, so `&pool[i]` in a literal
  — the one position `@FR-B-Ref-StoredRef` admits — was refused once the `?` was written.
* **`collections.rs`'s #328 repoint arm** matched unpeeled too, so `h.l = &pool[i]` fell through
  to `copy_ref`: an `OpCopyRecord` through the field's CURRENT value.  Against the pre-fix build
  that compiles and answers plausibly — the same program prints `11` where a pointer prints `22`
  — so declaring `?` replaced sharing with a copy in silence.

All three read the marker or peel with `base()`.  `@FR-L-Null-Which` is added to the contract:
the split is decidable from the field, and `synth_nullable_struct_fields` is its one home.

Consequences.  loft#1313's suppression (`field_has_no_nullable_spelling` +
`Data::reference_cycle_back_to`) is deleted, and its guard cells in `tests/heap_nstore.rs` flip
from silent to warning.  The notice they now emit had the same defect one layer up: `Type::name`
renders a pointer field as the bare struct name, so the cure read `Node?` — the inline form,
which on a self-referencing struct does not compile at all and on an acyclic one compiles while
swapping the pointer for a copy.  `Parser::cure_spelling` names the field's own type.  Two
`issue_328` corpus tests adopt the spelling they always wanted (`next: reference<Node>?`, walker
`cur: Node?`); both were carrying an undeclared null because nothing else was writable.

Alongside, `@FR-B-Ref-StoredRef` gains the half its gate was missing (D-bind-14): the rule admits
the `&` on the FIELD'S TYPE and says nothing about field order, but the gate accepted only `;`/`}`
as the terminator — the tokens that end an ASSIGNMENT — so `Trail { link: &pool[0], id: 7 }` was
refused for its comma while the same literal with the fields swapped compiled.  `AmpHead` names
the position (`No` / `AssignRhs` / `StoredRefField`) and the terminator set is read off it.

Guard: `tests/scripts/1316-a-nullable-reference-field-is-still-a-pointer.loft`, falsified at
`3bae617b` (interpret exit 1 -> 0, native exit 1 -> 0), controls being the `?`-less pointer field
(still shares) and the embedded `T?` (still copies).  `tests/scripts/150-amp-head-position.loft`
gains the not-last cell.  All 42 published libraries pass unchanged.

### The revalidate-libs gate can be green (2026-09-03)

loft#1315.  `scripts/revalidate_libs_local.sh` printed `1 COMPILE-BREAK` and exited 1 on a tree
with no language break in it, and did so against the binary the offending package was published
with — so nothing downstream could gate on its status, a real break read `2 COMPILE-BREAK`, and
the closing sentence about the freeze printed on every green run.

The cause was not a design gap.  The matrix policy was written TWICE — inline in
`.github/workflows/revalidate-libs.yml` and again in the local script that documents itself as
re-classifying *"exactly as the workflow does"* — and the two had drifted on four questions:

| question | workflow | local script |
|---|---|---|
| the `loft` package | skipped: it is the compiler, not a library | checked, and its test corpus is not standalone-compilable |
| known-broken map | present, entries cite a tracking issue | absent |
| `subpath` default | `"."` (the repo IS the package) | the package NAME, a directory that does not exist |
| yanked versions | validated | excluded |

Only the first showed, because the compiler's `tests/` holds `--lib` fixtures and files that are
*supposed* not to compile: 26 of 400 failed the `--dump` re-classification, on any binary.  The
fourth is the one worth noting anyway — the shipped gate could have reported on a version the
registry has withdrawn.

`scripts/revalidate_matrix.py` is now the single source both read, with a `--self-test` that gives
each rule an input it must act on (an exclusion that is never exercised is indistinguishable from
a pass-through) and is wired into `make ci`.  The discover job checks the repo out to reach it.
A clean run now reads `42 pass, 0 runtime/env, 0 skipped, 0 COMPILE-BREAK` and exits 0; the
240-second hang the `loft` leg contributed goes with it.


### A fn-level `@EXPECT_FAIL` no longer costs its file the whole native suite (2026-09-03)

loft#1311.  The documented contract is that a fn-level tag excuses one function and *"sibling
fns still must pass"*.  `tests/native.rs` dropped the FILE for any declaration, and the finer
per-function mechanism under it — which emits `// skipped (EXPECT_FAIL): {name}` in place of a
call — hand-rolled its own parser:

```rust
.filter(|l| l.contains("@EXPECT_FAIL"))
.flat_map(|l| l.split_whitespace().skip_while(|w| *w != "@EXPECT_FAIL").skip(1) …)
```

The documented form `// @EXPECT_FAIL: <reason>` tokenizes with the colon attached, so the skip
set came back empty for every file written that way — the fn-level mechanism was inert, and the
drop above it was the only thing that ran.

One parser now: `common::expect_fail_fns`, positional as the documentation defines the
annotation, shared with `tests/wrap.rs`, returning the fn names and whether a file-level tag is
present.  Only the file-level one drops the file.

`75-native-stub.loft` must still be dropped, and now is for its real reason rather than its
annotation: it declares a `#native` fn with no registered implementation, so the generator emits
`compile_error!` into the Rust and the macro fails the build wherever it is expanded.  The gate
reads that refusal, which also covers a file carrying no annotation at all.

### `(N-Store)` reaches the heap half of the rule it was always written for (2026-09-03)

loft#1313.  `formal/types.md` `(N-Opt)` states the default for every type — *"Storage is non-null
by default: a binding, field, or `vector` element of type `τ` never holds `null`"* — and
`(N-Store)` carries no type restriction.  @PLN25's DN1 landed the model and gated the enforcement
on `Parser::is_non_null_scalar`, so a bare `null` into a non-null reference, collection or
struct-enum passed in silence at the four positions where the scalar twin warns: a field, a
return, a vector element, a call argument.  A heap LOCAL was never in the gap — `change_var`
refuses `x: It = null` on its own — which is why the hole read as deliberate.

Two other homes had already specified the heap half.  `LOFT.md` § Types says *"you cannot store a
`null` into a plain `integer` / `text` / `Row`"*, and `Row` is a struct; `keys::callarg_nstore_
enabled` describes its own split as *"a non-narrow scalar/heap param WARNS"*.  Only
`is_non_null_scalar`'s doc comment said otherwise, and it stated the carve-out as though it were
the model — which is what made the deviation survive a reading of the code.

The predicate is `data::is_dbref`, called rather than respelled: its own doc records how a
hand-written copy drifts short of the five KEYED collections, which do not look like references at
the call site.  The synthetic `__nullable<S>` is excluded exactly as the DN3 branch excludes it —
it is the inline spelling of `S?`.

**Warning, never an error**, and that is `(N-Store)`'s existing Phase-1 split rather than a new
call: there is no narrow heap width to run out of room the way a `u8` does, and loft#1232 settled
the compatibility half — reporting where there was silence is a strict gain, refusing what a
shipped package already compiles is the break the freeze forbids.  The scalar wording is unchanged
to the byte; the heap half drops the word `scalar` from the same message rather than adding a
second one.  Opt out with `LOFT_NO_HEAP_NSTORE`.

**One shape is excluded, because the cure does not exist for it.**  A `reference<T>` field
standing on a reference CYCLE back to its own struct has no nullable spelling — `struct Node {
next: reference<Node>? }` fails layout validation, and so does the mutual `A`/`B` pair, while the
same field on an acyclic type is fine (loft#1316).  A linked list's terminator therefore has to
be a bare `null` in a non-null slot, and reporting it would name `Node?` as the fix, which does
not compile.  `Parser::field_has_no_nullable_spelling` asks that question through
`Data::reference_cycle_back_to` — the reference-edge twin of `has_value_cycle`, reading the same
`u16::MAX` share-marker (#328) that one reads to EXCLUDE these fields, so the two walks cannot
disagree about which edge is which.  It is the CYCLE that excuses the field, not the
`reference<…>` spelling and not the type being recursive: an acyclic `reference<Leaf>` field and
a cyclic type's `-> Node` RETURN both keep their notice.  The exclusion is a workaround for
loft#1316 and goes when it closes.

Blast radius, measured A/B across the 1083-file script corpus: **4 sites in 2 files**, all true
positives, no exit code moved.  ⚠ That scan was INCOMPLETE — it covered `tests/scripts/*.loft`
and not the inline `code!` sources in `tests/issues.rs`, where the two cyclic-field cases live.
The full suite is the corpus; a glob over one directory is not.  Both keep their signatures and declare the notice —
`98-struct-order-in-use.loft`'s subject IS a `return null` under a non-null heap return, and
`754-tail-place-read-return.loft` guards the non-null RECORD return ABI, which `Section?` does not
use (no hidden buffer), so "correcting" either signature would have retired the shape it guards.

`tests/heap_nstore.rs` COUNTS notices on both backends, because the negative half has no corpus
channel: a `.loft` guard can declare a notice it expects and cannot assert one that must not fire.
Falsified by reverting the predicate — the two positive tests go to 0 notices, the five controls
stay green, which is the correct signature for a change that only adds a report.

### A custom iterator's loop: a missing return buffer, and a break test that asked the wrong question (2026-09-03)

loft#1310 was filed as an ICE on a non-nullable STRUCT item.  The matrix said the filed scope
was one cell of six broken ones, and that the filed workaround — declare the item `Item?` —
fixes two of them.  Three independent defects met in one loop.

**The synthesised call skipped `add_defaults`.**  A `next` answering a heap value takes a
hidden buffer the CALLER allocates; the for-loop desugaring hand-built its `Value::Call` with
`self` alone, so the compiler aborted with *"Too few parameters on t_7Counter_next (got 1, need
2)"*.  This is the third site in the class — loft#945 at the combinator callbacks, loft#1114 at
a lambda — and the cure is the helper those already share, `callback_call`.

**The break test asked a second spelling of "is this null".**  `null_test` documents itself as
the ONE place that answers *what is `τ`'s null*, and warns that answering it elsewhere mints
another spelling — the defect loft#1014 was.  The loop reached for `convert(τ, Boolean)`.  The
two agree for an `integer`, a `text` and a bare reference, and diverge for a `vector` and a
struct-enum, whose truthiness rule is written against the NULLABLE form — and the loop variable
is deliberately typed as the non-null item (@PLN102 D1).  So the test saw the bare type, got no
conversion at all, and `OpNot` inverted the raw handle.

**And the fallback asked whether the item was FALSY.**  That coincides with "is it null" only
where the type's null is its only falsy value.  An `integer`'s conversion is `!= i64::MIN` and a
`text`'s null is out-of-band, so `0` and `""` elements correctly kept iterating; a `boolean`'s
conversion is the IDENTITY and its null is the three-state `255` (C73), so a `boolean?` iterator
ended on its first `false`.  The fallback is now `null_test`'s own documented one — compare
against the TYPED null — so the loop asks `item == null` and nothing else.

Both later defects are **`silent-wrong`**: four of the ten item types yielded zero elements and
exited 0, with no diagnostic.  The ICE was the only reason the type was looked at.  Fifteen
cells on both backends in
`tests/scripts/1310-a-custom-iterator-yields-every-heap-item-type.loft`, each asserting the
ELEMENTS — a count- or leak-only cell scores a zero-iteration loop as a pass.

### A keyed `&` write-back does not release the caller's collection (2026-09-02)

loft#1287 settled that a `&` parameter's whole-value write-back may release the store it
displaced only where the caller's binding OWNS it, and through a plain forwarder it does not —
`formal/calls.md` `(F-ParamHeap)` makes a plain heap parameter alias ITS caller's argument, so
the store belongs two frames down.  The rebind WITNESS carries that fact: it names the
parameter's ENTRY store, `scopes::scan_args` marks that store free-protected for the call, and
`Stores::free_displaced` refuses it.

The witness was minted behind an ALLOW-LIST written TWICE — `Type::Reference |
Type::Enum(_, true, _)`, once in `parser/mod.rs` and once in `scopes.rs` — so it covered a
struct and a struct-enum and nothing else.  Every KEYED collection fell through both:

```loft
fn set_h(x: &hash<E[k]>) { x = mk(); }
fn fwd_h(x: hash<E[k]>)  { set_h(x); }
```

and the callee released the caller's collection.  **The value read back CORRECTLY**, which is
why two independent boundary matrices scored this row as passing.  A freed store keeps its bytes
until its slot is handed out; put one allocation between the call and the read and the
interpreter panics on a corrupt reference, with `LOFT_STRICT_STORES=1` reporting seven
lifetime violations and one store never freed.  `sev:high`, `silent-wrong`.

The predicate has one home now — `Type::is_amp_rebindable_heap` — because its two askers must
agree: one mints the witness and the other uses it, and a site that said yes while the other
said no would either free a store belonging to a frame below or leak the fresh one.

### A `&sorted` write-back reaches the caller, where the write used to vanish (2026-09-02)

`formal/calls.md` `(F-ParamRef)` makes a `&` parameter the explicit write-back channel.  A
`&sorted<T[k]>` was refused instead — *"Parameter 'x' has & but is never modified"* — and that
refusal was the only thing stopping a lost write.  Give the body any other write and the program
compiles, and the assignment is silently discarded with the callee's collection leaked:

```loft
fn set_s(x: &sorted<E[k]>) { x = mks(); for e in x { e.v = e.v; } }
    IR:  [3] n_mk();          // no Set at all; the result is thrown away
```

`&hash` and `&index` were correct in the same position, and `is_keyed`, `keyed_type_id` and
`base()` treat the three alike — so the difference had to be a site that names ONE kind.
`collections.rs::towards_set` returned the right-hand side alone for `RefVar(Vector | Sorted)`.
That is right for a vector: `assign_refvar_vector` has by then lowered the write into ops that
fill the target in place, and the shapes it declines — a bracket literal, a comprehension —
carry their own appends.  A `sorted` never had that lowering, so its right-hand side was a bare
VALUE and returning it dropped the write.  The condition now names the fact (*the right-hand
side has already written the target*) instead of the type former, and the arm has read this way
since the initial commit, so a `&sorted` whole-value write-back has never worked.

⚠ Two things this leaves, both uniform across the keyed kinds and neither this fix's doing.
NO `+=` spelling works on a `&` keyed parameter — bracketed literal, bare element and whole
collection alike are refused with *"cannot change type from `&hash<…>` to `vector<E>`"*, a type
the program never wrote, because @P277's interception asks `is_keyed`, which peels `Optional`
and not `RefVar` (the remaining half of loft#1292). And a bare-VAR right-hand side mismanages
the store in both directions: from a caller-reachable value the displaced store LEAKS, from a
callee-local one the callee's scope-exit free makes it a USE-AFTER-FREE in the caller
(loft#1303, filed — one question, since `(O-Latest)` puts ownership on the binding that ends up
naming the store and nothing moves it).

⚠ The VECTOR half of loft#1291 is NOT closed.  A `&vector<T>` write-back is `OpClearVector` plus
a refill of the SHARED backing, so it never repoints and there is no displaced store to protect:
it answers WRONG where the keyed kinds corrupted quietly.  Letting it take the fresh-backing
rebind `vectors.rs::vector_db_init` already builds was measured and BACKED OUT — it does not
compile on `--native`, it turns two previously-compiling right-hand sides into refusals, and the
interpreter's `OpCreateStack` did not isolate the forwarder's frame the way the struct and keyed
kinds do.  The cells are on the issue.

### A generic header binds its OWN type variable (2026-09-02)

Two generic functions that both wrote `T` — the universal convention — shared one type
variable. The bound-method stubs hang off the type variable's placeholder definition and carry
a SIGNATURE, so whichever header was declared first owned the stub and the second's calls were
checked against a parameter list its author never wrote (loft#1301):

```loft
interface HasSize1 { fn sizer(self: Self) -> integer }
interface HasSize2 { fn sizer(self: Self, scale: integer) -> integer }
fn one<T: HasSize1>(x: T) -> integer { x.sizer() }
fn two<T: HasSize2>(x: T) -> integer { x.sizer(10) }
    ->  error: Too many parameters for T#g.sizer
```

Order-dependent: swap the two declarations and the error swapped with them. `formal/
interfaces.md` `(G-Gen)` says a header *introduces* its type variable, so the binding is
per-header and this was a deviation (`D-gen-3`), not a design question.

The placeholder is now keyed on `(spelling, bound set)`, and the spelling resolves against the
enclosing header before the flat namespace — `Parser::def_nr_in_scope`, read from
`parse_type_inner` and `parse_constant_value`. Sharing stays the norm: two headers with the
same bounds reach one placeholder and one set of stubs, which is what keeps the stdlib's many
`<T>` templates on one definition.

Four spellings were broken and two of them were unreported, because the conflict diagnostic
loft#1301 shipped compares parameter COUNTS and a signature is not an arity: different arity;
the two arities of `-`, which both desugar to `OpMin` (loft#1300); same arity with a different
parameter TYPE, which failed as *"expected integer, got text"*; and same parameters with a
different RETURN type.

Two details the change carries. A second placeholder is minted under a name no source can
spell (`T#2`), so `Type::name` and every diagnostic that prints one render the spelling
instead, and the `x?` default — which sub-parses `T {}` as SOURCE — pins the header's binding
over the sub-parse. And the mint asks `Data::name_taken_anywhere`, not `def_nr`: the
placeholder is registered as a store structure under `__typevar_<name>`, a registry with no
source in its key, so two libraries each taking the first name their own source had free
registered the same structure twice and aborted the compiler.

Still refused, and now with a message that says why: ONE bound set requiring two signatures of
one method name — an interface declaring `-` at both arities, or `<T: A + B>` where both
declare `sizer`. There the two requirements really are on one variable and a bound method is
reached by NAME; that is loft#1275 (`D-gen-4`).

### A bounded generic that DELEGATES still owns the store it hands back (2026-09-01)

`fn add<T: Addable>(a: T, b: T) -> T { a + b }` retained one record per call when its result
was consumed inline — unbounded in a loop — while `r = add(…); r.v` was clean all along,
because the `Set` gives the store an owner. The answer was right and nothing reported it
(loft#1273).

`scopes::inline_struct_return` lifts a call's owned aggregate into a `__lift_N` the caller
frees, and for a monomorph it needs POSITIVE proof the return is fresh: specialisation loses
the return dep, so the dep-based guards cannot tell a minted return from one handing back an
argument, and lifting the latter would double free. `monomorph_return_is_fresh` reads the
body's return sites for that proof and answered `false` here, because the tail of `{ a + b }`
is `Call(n_OpAdd, …)` — whether a CALLEE's result is owned is not a fact that body carries.

`Data` holds every definition, so the caller now resolves the tail's target and asks it the
same three questions the fn-ref twin asks (loft#1176): has a body, does not return a borrowed
view, and is itself fresh. One level, and one unreadable link refuses the chain — the proof
stays positive and under-approximating.

⚠ **`loft --tests` does not report a leak**, so the guard lives in
`tests/leak_cases/clean/`, which runs a plain program on both backends; the `tests/scripts`
file beside it carries the values, and specifically the rows that must NOT lift.


### A format hole holding an escaped quote ended a top-level item early (2026-09-01)

`"got: {shout("a\"b")}"` compiled inside `fn main` and was refused in every other function,
with `fatal: String not correctly terminated` pointing at the closing quote of a string the
compiler accepts one function up (loft#1271).

**The lexer was never the defect**, though the message came from it —
`Lexer::hole_closes_on_this_line` already keeps the right stack. `split_top_level`'s item
scanner read the string FLAT, stopping at the first unescaped `"`, which is the one before
`a`. The item then ended inside the literal, the `fn main` that FOLLOWED no longer STARTED an
item, `is_script` stopped seeing it, and an ordinary program was desugared as a beginner
script; the lexer was reporting the mangled source. That is why ORDER decided it: with
`fn main` first, the misplaced boundary lands after it and the same bytes compile.

`scan_string_end` now keeps the lexer's rule — inside a string `{` opens a hole (`{{`/`}}`
are literal braces), inside a hole `"` opens a nested string, and the literal ends only at a
`"` at hole depth 0 — which also clears the corpus sweep
`script::tests::no_corpus_file_classifies_as_script` was failing on.


### A bound is satisfied by a signature, not by a name (2026-09-01)

`a - b` inside a `<T: Numeric>` body compiled and computed `-a`, dropping the second operand,
on both backends with no diagnostic — and the float case answered an integer, so the result
type was wrong too (loft#1274).

`formal/interfaces.md` (G-Sat) satisfies an interface when a function with the interface's
SIGNATURE is visible, parameter list included. `has_bound_for_method` compared only the name,
and `-` desugars to `OpMin` at BOTH arities: `Numeric` declares `op - (self: Self) -> Self`,
the unary negation, so a binary `-` matched it and the call bound one operand too many. It now
compares the arity the use site passes, and `a - b` takes the refusal `Addable` already gave
the same expression.

No built-in bound offers binary subtraction — `-` being one name at two arities is why — which
is loft#1275, a design question rather than part of this fix. `INTERFACES.md` § Standard
library interfaces and the reference's Generics chapter both claimed a wider `Numeric` and a
wider `Addable` than `default/01_code.loft` ships; both now describe the file.


### `#remove` inside a keyed range: an ICE, and then a skipped element (2026-09-01)

Two defects, one behind the other (loft#1272).

**It did not compile.** A keyed iteration has two lowerings and they name their cursor
differently — `{loop}#iter_state` for an unbounded walk, `_iter_N` for a bounded range.
`#remove` rebuilt the first spelling by hand and fell back to `{loop}#index` when it missed;
a range ELIDES that local, so `add_const` measured the operand against a slot that does not
exist and `before_stack - r` underflowed. An ICE on both backends, and on the released
2026.8.0 the same program compiled and read a corrupt reference out of the store. The loop
now records its cursor at the site that creates it (`Vars::set_loop_state_var`).

**Then it skipped an element.** `remove` ended the walk when the SUCCESSOR of the removed
node was `finish`, but `finish` names the last node to VISIT — `step` yields it and only then
marks the end. So removing consecutive elements dropped one: `[1..4]` over 1..5 removing
everything left `3` behind. The test belongs on the node removed (`cur == finish`). Invisible
on an unbounded walk, where `finish` is `0` and the comparison can never fire, and that was
the only shape the suite covered.

Both backends carried their own copy of that decision and both carried the defect, so it now
has one home, `Stores::remove_during_tree_iteration` — the same treatment
`tree::range_cursors` and `vector::ordered_range_cursors` already had.


### A descending key orders an `index` twice, so every query answers the reverse (2026-09-01)

`keys::compare` reverses per descending key, so the red-black tree `tree::put` builds is already
in the declared order and a forward walk of it IS the declared order. Two sites applied the sign
a SECOND time: `fill_iter` XOR-ed the iterator's reverse bit when `keys[0].type_nr < 0`, and
`tree::range_cursors` swapped which user bound sat at which end of tree order. Reversing a total
order twice is the identity, so every query on a descending `index` — plain `for`, `rev(...)`,
and every range form — answered the exact reverse of its declaration, on both backends, with no
diagnostic (loft#1267).

One key hid what it was: `[-nr]` reversed reads as plain `[nr]`, so it looked like the `-` was
being dropped. Two keys showed it, because `[-nr, key]` reversed is `[nr, -key]` — the SECOND
field came back descending though it is declared ascending.

`sorted` was correct throughout and is the fix's oracle: `vector::ordered_range_cursors` reads no
sign at all and leaves direction to its comparator. That is now the written rule
(`formal/collections.md` `Col-Order-Sign`) — a `-` is applied by the comparator and by nothing
else, from which it follows that a range names its bounds in the COLLECTION's key order and that
`sorted` and `index` answer identically for the same declaration.

The P98 guard had locked the compensated behaviour, and could not have caught it: it summed the
scores over `["a".."c"]` on a descending index and read 3, which is `{a, b}` — the ascending
answer, and a number indistinguishable from `{c}`. It asserts which records, in which order, now,
beside its `sorted` twin.

### `--dev-soft-halt` surfaces integer overflow (2026-09-01)

`(E-Report)` promises the flag surfaces the recoverable faults uniformly — div0, overflow, OOB —
and overflow was the one it missed. It is also the one with no other channel: its peers write a
log record and overflow deliberately does not, the null being the signal, so this flag was the
whole of its observability (loft#1265, deviation D-op-9, now closed).

Reported from `checked_long!`'s `None` arm — the single place an overflow becomes the sentinel,
and a branch that already existed to build it, so no operation that does not overflow gained a
test. Both backends call the same `ops::` functions, so one site serves the interpreter and
`--native` alike. The guarded peer `checked_long_nullable!` stays silent, which is the answer
`(E-Report)` already gives that site's divide-by-zero half.

The run now also ends non-zero, the way its peers do; `Stores::run_failed` is the one home for
that question, asked by the interpreter's `main` and by both generated `fn main()` templates.


### A nullable element meets @PLAN52's bracket rule, and the push branch it kept alive is dead (2026-08-31)

@PLAN52's rule is a blanket requirement on the SPELLING — `vector += elem` is refused whatever
the element's type — and the ambiguity check asked it of an unpeeled source. So a `τ?` was not
recognised as an element and slipped past: `d.c += n` with `n: integer?` was accepted where the
dense `d.c += 9` is refused, making the `?` spelling of one statement MORE permissive than the
plain one (loft#1223).

**Not a design call, and the precedent is in the same function.** @PLN25 met this axis once
already on the DESTINATION — *"matched on the target's STORAGE, so a `vector<T>?` is refused the
same way and gets the same cure"* — so the answer for the SOURCE is the same reading one position
over: `τ?` occupies τ's storage plus one reserved null, and nullability does not decide whether a
bare element is ambiguous with a concat. The fix is `s_type.base()` in the two places the check
compares, mirroring the `f_type.base()` @PLN25 added.

**The vector single-element push is now measured UNREACHABLE, and deliberately kept.** Its one
corpus caller was exactly the un-bracketed nullable element this refuses; a re-run of the same
env-gated probe over `tests/scripts`, `tests/docs`, `tests/lib`, `default`, `examples`, `doc`,
`bench` and `tools` reports zero. It is unreachable by argument too — for a `Type::Vector`
destination an ELEMENT is claimed by this refusal, a VECTOR by the concat branch, and anything
else by loft#1215's classifier. It stays because the failure modes are not symmetric: a dead
branch costs a reader's attention, while a wrong deletion drops the shape through to the generic
path, which for a collection destination emits no write — the precise failure loft#1221 was. The
argument rests on the ordering of four checks, and this same branch was called dead once before
on a reading a measurement then contradicted. The reasoning is at its head; delete it behind a
fresh probe.

**The cure is under-diagnosed and that is filed rather than folded in.** `d.c += [n]` stores the
null into a dense `vector<integer>` with no diagnostic — the whole vector-literal family is
silent, including `v: vector<integer> = [n]` and a struct constructor, on both backends and on
2026.8.0. So closing this moves that reader from warned to silent until loft#1232 lands. The
trade is right — a rule violation should not ship to preserve a warning the correct spelling
ought to carry too — but the gap is named at the guard and in the issue so it is not rediscovered
from our own diagnostic.

Two guard cells moved to the bracketed spelling with their assertions unchanged, which is what
says the SPELLING changed and not the store: loft#1210's nullable-element axis cell and
loft#1215b's. Each `@EXPECT_WARNING` that went with them was removed for loft#1232's reason, not
because the store changed, and both files say so.

### The `+=` routing table is total: every collection append routes or reports (2026-08-30)

`towards_set`'s `+=` handling is a chain of route branches, each testing a hand-spelled
condition, with no site deciding that an append is unrouted. So the chain was neither exclusive
nor total, and both failure directions had shipped.

**Over-claiming (loft#1215).** The vector single-element push is gated on `!Insert(code)` and
compares `s_type` with `elm_tp` nowhere — the two branches that DO compare (`+= elem`'s ambiguity
diagnostic and the concat branch) both return before it, so only mismatches arrive and it wrote
them raw as one element of the element type. A `float` read back as its IEEE-754 bits, a
`boolean` as `8705`, a `text` panicked `database/allocation.rs`, a struct source and a
`vector<text>` element each ended in a SIGSEGV, and `--native` emitted Rust that would not
compile (E0308) for all of them. A keyed destination has no catch-all, so the same source fell
past every branch to a statement emitting no write, with `len` reading 0.

**Under-claiming (loft#1221).** Three admissible sources reached no route: a record VARIABLE at a
keyed FIELD (the record branch is gated on `matches!(code, Value::Insert(_))`, a vector's
requirement); a VARIANT at a vector over its enum (the ambiguity check asks `is_equal`, which
reads `Reference(Named)` and `Enum(Tagged, …)` as unrelated, and the generic path then grew the
vector by three); and a whole keyed collection at another of the same type.

**Cure — one classifier.** `Parser::append_source` (`parser/vectors.rs`) answers which of three
shapes a `+=` source is against its destination — `Whole`, `Element`, `ElementVector` — so the
fourth answer, `Unrelated`, becomes expressible. It is asked at the single point where every
destination kind and every route are still in play, beside `(N-Store)`'s check and for the same
reason: the push, the concat, the keyed fill and the record routes are all downstream, so a check
at any of them is one more copy of a question this file already asks in four places.

Its element test goes through `Parser::can_convert`, the predicate that already answers *"may
this value satisfy that slot"* for arguments, returns and struct literals. That delegation is
load-bearing rather than tidy: the element type has more than one SPELLING — a nullable record is
`Reference(d)` where a keyed kind's `content()` reads it and `Enum(d, true, …)` where a vector
carries it, and `(C-Var)` makes a variant satisfy its enum — and a fresh `is_equal` refuses two
working corpus programs. `can_convert` was missing `(C-Var)`, so the rule was added there rather
than spelled a fifth time.

`holds_element` answers FALSE for an `Unknown` on either side, and that guard is the whole
difference between a validator and a refusal: `can_convert` answers TRUE for an unknown
`test_type` — right for a validator, which must not report a generic body's placeholder as a
mismatch — but read as *"this IS an element"* it turns every unresolved element type into a hit.
A struct-enum's collection field resolves lazily, so `j.xs += [Item { … }]` asked while `xs` was
still `Unknown` earned @PLAN52's ambiguity refusal with the brackets already written
(`tests/scripts/977-struct-enum-collection-field-write.loft`, caught by the corpus sweep).

**Blast radius measured, not argued.** An env-gated probe at the check reported every append the
classifier calls `Unrelated`, over ~2000 `.loft` files (`tests/scripts`, `tests/docs`,
`tests/lib`, `default`, `examples`, `doc`, `bench`): **one hit**, an archive probe appending a
`float` to a `vector<single>` in a file that already failed on a later line. A second sweep after
the ambiguity check was rederived reports only files that already expect that message.

**The push branch is NOT dead, and that was measured rather than reasoned.** With the refusal in
place a vector's push should be unreachable — `Element` is refused earlier by the bracket rule
and `Whole` is claimed by concat — but a probe at its head across the same corpus reports one
caller: a nullable ELEMENT source, peeled by `(N-Store)` after the ambiguity check has already
run. That spelling bypasses @PLAN52's bracket rule its dense twin obeys, filed as loft#1223.

**Not fixed here:** loft#1222 — a `trie<T[k]>` field whose element struct is declared LATER
emits `db.trie(t, "k")` before `t` is bound (E0425, `--native` only). It reproduces with a
struct-literal source, whose route is untouched by either fix, and `hash` / `sorted` / `index` /
`vector` all tolerate the same forward reference.

### A `?` on an assignment place discharges the READ and writes through to the place (2026-08-30)

`place? op= e` says two things — write `place`, and read the type's default when `place` is
null — and the second half was eating the first. The left-hand side lowers to the same
null-check the expression form uses, which is a VALUE and re-evaluable, so every writer that
took it for the destination wrote somewhere else. Five faces, one cause, all silent and
identical on both backends: a vector field built the appended record INTO the destination and
appended the destination to itself (`b.d? += [r]` on a one-element field read len 4); a null
place threw the write away and stayed null; `linked-group` maintenance compares the written
place structurally, so the vector's keyed sibling received nothing; a `text` place reached
codegen with no variable to write and took the compiler down; and a scalar place was refused
outright as *"Not implemented operation + for type integer"* (loft#1205).

**Cure: peel the place back out of the discharge at the assignment dispatcher, and seed it
with the default when the place is one that propagates.** The peel is what makes every form
below see the field or local it is really writing, and `(E-Asgn-Compound)` then holds for this
place spelling too. It is not the whole fix, because the `?` does not cost the same everywhere:
a COLLECTION's own `op=` already reads through the discharge — appending to a null collection
builds the empty one first — while a scalar or `text` PROPAGATES, so for those the read is
discharged by seeding the place with its default when, and only when, it is null. `x? += 3` on
a null `x` is therefore 3, which is the accumulate-from-the-zero idiom and the whole reason to
write the `?`. Only the postfix `x?` peels: an explicit `(a ?? d)` names two values and no
place and stays refused. The seed reads the place twice, so it is built on the place the
@PLN102 F2 hoist has already bound rather than on the spelling the author wrote —
`w[idx()]? += 1` calls `idx()` once, where off the original spelling it called twice and read
one element while writing the next. `operational.md` states it as `@FR-E-Asgn-Discharge`.

### `(N-Store)` reaches a collection literal's elements (2026-08-31)

`v: vector<integer> = [n]` with `n: integer?` stored the null and `v[0]` read it back, silently,
on both backends — as did `d.c = [n]` and `D { c: [n] }`. `(N-Dense)` says a `vector<t>`'s
elements are non-null unless written `vector<t?>`, and the rule was enforced at the scalar seam
and the append seam and nowhere inside a literal (loft#1232).

The check goes at `parse_item`, where each element's type meets the declared element type, and it
asks `n_store_violation` — the same home the other two seams use, so the three cannot drift. One
point covers the typed local, the field assignment, the constructor field and nested literals.

**Held to WARNING even at the narrow widths the shared split escalates.** That escalation is
right about the slot and wrong about the moment: this seam was silent, so refusing at it
retro-breaks working code. Measured on the whole registry — `assets 0.2.0` writes
`bp += [0 as u8?]`, whose value is never null, and the gate went 42 pass → COMPILE-BREAK.
`n_store_violation_inner` carries a `never_error` flag for exactly this one caller.
`formal/types-history.md` records it as `D-Null-Elem`, opened and closed — and it came from
outside the bound that doc's own `OPEN: 0` states (*"for the DIRECT store"*).

### One home for what a `null` looks like at a parameter's type (2026-08-31)

`c += null` ran on the interpreter and had never compiled on `--native`: the generated Rust was
`let v = (()); … set_int(…, v)`, and rustc refused the program at `integer`, `float`, `single`,
`character`, narrow-int and struct element types. `arguments 0.2.1` depends on the construct, so
a signed registry package built on one backend only (loft#1234).

`write_typed_null_in` is the one home for that question, and the two argument-emission paths each
re-spelled a SUBSET of it by hand; every type the subset omitted fell through to the generic
expression emitter, which renders `Value::Null` as `()`. The subsets had already drifted from each
other — the template path's enum arm claimed the struct-enum spelling `Enum(_, true, _)` and
answered `255u8` for a DbRef-backed parameter, which also made the reference arm's own
`Enum(_, true, _)` case dead. Both paths now ask the home. A third site, `OpCopyRecordEmitter`,
emits reference operands through the new `EmitCtx::emit_ref`, which asks it too; the runtime
`OpCopyRecord` already gave a null source its meaning (nothing to read, destination left absent),
so only the operand's rendering was missing.

### An un-discharged nullable appended to a collection warns and stores (2026-08-30)

`d.c += s` with `s: vector<integer>?` into a dense `vector<integer>` FIELD panicked the
interpreter writing a read-only const store, and `--native` emitted `set_int(…, v)` with a
`DbRef` for `v` — E0308, so the program could not be built. A KEYED destination took the same
value silently. Neither symptom named the statement: the interpreter reported a position inside
`default/05_coroutine.loft`, which the program does not use (loft#1210).

**The rule decides the severity, and it is not a refusal.** `(N-Store)`'s split is
REPRESENTABILITY (`formal/types.md`, per-type table): a hard error only where the null sentinel
collides with a real value of τ — the narrow widths `u8…u32`, and nothing else — and a WARNING
everywhere the null is representable-and-distinct, where the store compiles and runs. A
collection is out-of-band (`nullref`), so it warns. Measured against two siblings before
shipping: `d.c = s` already stores a nullable vector into a dense field and works, and
`return s` warns and works, so a refusal on `+=` would have made three operators disagree.

The issue's own reading pointed the other way — it takes the dense LOCAL as the reference route
and infers the FIELD should refuse. The local is refused by a DIFFERENT rule: @PLAN52 makes a
bare `local += elem` ambiguous whatever its source type. `1210b` pins both, and they report
different diagnostics.

**Cure: peel the source where `convert` already peels it for `=`.** `s_type` is what the routes
below read to choose between concat, single-element push and the keyed fill, and an `Optional`
matched none of them — so the value fell to the push, which writes what it is handed as one
element of the element type.

**Not fixed here: loft#1215.** That push site never compares `s_type` with `elm_tp` at all, so a
source matching neither the element nor the collection type is written raw — `float` stores the
IEEE-754 bits of the value read back as an i64, `boolean` stores 8705, `text` panics the
allocator, and `--native` rejects all three. A probe at its head found **no `.loft` file in the
repository reaches it** (all of `tests/scripts/` plus 176 files across `tests/docs/`, `default/`,
`tests/lib/`, `examples/`), so it serves no correct program and funnels the broken ones.

### A nullable collection appends a non-literal source (2026-08-30)

`n.v += src` on a `vector<τ>?` field was refused as *"No matching operator 'Add' on
'vector<integer>?'"*, and `n.h += src` on a `hash<τ[k]>?` field emitted no write at all — the
records vanished, `len` read 0, no diagnostic (loft#1207). A bracketed source was correct
throughout, which is the control that says the axis is the SOURCE shape crossed with the `?`
on the declaration, not the field.

**Two halves, both load-bearing, each measured so by reverting it alone against the guard.**
The keyed half is `keyed_field_kt` matching unpeeled, so every nullable keyed field fell to
its `None` arm and the two callers that gate a write on it emitted nothing.

The vector half is not at any of the assignment-path routing sites the issue named.
`vectors::is_collection` is `is_keyed(tp) || matches!(tp, Vector)`, and `is_keyed` gained its
`.base()` in `d1220a1b` while the `Vector` arm did not — so a `vector<τ>?` was the one
collection the predicate denied. `towards_set`'s collection interception asks it in **pass
1**, before any `!first_pass` route can claim the statement, so the append fell through to
the generic operator lookup and was refused a whole pass before the concat branch could see
it. Its doc asserted the two predicates "differ by that one variant BY DESIGN" while they
differed on two axes, and 6 of `is_collection`'s 23 call sites had already grown a hand-peel
at the call site — the shape a half-applied peel makes from outside.

**Not fixed here, and now separate: loft#1213.** An ABSENT (rather than empty) keyed
destination keeps its reserved-null marker into `OpFillKeyed`, so the fill writes against
`rec=4294967295` — length right, records reachable by no key, panic on the first lookup, both
backends. It reproduces on the parent commit through the shipped discharged spelling
(`n.h? += src`), so it is older than this fix; what this fix changes is that the bare
spelling now reaches it instead of dropping the records in silence. The vector twin is its
control: `vector_add_array` materialises an absent destination and the keyed path has no
equivalent step.

### An absent keyed field materialises on append instead of dereferencing its marker (2026-08-30)

A `τ?` collection slot holds `DbRef::ABSENT_REC` when the field was never constructed, and
`Store::collection_rec` is the one accessor that maps it back to `0` — absent and empty are the
same answer to *"which record holds this collection's elements?"*. All twenty of its call sites
were in `vector.rs`. The keyed family read its slots raw, so `n.h += src` on an absent
`hash<E[k]>?` field followed `rec=4294967295` into a store that has no such record: a panic on
both backends, byte-identical, where the vector field one declaration over was correct
(loft#1213).

Two sites, each proved necessary by a cell rather than by reading: the DEDUP lookup inside
`insert_keyed_copy` reaches `Stores::find`, which every keyed kind funnels through — stating the
absent test once above its kind dispatch fixes `sorted` and `index` and leaves `hash` still
crashing — and the table claim on the write side, `hash::ensure_table` / `hash::add`, which is
what materialises the destination the way `vector_append` always has.

The controls say the axis is the field's STATE: the same declaration constructed `{ h: [] }`, the
dense `hash<E[k]>` beside it, and an absent `vector<E>?` were all correct throughout.

### An explicit `??` coalesce is refused as an assignment target (2026-08-30)

`(E-Asgn-Discharge)` already said an explicit `(a ?? d)` names two values and no place and takes
no assignment at all. The rule was written when loft#1205's peel landed; the enforcing site was
not. `Parser::last_place_discharge` tells the postfix `x?` from the explicit `??` — the two build
identical IR — and only its TRUE branch had a site, so the explicit spelling fell through to the
pre-#1205 path and reproduced all four of that issue's wrong answers on the other spelling: a
present vector field appended to itself with its keyed sibling never re-indexed, a null one
losing the write, a `text` target an ICE, a scalar one an arithmetic message about the operator
(loft#1212).

The refusal sits at the one point every assignment form still shares a target, ABOVE
`assign_var_nr` — that is what separates the `text` face's diagnostic from its ICE, since the
text `+=` path mints a work variable before anything downstream can object.

Both halves read one predicate, `null_discharge_subject`: the peel takes the postfix branch, this
takes the explicit one, so the two spellings cannot drift into disagreeing about what a discharge
looks like.

### `const` binds through a discharge interior to an assignment place (2026-08-30)

`lhs_base_var` is the one home for *which binding does this write reach*, and it walks the place
looking for it. It had an arm for loft#980's variant-field guard `if` and none for either shape a
NULL DISCHARGE lowers to — the `ncc`/`ncr` temp block, and the bare-variable `if` — so a place
ROOTED in a discharge answered `u16::MAX`, no binding at all, and both of its readers took that
at face value. `validate_write` had nothing to check, so `(Const-Value)` never fired and
`h.i?.x = 99` mutated a `const` parameter in silence on both backends; the text-assignment arm
read the same answer as *"this left side names no variable"* and reported the file-scope-constant
message about code the author had not written (loft#1211).

**One home, three questions.** `null_discharge_subject` now answers what a discharge was applied
to, and both `lhs_base_var` and `(E-Asgn-Discharge)`'s `peel_place_discharge` read it. They were
two matchers for one shape, and they disagreed: the peel claimed ANY `if` on a left-hand side,
including loft#980's variant-field guard, whose then arm is the RECEIVER rather than a place. No
spelling was found that reaches it — a field target lowers to an `OpGet` call, never to the bare
guard — so this is the restatement removed rather than a second defect closed.

The boundary stays where `(E-Asgn-Discharge)` put it: a discharge that IS the target is that
rule's question, and one INTERIOR to the place leaves an ordinary write that must simply resolve
to its root.

### Appending to a `text?` struct field is no longer an internal compiler error (2026-08-30)

`n.t += "cd"` on a `t: text?` field was an ICE on both backends — the plainest thing the field
can do. Two sites decide a text `+=` and they read the type differently: the router peels the
optional (@PLN25 slice (c)) and sends the statement down the text-append path, while the site
that mints the temp the append writes THROUGH matched `Type::Text` unpeeled and answered "no
variable". The append then emitted a store to variable 65535 and the scope pass asserted on it
(loft#1206). One notion, two spellings.

**Cure: the minter peels too, and the temp is typed the way the field is.** The second half is
what the first uncovered: `--native` decides whether an append propagates a null from the
DESTINATION VARIABLE's static type, so a dense temp told it there was nothing to propagate and
native appended onto the null sentinel — `"\0cd"`, reported non-null — where the interpreter
left the field null. The temp holds what the field holds, null included, so it carries the
field's type and both backends read one fact.

### A nullable heap-record local releases what its reassignment displaces (2026-08-30)

`@FR-O-Latest` makes ownership a property of the latest assignment, and a `c: S?` reassigned
from a call kept every store it displaced — unbounded in a loop, both backends, values right
throughout, so only the leak channel spoke.

The dense twin is clean for the reason that names the defect: a `-> S` callee is handed a
`__retbuf` and fills the store the local ALREADY owns, so nothing is displaced. A nullable
RECORD return gets no such buffer — `-> S?` is a synthetic `__nullable<S>` with its own
delivery, and giving it a buffer as well leaks one record per call — so every call mints and
the caller owes the release. `vector<T>?` and `text?` are clean because both do reuse one
buffer (loft#1200).

**The free cannot be static, and that is the substance of the fix.** The local's first store is
normally an inline mint into a work-ref, so the local and that work-ref name ONE store; freeing
through the local double-frees it against the work-ref's own scope-exit free — latent
everywhere and an observable wrong answer where the local is returned. One static site cannot
separate the first iteration from the rest. The cure is a per-RUN witness: a boolean per
qualifying local, false at entry, set true only by a MINTING CALL, with the displaced-store
free conditional on it. That flag records SOLE ownership, which is strictly narrower than the
existing `owned_refs` fact — an inline mint into a work-ref is owned and still not solely
owned.

Two enabling halves beside it: `owned_refs` was keyed on an UNPEELED shape so a nullable local
was never tracked at all (`@FR-L-Null` — `layout(τ) = layout(τ?)`), and the free's gate wanted
ownership established at the current loop depth where this shape establishes it one level out.

Measured wider than filed: the loop is not the axis (straight-line leaks too) and neither is
the callee's spelling (a dense `-> S` call into an `S?` local leaks the same). Two shapes stay
open on `formal/ownership.md` D-own-16, which this narrows to the condition they share — the
assigned value READS the local it assigns.

### A mapped lambda's collection does not own the buffer it was delivered through (2026-08-30)

`@FR-O-Owner` says every heap store has exactly one owner, and `xs.map(|x| { [x, x + 1] })`
gave one store two: the caller allocates a single `__ref_N` delivery buffer, hoists it out of
the loop and reuses it, and the callee fills it and hands it back — so the per-iteration yield
slot IS that buffer, and reading it as an owner released the caller's buffer at the end of
every iteration.

The deciding fact was already computed, and the two type formers need opposite readings of it.
`return_adopts_fresh_store` answers *does the callee mint its own store, or fill the one I
passed?*  For a `Reference` its false case is safe alone, because a deep copy is interposed
and the slot cannot alias the buffer; a vector has no copy path — it is aliased to the
work-ref argument — so there false is exactly the aliasing case. The pairing that emits the
runtime-conditional `OpFreeRefIfDistinct(slot, buffer)` never reached the vector spelling
(loft#1201).

Two things the measurement corrected in the report. On `--native`, the default backend, this
is a WRONG ANSWER rather than a latent hole: the recycled buffer is appended to, so a `map`
asking for six rows of three answered one row with six elements, silently. And the
named-function control that made it look like a lambda question was clean only by accident —
its yield slot's dep was a callee ATTRIBUTE index resolved against the caller's VARIABLE
table, so adding two unrelated locals moved it onto a `text`. The real axis is the return
former: struct clean, vector broken.

### A captured record enum is not the caller's to take (2026-08-30)

`@FR-L-CapHeap` says a captured heap value is SHARED — the caller may read it, never take
it — and three deviations had already made that hold for a record, a collection and a
struct. The record ENUM is the second spelling of a struct-like heap store, and it kept the
old behaviour: `r = g(1)` on `g = fn(v: integer) -> Shape { cap }` adopted the captured
record, and the next iteration's rebind released it while `cap` still named it.

The cause was one arm above the machinery: `block_result` picks a return's delivery from a
chain of `else if`s keyed on the type former, and the record arm spelled `Type::Reference`
by hand — so `Type::Enum(td, true, _)` matched no arm at all, got no delivery, and
published an empty return dep, which is what `returns_borrowed_view` reads as OWNED. That
is the keyed-collection story of loft#1140 one former over. Opening the gate exposed three
more sites downstream that had specialised to the traffic it let through, and closing the
use-after-free without them would have traded it for a leak (loft#1202).

The boundary is the type FORMER, not the tail: every tail shape that reads the closure — a
bare capture, a field projected out of a captured holder, a capture on one arm of a join,
and a capture handed back from a lambda passed inline to `map` — was the same fault, while
every shape that does not read the closure was already correct. `--native` answered
correctly throughout, which is what kept it out of sight.


### A format spec tunes what the value renders as, whatever its type (2026-08-29)

`@FR-F-Spec` tunes ⟦v⟧ and `@FR-F-Render` says what ⟦v⟧ is per type, so the two COMPOSE —
there is no type whose rendering a width cannot pad. Two families dropped the field-shaping
half in silence. `OpAppendCharacter` takes the accumulator and the value and nothing else, so
a `character` lost the whole spec (loft#1165); `OpFormatDatabase` has room for the two
`db_format` bits, so a vector, struct or record-enum kept `#` and `:j` and discarded width,
alignment and pad token (loft#1166). Both backends agreed, so neither had a differential
oracle, and the L9 escalation directly above the dispatch already stated the rule they broke
(*"a specifier that can never have any effect on the value type is always a bug"*) while
asking it only of `text` and `boolean`.

**Cure: render into a scratch text with the padding removed, then format that text with it.**
The composition is what the rules say, so it reaches every such type at once rather than
widening one op signature per family across both backends — which is what both issues
predicted the fix would need. The flags that choose the RENDERING stay on the inner call;
only the field-shaping ones move out. `formatting.md` now states the composition, and the
edge it decides: a null character renders as nothing, so a width pads a full field.

Guard `tests/scripts/1165-a-format-spec-tunes-every-rendering.loft` asserts each field by
LENGTH as well as content, and its controls are the `text`/`integer` arms the fix routes the
others through.

### A number that ends a scan is recorded, so a look-ahead can be undone (2026-08-29)

A number that ends at a `..` (or a field `.`) emits TWO tokens from one scan: the lexer
returns the NUMBER and QUEUES the follow-up in the replay buffer. Only the queued token was
recorded — `cont()` remembers when `link == memory.len()`, which a queue makes false — so the
number was the one token the buffer did not hold, and a look-ahead that reverted over it
replayed `i`, `+`, `..`. The `+` was left with one operand: `(s[i + 1..])` was refused with
*"missing argument for parameter 'v2' of `OpAddInt`"* (loft#1164). `LOFT_TRACE_LEX=1` prints
the replay sequence and shows the missing integer directly.

The number is now inserted BEFORE the queued token and stepped over, so the live sequence is
unchanged and the buffer says what was read. Four conditions had to hold at once — a
look-ahead (the tuple-literal classifier, which runs only for a parenthesised expression
assigned to a plain variable), a slice, and a number immediately before the `..` — which is
why `s[2..]`, `s[i..]`, `s[i + j..]` and every unparenthesised spelling were unaffected. The
`.` queue beside it (`n.v.0.0`) takes the same path and was latent; one home covers both.

`lexer::test::link_revert_replays_a_queued_number_split` asserts the replayed sequence at the
mechanism, beside the existing `link_revert_repeatable_same_region`.

### The `data_ptr` invariant is stated once, where the pointer lives (2026-08-29)

Ten `unsafe` derefs of `State::data_ptr` each carried their own SAFETY note in seven wordings,
two of which named a mechanism that does not exist (*"cleared at exit"* — nothing clears it;
`execute_argv` stores its `&Data` and the only null written is the struct initialiser). The
`is_null()` guard each site carries covers "no program has run yet" and a parallel worker,
and says nothing about whether the `Data` is still there. What makes every deref sound is the
CALLER: a `State` must not outlive the `Data` it was run against, which both local-owning
callers get from drop order by declaring the `Data` first. That is now written once on the
field, and the sites cite it. Found via the `rust/access-invalid-pointer` code-scanning alert,
which is a false positive as a memory-safety finding and was right about the comment.

### 81 assertions the corpus never ran, and the wrong line one of them reported (2026-08-23)

**Instrument.** `LOFT_TRACE_ASSERTS=<path>` appends `file:line` for every `assert` that
EXECUTES, from `n_assert` — the interpreter's implementation and the one a `--native`
binary links, so one setting covers both backends and every process a suite spawns.
Diffed against the `assert(` sites in the source it names the assertions a suite contains
and never runs, which no channel a suite reports can distinguish from passing ones.

**Result over `tests/scripts`: 9 722 sites executed, 81 never.** Three mechanisms:
a firing `@EXPECT_ERROR:` stops the whole file (52 — including all 21 of
`1067-lambda-expected-type.loft`'s positive half, in a file whose own header says the
negative cell exists so a compiler that stopped checking could not pass it); a file with
`main` runs only `main`, so every other zero-parameter function is dropped (21, in
`05-enums.loft` and `06-structs.loft`); and 8 deliberate. A fourth, native-only:
`native_scripts` skipped on `src.contains("@EXPECT_ERROR")` over the whole source, so five
files — 79 assertions, `93-vector-advanced.loft`'s 49 among them — left that suite because
a comment in each *mentioned* the tag while recording the file had STOPPED being a refusal
case. Both runners now read one `common::expect_tag`; native 801 → 806 scripts.

**The live bug it found.** The trace records the position the COMPILER injected, and every
assert in `685-mutated-scalar-param-capture.loft` traced exactly seven lines early
(`50-tuples.loft`, five). `Lexer::to` moves the reporting position without moving the read
cursor, and the tokenizer keeps incrementing it — so a seek never undone shifts the caret,
runtime spans and injected `assert` lines for the rest of the file by one constant.
`parse_function` wraps its warning passes in a save/restore whose comment states exactly
this hazard; `check_ref_mutations` (needless-`&` / `needless-const-parameter`) runs
eighteen lines above that save. A failing assert on line 184 printed line 177's source —
itself an `assert` — under this one's message, on both backends, with no other signal.

**Fix at the chokepoint, not a second save/restore:** `to()` records where it seeked FROM
and the next token scanned from source restores it, so a missing restore costs the one
diagnostic it was made for. A file switch clears the pending seek — without that the first
token of a `use`d file inherited the previous file's line (`88-imports`, `850*`, −13 to
−20). Guard `runtime_warnings.rs::a_seek_to_a_warning_site_does_not_shift_later_positions`,
proven able to fail.

**Ratchets** (`tests/wrap.rs`, both proven able to fire):
`a_refusal_file_carries_no_runtime_assertions` and
`every_assertion_is_reachable_from_the_entry_point`. Both are under-approximations by
construction and allow the documented dual guard (`432b`, `751`). Full account:
[QUALITY.md § 81 assertions the corpus contained and never ran](QUALITY.md).


### A Join whose arms each own a store: one leak and two silent wrong answers (loft#1078, 2026-08-23)

**Symptom.** `fn pick(c) -> S { w = S { a: 7 }; if c { S { a: 9 } } else { w } }` retained
one record per call — the value was right, only the ownership was wrong, so a single call
looked clean and only a loop showed it. `loft_planet` reached ~16,000 retained records per
planet, and four planets exhausted the 65,535-entry `store_nr` table.

**Cause.** `w` is renamed onto the hidden return buffer (NRVO), so the `else` arm delivers
the buffer and the `if` arm delivers a different store. `scopes::free_vars` reaches this
class through three legs, and the one covering *"several owned candidates, one winner"*
excluded every ARGUMENT. Right for a user parameter, which belongs to the caller; wrong for
the promoted buffer, the one argument that is really a local this function minted.
loft#1022 had already written that carve-out down and applied it inside its own gate,
noting that loft#688's leg "cannot claim it here because it excludes anything in
`sources`" — the sibling leg three lines up needed the identical sentence.

**Two more, found by moving the axes the report pinned** (return position and arm count),
both `silent-wrong` and both IDENTICAL on the two backends, so the interp-vs-native
differential was structurally blind to them:

* **Two owned locals** (`if c { u } else { w }`) — the first is renamed onto the buffer and
  the second's copy leg emits `OpDatabase(buf); OpCopyRecord(<tail that reads buf>, buf)`.
  The re-mint destroys the store the copy is about to read, so `u` answered a zeroed record.
  A three-arm `match` broke only its FIRST arm, which named the RENAME as the mechanism.
  New verdict `RetPromotion::SkipJoinArm`: a named local is not deep-copied into a buffer the
  tail reads — it stays a plain local, and the conditional free above releases the loser.
* **Bound, then returned** (`r = if c { … } else { w }; r`) — loft#848's class one arm over.
  `parser/objects.rs`'s value-position `Object` arm is `!first_pass`-guarded, so it mints on
  pass 2 only; on the shared `__ref_N` counter it was handed the name pass 1 had left on the
  return buffer, and `return_buffer()` resolves that buffer BY NAME. It now draws from
  `__ref_p2_N`, as loft#848 already made its sibling arm do.

Collections and text were measured clean on the same shape — each carries its own
aliasing-aware delivery — so the guard is scoped to the record path that re-mints.

**Opt-out.** `LOFT_NO_P2_OBJECT_WORKREF=1` restores the shared counter: the A/B on one binary
and the first bisect step for a wrong value out of a struct literal in value position. It is
the THIRD independent guard on the collapse `LOFT_NO_A1B` and `LOFT_NO_WORKREF_STEPOVER` also
guard, which is why `oracle_flags_the_a1b_wrong_plan` now disables all three to have a defect
to catch — that test going green is how the independence was measured rather than argued.

**Guard.** `tests/scripts/1078-join-arms-that-each-own-a-store.loft`, 10 cells on both
backends, falsified on a pristine worktree at `f7a57124`: the value cells by assertion, the
leak cell by the wrap leak gate (`1 store(s) leaked at program exit: kt=78 S1078×39`).
`formal/ownership.md` gained D-own-7, opened and closed the same day.

Also: the store-table-exhaustion panic advised `LOFT_STORES=summary`, which has never been an
accepted value — it now names `timeline`, the one that answers the leak-vs-working-set
question. Two stale doc-comment echoes of the same non-value corrected with it.

### A browser page can read a store out of its own filesystem (@PLN146 F4, 2026-08-22)

**Symptom.** `store_load` on a `--html` page answered `false` for every path — politely,
no panic, nothing to act on. So "a pack IS a loft store" was true on desktop and
HTTP-only in a browser, and a page could not carry its own assets at all.

**Cause.** `Store::load` reads via `std::fs::read`, and `Stores::load_path` gates on
`std::fs::metadata`. `wasm32-unknown-unknown` has no filesystem, so both fail there. The
page's own tree — which `doc/loft-fs.js` already serves from `globalThis.loftBaseFS`, and
which `png_store.rs` already reads for PNGs — is reachable only through the
`loft_host_fs_*` bridge, and the store loader never called it.

**Fix.** `store::image_bytes` and `store::image_at_least`: `std::fs` first, the host-FS
bridge second, behind one `host_fs`-gated helper so the two read as a single path rather
than a pair of cfg forks. Native is unchanged by construction — `metadata` succeeds, the
host arm never runs — and 98 store tests confirm it. On wasm the existence probe costs a
read and the load costs a second; one code path is worth that against bytes the page is
already holding.

**Gate.** `tests/html_page_store.rs` — the same emitted wasm run twice, differing only in
whether the store is in the page tree: `load=true read=true` when carried,
`load=false read=false` when not. Proven red by making the host arm answer `None`. The
absent-file half is the one that matters: a loader reporting success for a file nobody
supplied would pass the positive assertion and be worse than the refusal it replaced.

**And the other half — `[[embed]]` (2026-08-22).** `--html` now seeds `loftBaseFS`
from the manifest: `[[embed]] path = "assets/game.pack"` (plus an optional `source`,
where the bytes are on the build box) puts the file in the page under `/` + `path`,
which is what `loft-fs.js` resolves the program's own relative string to. So one
`store_load(q, "assets/game.pack")` reads the pack on the desktop and in the page.
`src/html_embed.rs` owns validation and the emission, `src/manifest.rs` parses the
section, and `Data::declared_embeds` carries a library's — the same three-part route
`[[font]]` takes, and a library's `source` resolves against the LIBRARY.

**Refused, before the wasm build:** a `path` that is not relative and in normal form
(`/abs/x`, `./x`, `a/../b`), a `source` that is not there, and one name declared from
two files. Each would otherwise be carried faithfully under a key the program never
asks for — `store_load` answers `false`, the page draws no art, nothing says why.
That is F5's silent failure in a new place, which is why the spelling is strict.

**Both are resolved from beside the PROGRAM**, not the manifest — loft resolves any path
a program passes relative to the program file, so `assets/game.pack` in `src/game.loft`
is `src/assets/game.pack`. Rooting at the manifest put a *different file* in the page
under the key the program asks for; measured as a desktop run answering `load=false`
while its own page answered `load=true`. A library's declarations keep the library's
root. Found only by moving the fixture off the package root, which the first one had
pinned.

**Also fixed, from the same probe.** `loft build` reported *"asset `X` ran but a declared
output is missing"* and then `asset `X` ✓` on the next line. A step that names its
`outputs` promised them, so it now FAILS, and names the likeliest cause: a `run` script
resolves its own paths against the script, so `scripts/pack.loft` writing `assets/x`
writes `scripts/assets/x`.

**Gate.** `tests/html_embed.rs`, four tests. The browser one is the invariant: a page
declaring a **nested** pack prints exactly what the desktop run of the same source
prints (`load=true a=7 b=41`), and the control is that same page with the seed
statement stripped (`load=false a=-1 b=-1`). Proven red by emitting the file under its
base name — bytes carried, key wrong, which is the defect the shipped loader gate
could not see because it only ever used a flat path. The library test carries a decoy
of the same name in the consumer's own directory, so it measures which root `source`
resolved against rather than whether a file was found.

### A browser page can declare the font it draws with (@PLN146 F5/F6, 2026-08-22)

**New.** `[[font]]` in `loft.toml` — `family`, `native`, and one of `url` (a font file we
serve) or `stylesheet` (a provider's). `--html` emits an `@font-face` or a `<link>` per
declared source and, ahead of `loft_start`, awaits `document.fonts.load` for every
declared family. A library's declarations reach a consumer's page by the same route as
`[wasm.bridge] host_js`. `src/html_fonts.rs` owns validation and both emissions;
`src/manifest.rs` parses the section; `Data::declared_fonts` carries a library's.

**Refused, before the wasm build:** a `family` that differs from the base name of
`native`. The browser sees only that base name (`gl_load_font("fonts/Foo.ttf")` reaches
the page as `Foo`), so the drift means the page registers one family while the program
asks for another — text draws in a generic face and nothing says so. Also refused: an
empty family, characters that would break out of the page's CSS/HTML, `url` and
`stylesheet` together, and one family declared twice with different sources.

**Fixed on the way — `familyFor` resolved backwards.** The bridge decided a font's CSS
family by asking `document.fonts.check` whether the page had it. That question has no
answer: `check` is **true** for a family nothing declares (nothing unloaded matches) and
**false** for an `@font-face` that is still loading. So a page that declared nothing took
the exact-font branch, and the one page that had brought its own font took the *generic*
branch — cached per handle, so a single early `gl_load_font` locked that handle to
`sans-serif` for the run. The name heuristic (`mono` → `monospace`) was unreachable in
every other case. `familyFor` now answers `"Requested", <generic>` and lets CSS choose.

**And the silence is closed.** `gl_load_font` measures whether the family actually
resolved — a family the browser has overrides both `monospace` and `sans-serif`, so the
two measure the same; one it does not have follows each and they differ — and
`console.warn`s once when it did not, naming the family and the generic that will draw.
Every resolution is recorded on `globalThis.loftFonts`.

**Gates.** `tests/html_fonts.rs`: the three sources resolve to the requested family in
headless Chromium (proven red with the head block suppressed); the same page against a
font server delaying font files by 800 ms still resolves, while the same page WITHOUT
the await leaves both brought families unresolved (the control, asserted every run); a
real `loft --html` page carries the block with the await ahead of the `loft_start` call;
and a drifting family is refused with no page written.
`tests/data/slow_font_server.py` is the throttle.

### The viewer smoke test read a viewer it had not started (2026-08-22)

`tests/viewer_markdown.rs` spawns the viewer with `LOFT_VIEW_PORT=18765` — deliberately
not 8765, which is what a developer's `make view` takes — and then polled **8765** for
its listener, with two comments saying the viewer did not support the env var. It does.
So the test connected to whatever viewer happened to be up on 8765 and rendered its
pages, in another checkout or from an earlier session; on a box where 8765 was free it
waited the full 240 s and failed. Both halves now use `VIEWER_PORT`: 0.66 s, and it
examines the process it spawned.

### A paged load refused every entry type with an enum field (@PLN146 F2, 2026-08-22)

**Symptom.** `store_load_key_text` (and its `_key` / `_range` / `_prefix` siblings) refused
a `hash<Blob[bl_key]>` whose element carries `bl_kind: BlobKind`, loading nothing and
reporting *"has a field the working-set copy cannot relocate (a `vector<text>` /
`vector<vector>` element pointer would dangle)"* — of a type with neither.

**Cause.** `Stores::is_copyable_field` has arms for `Parts::Struct` and
`Parts::EnumValue` (a struct-enum VARIANT) and had none for `Parts::Enum` (the enum
itself), so every enum field fell to the `_ => false` catch-all. The message was a
second, independent defect: `not_copyable_reason` named `vector<text>` /
`vector<vector>` unconditionally rather than reading which field had failed.

**Fix.** An enum's value is a tag byte stored inline, and `copy_claims` recurses on the
SAME position for a struct-enum's payload — so an enum relocates exactly when every
variant it could hold does, and a payload-free variant (`u16::MAX`) always does:

```rust
Parts::Enum(values) => values
    .iter()
    .all(|(vt, _)| *vt == u16::MAX || self.is_copyable_field(*vt)),
```

`not_copyable_reason` now finds the first field the predicate rejects and names it with
its type (`` `b_names: vector<text>` ``), falling back to "a field" for an element type
that is not a record.

**Guard.** `tests/store_persist_loft.rs::paged_load_carries_enum_fields_both_backends`
over `tests/scripts/store_paged_enum_field.loft`. It asserts the VALUES — plain variants,
a `Box { bw, bh }` payload, a `Line { ln }` payload, the byte vector and a trailing scalar
— because a copy that is accepted and wrong is the worse of the two failures. Red on the
pre-fix binary with the refusal quoted above.

### `store_load` looped for ever on a pack-shaped image (@PLN146 F1, 2026-08-22)

**Symptom.** `store_load` never returned — no diagnostic, no bound, on a call that only
reads a file. `LOFT_NO_COMPACT_ON_LOAD=1` made it return, which is what named the
subsystem.

**Cause.** `Stores::rebuild_into_scratch` claims the root record out of a fresh
`Store::new_in_use(root_words + PRIMARY + 1)`, raw-copies the source root block over it,
and then re-asserts the header — as `root_words`. `Store::claim_block` only splits a block
more than a third larger than the request, and here it never is (`root_words + 1` against
`root_words`), so the claim had handed out the WHOLE arena. Writing the request back
shortened the block by the word it owned, and that word became a block whose header reads
zero. `Store::claim_scan` walks the chain with `pos += abs(claim)`, so it added zero to its
position for ever. The `debug_assert_ne!(pos, last, "Inconsistent database zero sized
block")` that states the invariant is compiled out of the loft library in every profile
(`[profile.dev.package.loft] debug-assertions = false`), so no build had it armed.

**Fix.** Two, at two different chokepoints:

- `rebuild_into_scratch` reads the destination's own header after its claim and restores
  THAT, so a raw block copy can no longer change the size a block says it owns.
- `claim_scan` cannot fail to advance: a zero header reports the malformed chain on stderr
  once, and the walk treats the chain as ending there so the store grows past it. A
  corrupt chain now costs a few leaked words and a message instead of a hang.

**Reachability.** It needs a container of several keyed collections (three or more root
words is enough for the claim to decline the split) plus values big enough that a later
claim misses the free list and reaches the linear scan. That is exactly an asset pack, which
is how @PLN146 F1 found it.

**Guard.** `tests/store_persist_loft.rs::compaction_on_load_returns_both_backends` over
`tests/scripts/store_load_compaction_scratch_header.loft`, on both backends, with its own
`LOFT_TIMEOUT` — the failure under test is an unbounded loop, so leaving it to the suite
watchdog would report it as a slow test. Reverting either half alone was checked: the
producer fix alone is silent and correct, and the `claim_scan` guard alone turns the hang
into three stderr lines and a correct answer.

### A `par` worker read the element, whatever the call site wrote (loft#1060, 2026-08-21)

```
$ loft --interpret r.loft            # identical on --native
b=100      <-- for a in rows par(b = takes_int(a.n), 2) — `tag * 100`, `n` never read
c=2        <-- for a in ns   par(c = dbl(other), 2)     — the element, `other` never read
```

`parse_parallel_worker_fn` parsed the worker call's first argument into a `dummy` and
dropped it, because the dispatcher passes the element itself. Nothing checked that the
argument named the element, and nothing checked that the worker's first PARAMETER could
take it — so `takes_int(a.n)` handed the worker the whole record and it read the first
eight bytes as its `integer`. The answer depended on struct FIELD ORDER, which is how it
stayed hidden: put `n` first and the same program is right.

**The differential oracle could not have caught this.** Both backends agreed — they share
the parser. `formal/concurrency.md` `(C-Det)` names the SEQUENTIAL loop as the standard,
and the sequential `b = takes_int(a)` refuses this program outright (*"expected integer,
got Sq on argument 1"*). So the accept/reject sides of the two forms had drifted, in a
place a two-backend comparison structurally cannot see. That is now written into the doc's
Conformance section beside the differential bullet.

The `(A)` kind check already sitting at this site had been narrowed deliberately — scalar
element paired with a collection first param — to stop false positives, which left the
REVERSE direction open, and the reverse direction is the one that reinterprets. The fix
takes the predicate the ordinary call site uses (`can_convert`, after an `is_equal`
identity test, because the element's type carries the dep list of the collection it came
out of and the declared parameter carries none) rather than adding a third hand-rolled
kind test to keep in step with the other two. Identity is compared by VARIABLE NUMBER, so
a loop that re-spells the element name (loft#915) is handled.

Four shapes refused, all previously silent: a constant, an unrelated variable, a field of
the element, and a worker declaring no parameter at all. Guards in
`tests/scripts/36-parse-errors.loft`; the legitimate forms — struct element, extra context
argument, `a.method()`, scalar element — are verified value-by-value on both backends.

### One cap, one guard — the call-stack limit stops meaning two things (loft#1058, 2026-08-21)

```
$ loft --interpret p.loft      $ loft --native p.loft        # the DEFAULT backend
depth=9999                     error: call stack overflow — exceeded 10000 nested calls
```

The same program, one call short of the cap, answered on one backend and halted on the
other. That was found while closing what #1058 filed — a *rendering* divergence, the last
halting fault still printed two ways: `--interpret` gave the loft diagnostic (`-->`, source
line, caret) and `--native` a hand-rolled line with no position block, its own frame layout,
and the depth count only on that side.

**The filed blocker was the wrong question.** The issue said converging the caret would cost
either a per-frame current-line slot on every native call or a caret pointing at the
declaration — "a decision about a shipped surface and about native call cost". Both options
took for granted that the position had to be the CALL SITE. It does not: the fault is a
property of ten thousand frames, not of a point, and the running function is what both
backends already know. Nothing was added to the hot path.

Two guards were enforcing one cap and counting different things:

- **`State::fn_call` tested a `call_depth` counter, not the stack.** The counter never
  counted `main`, and the coroutine paths truncate `call_stack` without touching it — so it
  drifted from the real depth in two independent ways. It now reads `call_stack.len()`, the
  quantity `cr_call_push` tests and `stack_trace()` reports on both backends, and the
  counter is deleted rather than corrected: there is no second number left to keep in step,
  and with it go its save/restore in `snapshot_checkpoint` and on the host-call path.
- **`cr_call_push` pushed the frame and tripped afterwards.** A refused call never runs, so
  its frame is not part of the stack the diagnostic reports — but native put it on top of
  the chain, one frame longer than the interpreter's, and named the callee where the
  interpreter named its caller. It now checks before pushing, as `fn_call` does, and reads
  the position off the frame below.

`cr_stack_overflow` is now three lines over `RuntimeError::stack_overflow(...).report_and_exit()`,
so the diagnostic, the frame block and the `in_lazy_driver` containment are the ones #1056
built. The depth moved into `RuntimeErrorKind::describe()`, which gives BOTH backends the
count, and it says `stack frames` rather than `nested calls` because `main` is one of them.

**Where the axes had to move together.** Self-recursion cannot see any of this: `deep` calls
`deep`, so caller and callee are the same name and every wrong reading looks right. Mutual
recursion separates the callee from the running function; a nested argument
(`runaway(helper(n))`) separates the refused call from both; and only a program sitting
exactly ON the cap shows that the budgets differed at all. The regression tests carry all
three, and `tests/oracle/32-stack-overflow-halt.loft` pins the boundary from both sides in
one program — 9 998 must answer, 9 999 must halt — because a budget that moves by one is
invisible to a test that only checks the side it moved away from.

### One fault, one rendering, whatever backend ran it (loft#1056, 2026-08-21)

```
$ loft --native p.loft            # the DEFAULT backend, before
thread '<unnamed>' (2466378) panicked at /tmp/loft_native_2466316.rs:966:18:
p.loft:1 plain assert
```

A failed `assert` reached the user as a Rust panic naming a generated temp file, where
`--interpret` printed a loft diagnostic naming their own source. `panic` next door was
already right, so the two explicit halt statements — one event — read two different ways.

The issue was filed with the convergence blocked on a decision, and the decision turned out
not to exist. `RuntimeError::call_chain` is hardcoded `Vec::new()` at both the `user_panic`
and `assertion_failed` constructors, so on `--interpret` and on plain `--native` there were
no frames to trade away; only the browser target's panic hook had any. What read as "this
costs the call chain" was a hole, and one three-deep `--interpret` probe says so in ten
seconds. Filing from a code reading rather than a measurement is what made it look like a
design call.

Four chokepoints:

- **`RuntimeError::render()`** — one renderer for the diagnostic plus the frames, called by
  `main.rs` (interpreter) and by `report_and_exit` (generated binary). Frames come from
  `State::current_call_chain` on one side and the native shadow `CALL_STACK` on the other,
  so the two backends agree by construction rather than by two spellings being kept in step.
- **`State::note_runtime_error_halt()`** — the three interpreter dispatch loops each carried
  their own copy of the halt check; now one method, which also BACKFILLS the chain. `assert`
  and `panic` are native fns and every `Stores`-side raise sees only `&mut Stores`, so none
  of them can reach a `State`; the dispatch loop is the first point that holds both.
- **`State::run_to_return()`** — ten textually identical worker / `parallel` arm / host-call
  dispatch loops folded into one, and not one of the ten checked for a fault. That is
  loft#1053's residue: a failed assert inside a `par` worker ran the worker's remaining rows
  to the end, and the frames it was finally reported with were the PARENT's — `main`, which
  is not where the fault happened. Naming the wrong function is worse than naming none.
- **`report_and_exit` takes a never-released lock** — a halting fault is the program's halt,
  so it is reported once however many workers reach it together (six rows over two workers
  printed it twice on `--native`, once on `--interpret`).

`assert`'s generated body now takes `panic`'s path:
`RuntimeError::assertion_failed(msg, file, line).report_and_exit()`.

Eleven differential cells (top-level and nested `assert` and `panic`, three `par` families,
a runtime-built message, two clean controls) are byte-identical on both backends, with the
comparator proved able to report a difference. Five regression cells in
`tests/runtime_errors.rs`, each shown to fail under a deliberate break of the mechanism it
names, and `html_panic_names_itself_and_its_loft_frames` now pins the diagnostic as well as
the frames, so a page that fell back to a bare Rust panic cannot pass it.

**A hazard the differential matrix missed because it held one axis fixed.** All eleven
cells ran outside a lazy-store driver. @PLN133 S8 decided a fault inside a DRIVER is
contained — the lookup answers null and the reason reaches `store_lazy_error` — and the
generated driver call runs under `catch_unwind`, so moving `assert` onto `report_and_exit`
made it exit the process where the interpreter contained it. The same probe showed `panic`
had been doing exactly that since it started using that path. Both now take the
`in_lazy_driver` bypass `cr_stack_overflow` and the crash-report hook already carried, and
unwind with the payload spelled the way the interpreter's contained-fetch spells it
(`<kind label>: <message>`), so `store_lazy_error` reads identically on both.

**The oracle had been collecting the evidence and discarding it.**
`tests/differential_oracle.rs` has captured stderr since it was built and compared it only
for the leak substring — so the channel that would have caught this the day
`31-assertion-halt.loft` was added was never asked. It is now a compared channel (leak line
filtered out: leaks have their own channel, and the native binary prints one only under
`LOFT_NATIVE_LEAK_CHECK`), with a positive control for it and one asserting that a leak
stays ONE divergence rather than also a stderr difference. Seven of the corpus programs
write to it, so it is exercised rather than agreeing by emptiness.

### A generic function is callable as a `par` worker (loft#1033, 2026-08-20)

```loft
pub fn idf<T>(x: T) -> T { x }
for e in [1,2,3] par(r = idf(e), 1) { … }   // error: 'idf' is not a function
```

The par path resolved the NAME to a def and then demanded `DefType::Function`; a template
is `DefType::Generic`. Resolving a name and never INSTANTIATING is the whole defect — the
refusal was the symptom of a missing step, not a rule being enforced. The same `idf`
resolved everywhere else in the same file.

Two halves, because a generic call site has two:

- **Instantiation** — from the ELEMENT type (the first parsed argument is skipped precisely
  because it names the element) plus the context arguments, which is the argument list an
  ordinary call would hand the same function.
- **The return type, on BOTH passes** — instantiation runs on pass 2 only, so pass 1 read
  the template's `T` and pass 2 the monomorph's `integer`, and the result variable's table
  entry carried the pass-1 answer forward: *"Variable '_discard_1' cannot change type from
  T to integer"*. `predict_generic_return_type` is the cross-pass contract an ordinary
  generic call site already uses for exactly this.

`instantiate_nested_generics` also had to learn the worker. A par worker does not ride in
the IR as a `Value::Call` — the parallel ops carry it as a d_nr INTEGER argument — so the
nested-generic walk could not see it and a template's worker stayed the template
(`--native`: `cannot find function n_idf`, since a template emits no code). It is
recognised by TWO facts together, never by operand position: the enclosing call is one of
the `n_parallel_*` family, and the integer is a d_nr already recorded in `par_worker_defs`
whose def is a template. Either test alone could match an ordinary integer that happens to
equal a d_nr; together they cannot.

⚠ **A generic worker inside a generic FUNCTION is REFUSED, on both backends, on purpose.**
`build_parallel_for_ir` picks a queue variant, an element and return size, a result
accessor and a re-wrap from the element and return types, and inside a template those are
the type VARIABLE; substitution rewrites the types and leaves the route behind. Measured
before the refusal existed: `--interpret` answered correctly (it dispatches by `d_nr`)
while `--native` failed with `non-primitive cast: DbRef as i64` — the buffer read with the
reference accessor at stride 12 and cast to the monomorph's scalar. That divergence is what
D-op-1 forbids, so this refuses on both rather than shipping it. Closing it means deferring
the route and replaying it per monomorph, the same shape as #1016 / #1020 / #1028 / #1032
but larger, since a route is a family of ops rather than one baked constant — loft#1040.

Guard: `tests/scripts/1033-generic-par-worker.loft`. Its load-bearing cells USE the
worker's result rather than discarding it — a cell that only counts iterations passes while
`r` is still `T`.

### A declared local may name a tuple with a nullable element (loft#1034, 2026-08-20)

```loft
c: (text?, integer) = ("c0", 3);   // error: cannot change type from (text?, integer)
                                   //        to (text, integer)
fn mk() -> (text?, integer) { ("c0", 3) }   // ...but the RETURN position accepted it
```

Two positions disagreeing about one type — `formal/tuples.md` D-tup-1's shape, and the same
cause: two specified halves whose COMPOSITION was not. @PLN25 `(N-Decl)` says storing a
non-null `τ` into a `τ?` slot is not a type change, and it peeled ONE `Optional`, at the
top, so it never saw a `τ?` sitting at a tuple POSITION.

Two halves, and neither subsumes the other — measured, by disabling each and watching a
different cell go red:

- **Typing** — `Variables::decl_accepts` asks `(N-Decl)` element-wise (and recursively, for
  a tuple inside a tuple), so the declaration is legal. It answers on pass 1, before any
  lowering; without it the declaration is refused outright.
- **Lowering** — a tuple target now reaches `convert`. `scalar_target` listed the types
  whose annotation drives a conversion and a tuple was not among them, so the literal was
  never converted against what the annotation asked for. Without this half the declaration
  compiles and a `null` ELEMENT stays a bare null instead of the element type's sentinel:
  `(null, 3)` stored the empty text (`h.0 == null` answered FALSE) and `--native` emitted
  `()` and would not compile.

⚠ **The `null`-element cell is what separates a fix from a silently-wrong one.** A test
carrying only non-null elements passes on the typing half alone, which is exactly the
half that makes the language answer wrongly. It is in the guard for that reason.

The RETURN position always converted — `convert`'s own Tuple arm walks the elements — which
is why it accepted the type all along. Routing the local through the SAME function is the
point: the alternative was teaching this site a second opinion about tuples, which is the
three-lists shape D-tup-1 collapsed.

Deliberately asymmetric: `decl_accepts` widens `τ → τ?` and never the reverse, so
`(text, integer) ← (text?, integer)` is still the `(N-Store)` violation it was. Verified
alongside a wrong element type and a wrong arity, both still refused.

Guard: `tests/scripts/1034-declared-nullable-tuple-element.loft`.

One defect surfaced and NOT folded in: on `--native`, `== null` on a `text?` tuple element
MOVES it, so a later read of that element does not compile (loft#1038). Pre-existing and
independent — it reproduces from a tuple RETURN, which this change never touched. The guard
binds such an element once, which is that issue's verified workaround, so it tests this
issue and not that one.

### A compound assignment is bounded by the range it DECLARES, not by how the range is spelled (loft#1030 + loft#1031, 2026-08-20)

The two residuals 447564a1 recorded and deliberately did not fold in. They are one guard
missing one axis each, in opposite directions:

```loft
l: integer limit(0,255) = 250;  l += 10;    // kept 260 — the u8 spelling clamped to 0
s.f -= 10;  // f: u32, was 5                 // wrapped to 2^32-5 — the u32 LOCAL clamped to 0
```

**loft#1030 — the guard read the width SPELLING.** `guard_narrow_alias_local` tested
`forced_size`, which only a narrow ALIAS sets, so `limit(lo, hi)` reached no guard on the
compound path at all. `formal/types.md` `(C-Int)` puts width INSIDE the conversion relation
— "an integer flows into another integer iff its range fits", with no separate width
authority — so `u8` and `integer limit(0,255)` are one range and a guard keyed on the
spelling could only ever disagree with itself. Not a design call; the rule already said so.

**loft#1031 — the store guard covers 1 and 2 bytes only.** `set_byte` / `set_short` carry a
`min` operand and substitute the range's low end when a write does not fit; the 4-byte
setters take no range and truncate. So a `u32` field wrapped where its local clamped. `i32`
was wrong the same way and the issue did not notice — which also closes the residual
447564a1 recorded (an `i32` FIELD answering `null`, because a wrapped value could land on
`i32::MIN`): clamping now happens before the store, so the sentinel is never written.

Both close at ONE seam — the composed value in `compute_op_code`'s caller, which every
compound assignment passes through while `to` is still a `Var`, a field read or an element
read and before the store-op dispatch. So the local, the field and the element cannot
disagree, and `compound_range` answers the range for either spelling in one place.

Teaching the four 4-byte opcodes a range was the alternative and was rejected: it changes
opcode signatures (and `fill.rs` is generated), where the guard is one rule in one place.
For the 1- and 2-byte widths the store guard becomes a backstop this path can no longer
trip — the value reaching it is already in range, clamping is idempotent, and `set_byte`'s
out-of-range return is discarded, so nothing is judged or reported twice.

⚠ **Plain `integer` stays unbounded on purpose.** 447564a1 measured that a guard clamping
every integer satisfies every other assertion in the file, so `integer` running past the
4-byte range is a live regression cell, not an oversight.

Measured on both backends, all three target kinds, both directions — and the sweep found
two cells neither issue named: `integer limit(0,70000)` was wrong in ALL THREE positions
(a 4-byte span, so neither the alias arm nor the store guard saw it), and it is fixed by
the same change.

Guard: `tests/scripts/1030-compound-range-both-spellings.loft`, written as local/field/
element TRIPLES so a fix reaching one target kind and not the others fails. Both halves
were falsified independently: removing the `limit` arm reds the limit cells, restoring the
Var-only target reds the field cells.

Two further defects surfaced by the sweep and NOT folded in — both are plain write-then-read
with no arithmetic, so neither is this guard: a `limit` element with a non-zero `lo` reads
back exactly `lo` too high (loft#1036), and a `limit` span wider than 4 bytes cannot hold a
value inside its own declared range (loft#1037).

### A generator call was back-patched at a one-byte opcode, and a generic never learned its `iterator<T>` (loft#1032, 2026-08-20)

Filed as one generic bug; it was four, and only one of the four is about generics.

```loft
fn main() { for y in h() { … } }                 // interp: subtract overflow in coroutine_create
fn h() -> iterator<integer> { yield 11; }        // NO generics needed

fn h2(v: vector<integer>) -> iterator<integer> { for e in v { yield e; } }
for y in h2([4,5,6]) { … }                       // --native: E0308, also no generics

fn g<T>(v: vector<T>) -> iterator<T> { for e in v { yield e; } }
fn o<T>(v: vector<T>) -> integer { c=0; for y in g(v) { c+=1; } c }
o([1,2,3])                                       // store corruption at SCALAR T only
```

**1 — the back-patch assumed a one-byte opcode.** A forward call emits its target as a
placeholder and `Codegen::calls` remembers where to write the real address once the callee's
body is generated. Both the recorder and the two consumers derived that spot as
`opcode(1) + d_nr(8) + args_size(2)`, but `state::emit_op` writes TWO bytes at or above op_code
255 and `OpCoroutineCreate` is one of those. The i64 landed a byte early, over `args_size`'s high
byte: codegen computed 24 and the bytecode decoded 61720, then `coroutine_create`'s
`stack_pos - args_size` underflowed. Measured exactly — the clobbered byte is the target's byte 0,
and the reported `to` is the target shifted down eight bits.

`calls` now records the address of the i64 operand ITSELF, so no consumer re-derives it. There
were **two** consumers with the same hardcoded `+ 11`: `byte_code_for` and `live_reload`'s
dispatch re-link. Changing the meaning in one and not the other is a real trap — it turned three
`engine_host_*` live-reload tests red until the second was updated, which is the measurement that
the duplication was load-bearing.

**2 — `--native`'s argument-hoist path was out of lockstep on the RETURN side.** A generator call
is wrapped in `alloc_coroutine(…)` to turn `Box<dyn LoftCoroutine>` into the `DbRef` every caller
holds. The hoist path (taken when an argument mutates a store — a vector literal does) duplicated
the call emission and applied the per-parameter coercions issue #366 put in lockstep, but not the
wrapper. A bare call went into a `DbRef` local.

**3 — `substitute_type` had no `Iterator` arm, in BOTH twins.** `Parser::substitute_type` and the
variable table's `Function::subst_type` each carried `Vector` / `Tuple` / `Optional` and stopped
there, so a generic's `iterator<T>` return kept the type variable — the caller's handle typed
`DbRef` on native (`expected DbRef, found Box<dyn LoftCoroutine>`) and the loop variable left at
`T`, unusable in a sum or a format string. `formal/interfaces.md` `(G-Mono)` names the RETURN
explicitly, so this was a deviation; the rule did not move.

**4 — `OpCoroutineNext` pairs a size with a channel, and only the type moved.** The operands are
`(channel_tag << 8) | byte_size`, both a function of the yielded type. A template lowering
`for y in g(v)` while `T` was the type variable baked the 12-byte DbRef channel; substitution
retyped the loop variable and left the pairing behind, so a scalar `T` read a 12-byte DbRef out of
an 8-byte slot and indexed off the end of the store. This is the same shape as loft#1016, #1020 and
#1028 — an operation whose choice depends on `τ` decided before `τ` was known — and is now the
FOURTH entry in that class in `formal/interfaces.md`. The decision moved into one home,
`coroutine_layout::next_operands`, which the for-loop lowering and the new per-monomorph
`retarget_parametric_coroutine_next` both ask; the retarget reads the generator VARIABLE's
now-concrete type rather than pattern-matching the baked constant, so a genuinely non-parametric
nested iterator re-derives to what it already had.

**5 — the hidden return buffer was declared `DbRef` by name.** The coroutine emitter pre-declares
every `__ref_*` work-var as `DbRef`/`DbRef::NULL`, named for the Reference-typed yield arms that
motivated it. A generic's return buffer joins that family with the monomorph's own type, so
`-> iterator<T>` at `T = integer` got a `DbRef` declaration and an integer assignment. It now asks
`persistent_default` / `rust_type` — the existing one home, whose doc already records a
hand-maintained second list drifting on three arms.

⚠ **The scalar axis is what made 3–5 invisible.** At `T = text` or a struct the DbRef channel and a
DbRef buffer are the right answers anyway, so every cell of the new guard passes before the fix at
those types — exactly the missing oracle axis `formal/interfaces.md` records for loft#1028.

Guard: `tests/scripts/1032-generic-iterator-return.loft` — count AND accumulated value per cell,
because a yield channel reading the wrong width still advances the right number of times. It
carries the two generic-free cells (forward declaration, hoisted argument) beside the generic ones,
since those are where the filed scope was wrong.

Out of scope, both verified generic-independent and left alone: yielding a struct/vector from a
generator's LOOP body is still a documented `--native` refusal, and a `text?` generator parameter
does not compile on `--native` (filed as loft#1035).

### A keyed collection argument was neither protected nor countable, so its callee's record was orphaned (2026-08-20)


Found while closing loft#1029, in the one argument kind its widened witness still refused:

```loft
fn take(h: hash<KS[k]>, n: integer) -> KS { h[n] ?? mk() }
take(v, 99)      // one record leaked per call, both backends — hash, sorted and index alike
```

The @P290 bracket arms the source-free only when it covers EVERY argument that carries a store,
and `protectable_ref_args`'s emit filter was `Reference | Vector | Enum` — so a keyed collection
left the set incomplete, the caller kept the conservative never-free answer, and the record the
`??` fallback minted had no owner.

⚠ **This is loft#981's hole from the other side, and the direction is the whole point.** There
the keyed argument was neither protected NOR counted: the set read complete while protecting
nothing, and the free it licensed took a hash parameter's element out from under the caller
(`tests/scripts/882-…` red under poison with `rec=0xDEADBEEF`). Making it INCOMPLETE was the
right cure for the use-after-free and left the leak behind. Protecting it closes both, and the
emit could always do it: `protect_store_frees` marks `allocations[r.store_nr]` and reaches it
through the argument's own `DbRef`, which a keyed collection variable holds like any other. The
filter is now one predicate, `is_protectable_store_type`, whose doc says which way each error
costs — a store-carrying type missing from it leaks, a non-store type added to it is the #981
use-after-free.

Guard: `tests/scripts/keyed-argument-witness.loft`, whose BORROW cells are the load-bearing
half — a cure that widened the COUNT without widening the BRACKET passes every leak assert and
fails these, because the container is read after the call by length and by value on both keys.
The loop cell leaks 32 records on a pre-fix binary. `tests/keyed_element_borrow.rs`,
`store_lifetime_890_889`, `store_lifetime_953` and the leak suites are green, and the adversarial
probes run clean under `LOFT_POISON=1 LOFT_STRICT_STORES=1` on both backends.

### An argument the call site could not NAME left the Join witness incomplete (loft#1029, 2026-08-20)


A callee whose return may be a BORROW of an argument cannot be classified statically — it
hands back either the argument's store or one it minted. loft#981/#982 settled that with a
RUNTIME decision: the @P290 bracket marks each ref argument's store, and `OpCopyRecord`'s
source-free is refused for a marked one and taken for a callee-minted one. The bracket needs a
SLOT to name, so `protectable_ref_args` accepts only a bare `Var` — and its own doc-comment
records the consequence: *"When some ref argument is not one, the witness set is incomplete and
the caller keeps the old, conservative 'never free' answer — the leak stays for that shape."*

Two argument shapes were sitting in that hole, one record leaked per call on both backends:

```loft
fn pick(s: S, c: boolean) -> S { if c { s } else { mk() } }
pick(S { a: 7 }, false)     // the literal is a construction BLOCK, not a Var
fn take(f: S?) -> S { f? }
take(null)                  // `convert` lowered it to `OpNullRefSentinel()`, not `Value::Null`
```

The first is cured where it went wrong — at the CALL SITE, not in the witness test. The slot
always existed and this frame always freed it (the parser builds a literal argument into a
function-scope work-ref whose block yields it); only the call site could not say its name. So
`scan_args` hoists the construction into the preamble and passes `Var(w)`, and the emitted code
becomes byte-for-byte the hand-written spelling that was always clean (`q = S { a: 7 };
pick(q, …)`). Widening `protectable_ref_args` to see through the block instead would have been
WRONG: `protect_store_frees` reads the DbRef VALUE and the bracket is emitted BEFORE the
arguments are evaluated, so a work-ref still holding its null would be "protected" while empty —
the witness set would read complete while protecting nothing, and the source-free it then
licenses would release a store the caller still reaches. That trades a leak for a UAF.

The second is loft#1021 one lowering later, and its own reasoning applies unchanged: a sentinel
holds no store, so nothing the callee returns can be a borrow of it.

**⚠ The issue's filed boundary was wrong, and the correction is the finding.** It named the
axis as *"the borrow arm names a PARAMETER"* versus loft#1019's vector element. Moving the axis
I had pinned shows the real one is the ARGUMENT SPELLING: a vector-element borrow arm leaks too
when its argument is a literal, and a parameter borrow arm is clean when its argument is a
variable. #1019's guard is not narrow in the way I filed — it binds every argument to a
variable first, which is what its cells hold fixed.

**And the axis had more on it than the two shapes above.** Moving the argument spelling across
a 42-cell matrix found six more leaking spellings, none of them in the issue and all ordinary to
write — a field (`pick(b.s, …)`), a nested field, a vector ELEMENT (`pick(w[0], …)`), a
vector-typed field, a `??`, and an `if` in argument position. Each leaked one record per call on
BOTH backends.

**The witness names a STORE, not the argument.** `protect_store_frees` marks an allocation
(`allocations[r.store_nr].set_free_protected()`) and reaches it through any `DbRef` in that
store, so an argument only has to be DERIVED from a nameable slot by operations that stay inside
one store. `b.s` is `OpGetField(Var(b), …)` — `b`'s store at another `pos`; `w[0]` is
`OpGetVector(Var(w), …)`, whose out-of-range sentinel preserves `store_nr` too. So the ROOT of a
projection chain is not an approximation of the borrow source, it IS it. A JOIN argument
witnesses both arms, which is safe in the direction that matters: an extra marked store can only
refuse a free, never license one. `Parser::projection_root_mut` already owned the two-op list
for the mirror question ("which inline container needs a NAME"), so `is_projection_op` is now
that one list and both read it — a native op's def carries no return dep (measured `deps=[]`),
so the `-> reference[v1]` in the declaration cannot be read there.

The CONSTRUCTION-block family stays on the hoist, and three wrappers each hid the hoisted value
from a pass that matches a bare variant:

* The hoist's own scope test was EQUALITY. The parser allocates a literal's work-ref at FUNCTION
  scope, so `p = pick(S { a: 7 }, false)` written one `if` or one `for` deeper compared 1 against
  the block's scope and declined. It is now membership in `self.stack`, the chain of open scopes
  — a numeric `<=` would not do, because scope numbers are allocated in encounter order and an
  earlier SIBLING compares less while enclosing nothing.
* `generation/pre_eval.rs` counted only `Value::Block` when deciding which argument to lift into
  a `let _pre_N = { … }` binding for a NATIVE template — the user-fn branch already counted both.
  An `Insert` then fell through to the template's own `let _haN = @v1;` binder, which is not
  braced, so it bound the FIRST statement (an assignment, type `()`) and rustc rejected the use
  with **E0609**. Nothing about #1029 is special there: any `Insert` argument to a native
  template was mis-emitted. All four sites now read one predicate.
* The lift that gives a call's result an OWNER matches a `Call`, and a hoisted argument leaves a
  SEQUENCE in its place — @P297's `Span` pitfall exactly one wrapper later, whose comment already
  says *"unspan before matching this branch or the lift never fires and the call-result temporary
  leaks."* Same cure: read through to the value.

A VECTOR literal is the one block that does not yield the slot it filled — it fills `__vdb_N` at
the enclosing scope and yields `_vec_N`, a view it opened at its OWN scope. Hoisting is still
ownership-neutral (the owner is not moving); what moves is the view's DECLARATION, out of a block
that then ceases to exist, so the block's scope is ABSORBED into the one the ops land in.
Otherwise `var_scope` points those vars at a scope no emitted code opens and slot assignment
places them against a sibling's zone.

⚠ **The E0609 is why a targeted probe is not a gate here.** Every cell was green on
`--interpret` while `--native` refused to compile one of them; only running both backends per
cell showed it. That is the same failure the first half of this issue hit
(875-json-absent-text-field) from the opposite direction — there the hoist fired where it should
not, here it fired where it should and the emitter could not render it.

Guard: `tests/scripts/1029-inline-argument-borrow-source.loft`, 18 cells, each asserting BOTH
arms plus the source's own value and, for a collection, its length — a cure that freed the
DELIVERED store answers the same number on the owning arm, and only a length or a source read can
witness it. It hard-fails on the pre-fix tree (`wrap.rs`: 25 leaked records). 42 probes run clean
under `LOFT_STRICT_STORES=1` on both backends, and four further axes the corpus had pinned were
moved as probes and are clean: a GENERIC (`g<X>(x: X, a: X?) -> X { a? }`, the spelling that
surfaced this issue), a struct carrying TEXT, a struct carrying a VECTOR, and a METHOD receiver.
`formal/ownership.md`'s D-own-6 is closed with it.

### A generic returning a discharged `T?` was lowered by a route its non-generic twin never takes (loft#1026, 2026-08-20)


`pub fn g<T>(x: T, a: T?) -> T { a? }` at `T = text` SIGSEGV'd under `LOFT_POISON` and orphaned
one `String` per call without it. Two faults, one shape, and the second is what the issue's
"not fixed" note meant by *"a choice in the monomorph's return lowering"*. There was no choice:
`parse_block` already decides this, and the monomorph promoter was replicating half of it.

**The crash — `set_var` emitted a put-op for a value nobody pushed.** A template's zero-value
work-ref is `Set(v, Null)` on a `Reference`; substituting `T = text` retypes the slot and leaves
the null alone. `generate` has no arm for a bare `Value::Null`, so it pushed nothing while the
`OpAppendText` below it popped a full 16-byte `Str` — whatever the eval stack held under it.

```
   3[88]: InitText(var[40]) var=__ref_1[40]:text
   6[88]: AppendText(var[40], v1: text)   <-- nothing pushed; crash pc = fn base + 6
```

The two sibling sites already guard it — `gen_set_first_at_tos` returns on a zero-width push,
`gen_dest_call_args` repairs an omitted argument with `emit_typed_null` — and `--native` emits
`STRING_NULL.to_string()` for this very IR. `set_var` now makes the same repair, so the two
backends store one value instead of disagreeing. Silent without `LOFT_POISON`: a plausible
stale `Str`, which is exactly the blind spot that gate exists for.

**The leak — the monomorph promoter replicated `do_tret_bind` and not `do_if_acc`.**
`parse_block` has two text-return promotions and they are mutually exclusive: a CALL tail binds
`__tret`, and a value-yielding `if`/`match` tail pushes each ARM into an accumulator that
`text_return` delivers through the caller's hidden `&text` buffer. `promote_monomorph_text_
return` — whose own doc-comment claims the monomorph is lowered *"identical to its non-generic
twin"* — implemented only the first. An `a?` discharge is an `If`, so a `-> T` generic lands on
the missing half every time:

```
n_g       (non-generic)  fn n_g(x, a, ___acc_1:&text) -> text["___acc_1"]      0 leaks
t_4text_g (monomorph)    fn t_4text_g(x, a) -> text   __ret_1 skipfree          1 leak/call
```

The scope pass materialises the tail into a `skipfree` `__ret_N` the callee hands out and
nobody frees. `LOFT_TEXT_TIMELINE` reads it as `1 text buffer(s) LEAKED "z"` and a nine-call
loop leaks nine. The @PLN104 post-pass promoter does not cover it either: `return_ownership`
reads the pre-scope `If` as `Own::Join{base: a}` with `a` an argument, so
`text_return_orphan_risk` answers `None` — true of the IR it reads, false of what the scope pass
then emits.

With the `do_if_acc` half added, the monomorph's IR is its twin's, on both backends
(`&mut String` in, `Str` out, where it used to return an owned `String`).

**⚠ The issue's own boundary table was five-tenths vacuous and its cause was refuted twice** —
both recorded on the issue by the session that filed it. `protectable_ref_args` / `Type::Text`
not being in `heap_dep()` is NOT this bug (instrumented: never called for the generic), and
`substitute_type` dropping the template's deps is not the site either (the template's return
deps are already empty). The residual that "position matters" was a broken probe matching the
word `ok` inside an echoed source line.

**The same leak behind an EARLY return.** `fn gd<T>(x: T, a: T?) -> T { if a { return a?; } x }`
reaches the promoter by the other door — `early_text_return_orphans`, which asked
`classify_text_return` alone. That verdict is `Plain` for this guard (an argument borrow and a
literal), and the two verdicts it accepted missed it, so the same orphan appeared one statement
earlier. It now also accepts the `if_tail_yields_text` shape, which is the identical question
the tail gate asks. Measured: the leak closes, and the nested-guard spelling
(`if c { if a { return a?; } else { return x; } }`) stops failing `--native` with E0308.
The blast radius is monomorphs only — `early_text_return_orphans` has one caller.

**What this fix EXPOSED, filed as loft#1028.** With the `Set(v, Null)` crash gone, a `-> T?`
generic whose body writes `null` answers the empty text on the interpreter and will not compile
on `--native`: `Parser::null()` maps `Type::Reference` to `OpNullRefSentinel`, a type variable
IS a `Type::Reference`, and substitution retypes the slot without re-choosing the op. Same class
as loft#1016 and loft#1020, same cure (mark in the template, answer in the monomorph) — but
`null()` is the parser's ONE null producer, so re-pointing it reaches every template's every
null. Pre-existing (identical on a pre-#1026 binary), and not a regression from this change: the
shape used to SIGSEGV on the fault above instead.

**And what the new corpus caught on its way in, filed as loft#1029.** Its `T = <a struct>`
control tripped `loft_suite`'s store-leak gate: a returned join leaks its FRESH arm when the
borrow arm names a PARAMETER. loft#1019 fixed that classification, but its guard holds the
borrow SOURCE fixed at a vector element in all nine cells including the escaping-join one, and
the parameter spelling — which needs no generic at all — still leaks one record per call on
both backends. Not from this change (identical with the `set_var` repair disabled, and on a
branch with none of this work). The corpus binds that cell rather than reading it inline, with
a comment saying why, so it tests its own subject instead of riding someone else's defect.

### The registry index's download cap truncated instead of refusing, and every compile parsed the whole catalog (2026-08-20)


`http_get_bytes` bounded a response with `.take(50 MB)`, so a body past the ceiling came back
SHORT and passed for a complete document. A 70 MB index cut to exactly 52,428,800 bytes failed
as `JSON parse error: unterminated string` at a byte offset inside an unrelated package. That
number is compiled into every released binary, so an index growing past it would have taken the
registry down for every client already in the wild, with nothing the registry could do about it.
The ceiling is now 512 MiB, overridable with `LOFT_MAX_DOWNLOAD`, and exceeding it is a refusal
that names the ceiling.

Separately, the parser's Tier-1 trigger fallback parsed the ENTIRE index to read the `triggers`
field — ~300 bytes of a 693 kB document, on the compile path. The map now lives in a derived
sidecar (`~/.loft/registry/triggers.json`) stamped with the index's length + mtime. `loft --check`
on a four-line program, against a catalog 100× today's size: **1.13 s / 344 MB RSS → 0.02 s /
12.3 MB** — what the same program costs with no catalog installed at all. The stamp is the
caller's, taken before the map's source is read, so a concurrent refresh can only cost an extra
rebuild, never mark stale triggers as current.

`PKG_REGISTRY.md`'s sizing predated the per-version `api` field, which is 91 % of the live index:
its "10,000 packages × 20 versions, ~80 MB" is **825 MB** measured. Corrected there, with the
shape that keeps scaling (a thin resolution index, `api` per package on demand).

### `registry_validate.sh` validated whatever was cached locally and reported OK about it (loft#1027, 2026-08-20)


Run straight after publishing `hex_way 0.1.1`, it validated 0.1.0 and printed a bare `OK`. Two
steps compounded: `loft install <pkg>` resolved against an un-refreshed index that still ended at
0.1.0 (so the install was a no-op), and the next step picked the highest version DIRECTORY under
`~/.loft/registry` — which also made the verdict depend on what that machine had downloaded
before. The version is now resolved from the index with a forced refresh and installed as an
explicit `<pkg>@<version>` pin, `<pkg>@<version>` is accepted on the command line, the verdict
names the version, and a non-newest version says so on its own line. The cache root honours
`LOFT_HOME`, which the hardcoded `$HOME` did not.

### A `null` written inside a generic was the TYPE VARIABLE's null, not the concrete type's (loft#1028, 2026-08-20)


`Parser::convert` lowers a `null` literal to its target's typed null. A template's `T` is an
attribute-less placeholder STRUCT, so `Type::Reference` won and the site became
`OpNullRefSentinel` — a 12-byte DbRef. Monomorphisation then substituted the TYPE and left the
already-chosen OP, so `t_4text_nl` wrote that sentinel into a `&text` slot:

```
n_nl        (non-generic)   ___tret_1(0):&text = OpConvTextFromNull();
t_4text_nl  (monomorph)     ___tret_1(0):&text = OpNullRefSentinel();     <-- the defect
```

The interpreter read the sentinel's own bytes back as the answer and said nothing; `--native`
refused the program (`DbRef` has no `Display`). `(G-Mono)` in `formal/interfaces.md` requires
`[T ↦ C]` "applied throughout … body types", and it also promises the two backends cannot drift
on a monomorph — this broke both halves.

**The filed scope was too narrow in two ways, and both matter.** The issue named `Parser::null`
as the site: patching it changes nothing, because `null()` is never called with a reference type
for this shape (18 calls in the reproducer, none of them a `Reference`), and
`cl("OpNullRefSentinel")` is never called at all. A backtrace on every resolution of that op
names `Parser::convert`. The issue also reported `T = integer` as clean; it answers **65535**.
Measured across a type sweep, generic against the non-generic twin as control:

| `T` | before | why |
|---|---|---|
| `text` | the empty text | sentinel bytes read as a string |
| `integer` | `65535` | the sentinel's low 16 bits |
| `character` | U+FFFF | same bits, char domain |
| `float` | a denormal | same bits, float domain |
| `boolean` | *correct* | **coincidence** — the sentinel's low byte is `0xFF`, which is also boolean null |
| `struct`, `vector` | correct | for a reference the sentinel IS the answer |

`boolean` is a control that does not control: a sweep starting there reads "generics are fine".

The cure is the one loft#1016 (`x?`'s default) and loft#1020 (`x == null`) already use — the
template MARKS the site, `rewrite_generic_type_defaults` answers it once `T` is concrete. Here
the answer is to re-run `convert` itself rather than to pick a null, so there is still exactly
one spelling of *"what is `τ`'s null?"* (minting a second is what loft#1014 was). A nested
generic re-stamps through that same call and stays deferred until an outer instantiation names a
real type. Two further symptoms went with it: `x == null` answered **false** for a null returned
from a generic, and one program instantiating a template at two types got neither answer right.

Blast radius, measured rather than argued: a corpus with one function per null→type conversion
path (struct, vector, value enum, text, integer, float, character, a reference-kind generic, a
generic with no null) emits a **byte-identical** `loft introspect` before and after. The
`Parser::null` arm patched first was removed again — it was never on the path.

### The `--native` copy-or-adopt guard was gated on a NAME, so a METHOD never got it (loft#1017, 2026-08-20)


A callee whose return may be a BORROW of an argument cannot have its result aliased into a
caller local: the local's own `OpFreeRef` then whole-store-frees the caller's RECEIVER. The heap
first-bind in `dispatch.rs` handles that — the @P290 protect bracket around the call, and a
copy-or-adopt guard after it (`_src` aliases the receiver → deep copy; callee-minted → adopt).
The gate read `name().starts_with("n_")`, so a `t_` METHOD, or a generic monomorph, with a
byte-identical body and the same `return_adopts_fresh_store()` verdict fell straight through to
a plain alias — no bracket, no copy — followed by the same unconditional free.

`scopes.rs`'s own lift gate already says the two *"have to name the SAME set of callees, which
is why the predicate lives in one place (loft#810)"* and uses `is_loft_defined()`. This end had
drifted off it, and the comment directly above it records an earlier correction from a "coarse
proxy" to the canonical `return_adopts_fresh_store()` fact — which the name prefix survived
untouched. The interpreter was never affected: `gen_set_first_ref_call_copy` reads the fact.

**The 10-line reduction the issue says it could not find**, and why two attempts missed it:

```loft
fn view_at(self: const St, i: integer) -> V {
  if i < 0 or i >= len(self.vs) { return V { x: 0.0, … }; }   // FRESH
  self.vs[i] ?? V { x: 0.0, … }                                // BORROW
}
for i in 0..6 { println("x={view_at(s, i % 3).x}"); }
```

```
interp:  1.5  3.5  0  1.5  3.5  0
native:  1.5  3.5  0  0    0    0     <-- everything after the FRESH arm ran
```

The borrow arm reads CORRECTLY and then frees its store; the fresh arm's next allocation
recycles that slot over the receiver's records. In `stage` the recycled slot was a canvas, which
is why the corrupt `rec` read back as `0xFF000000`.

Both axes have to move together. The identical body as a FREE function is correct, and so is a
single call instead of a loop — so a reduction "with a plain `vector<V>`", written as a free
function and called once, is two controls at the same time and passes. The METHOD spelling was
the missing axis, and the report's "the shape alone is not sufficient" was measuring cell B.

The regression test carries all six cells including the two that a too-eager fix would break —
the free-function spelling, and the borrow arm alone. It fails 6/6 on the pre-fix binary.

### A `null` argument read as an incomplete witness set, so the caller never freed (loft#1021, 2026-08-20)


`fn f(a: P? = null) -> P { a? }` called as `f()` leaked the record the null path built, once per
call, on both backends. The value was right; only the ownership was wrong.

loft#981/#982 already built what this needs, and `protectable_ref_args`' own comment states it:
a callee whose return dep names a visible parameter may hand back that parameter's store (the
caller must not free it) or one it minted itself (the caller must), *"no static bit can carry
that split, so the decision is made at RUNTIME by the bracket."* The bracket needs a SLOT to
name, so a non-`Var` argument leaves the witness set incomplete and the caller keeps the
conservative never-free answer.

An OMITTED `τ? = null` parameter is filled with a bare `Value::Null`, which is not a `Var` — so
every such call read as uncovered. But a `null` argument holds NO STORE: nothing the callee
returns can be a borrow of it, so it neither needs protecting nor can it break coverage. One
arm, at the predicate both ends of the emitted sequence already read.

**The measurement that localised it.** `q: P? = null; b = f(q)` — a bare VAR holding null — was
always CLEAN, while `b = f()` leaked, over a byte-identical callee and a caller IR differing only
by that variable. Two readings of `protectable_ref_args` predicted both should leak. One
env-gated `eprintln` on `call_return_frees_source` settled it in a single run: for `f()` it
answers `covers_all=false -> false`; for `f(q)` it is **not consulted at all**, because that
caller takes a different lowering. The predicate was right and the model of who asks it was not
— which no amount of re-reading the predicate would have shown.

**Still open, found while measuring this:** the BORROW arm of the same mixed-ownership return
leaks and SCALES — `fn pick(bx: Box, take: boolean) -> P { if take { bx.p } else { P{…} } }`
called five times leaks four records plus one unknown-typed store. A displaced owner rather than
a coverage gap, and the closest interpreter-side shape to loft#1017's `--native` corruption.

### `sum`'s identity got a default, so the one `#superseded` fix loft ships can be applied (loft#1003, 2026-08-20)


`superseded-call` is the only ADVICE-level fix carrying a machine `edit`, and it was **always
rejected**: the edit is a bare rename, `sum_of` is the only `#superseded` symbol in the stdlib,
and `sum<T: Addable>(v, init: T)` had no default for `init` — so the verified rewrite `sum(v)`
was *"missing argument for parameter 'init'"* on every program that triggered it.

```console
$ loft fix s.loft
  s.loft:1  call `sum` instead  [REJECTED (the rewrite introduces an error)]     # before
  s.loft:1  call `sum` instead  [verified]                                       # after
```

`init: T? = null` with `result = init?`. A literal default is not spellable (`init: T = 0` is
*"expected T, got integer on default value"*), so the nullable-with-discharge form is the only
one available — and it is exactly the declaration loft#1016 had to fix first, which is why the
two land together. Additive per COMPATIBILITY.md: adding a default keeps every existing
`sum(v, init)` caller working.

This closes the "always rejected" half of loft#1003. The larger half — 20 fixes advertised
MECHANICAL in DIAGNOSTICS.md that carry no `edit`, and no gate asserting they must — is
untouched.

### `character`'s null had four spellings and the backends picked different ones (loft#1014, 2026-08-20)


`types.md` pins one: `Char`'s in-band sentinel is CODEPOINT 0 — the same `'\0'` that
`construct_default(Character)` answers, that `op_conv_bool_from_character` tests for, and that
the parser's `null()` emits as `OpConvCharacterFromNull`. Two of the five sites disagreed, in
different directions, so the same program answered differently per backend.

- **Interpreter** — `emit_typed_null` grouped `Character` with `Integer` and pushed EIGHT bytes
  of `i64::MIN` into a FOUR-byte slot. It read as null only because the low word of `i64::MIN`
  is zero on a little-endian box: right by accident, at the wrong width. Now
  `OpConvCharacterFromNull`, the spelling the parser already uses.
- **`--native`** — `write_typed_null_in` wrote `i32::MIN`, which is not codepoint 0 at all, so
  `a == null` on an omitted `character? = null` answered FALSE while the interpreter said true.
  Worse, `ops::to_char` reaches that value through `from_u32_unchecked`, for which `0x8000_0000`
  is undefined behaviour — the release optimiser is entitled to fold the sentinel test away.
  Now `0`.

**And a `character?` DISCHARGE would not compile on `--native` in any form.** The `ops::to_char`
wrap that converts the i32 storage form to the `char` an op template wants was chosen by IR NODE
KIND — four arms (`Var`, `TupleGet`, `Call`, `Block`), each added when a new shape turned up.
The `?` discharge lowers to an `If`, which was on none of them, so the template compared an
`i32` against `char::from(0)` and rustc rejected the whole build. It is a TYPE test now: an
operand of a character-typed parameter arrives as the i32 storage form whatever produced it, and
only the bare integer literal (which constructs a `char` directly) is excluded. An allow-list
here costs correctness, not an optimisation — the opposite of the hoist gate's allow-list, and
the reason this one had to go.

Not changed, and asserted so it stays visible: a `character` holding `'\0'` reads as null,
because that codepoint IS the reserved sentinel — so `'\0' as integer` answers `null` on both
backends. The issue read that cell as a discharge failure; a literal `'\0'` behaves identically,
so it is the documented in-band collision, not this bug.

### `x?` on a generic parameter discharged with the type VARIABLE's default (loft#1016, 2026-08-20)


`x?` is `x ?? construct_default(T)` (`types.md` `(N-Default)`) and `construct_default` is a
function of the CONCRETE type. Inside a template `T` is an attribute-less placeholder STRUCT,
and `build_default`'s record arm defaulted it perfectly happily — to an empty record of the
placeholder's own store type. Substitution then retyped the slot and left the allocation, so the
monomorph read a `__typevar_T` record back as whatever `T` became:

| `T` | answered |
|---|---|
| `integer` | `34359738369` (a zero-filled DbRef read as a number) |
| `float` | a denormal |
| a record `P` | `4294967198`, **plus a leaked `__typevar_T` record** |
| `text` | SIGSEGV |

The rule was simply not applied where it is decidable. `build_default` now MARKS such a site
(a block named `TV_DEFAULT_BLOCK`) and `rewrite_generic_type_defaults` answers it per monomorph
— in the MONOMORPH's own frame, via the `self.vars` / `self.context` swap the drop-cascade
builder uses, because a record default parses `S {}` and its work-ref belongs to the function
the code lands in. A nested generic (`concrete` still a type variable) re-marks and stays
deferred until the outer instantiation names a real type.

⚠ **A second defect sat behind it, reachable only once `T = text` stopped crashing.** The
declaration sites paired `rust_type(tp, &Context::Variable)` with the context-free
`default_native_value`, which answers the `Str` form — `let mut var_x: String =
Str::new(STRING_NULL)`, rustc E0308. The tuple arm had already patched exactly this by hand for
its elements. `default_native_value_in` asks the question once now, and the tuple arm calls it.

**Still open, found while measuring this and filed rather than fixed:** `a == null` on a generic
`T?` picks `OpRefIsNull` at TEMPLATE time (the operand is `Optional(Reference(tv))`, so the
reference arm of the `== null` dispatch wins) and the monomorph keeps it — a 12-byte DbRef read
out of an 8-byte integer slot. Same root class, but curing it means extracting the five-branch
null-test dispatch so a monomorph can re-run it, which is its own change.

### A `??` default that CALLS a function leaked its record, once per index miss (loft#1013, 2026-08-20)


`x = v[i] ?? mk()` types its result from the SUBJECT, which names the vector (`ref(S)["v"]`), so
the consumer reads `x` as a borrow and frees nothing. On the miss path the value is a store
`mk()` freshly allocated and handed back — owned by no one. One record per MISS, unbounded in a
loop, identical on both backends.

The boundary says it is not `??` in general and not calls in general: a struct LITERAL default
is clean, and the same call bound plainly is clean. The literal is clean for the opposite
reason — `parse_object` allocates it into a work-ref this frame already frees — which is exactly
the model `build_null_coalesce_default`'s own comments state ("the default arm keeps its own
owner, freed independently"). A call default simply had none. And the compiler's refusal of a
struct-valued constant prescribes the call spelling, so the leaking form is the one it tells you
to write.

The buffer to bind is the one the call ALREADY carries: a heap-returning callee is given a
hidden `__ref_N` destination the caller mints and frees (`add_defaults`), and the only thing
missing was capturing what the call answered into it. Reusing that buffer keeps ONE owner and
ONE free — a freshly minted second work-ref would double-free the callee that DOES deliver
through its buffer. Guarded on a BORROWING result (an owned subject makes the result owned and
the consumer's own free claims the store) and on a callee that does not return a borrowed view.

⚠ **That exposed a second half in `scopes.rs`.** `paired_witness` made the buffer its OWN
witness for `__ref_N = f(__ref_N)`, so the scope-exit `OpFreeRefIfDistinct(__ref_1, __ref_1)`
compared the store with itself and never fired — the leak survived the parser fix unchanged. The
pairing exists to skip the buffer's free when ANOTHER variable adopted its store; a buffer is
never its own witness.

**Still open, found while measuring this:** the same mixed-ownership merge written as a plain
`if` — `m = if i < len(v) { v[i] } else { mk() }` — leaks identically. The ownership inspector
names the shape (`m  Join(base=v)`), and it is the leak face of the corruption loft#1017
reports. Curing the class means giving the OWNING arm of any such join a frame owner, which is a
change to the ownership merge itself.

### An omitted `τ? = null` argument arrived as the wrong kind of null (loft#1015, 2026-08-19)


`fn f(a: integer? = null) -> integer { a? }` called as `f()` answered **65535** on the
interpreter, and `float?` answered a denormal — silent wrong NUMBERS. On `--native` the same
shape with `boolean?` did not compile at all. Against `types.md`'s
`(N-Default)` / `(D-Scalar) construct_default(Integer[r]) = 0`.

Two independent halves, one per backend, which is why neither alone showed it:

- **Interpreter** — `emit_typed_null` matched the UNPEELED type, so every `τ?` missed the scalar
  arms and fell to the catch-all "push a zero-filled DbRef as a generic null". An `integer?`
  parameter received a REFERENCE sentinel and `a?` read it as a number. The parser's own `null()`
  already peels for this exact reason (@PLN25 slice (b)) — one fact in two places, one of which
  peeled.
- **`--native`** — `write_typed_null` peels correctly but wrote `false` for `boolean`, while
  `rust_type` declares a null-capable boolean slot as the @PLN17 tri-state `u8`. `bool` into `u8`
  is rustc E0308.

⚠ **A third defect was hiding behind the first, and fixing the peel exposed it.** The
interpreter's `Boolean` arm said `i64::MIN` — a value the tri-state byte cannot hold — and had
never been reached, because the missing peel routed `boolean?` to the catch-all whose ref
sentinel happened to read as null. With the peel in, `a == null` on an omitted `boolean? = null`
began answering FALSE. The sentinel is 255, which is what `--native` writes into the same slot;
the two backends now agree by construction rather than by coincidence.

`write_typed_null_in` takes the storage-vs-expression split `rust_type` already makes for
`boolean`: the call-argument site asks for the storage form, the two `if`-branch callers keep the
2-state `false` they unify against.

The regression test asserts three things, because the obvious one is not sufficient: the
discharge, that a supplied argument is still delivered, and that the omitted argument really IS
null — including that a supplied `false` is not, which is the whole point of a tri-state.

`character? = null` is NOT fixed here and is filed as loft#1014: it needs a width-correct
sentinel of its own, and `--native` additionally fails to compile one of its two forms.

### A reference tuple read its elements at the wrong layout's offsets (loft#1006, 2026-08-19)


`TupleGet`/`TuplePut` derived the element offset from TWO layouts for one datum: the
`RefVar(Tuple)` branch used `stored_tuple_field_offset` — the synthetic `__tuple<…>` RECORD
field positions — while the plain branch beside it used `element_stack_offsets`. A reference
tuple points into the caller's STACK FRAME (`OpCreateStack`, whose own contract says the DbRef
targets the frame and is "NOT a valid data store"), so the stack layout is the one the data has.

Measured: `(integer, integer)` is `[0, 8]` under both derivations, `(text, text)` is `[0, 4]` as
a record against `[0, 16]` on the stack. So a reference read of element 1 of a text pair landed
12 bytes early, inside element 0. Scalars coinciding is what hid it — the shape that reads as
proof and is not. `ref_tuple_field_offset` now answers the stack layout and the two branches
agree.

No shipped program changes: scalars coincide, and a heap element is still refused at the
signature. What it removes is a hazard that would have made any future heap-element work wrong
in a way every scalar test passes.

⚠ **It is not, on its own, enough to lift the refusal** — measured, not assumed. With the offset
corrected, adding the `text` arms still SIGSEGVs, because the two element paths speak different
op families: the plain tuple writes with `OpPut*` at a position in the CURRENT frame (16-byte
inline stack form), while a reference writes with `OpSet*` through a DbRef (4-byte record
handle). A callee must reach the CALLER's frame, so only the DbRef family can get there, and
that family speaks the record form. `i64` is 8 bytes either way, which is why scalars are
immune. Closing loft#1006 needs a representation decision — recorded as `D-bind-11` /
`D-tup-1` in `doc/claude/formal/`.

### `loft fix` can take the rest of a text slice (loft#1003, 2026-08-19)


`text-slice-char-bound` is the fifth mechanical fix `loft fix` can apply, and the last row in
`EDIT_BLOCKED` that claimed it should carry an edit.

`len(t)` -> `size(t)` is NOT the one that could carry it: both spellings warn — `s[i..len(s)]`
and `s[i..s.len()]` — and they put the `len` token in different places, so a rename at the
bound's start would turn `s.len()` into `sizelen()`. That needs the `len` TOKEN's position,
which the emit site does not have.

The fix's SIBLING does not care which spelling it is. Deleting the bound turns `s[i..<anything>]`
into `s[i..]`, which takes the rest — the cure the fix already named and the one the docs
recommend for this shape. The span is the start of the end expression to the `]` that closes the
slice, both taken at the one point that brackets the whole bound. Verified on both spellings by
applying it and running the result: `s[0..len(s)]` over `"héllo"` printed `héll` before and
`héllo` after.

### A `both` method named where a value is wanted says so (loft#1008, 2026-08-19)


The `self` half of loft#1008 already reported *"`f` is a method on `P`, and a method is not a
function VALUE"* from every position measured. The `both` spelling did not: `x = f` bound
**null with no diagnostic at all**, and the error surfaced later as whatever used it ("Cannot
format type null"); as a fn-ref argument it reached the call check as a bare `Value::Null` and
came out as *"expected fn(P) -> integer, got null"* — a value the author wrote nowhere.

**The two receivers register identically** (`t_<len><Type>_<name>` — checked, the def tables
match), so registration was not the axis and the earlier note that the name "cannot be
recovered" was right about the ARGUMENT site and wrong about the cause. Instrumenting the
bare-name path found it: a `both` receiver also leaves a `Dynamic` def under the PLAIN name —
which is what makes the free-call spelling `f(x)` work — so unlike a `self` method the name
is FOUND, skips every unknown-name branch, and falls through a final `else` to a silent
`Type::Null`. The name is still in hand there, which is exactly what the argument site lacked.

⚠ **Reporting alone left a cascade.** The first version emitted the right error and then two
more — the generic "got null" and a "missing argument for parameter 'f'" — because `Null` is a
real value downstream. The `self` path poisons with `Type::Never` for that reason; this one now
does too, but only when it actually reported, so every other def kind that lands here keeps the
null it always produced. One error for one mistake, matching the `self` case byte for byte.

Controls unmoved on both backends: a plain fn-ref (20), `map` with a plain fn, both method
spellings and both free spellings (10/10/15/15), the lambda wrapper the message recommends,
and the stdlib's own `both` functions (`len`, `abs`, `round`). Pinned as
`tests/error_messages/cases/57_both_method_is_not_a_fn_ref.loft`; no other baseline moved.

### `verify-self` no longer exits 0 when it verified nothing (loft#1012, 2026-08-19)


`loft verify-self` on a source-built install reported *"not a release bundle — nothing to
check against"* and exited **0**. The message was honest; the exit code is what gets read, so
`loft verify-self && deploy` was green on an install the command could not examine. *Verified
intact* and *could not verify anything* are the two answers a caller most needs to tell apart,
and they were the same answer — the same shape the command exists to prevent, one level up: a
CHECK that silently did not run.

Three exits now, following `loft audit`'s precedent (`0` clean, `1` low, `2` high,
`3` security_critical): `0` verified and intact, `1` verified and something does not match,
`2` could not verify. Both `return 0` sites for the nothing-to-check case became `2`; the pass
and fail paths are untouched.

Nothing in the tree read the exit code — checked before changing it — so no caller had to
move. Pinned in `tests/exit_codes.rs`, which runs the real binary and can therefore see an OS
exit code; the test asserts the precondition (the output really is the unverifiable case)
before the code, so it cannot pass vacuously against a bundle that genuinely verified.

### `filter` freed the source's records, so a later loop over it answered nothing (2026-08-19)


Found while verifying loft#1000's table cell by cell. On BOTH backends, a `filter` over a
`vector<vector<T>>` left the source unreadable: `len(nv)` still answered 2 and `nv[0]` still
read its contents, while a fresh loop over the same collection yielded 0. Not a crash, not an
obviously wrong length — a loop that silently does not run.

The comprehension's per-iteration yield slot is created with the bare element type, which
carries no deps, so scope handling reads it as an OWNER and frees it each iteration. For `map`
that is right: the body is a call and the slot owns its result. `filter`'s body IS the loop
element (the identity yield that makes it a filter rather than a map), so the slot was an
ALIAS of a borrow of the source, and freeing it destroyed a record the source still pointed at.

`LOFT_VAR_TABLE` is what named it — `_comp_1 … OWNS` beside `_filter_elm_1 … deps=[_filter_vec_1]`,
a borrow classified as an owner, which is @PLN130's shape. INVISIBLE ON A SCALAR ELEMENT,
whose copy is the value itself, which is why it stood so long.

A variable bound to a borrow is a borrow: the slot now carries the body variable's deps when
the body is a bare variable that has any. Narrowed to a bare `Var` deliberately — that is the
only shape where the slot aliases rather than owns.

### The open spatial slices walk OUTWARD from the point (loft#1002, 2026-08-19)


`xs[(x,y)..]` and `xs[(x,y)..:n]` were the Z-order TAIL: only records whose Morton code is
`>=` the query's. Half the neighbourhood was structurally unreachable, so the answer depended
on where the query sat in the curve — an entity at the far end of the map asking for the three
things nearest it got NOTHING, and `..:n` answered 3, 3, 3, 2, 1, 0 over five records as the
query moved along, with nothing to separate "only two things are near me" from "I am near the
end of the curve". Four docs and the catalogue entry all described an outward walk.

`radix_db::near_range` is the n-axis form of `spatial::near`, which was correct, unit-tested,
and reachable from no loft program. Two cursors seeded either side of the query, each step
yielding whichever is closer. The distance is computed word by word with a borrow: one axis is
64 bits, so two axes are 128 and three are 192, and truncating to a `u64` would make every pair
of points sharing a high word look equidistant — most of a map.

APPROXIMATE, and now said so in every doc: ordered by Morton distance, which jumps at quadrant
boundaries, so a truly-near point can arrive a little late. `xs[lo..hi]` stays the exact form.
`tests/scripts/48b-spatial-slice.loft` pinned the tail and is rewritten to pin the walk, each
expectation beside the euclidean distances it follows from.

### A tuple variable carrying text reaches a tuple parameter on `--native` (loft#1005, 2026-08-19)


The generated parameter was `(i64, &str)` while the caller's local was `(i64, String)`, so
`--native` refused the whole program. A tuple LITERAL argument is emitted in place and already
spelled `&str`, which is what made this narrow and what pointed at the seam.

Closed by BORROWING at the call site. The issue expected this to need a lifetime on the
generated signature; it does not — Rust elides it. Only the argument had to be re-spelled,
element-wise, with `&*place` on each text leaf, which derefs `String`, `Str` and `&str` alike.
A by-value tuple parameter is a COPY in loft, so the callee cannot write back through it and
the call stays allocation-free. The matrix found a second cell: a NESTED tuple parameter
reaches its inner tuple as a tuple-get, which loft#840's whole-parameter rule did not see.

### `loft --tests` runs the functions a file NAMES as tests (loft#1010, 2026-08-19)


Every zero-parameter function ran: a `setup` helper (whose `assert` could fail the suite), and
a `main` alongside the tests with whatever it prints or writes. Arity was the whole rule, so a
parameter was the only way to say "not a test". A file declaring any `test_*` now runs exactly
those; one declaring none keeps arity, which is the demonstration shape and the only reason
`--tests` can be pointed at a plain program. Measured first: 1785 files in this corpus name no
`test_*` against 216 that do.

⚠ **The `--native` half was worse than what was filed.** `has_main` was true whenever `n_main`
merely EXISTED, so the generated binary ran `main` and nothing else, exited 0, and the harness
reported every `test_*` as PASSED without running one — a test asserting `1 == 2` beside a
`main` was green on `--native` and red on the interpreter. The entry point is now decided by
whether `main` is among the functions the run means to call.

### Four mechanical fixes carry an edit, and `loft fix` can act on a warning (loft#1003, 2026-08-19)


`loft fix` could act on no WARNING-level fix at all. `needless-reference-parameter`,
`needless-const-parameter`, `empty-braces-not-collection` and `not-null-deprecated` now carry
an edit. The missing fact was the modifier's OWN position, taken at the declaration with
`peek_pos`; the CARET was wrong for the same reason and one capture fixed both — the
`needless-*` checks run after the body and fell back to the variable's source, a position
INSIDE the body.

⚠ **The attached edits surfaced a latent hazard.** `apply_fixes` claimed its candidates' spans
were disjoint instead of enforcing it, and each is verified against the ORIGINAL source.
`empty-braces-not-collection` fired on BOTH parser passes, so `{  }` -> `[]` applied twice
deleted the four characters after the replacement and ate the enclosing `}` — in a file the
user asked to have fixed. The notice is pass-2 only now, and overlap is settled in
`apply_fixes`, the only place that knows the candidates share a buffer.

### A vector builtin ends on LENGTH, not on the element's null (loft#1000, 2026-08-19)


`map`, `filter`, `reduce`, `all`, `count_if` and the vector comprehension NEVER TERMINATED
over a `value struct` element; `any` terminated only by stopping on a phantom element read
one past the end, and answered from it. Both backends, and a plain `struct` was correct in
every cell — `value` was the only axis.

Two correct-in-isolation mechanisms met. The builtins emit `Set(elm, iter_next)` followed
by `if !bool(elm) { break }`, relying on `OpGetVectorNullable` answering null past the end.
@PLN101's `value_struct_copy` rewrites exactly that bind: a `value struct` reaching a user
callback is deep-copied so the local OWNS its record — and a freshly minted record is never
null, so the break could not fire at all. The phantom `x=5695106865` was the same on both
backends and every run: whatever the out-of-bounds `DbRef` addressed.

The cure already existed one construct over. The `for` STATEMENT had the mirror defect (a
null ELEMENT ending the loop EARLY) and was cured by terminating on LENGTH, which is why it
was the one correct row in the report. `Parser::vector_loop_break` is now the shared home —
sibling of `text_loop_break`, which exists for the same reason — and all six lowerings
route through it, so "how the loop ends" is one decision rather than seven.

**The index semantics were read off the emitted IR rather than reasoned about**, because an
off-by-one here is a silently dropped or doubled element: `#index` starts at `-1`, the
`{#iter next}` block PRE-increments and then reads, so at the test the index is the 0-based
one just read and `len <= idx` is exactly "past the end". Re-read each iteration rather than
hoisted, so an in-loop `#remove` still terminates. Forward-only — these builtins have no
reverse form, so the `for` statement's companion `idx < 0` test has nothing to answer.

⚠ **The first version took the length of the wrong vector.** `build_comprehension_code`'s
`vec_expr` is the DESTINATION being appended to — `map` builds a result vector and hands
that in — so measuring it would have measured the thing that grows every iteration. The
parameter is therefore `Option<(Value, u16)>` carrying the SOURCE explicitly, and each of
the four call sites passes its own (`vec_copy_var` for map/filter, `source_expr` for the par
materialiser, the pre-`iterator` collection for the comprehension — captured before
`iterator` rewrites it, mirroring the `for` statement's `orig_coll_expr`).

Guard: `tests/scripts/1000-value-struct-vector-builtins.loft`, 12 cells on both backends —
all six constructs, the `for`-statement control, an EMPTY vector (which hung too, so the
element count is not the axis), five elements (so "2" is not mistaken for the rule), a
plain-`struct` control, and a scalar vector holding real values, since a null the vector
genuinely holds is the OTHER shape that shares the out-of-bounds sentinel. A pre-fix binary
hangs on the first cell.

### `loop_nr` answers "not found", so `x#break` on a non-loop local is a diagnostic (loft#998, 2026-08-19)


Naming a declared local that is not a loop variable in `x#break` / `x#continue` was an
INTERNAL COMPILER ERROR on both backends — *"index out of bounds: the len is 1 but the
index is 18446744073709551615"*, a `usize` underflow.

`Variables::loop_nr` walked the enclosing-loop chain with the match in its loop CONDITION:

```rust
while c != u16::MAX && self.loops[c as usize].variable != target { c = …; nr += 1; }
nr
```

so falling off the end returned the chain length — one past the deepest valid level, and
indistinguishable from a real answer. Three sites in `Scopes::scan` then indexed
`self.loops[self.loops.len() - lv - 1]` and underflowed. The `in_loop` guard catches
"outside a loop", which is why `k#break` with no loop at all was always clean; nothing
checked that the NAME belonged to a loop.

It now returns `Option<u16>` and RETURNS ON THE MATCH, which is what makes the missing
case impossible to fall out of — "not found" and "the outermost loop" are different
answers and one number cannot carry both. An unbound name is `None` too: it names no loop,
which is the same answer as a bound one that names no loop.

Reported at the PARSE, not in `scopes.rs`: that is where the source position and the
author's spelling are, and `scan` has only a level. The message names what CAN be written
— `Variables::enclosing_loop_names` lists the enclosing loops' variables innermost first,
so a nested pair reads *"or `j#break` or `i#break`"*, and a `while` (which binds no
variable) offers the plain form instead. Outside a loop the pre-existing message stays the
only one; adding "…and `k` is not a loop variable" says the same thing twice from further
away.

Guards: four `parse_errors` cells — `for`, `while`-with-`continue`, the nested pair, and
the outside-a-loop control. The `while` cell is the one that would not have been written
from the `for` case alone: a `while` binds no variable, so the cure list is empty and the
message has to say something else.

### A package's own NAME is not one of its module names (loft#976 follow-up, 2026-08-19)


loft#976 made a bare `use <id>` inside a package bind that package's OWN `<id>.loft`, so a
stranger's same-named module can no longer amputate a library's public surface. The rule
had no stop at the package's own name, and `own_module_path` finds a same-named file
ANYWHERE in the package — including `tests/`.

Every library in the ecosystem writes its own suite as `tests/<pkg>.loft` containing
`use <pkg>;`. That import therefore bound the TEST FILE as the package's module, the
entry's `pub` surface never loaded, and every symbol read *"Unknown function … — the `X`
this build resolved does not have it, and the registry's `X` does"*. **Nine published
libraries went red** on the PR's `revalidate-libs` gate: hex_world, glb, regex, cbor,
crypto, server, shapes, pluginabi, zttext.

`use <pkg>` inside `<pkg>` asks for the PACKAGE. The guard now stops there (`id != pkg`),
and `use self::<pkg>` remains the explicit spelling for the file. The distinction is what
lets a package refer to itself at all: a name that means the package in one file and a
sibling module in another is a name that means nothing.

**Two wrong turns before the right one, both worth recording.** The first hypothesis was
"a package importing itself by name", and the minimal repro built from it PASSED on both
binaries — refuted, not confirmed. The trigger needs the same-named file to exist and NOT
be the entry, which is what `tests/<pkg>.loft` is; reducing DOWN from the red library
found it, building UP from a guess did not. The second was a mis-measurement of my own
making: `loft --tests src` walked `overland.loft`, a file the entry never imports and
which fails to parse on BOTH binaries, and that pre-existing error read as the regression
for several minutes.

Measured before/after on five libraries, each a clean copy in a scratchpad (running a
suite inside a consumer's tree writes `native-auto/` and `.loft/` and is not read-only):
regex 1→11, cbor 0→5, glb 0→21, zttext 0→46, hex_world 0→21 passing.

Guard: `module_name_clash::a_packages_own_name_means_the_package_not_a_same_named_file`,
built by hand rather than through `parse_two_packages` because the whole point is a file
named after its package in a directory that is not `src/`. loft#976's own sixteen cells
use module names distinct from the package name, which is why none of them covered this.

### `OpIndex` reaches the composite subscript, and the slice says what to write (loft#996, 2026-08-19)


@F114 says a type defining `OpIndex` "is subscripted like a built-in collection" and names
A MATRIX as the motivating case — and `m[r, c]` answered `error: Expect token ]` at the
comma. The dispatch half was never the gap: a two-index declaration is accepted and
`OpIndex(m, 1, 2)` works, so an author could write the method the feature exists for, call
it by hand, and never reach it as a subscript.

The indices are now parsed comma-separated and passed as ARGUMENTS, which is what the
accepted declaration already means (the issue's first design question, answered by the
declaration itself). That keeps the arity and type checks in the ordinary call path,
reported against the signature the author wrote.

**The SLICE half is refused, not implemented, and that is a decision.** `x[a..b]` is not a
subscript with a different argument: every built-in kind lowers its own
(`parse_vector_index`, `parse_text_index`, `parse_spatial_slice`, `parse_trie_slice`), each
to a dedicated runtime call, and there is no range VALUE in the language for a user method
to receive. So it needs a range type or an `OpSlice` of its own — a language addition, not
a parse — and the issue's own "expected" allows saying so. Making `x[a..b]` mean
`OpIndex(x, a, b)` was rejected: a matrix type would then have `m[1..2]` silently mean
`m[1, 2]`.

The refusal has two halves and the first version had only one. It must also CONSUME the
rest of the bracket, the way the pass-1 `Unknown` receiver arm does: returning with `..2`
unread cascades into `Expect token ]` on pass 1, and a pass-1 abort silences every pass-2
diagnostic — so the message existed and nothing printed it.

**One diagnostic beyond the issue**, because the comma form makes it reachable: a
one-index `OpIndex` given two reported `Too many parameters for t_4Ring_OpIndex`, naming a
storage symbol that appears in no source file. Two fixtures in the suite already record
that as a defect of its own (`tests/lib/dupmethod_a/…`, `tests/scripts/850-…`).
`Data::user_facing_name` renders `n_<name>` as `<name>` and `t_<LEN><Type>_<name>` as
`Type.name` — the receiver stays visible because that is exactly what is ambiguous where
these messages fire, between two packages or two arities — and anything it cannot parse
comes back unchanged, since it decides how a name is SHOWN and must never lose one.

Guards: `tests/scripts/996-opindex-composite-subscript.loft` (two / one / three indices,
and index EXPRESSIONS including a nested subscript, so "two" is not mistaken for the rule)
and two `parse_errors` cells pinning the slice refusal and the demangled arity message.

### A void native with no lowering is a hard error, not an empty body (loft#993, 2026-08-19)


`output_function` escalates an unimplemented native to `compile_error!` when reachable and
`todo!()` when not — P269's "fail at startup, not runtime". Both legs gated that on
`*def.returned() != Type::Void`, so a VOID one was emitted as `{}`: a function that
compiles, is callable, and does nothing. The principle had no effect on the half of the
surface where the failure is silent instead of a panic, and that is what hid the par
discard route for its whole life — `--native` emitted `n_parallel_discard`'s declaration
with an empty body, and only an unrelated arity mismatch made "runs no workers" visible
(loft#987).

**The filed analysis was wrong on its central claim and is corrected on the issue.** It
said dropping the guard was unsafe because `self.reachable` counts a call even where a
custom emitter renames it. It does not: the internal leg's predicate already answers *"is
this def actually called through its declaration"* —

```rust
let reachable = (self.reachable.is_empty() || self.reachable.contains(&def_nr))
    && def.rust().is_empty() && !is_iface_stub && !is_t_stub && !has_custom_op_emitter;
```

— so the escalation was never a predicate short, it was one `if` too narrow. Re-measured
over the ten stubs a typical emit carries: three are excluded by their `#rust` body
(`n_eprint`, `n_store_lazy_fail`, `n_host_output` — inlined at the call site), six by a
registered `OpEmitter` (`n_parallel_buf_drop*` via the rename emitter, `n_parallel_discard`
since loft#987), and exactly ONE was relying on the silence.

That one is `yield_frame`, and its no-op is genuine: a `--native` binary has no interpreter
state to resume and no host loop to return to, so a frame-driven program runs straight
through. It now carries `#rust "()"` — the silence is what the declaration SAYS rather
than what falls out of "nobody implemented it", which is the whole distinction loft#993 is
about.

Both legs lose the guard. The `#native` leg's own predicate is untouched: a `#native`
binding whose symbol no registered crate provides, CALLED by the program, is now the same
hard error for a void return that it always was for a value return.

Guard: `tests/native_no_silent_stub.rs` — a property of the emitted source rather than a
list of names, so it keeps holding for built-ins that do not exist yet, which is the point
of a guard for a silence: **no generated function may be both CALLED and EMPTY.** An
unreachable empty declaration is harmless and stays legal.

The control cell changed shape while being written, and that is the measurement worth
keeping: it first asserted the emit still CARRIES empty-bodied declarations, so the
property could not pass vacuously — and it does not. Lifting the guard took the count to
ZERO on the probe. All ten became `todo!("native function …")`, and no `compile_error!`
appeared, which is the other half: nothing that is lowered elsewhere started being refused.
So the control pins the signature instead — `n_parallel_discard`, the void stub loft#987
was about, must appear LOUD while its CALL goes to the runtime helper. Plus a behavioural
cell that `yield_frame` still runs on both backends.

### `FieldInfo.nullable` follows the declaration for every field kind (loft#995, 2026-08-19)


Documented as *"was the field DECLARED nullable"* and named as the fact a generated
`CREATE TABLE` needs for `NOT NULL` (@F107); correct for the seven scalar kinds, a
constant `true` for enum / record / vector / keyed. A generic serialiser emitted all four
as nullable columns. The two spellings genuinely differ — construct every field with
`null` and `x.f == null` answers `false` for the non-null spelling — so this was a fact
being LOST, not two things that are one.

Scope, not logic. @PLN25 DN1 derives the flag from the `Optional` wrapper and the rollout
gated that on `is_non_null_scalar`; everything else kept the pre-DN1 parser default
(`true`). The gate is gone, so the derivation is `matches!(a_type, Type::Optional(_))` for
every field. The synthetic tuple attributes have derived it that way from every element
type since @PLN114 — a declared field now agrees with them.

The flag is not reflection-only, and the suite is what settled the blast radius: it feeds
the JSON-import default (`set_default_value_nullable`), the `not null` hint counter, and
the narrow-integer op pair (`NarrowIntKind::of` — Integer fields only, which already
derived correctly). Each moves in the same direction, toward the declaration. One visible
consequence: `redundant-null-check` now fires on a non-null heap field compared against
`null`, where before it saw only scalars.

The forward-reference hazard was checked because the flag is deposited on pass 1 and never
revisited — a member type declared BELOW its user reports correctly on both backends.

**One kind is exempt, and the suite is what found it.** `reference<T>` in field position is
#328's documented POINTER, and a pointer holds null however it is spelled: `n.next = null`
is legal on it and an omitted one DEFAULTS to null, both pinned by
`issue_328_reference_field_pointer_semantics` — which went red on the first version of this
fix, reporting `redundant-null-check` on the very comparison it then asserts. Deriving from
the `?` there answers a question the spelling does not decide. The pointer marker
(`Deps::pointer_marker()`, the `u16::MAX` dep the parse stamps to select that 12-byte
layout) is the exact discriminator — a by-VALUE `r: At995` is `Type::Reference` too and
genuinely cannot hold null. Measured on all four: `byval` false, `byval?` true, `ptr` true,
`ptr?` true, reflection agreeing with `x.f == null` in every cell.

⚠ Writing that cell surfaced a separate PRE-EXISTING defect, measured identical on a
pre-fix binary and NOT filed here: a forward-declared nullable record or enum field types
as `unknown?` at a comparison site, so `x.rq == null` is *"No matching operator '==' on
'unknown?' and 'null'"*. It is why the declared-below cell compares reflection against a
written-down table instead of against the language, which is the weaker oracle — the
declared-above cells carry the real one.

Guard: `tests/reflect_declared_nullable.rs`, both declaration orders on both backends. The
above-cells read the truth out of the RUN (`x.f == null` per field) rather than a table, so
a cell that agreed with a table but not with the language would still fail.

### A paged source validates the store signature (loft#994, 2026-08-19)


A lazy binding reported every failure to OBTAIN bytes and none to INTERPRET them. Missing
file / HTTP 404 / connection refused each set `store_lazy_faults` and `store_lazy_error`;
an empty file, eleven bytes of text, 8 KB of noise, a directory, and an HTTP `200` serving
an error page set neither — `faults 0`, `err ""`, `store_verify true`, every key `null`,
which is precisely what a valid image with an absent key answers. Same family as loft#802,
which fixed the refusals that route through `refuse_paged`; this is the hole where nothing
reaches such a site at all.

`PageSource::open` validated nothing — it opened the file and read its SIZE, and a size is
not a format — so a non-image failed deep inside `load_one`, which has only `false` to
return. Both legs now read the four-byte signature the format has always carried:
`LocalFileProvider::open` from the handle it just opened, `HttpRangeProvider::open` with
one extra four-byte range read per bind. Refusing THERE is what makes every existing
`refuse_paged(path, "it cannot be opened as a paged source")` site inherit a report it
already words correctly.

`Store::has_signature` is the one home for the fact, and `is_store_file` (the startup
cache's pre-check) now calls it too — the question is asked from two very different
distances, a whole file and four range-read bytes, and one predicate answers both. Fewer
than four bytes is not an image either, which is how an empty file gets its reason.

The boundary, measured per-cell in its own process. Five sources now fault with a reason
where four were silent; a real image is unchanged (`null=false`, `faults=0`, quiet).

Separate processes because of a NEIGHBOURING defect this measurement surfaced, and the
first reading of it was wrong: the channel is not per-run. Two collections alive at once
report their own sources correctly. What is missing is that `store_bind_lazy` does not
CLEAR the channel of the collection it binds — rebind a faulted collection to a good
source and the lookup answers correctly while `store_lazy_faults` still says 1 with the
old source's message. A function that binds a fresh local six times reuses one slot and
so accumulates: `1, 2, 3, 4, 4`, the last of them under a healthy answer. Not slot reuse
(`LOFT_NO_SLOT_REUSE=1` changes nothing) and not this bug.

**One row moved on purpose**, and it is the row loft#994 records as "not a defect": an
image whose first four bytes are overwritten used to answer the correct record, because
the lazy reader only touches the pages it needs and those were intact. It now refuses.
Answering from a file whose magic is wrong was luck, not a promise, and refusing it is
what a magic number is for. A merely TRUNCATED image is unaffected and is pinned in the
same cell, so neither direction can move by accident.

**A neighbouring "defect" that turned out to be the contract — recorded because the wrong
turn is the useful part.** Measuring the boundary showed a rebound collection still
reporting the previous binding's faults, and `bind_lazy`'s own comments say a rebind
re-pins the source and re-decides the schema (*"a different world now"*), so clearing the
channel there looked like the missing half. It is not. `tests/scripts/129-lazy-bind.loft`
pins exactly that shape — fail, rebind, succeed — and its comment gives the reason: the
faults describe the CONTENTS, not the source, and the contents do not reset on a rebind.
Whatever the previous binding materialised is still resident, minus the rows that failed,
so "healthy" after a rebind is the silent wrong answer the channel exists to prevent. Only
`store_lazy_clear` clears. The change was reverted with that reasoning written at
`bind_lazy`, where the next reader will have the same idea.

Two readings of the same measurement were wrong before that one: it is not per-run state
(two collections alive at once were always isolated) and not slot reuse
(`LOFT_NO_SLOT_REUSE=1` changes nothing). What remains unexplained, and is NOT the same
thing, is a FRESH collection landing on a recycled `(store_nr, rec, pos)` and inheriting a
channel that was never its own — a function binding a new local six times accumulates
`1, 2, 3, 4, 4`. Clearing on bind is the wrong cure for it (that is the contract above);
clearing when the SLOT is released would be the right place, and is not attempted here.

Guard: `tests/lazy_source_not_an_image.rs` — the five silent sources, a valid-image
control, the truncated/broken-signature pair, and the rebind. Each cell gets its own fixture directory:
they run in parallel and each removes its tree, so one shared path is one cell deleting
another's image (the first version of the file did exactly that).

### A `never` block places its frees like a `Void` one (loft#992, 2026-08-19)


A `match` in a function's TAIL position with a `return` in ANY arm freed that function's
locals BEFORE the arms ran. The arm that does not return then read a released variable:
`null(oob)` on `--native` against a correct interpreter answer, an EMPTY text on both, and
on a droppable a drop before the arm plus a second one at the `return` — a use-after-free
that panics native on the 65535 freed-record marker.

A `return` in an arm types the match block `never`, and `Scopes::insert_free` routed
everything that is not `Void` down the value-returning leg. That leg exists to hoist the
tail into a `__ret_N` temp so the tail EVALUATES before the frees (the @PLN85 / B5-L3
invariant) — but it hoists only a result type it can hold, and `never` yields no value.
So `is_value_return_type` / text / heap-ref all answered no, `hoist_tmp` stayed `None`,
and the final `else` emitted `ls.extend(ret_frees)` and then the tail. Frees first, tail
second, with the tail still able to read them.

`Never` now joins `Void` at that branch, because it is the same SHAPE for free placement:
no value, nothing to hoist, nothing to return. The two legs there already decide correctly
by whether the tail can COMPLETE — a tail that unconditionally returns keeps the frees in
front of it, a tail that may still complete runs first and the frees follow. The
returning arm emits its own frees on its own path, as it always did, so nothing is freed
twice.

The boundary, 20 probe cells, 9 of them red against the defect. Not axes: the arm count;
which arm returns; whether the returning arm is the taken one; a scalar match vs an enum
match; nesting; two locals vs one; `return` in EVERY arm. Axes: the match must be the
function's TAIL (a statement after it, or the match wrapped in an `if`, was always
correct), the terminator must be `return` (`break`, `continue` and `panic` were correct),
and the function must return VOID — a value-returning one was correct all along, because
its result type is hoistable and the tail therefore already evaluated first.

Guards: `tests/scripts/992-match-tail-with-return-arm.loft` (11 cells on both backends,
using a VALUE read in the arm) and `tests/match_tail_return.rs` (the drop COUNT and its
position relative to the arm body, both backends). The two oracles have different
sharpness and that is why both are here: on the interpreter a freed store keeps its bytes
until something claims them, so only the TEXT cell of the script goes red against the
defect — against seven cells on native, and against every cell of the drop-count test on
both.

### A lock-less project is not governed by the invoking directory (loft#991, 2026-08-19)


Already fixed on this branch by @PLN143 arc C2, which deleted the cwd `loft.lock` leg from
the resolution chain; what lands here is the GUARD for the row the arc's own cells left
open. `probe_project_lockfile` finds the project root, finds no lock in it, and returns —
and the chain then fell through to the cwd probe, so a stranger's pin governed a project
that declares its own dependency. The two existing arc C2 cells both use a BARE script,
which belongs to no project, so neither covered it. Measured against `tuxedo-post-973`
(no arc C2): one project, one manifest, three invoking directories, three library
versions.

`arc_c2_a_cwd_lockfile_does_not_pin_a_lockless_project` asserts the ABSENCE of the
stranger's version, and `arc_c2_a_projects_own_lock_outranks_the_invoking_directorys` is
what keeps that from being vacuous — same fixture, project lock present, resolves. The
absence is what the cell can assert because offline a lock-less project resolves NOTHING:
`probe_cache_newest` is `Bare`-scope only by design (a fallback takes the newest cached
copy, and only where nothing is declared can that violate no constraint). That refusal is
PRE-EXISTING — measured identical on both branches with no lockfile anywhere — so the
difference the cell measures is exactly the filed one: `probepkg-0.1.0` against the
defect, no version at all with the leg gone.

### A backtick block dedents from its first content line, holes included (loft#990, 2026-08-19)


A backtick block was dedented unless it contained a `{…}`, in which case it was not
dedented at all and kept its trailing whitespace-only line. One hole anywhere in the
block, before or after the affected lines, was enough — so the feature served the block
with no values in it (a GLSL shader, LOFT.md's own example) and stopped serving the
TEMPLATE, which is the shape it exists for. LOFT.md's second example sat under the
sentence describing the strip and was not stripped.

**The rule and the streaming were incompatible by construction, not by oversight.** The
strip was `closing-backtick column - 1`, computed and applied when `backtick_string`
reached that backtick. An interpolation makes the scanner emit the text accumulated so
far and return (`Mode::Formatting`) long before the closing column is known, so the
`Some(&'{')` arm built its segment with no strip and `backtick_string_resume` continued
without one. Every compat-preserving cure needs the closing column BEFORE the first hole:
a shadow scanner for the string grammar (a second home for it), or a rewindable lexer
that drives the real scanner once to measure and again to emit (M+, and it touches the
diagnostic path and the token-memory replay).

The base is now **the first content line's indentation**, which is knowable before any
hole can occur, so a holed literal and an unholed one answer alike. `backtick_line_start`
consumes a line's leading spaces as the line is ENTERED, settles the base on the first
line that has content, and returns `spaces - base`; both scanners call it, so there is one
home for the rule and the resumed segments get it too. `backtick_strip` is a STACK, one
entry per open literal, because a backtick literal can be written inside another's hole —
pushed where `next()` opens one, popped in `close_backtick`.

What settles the base is decided by one peek after the spaces: end-of-line means a BLANK
line (settles nothing — a template may open with one, and taking its zero would switch the
dedent off for the block), a closing backtick means the LAST line (dropped when it holds
only whitespace, so not content either), anything else is content. The opening backtick's
own line can never be the base: it starts wherever that backtick ended, so its indentation
is the statement's.

Two visible consequences, both only in blocks laid out unusually. A block whose closing
backtick sits at a different column than its own lines now follows the LINES. And a line
indented LESS than the base comes out flush (`spaces.saturating_sub(base)`) instead of
keeping all its indentation — which used to leave it further right than the siblings that
were indented PAST it, the inversion loft#990 lists as a related silent edge. A
TAB-indented block is still untouched: a tab is not a space, so the count is zero.

`backtick_string`'s closing arm no longer strips — every line arrives dedented — and keeps
only the two layout rules. `backtick_string_resume` gained the trailing-blank-line drop it
never had (`drop_trailing_blank_line`), which is the other half of the reported symptom.

**One in-repo program moved, and it is the instructive one.**
`scripts/build-playground-examples.loft` builds `doc/examples.js` out of six holed
backtick blocks, and took the newline BETWEEN two appends from the closing line the old
behaviour kept verbatim. With the block dedenting properly that line is layout and is
dropped, so the whole file came out as nine lines. The generator now ends each block's
last content line with an explicit `\n` — the newline is written rather than inherited —
and the regenerated `doc/examples.js` is byte-identical to the committed one once
whitespace is ignored (40 examples, 40 list entries, 2 groups, checked through `node`).
`doc_hygiene::doc_examples_js_is_up_to_date` is what caught it.

Guard: `tests/scripts/990-backtick-dedent-with-holes.loft`, 11 cells on both backends —
hole in the middle / on the opening line / on the first content line, blank first line,
outdented line, tabs, a nested block inside a hole, doubled braces, content on the opening
line, empty block, and the template shape LOFT.md advertises. 9 of the 11 fail against a
pre-fix binary. LOFT.md's shader example is corrected too: `void main() {` opens a hole, so
it never compiled; doubled, it compiles AND dedents.

### A `{` that opens a hole nothing closes says so (loft#989, 2026-08-19)


The two ways of getting a literal brace wrong got very different answers. `}` reached
`Lexer::unescaped_brace` — one home for four scanners, a code, and a `Mechanical` fix
naming `}}`. `{` had no equivalent: the scanner opened a hole and returned, and the failure
surfaced later at `objects.rs`'s generic `diagnostic!(… "Formatter error")`, the "the string
did not resume after a hole" path, which cannot know a `{` started it. Measured on
`println("a lone open { here");` — SIX diagnostics, the last of them blaming the function's
own closing brace, and not one of them mentioning `{{`.

Reporting it needs the hole's fate to be KNOWN at the `{`, and it is: a hole holds code, the
code scanner stops at the end of a line, so a hole that does not close on the line it opened
never closes at all (measured both ways — `"a {` + newline and the same in a backtick
literal are both errors today). `hole_closes_on_this_line` scans the rest of the line from a
CLONE of the char iterator with a four-state stack (code / string / backtick / char literal)
and brace depth; `Lexer::unclosed_hole` is the `}` twin — coded `format-unclosed-hole`,
`Mechanical` fix `{{`, caret ON the brace rather than one past it.

The scan's direction is the safety property: a wrong `true` leaves the pre-fix behaviour,
a wrong `false` would refuse a legal program. It answers `true` for the one thing it does
not model, a `//` comment. A `` ` `` in code can only OPEN a literal (code has no bare
closing backtick), and one that runs past the end of the line cannot let the hole close on
that line either — which is also what the enclosing literal's own terminator looks like
from inside an unclosed hole, so both readings are the same error.

Recovery is to treat the `{` as the literal brace the fix advertises and keep scanning, so
the string terminates where it was going to and nothing cascades: six diagnostics down to
one. Verified silent on `{x}`, `{S{x:7}.x}`, `{"q{y}q"}`, `{\"inner\"}`, `{if c == '}' {…}}`,
`{{`/`}}`, and a nested backtick literal in a hole.

Guards: `tests/error_messages/cases/54_format_unclosed_open_brace.loft` (the whole rendered
output, so the single-error shape is pinned too) and the `e1_code_set` registry row. No
existing error golden moved.

### The par discard route has a native lowering (loft#987, 2026-08-19)


`for x in v par(r = f(x), N) { }` — a body that never names the result — lowers to
`n_parallel_discard`. Every other live par route has a bespoke native emitter; this one had
none, so it fell through to the declaration-driven default and `--native` emitted
`n_parallel_discard`'s loft DECLARATION, whose body is EMPTY. The only thing separating that
from a silent no-op was an unrelated arity mismatch (the IR pushes six args, the declaration
has five), which rustc refused: add the missing parameter and the program compiles and runs
no workers, on one backend only.

`n_parallel_discard_native` (`codegen_runtime.rs`) + `ParallelDiscardEmitter` +
the registry row, and `n_parallel_discard` joins the `collect_calls` list so the worker fn
is in the reachable set. The closure returns `()` rather than the worker's value: with the
result dropped the return SHAPE stops mattering, so one runner covers scalar, float, text
and heap-reference workers alike, and the emitter needs neither a per-shape return bridge
nor `return_size` nor the heap-ref storage type — only the `&mut String` work buffer a
non-owned text worker must still be handed.

**Making the backends comparable is what exposed the INTERPRETER's half**, wrong in two
ways nothing could witness — a route that drops every result produces nothing to compare
against. `run_parallel_discard` had only the `DbRef` input arm, so a worker taking
`integer` read the row pointer's bits; it now runs the same input ladder
`run_parallel_queue` does (text / wide-tuple / primitive / DbRef, with `u32::MAX` leaving
before any size test — it is the TEXT marker, not a width). And it never pushed the hidden
parameters a compiled worker has: the `__work_N` buffer of a text return, the destination
of a heap return. Missing them shifted every slot the worker reads — the text case
SEGFAULTED the interpreter. Both counts now come from the same two readers
`parallel_queue_dispatch` uses, and the route picks `execute_at_text` / `execute_at_ref`
accordingly. No adoption or rebasing: the worker's whole `Stores` clone dies with the
batch, which is what discard means.

Guards: `tests/scripts/987-par-empty-body-discard.loft` — each worker ASSERTS the row it
was handed, across scalar / float / text / text-input / struct / vector / boolean returns
plus an empty input, with a queue-route control for count and value (a pre-fix binary
SIGSEGVs on it) — and `tests/par_discard.rs`, which uses a side effect as the oracle
(each worker prints its row) because a discarded result cannot be one, plus an emit check
that the CALL reaches the runtime helper.

Left standing, and worth knowing: `output_function` gates its `todo!()` / `compile_error!`
stub on `*def.returned() != Type::Void`, so a VOID native with no implementation is emitted
as an empty body — silently — where a value-returning one is loud. Ten such stubs are in a
typical emit; each is fine today only because its own call site is rewritten elsewhere
(`n_parallel_buf_drop*` by the rename emitter, `n_eprint` inlined from `#rust`) or is a
deliberate no-op on this target (`n_yield_frame`). Nothing checks that, and it is what hid
this bug for its whole life.

### A par worker declared below its loop keeps its return type (loft#988, 2026-08-19)


`b_type` in `build_parallel_for_ir` asked two questions in the wrong order:

```rust
if matches!(ret_type, Type::Unknown(_)) { I32 } else if fn_d_nr == u32::MAX { Unknown } …
```

On pass 1 a worker declared BELOW its loop answers `(u32::MAX, Unknown(0))` — BOTH
conditions at once — and the first arm won, pinning `_b_par<n>` to `integer`. Pass 2
refines only a slot that is still unknown, so it stayed. The comment above it already
described the intended behaviour ("On the first pass fn_d_nr is u32::MAX; use Type::Unknown
… Using I32 here caused the type to stick as integer even when the worker returns float or
boolean") — the code had the arms the other way round.

Only a COMPOUND assignment showed it: `t += b` retyped a float accumulator to integer and
the PASS-1 error aborted the parse before pass 2 could correct it, while `t = t + b`
coerces and passed, and `println("{b}")` printed correctly because `b` is inline-substituted
by the element accessor and only its DECLARED type reaches the body's type check. The
instrument that named it was `LOFT_VAR_TABLE=main`: `_b_par0 int` with the worker below,
`float` with it above.

**The arm order alone was not the fix** — the suite said so. `parse_parallel_worker`
answers `(u32::MAX, Unknown)` for two OPPOSITE reasons, and both callers were reading one
sentinel: a worker declared below the loop (not resolved YET) and a worker it REFUSES,
which is a generator return or a name that does not exist. The refusals lean on `integer`
deliberately — their own comment says so — because `b` has to carry a usable type or every
use of it in the body earns a second diagnostic under the reported one, and reordering the
arms alone added `Unknown variable '_b_par1'` to
`parse_errors::par_worker_returns_generator`.

The PASS is what tells the two apart: `fn_d_nr == u32::MAX && self.first_pass` is "not yet"
and answers `Unknown`; on pass 2 the same sentinel means "never, and the error is already
reported" and keeps `integer`. A RESOLVED worker whose return type is still unknown keeps
`integer` too — the width the downstream route decisions assume.

Guards: `parse_errors::par_worker_returns_generator` (which caught the first attempt) and
its new sibling `par_worker_that_does_not_exist_reports_once`, holding the pass-2 half for
the OTHER caller of the sentinel — one error, the body's `b > 0` silent, so the two intents
cannot quietly re-collapse. Behaviour:
`tests/scripts/988-par-worker-declared-below.loft`, 7 cells on both backends —
`+=` / `=` / `/=` over a float return, integer, text and boolean returns, and a vector
accumulate — every worker declared below its loop. A pre-fix binary fails 3 of them at the
parse. Each loop is the LAST statement of its function on purpose: a par loop with a
below-declared worker also reads as a value rather than a statement, so anything after it
demands a `;`. That is a separate defect with a separate fix, already landed on the
consumer stream's branch; keeping the loop last means this file measures the TYPE alone.

### A struct-enum field access checks the tag (loft#980, 2026-08-18)


`c.field` resolved at COMPILE time to the first variant declaring the name and read that
offset whatever the tag said:

```loft
enum Node { Named { label: text, n: integer }, Anon { k: integer } }
a: Node = Anon { k: 7 };
a.n            // 7 — that is Anon's `k`, handed back as Named's `n`
a.label = "x"  // stored in the Anon record, which goes on calling itself an Anon
```

Direct payload access STAYS — C89 decided permanently that enum payloads are named fields
you read straight, with matching for DISPATCH and never for extraction, so refusing
`c.field` (issue option 1) is the outcome that decision exists to prevent, and option 3
restricts it the same way. C80/C85/C90 then fix what the check must ANSWER: a read that
cannot be computed yields the type's null sentinel and the program keeps running, like a
hash miss or an out-of-range index. So the access TYPE is unchanged, `a.n` on an `Anon`
answers null, and a write to a field the value does not have is suppressed.

**The guard goes on the RECEIVER, not the access** — `if tag(c) ∈ declaring { c } else
{ null }` — which is what made the write half tractable. A null receiver ALREADY reads as
null and ALREADY swallows a write, on both backends and with no new opcode, so both halves
fall out of machinery that exists. And because only the receiver changed, the access is
still a PLACE: the assignment path needs no notion of a guarded lvalue, which is what the
issue recorded as the blocker (`if tag ∈ D { read } = rhs` cannot be an assignment target).
The one seam it does touch is `lhs_base_var`, which now looks through the guard — recognised
by its else arm being the zero-argument null sentinel, so an ordinary `if` on the left of an
assignment is still not a place and is still refused.

The guard is skipped, at no cost, where the question does not arise: every variant declares
the field (the common-prefix case, correct today because a shared name+type shares a slot),
a synthetic `__nullable<S>` (@PLN25's null model — guarding it would make `v[i].field`
answer null), and a receiver that is not a place read. That last is a real bound: the guard
reads the receiver twice — once for the tag, once as the value, which is what a struct-enum
`match` does with its subject — so a receiver that is a CALL keeps the unchecked access, and
the diagnostic says so rather than claiming a check that is not there.

**`OpNullRefSentinel`, not `OpConvRefFromNull`.** The latter's `Stores::null()` is
`database(u32::MAX)` — it ALLOCATES — so the first draft leaked one store per guarded
access, caught by `loft_suite`'s per-script leak gate on this issue's own probe.

**A write through an ABSENT destination was fatal, not refused** — and that one is not
about enums. `set_default_value_nullable` wrote a field's default into the destination
record without asking whether there IS one, so `allocations[u16::MAX]` panicked the
interpreter. Two ways in, one contract: `s.v += [1]` on a null `S?` (which reproduces on
`main`, independent of this issue), and — once the guard above exists — an append to a
collection field the value's variant does not declare. The scalar write path already
honoured the contract (`if db.rec != 0 { … }`); the default-init path honoured neither
spelling of absence. One guard, sibling to the `tp == u16::MAX` return directly above it
(nothing to write INTO rather than nothing to write), closes both. Found by this issue's
own composition probe, which is why the guard is only correct WITH it: without it, the
`c.field` fix turns silent corruption into a panic, which is the wrong trade.

`variant-field-unchecked` stays a WARNING and its message was rewritten: a message
describing behaviour the compiler no longer has is worse than none, and one derivation now
decides both the guard and what the message says. Tier unchanged because a suppressed write
is a lost write, which is the two-tier rule's own gating example. `LOFT_NO_VARIANT_FIELD`
silences the message only — semantics must not depend on a diagnostic switch, pinned by
`the_diagnostic_opt_out_does_not_change_the_answer`.

Guards: `tests/scripts/980-variant-field-answers-its-own-variant.loft` +
`tests/variant_field_semantics.rs` (behaviour); `tests/variant_field.rs` keeps the
diagnostic.

### The post-scope lints run under `loft test` too (loft#985, 2026-08-19)


Five lints share one precondition — they read the ownership verdicts and the materialised
copies that exist only after `scopes::check` — so they sat in one block on `main.rs`'s
PROGRAM path. `loft test` / `--tests`, which is the path a LIBRARY's CI takes, ran none of
them: a library could ship a `#superseded` steer pointing at nothing (a hard ERROR anywhere
else) and writes that land in a copy, with a green suite. That is the hole @PLN107's lint
was written for — its motivating case is a published `graphics` canvas that shipped every
drawing primitive as a no-op through the copy-mutate shape, checked by
`LOFT_DENY_WARNINGS=1 loft --interpret --tests tests`.

What hid it is that the split is INSIDE the diagnostic set: `warning[never-read]` reached
`--tests` and always did, so "tests are quiet" was never the rule.

`use_analysis::post_scope_lints` is now the ONE home for the set, error gate included
(loft#883: they all read RESOLVED types, and an aborting error means resolution did not
finish, so an unresolved type's empty deps read as OWNED and an unrelated library's `for`
variable reads as a lost write). `main.rs` and `test_runner.rs` both call it — two callers,
one list, so the sets cannot drift apart again.

**The ordering was the actual fix.** `scopes::check` ran in the test runner AFTER test
discovery, while diagnostics are collected into `FileResult` well before that — so a lint
reporting there wrote into a struct nobody read again. The scope check now runs directly
after the parse and before the collection, which is also where its own diagnostics become
visible. Called once per FILE, not per test: each test compiles its own bytecode from one
`Data`, so a per-test call would report every finding N times
(`one_finding_is_reported_once_across_many_tests` pins 3 tests → 1 report).

Guard: `tests/post_scope_lints_under_tests.rs` — the dangling steer FAILS the run, the lost
write is reported, `LOFT_DENY_WARNINGS=1` goes red on it, the count is one across three
tests, and `never-read` still reaches the test path (the control for the split that hid it).

### An empty struct literal parses before its declaration (loft#986, 2026-08-19)


`T { }` was a parse error when `T` was declared BELOW the use, while `T { port: 0 }` in the
same position was fine and `T { }` was fine with `T` declared above — `error: Expect token ;`
pointing at the line rather than the type, so it read as a syntax mistake in code that has
none. Legality by declaration ORDER, for the spelling that asks for the whole default record.

A type pass 1 cannot resolve falls to a fallback that consumes the `Name { … }` body, and
that fallback recognised a literal by SHAPE — an identifier followed by `:` or `,`. An empty
body has no field to shape-check, so it did not match, the `{` went unconsumed, and the
statement failed. (Fourth member of the family where a pass-1 fallback accepts fewer
spellings than the construct has; the other three were named arguments in the method
spelling, the shared argument-list skipper, and the compound-key index.)

The shape check is what keeps a control-flow body from reading as a literal, and an empty
body cannot be told from one that way — `if b { }` with an undefined `b` is identical. So
the head of an `if` / `while` / `for` sets `in_control_head` (saved/restored, like
`in_loop`) and only the empty-literal case consults it. Measured both directions: without
it, `if b { }` answered `Expect token {` where the useful message is `Unknown variable 'b'`.

The pre-existing sibling is left alone and noted: `if b { x: 1 }` with an undefined `b`
already read as a struct literal before this change, and still does.

Guard: `tests/scripts/986-empty-struct-literal-before-its-declaration.loft` — the empty
literal below its declaration, with a declared field default, nested, as an argument and a
return, plus the control-flow controls.

### Float `/0` is IEEE in every destination (loft#983, 2026-08-19)


`1.0 / 0.0` was `inf` inline and `null` once bound, returned, or stored — one operator, two
ops, and the DESTINATION picked between them: `OpDivFloat` forced `f64::NAN` on a zero
divisor while the `OpDivFloatNullable` peer did raw IEEE, emitted at "defended" sites.

The model is IEEE with **NaN as the one float null** (`doc/23-safety`: "NaN … is null"), so
`0.0 / 0.0` is null and `1.0 / 0.0` is `inf`. Three things already said so and now agree:

- `tests/scripts/02-floats.loft` pinned `1.0/0.0 is positive infinity`, `log(0)` as `-inf`
  "(not null)", and IEEE infinity arithmetic — beside a `runtime_warnings` test demanding
  the SAME expression be null when undefended. Both were green because the op split made
  defended ≠ undefended; that is the bug, not a coincidence.
- float OVERFLOW is the sibling: `1.0e308 * 10.0` is `inf` in every position and always
  was, though it yields the identical IEEE value. That asymmetry named the defect.
- `formal/types.md` DN3-Float, which introduced the nullable peers, says in bold that it
  was a TYPE-level change and *"Runtime is UNCHANGED"*. The forced NaN was a runtime change
  it had promised not to make.

`??` was self-defeating under the old split: at a `a / b ?? 0.0` site the peer that never
yields null was chosen, so the idiom every numeric library uses to defend a divide guarded
nothing (`mesh3d::normalize3` was sound only because a zero-length vector zeroes the
numerator too, making it a genuine `0.0 / 0.0`).

`OpDiv/RemFloat` + `OpDiv/RemSingle` keep the C80/E-Report Warn on an unguarded zero
divisor and return the IEEE result. The TYPE is untouched — `/` still types `float?`,
because `0.0 / 0.0` still yields null — so `(N-Store)` still fires and
`float_div_var_on_is_nullable` still counts its warning. The `*Nullable` peers are now
behaviourally identical and are dead weight, a separable cleanup exactly as the integer
split already is. **`src/fill.rs` is GENERATED** from these `#rust` bodies
(`cargo test --test issues regen_fill_rs -- --ignored`) — hand-editing it is what
`fill_rs_up_to_date` exists to catch.

Guard: `tests/scripts/983-float-divide-by-zero-is-one-answer.loft` (every destination ×
`inf`/NaN/overflow/`%`/`single`). Updated to the model, intent preserved: `runtime_warnings`
f4f float+single (renamed `…_reports_and_continues`, still exit 0, still warning),
`nullflow_phase3` float_div_var (still `warns == 1`), `184-i333` (its float cell split into
an `inf` cell and a `0.0/0.0` null cell), and one golden baseline.

### A limited field stores its default when a value does not fit (loft#984, 2026-08-19)


`integer limit(lo, hi)` stored three different wrong things and reported none: `x.b = 256`
on `limit(0,255)` ALIASED to `0` (`set_byte` admitted `min + 256`, storing `256 as u8` = 0,
so the field read back as `min`); `x.b = 260` was DROPPED (the setter returned `false` and
every caller ignored it, leaving the previous value); and `x.s = 70000` on `limit(0,65535)`
WRAPPED to `4464` (`set_i16_raw` had NO range check — a truncating `(val - min) as u16`).

Three encodings, three different range bugs, now one rule: a value the field cannot
represent stores the type's **default** — the lowest value in its range, or **null** where
the field is nullable, since absence is a value that type can hold and is the honest answer
for "this did not fit". A slot never holds a value its type cannot represent.

The bounds, each read off the encoding rather than assumed: `set_byte` stores `val - min` in
a u8, so `min ..= min + 255` (not `+256`); `set_short` stores `val - min + 1` reserving raw
0 for null, so `min ..= min + 65534` (the old bound was off by TWO); `set_i16_raw` stores
`val - min` with no sentinel, so `min ..= min + 65535`. Each is now a `Store::*_fits`
predicate read by BOTH the setter and the nullable wrapper — two derivations of "does this
fit" is how one field came to be stored three ways.

The width follows the SPAN, not the magnitude (`(L-Narrow)`): `limit(300, 400)` and
`limit(-200, 0)` are one byte each and round-trip their whole declared range — a check
written against the magnitude would break them, which is why the guard pins both.

**The DECLARED range needs a second layer, because the store cannot see it.** A store op
carries the field's `min` and its width — so it catches what the width cannot represent,
and nothing else. `m.v = 500` into a `limit(300, 400)` byte encodes to 200, fits, and used
to read back as 500. And a declared range on a LOCAL had no store op at all to carry it, so
`a: integer limit(0,255) = 7; a = 300` simply kept 300 — unenforced entirely, not merely
mis-stored, because `is_narrowing_int_store` gates on `forced_size`, which `limit(...)`
never sets.

`OpRangeDefault(val, lo, hi, dflt)` closes both by guarding the **value** rather than the
store: outside `lo..=hi` it reports `RangeDefaulted` (a Warn on the same recoverable channel
as ÷0 — the run continues, exit 0) and answers `dflt`, the lowest value in range or the null
sentinel where the slot admits null. A null passes straight through: whether a null may land
there is `(N-Store)`'s question, and substituting `lo` would invent a value the program never
computed.

Two emit sites, because neither reaches the other's shapes: the assignment seam in
`expressions.rs` (whose own comment records that it covers "both the annotated local and the
field WRITE"), and `Parser::convert`, which reaches a struct LITERAL's field, a call
argument and a return. The guard is idempotent, since both fire on a plain field write and a
guard wrapping a guard would report twice.

Two bounds, both measured rather than assumed:

- **emitted only where the value is not PROVABLY in range**, read off the range the source
  type already carries — so `x.b = 7` and `p.r = q.r` between two `limit(0,255)` fields emit
  nothing and cost nothing;
- **only for the `limit(...)` spelling** (`forced_size.is_none()`). A narrow ALIAS is already
  refused at compile time (`cannot implicitly narrow integer to u8`), and layering a silent
  default on it is both redundant and wrong — the first draft fired 24 times inside the
  stdlib's own `i8` stores and handed them `-128`, which is what `behavior_golden` and the
  nullflow suites caught.

Guard: `tests/scripts/984-limit-field-out-of-range-defaults.loft`;
`389-narrow-runtime-collision` pins the nullable half (it was what caught the first draft
storing `min` where a nullable field owes `null`).

### A split-ownership return is decided per run (loft#981, loft#982, 2026-08-18)


A heap return carries ONE static answer to *may the caller free this?*, read off the return
deps — a dep naming a visible parameter means BORROW (never free), an empty one means OWNED.
A return that is a view of a parameter on one path and a freshly minted store on the other
has no correct static answer, and the one it got orphaned the minted store:

```loft
fn get(b: Bag, k: text) -> Item { b.items[k] ?? Item { name: "miss", limbs: [] } }
fn pick(o: Outer, fresh: boolean) -> Outer { if !fresh { return o; } make_outer(99) }
```

`Item×41` for 41 misses, `Outer×N` for N calls — unbounded in a loop, both backends, exit 0,
every value correct. Found in `loft-libs-world`'s `hex_field::stencil_rotate`, in a published
library, invisibly: `loft test` runs the leak check under `--interpret` only and the leak
surfaces at PROGRAM exit.

loft#982 reads as the same defect for a different reason, and the measurement is the point:
its arms do NOT split. `return o` over a by-value struct parameter is hoisted to
`__ret_1 = o`, which DEEP-COPIES — so both paths deliver a fresh store, the caller gets a
copy either way (`b.o_n = 777` leaves the argument at `2`), and the dep on `o` is simply
STALE. That is why BOTH arms leaked, not just the fresh one, and why the runtime test is the
right answer for it too: it asks *is this store mine?* rather than trusting the static class.

**The cure reuses @P290 instead of adding a rung.** The call bracket already marks a caller's
arguments "do not free mine" for the call's duration, and both backends' `OpCopyRecord`
already refuse the `0x8000` source-free on a marked store (`state/io.rs::do_copy_record`,
`codegen_runtime.rs::OpCopyRecord`). So the bit is SET at a bracketed call site and the run
decides. No new opcode, and it scales to several witnesses where one `OpFreeRefIfDistinct`
could not. `use_analysis::call_return_frees_source` is the one fact; three emitters read it
(interp first-bind + reassign, native `generation/dispatch.rs`, which had no bracket and now
emits the same one), and `protectable_ref_args` is the one derivation shared by the gate and
the protection emit so the two cannot drift.

**The bound that the suite found.** The witness set must span every argument the return could
BORROW, not every argument the bracket accepts. The bracket takes `Reference`/`Vector`/`Enum`;
a keyed collection (`hash`/`sorted`/`index`/`radix`/`trie`) is a borrow source it does not
cover, and reading the narrower list as complete freed a hash parameter's element out from
under the caller — `keyed_cells_poison_clean_*` and `consumed_lift_cells_poison_*` went red,
`rec=0xDEADBEEF`. Coverage is now asked with `heap_dep()`, through `base()` so an `Optional`
wrapper cannot hide the storage under it. An argument the bracket cannot name keeps the old
conservative *never free*: the leak stays for that shape (a keyed-collection parameter)
rather than risking a free of a store the caller still reaches.

**`Store::free_protected` became `free_protect_depth`.** One call bracket inside another may
protect the same store, and a boolean let the inner release drop the outer bracket's
protection — after which the outer copy's source-free would free the caller's own argument.
No probe in the suite distinguishes the two today; the depth closes it by construction.

Guard: `tests/scripts/981-split-ownership-return.loft` + `tests/split_ownership_return.rs` —
leak as the DETERMINISTIC oracle (the census at exit does not depend on slot reuse), poison
and strict-stores for the opposite direction, and the keyed-collection control. Against a
pre-fix binary it orphans 161 + 41 records on both backends while every value cell passes.
`tests/scripts/978-…loft` now drives its return-position join down the fresh arm too — the
exclusion its comment recorded was this leak.

### A package's own module wins its own `use` (loft#976, 2026-08-18)


Two packages, neither depending on the other, each shipping `src/skin.loft` with DISJOINT
names inside, and each saying a bare `use skin;`. A consumer that pulls both:

```
error: unknown type 'PartBox'
error: Unknown function skin_covers
```

Swap the consumer's two `use` lines and the OTHER package loses instead. A module's short
name was one slot shared by the whole dependency graph, so the first loader took it and
every other package's own module never loaded — its public surface amputated in a build it
had nothing to do with, reported against a file its author had not touched. A qualified
`hex_part::skin_covers` did not help: the module never loaded, so there was no second name
to choose between. Each package's own test suite was green, because a package's own graph
holds only itself.

A bare `use <module>` inside a package now resolves that package's own
`src/<module>.loft` first, binding it under `<package>::<module>` — which is exactly what
`use self::<module>` already did (loft#949) and what the `module-name-shadowed` advice
already recommended. `parse_use_self`'s tail is now `bind_own_module`, shared by both
spellings, so `self::` is the explicit form for the rare case that wants a stranger's
module rather than the defensive form every library author has to remember for every file
they will ever add.

**A declared dependency still beats a local file of the same name** — that is `lib_path`'s
own shadow guard, deliberate — so the preference checks `package_declares_dep` first.
Without that, a package holding `src/<dep>.loft` stopped being able to reach the `<dep>` it
depends on; `a_file_named_like_a_declared_dependency_is_not_a_clash` caught it.

**What it deliberately does not do is merge two modules into one name.** Where both
packages' modules declare the same public name and a consumer calls it BARE with both in
scope, that call is now an explicit ambiguity error naming both spellings, where before one
was picked by load order. That is a tightening, and it is the one the pre-freeze mandate
asks for: COMPATIBILITY.md § *the error surface is one-directional* says every place loft
"produces a plausible-wrong value where it should reject" is a last-chance-to-add while
contract 0 allows it. The ambiguity message no longer assumes the reader wrote `self::`,
since a bare `use` now scopes the same way.

`module-name-shadowed` stays for what the scoping rule cannot reach: a file with no
`<module>.loft` of its own still takes whichever the search finds, and two of those in one
graph still resolve by load order.

Two things the short spelling has to keep, both found by the suite rather than by reading:

- **the `<module>::` qualifier.** `use math;` has always supplied it and code inside the
  package writes it, so the bare spelling registers the short name as an alias onto the
  same source; without that the shipped `graphics` fixture failed with `Unknown library
  'math'`. It is a flat map, so two packages both spelling it short still share that ONE
  qualifier — unchanged from before, and now the only thing they share.
- **a stable name in diagnostics.** One source reachable under two names met
  `qualified_type_name`'s walk over a `HashMap`, which returned the first match: the same
  program named the same definition `con::catalogue::part_list` on one run and
  `catalogue::part_list` on the next. It now picks the most qualified spelling, ties broken
  alphabetically. The order-sensitivity predates this change; a second key is what made it
  observable, and it read as a flaky test.

Guard: `tests/module_name_clash.rs` — the filed shape (two SIBLING packages, both `use`
orders) plus the dependency-direction cases, rewritten from asserting the mis-resolution to
asserting the fix. Those tests were written to go red when this landed and say so in their
own docs.

### A struct-enum field access never checked the discriminant (loft#980, 2026-08-18)


```loft
enum Node { Named { label: text, n: integer }, Anon { k: integer } }
a: Node = Anon { k: 7 };
print("{a.n}");     // 7 — that is Anon's `k`, answered as Named's `n`
a.label = "written";  // lands in the Anon record; the tag stays Anon
```

`c.field` resolves at COMPILE time to the first variant declaring the name, and the
layout gives a shared name+type ONE slot — so the read is right for the variants that
declare it and reads another variant's bytes for the rest. `match` afterwards still
reports the original variant, because nothing changed the tag. Both backends, exit 0.

**Direct payload access stays.** [C89](DESIGN_DECISIONS.md#c89) decided permanently that
enum payloads are named fields you read straight, with matching for *dispatch* and never
for *extraction* — refusing a bare `c.field` would force a matcher on every read, which
is the thing C89 exists to prevent. And the common-prefix case is already correct:
measured on variants whose preceding fields differ in width, a field every variant
declares reads right from each of them. The **silence** on the partial case was the
defect, and `variant-field-unchecked` closes it: `warning` tier by the two-tier rule,
since ignoring it produces a wrong result. `LOFT_NO_VARIANT_FIELD` opts out.

Quiet where the access is answerable, each exemption measured: every variant declares the
field (one slot, any tag finds it); a `match` / `is` binding, which is per-arm and is the
cure the message names; and a synthetic `__nullable<S>`, whose payload access is @PLN25's
null model rather than a user-visible variant question.

Swept before it spoke: the whole `.loft` corpus — `tests/scripts`, `tests/docs`, `lib/*`,
`default/*` — holds **13** partial-variant accesses, all of them inside loft#977's own
regression test, and every one on a value that IS the declaring variant. Partial access is
rare precisely because `match` is the idiom.

**What is still open** is the semantics, and it now has one answer rather than three. The
issue offered refuse-at-compile-time (contrary to C89), a runtime tag check, or a
common-prefix-only rule (restricts direct access the same way C89 rejects). The fault
model picks the middle one's shape: C80/C85/C90 say an uncomputable read answers the
type's null SENTINEL and the program keeps running — the same answer a hash miss, an
out-of-range index and an overflow already give — which leaves the access type unchanged,
so it breaks nothing. The read lowers with existing IR (`OpGetEnum` → `OpConvIntFromEnum`
→ `OpEqInt`, the tag test a `match` already emits), so it needs no new opcode on either
backend. The WRITE is what makes it design work: suppressing a write to a field the value
does not have needs an lvalue notion the parser does not have — a guarded read is not a
place, and the assignment path takes the parsed access as one.

### A branch whose arms disagree about ownership froze the wrong one (loft#978, 2026-08-18)


```loft
fn read(b: Bag, fresh: boolean) -> integer {
  // ONE arm is a fresh record, the other a view into `b`
  it = if fresh { Item { name: "fresh", limbs: [] } } else { b.items["one"] ?? Item {} };
  len(it.limbs)
}
// prints `2 0 0` where the same program without the fresh arm prints `2 2 2`
```

Silent, both backends, exit 0. `it` recorded no dependency at all, an empty dep list is
the OWNED reading at every free site, and scope exit released the container's record; the
next unrelated allocation claimed the recycled slot and every later read answered out of
it. `LOFT_NO_SLOT_REUSE=1` read *correctly* with the defect present, which is why no
poison or use-after-free sweep saw it — the store was freed and then legitimately
re-occupied.

The defect was **arm-order sensitive**, and that is what named the cause. Writing the view
first read correctly. `parse_if` parses the `else` block with the THEN arm's type as its
expected type, and `block_result` adopted that expected type WHOLE — deps included. An
expected type says what shape belongs in a position; it was written before the value in
hand existed, so it cannot say what that value aliases. Whichever arm came second got its
sibling's borrow list, and with the fresh arm first that list was empty.

- `Type::with_deps_of` keeps the block's own tail deps when it adopts an expected type,
  applied to the `else` arm alone — the only block handed a sibling EXPRESSION as its
  expected type. Every other caller's is a DECLARED type whose deps are attribute indices,
  and grafting frame vars onto those is the cross-space read loft#666 was made of.
- `Type::joined_deps` unions the arms' borrows at the `if`/`else`, at the `else if` chain
  (whose type is deliberately not adopted as `false_type`, so what it borrows had to reach
  the join another way), and at all six `match` arm sites.
- `Parser::arm_join_type` filters what an arm CONTRIBUTES: a dep naming a store the arm
  itself mints is its ownership marker, not a borrow (`[]` lowers to `OpDatabase(__vdb_N,
  …)` and types as a dep on it). Importing one told the return machinery the value views a
  local and turned @PLN85's `deliver` return from `["__retbuf", "e"]` into an unresolvable
  `["??"]`. The minted set comes from `use_analysis::minted_vars`, the same `collect_defs`
  walk the ownership classifier reads.
- `Type::with_deps` is now the one list of which variants carry a dep list; `depending`
  delegates to it.

Measured boundary — eighteen shapes, all previously wrong, all now correct on both
backends: the join construct (`if`/`else`, `else if`, `match`, nested), where the view
comes from (hash lookup, vector element, struct field), how it arrives (projection,
accessor return, parameter), and what the local is used for (collection field, scalar
field, returned). Controls that must stay green and do: two fresh arms are genuinely
owned and still freed; two views of one base were never in doubt.

Guard: `tests/branch_join.rs` + `tests/scripts/978-branch-join-carries-both-arms-borrows.loft`
— a static oracle on the recorded type (deterministic, unlike the value cells, which depend
on a freed slot being reused), the value cells on both backends, a strict-store run and a
leak run. Both oracles were checked against a pre-fix binary and fail there.

Residual filed as **loft#981**: an escaping join — a *return* whose arms disagree — has no
static answer that is right for both arms, so taking the borrow leaks the record the fresh
arm mints. Predates this fix; a plain `fn get(b, k) -> Item { b.items[k] ?? Item { … } }`
leaks identically on the released binary.

### Writing a collection field of a struct-enum panicked in the store layer (loft#977, 2026-08-18)


```loft
enum Shape { Circle { limbs: vector<float> }, Square { s: float } }
c: Shape = Circle { limbs: [] };
c.limbs += [1.0];     // index out of bounds: the len is 83 but the index is 65535
```

Both backends, no imports, no diagnostic and no source position — `65535` is `u16::MAX`
reaching `self.types[tp as usize]` in `record_new`.

`c.limbs` is written through the ENUM type, but the field lives in the `Circle` variant's
own record. The enum type is `Parts::Enum` — a variant list, no fields — so resolving a
field against it misses, and both resolvers hand back their not-found sentinel:
`field_nr` says `0`, which is a real field number, and `field_type` says `u16::MAX`, which
is then used as a type-table index. `field_ref` has the same miss and answers the record
base, silently, so the sentinel was the loud half of a defect whose other half was not.

The filed scope was a tenth of it. The issue reads "appending to a vector payload"; the
boundary matrix says **every allocating write to every collection payload field, through
every route** — `+=` and whole-assign alike, over `vector<float>` / `vector<record>` /
`hash<T[k]>` / `vector<vector<…>>`, reached as a local, an element of a `vector<Shape>`,
a function parameter or a struct field. An element write (`c.limbs[0] = …`) and a scalar
field write (`c.s = …`) never call `record_new` and were correct throughout — the controls
that place the defect in the allocating path rather than in struct-enum field access.

- `Stores::variant_owning_field(enum_tp, position, content)` resolves a struct-enum field
  to the variant that declares it, keyed on the byte offset **and** the content type —
  never the offset alone, because every collection field is one 4-byte handle straight
  after the discriminant, so two variants each holding one put it at the same offset.
  Measured, with the resolver built offset-only: `enum Box { Listed { xs: vector<Item> },
  Keyed { ys: hash<Item[name]> } }` resolves `ys` to `xs`, appends the record to the vector
  instead of keying it, and the lookup answers nothing — silent at the write. Identity for
  a non-enum parent and for a field no variant declares.
- `new_record_field_op` applies it after `key_owner`, so `OpNewRecord` and `OpFinishRecord`
  name the variant record. This is the same redirect @PLN25 already needed for a synth
  `__nullable<S>` payload, now for the user-facing shape.
- `record_new` / `record_finish` derive their sub-record type in one shared
  `sub_record_type`, which refuses a not-found field type by name instead of indexing the
  type table with it. The two halves cannot disagree, and the next instance of this class
  says which type and which field rather than `the index is 65535`.

Residual, filed as loft#980 rather than fixed here: field access on a struct-enum does not
check the discriminant, so `c.limbs` on a `Square` value reads and writes `Circle`'s slot.
That is the READ's long-standing behaviour — `len(c.limbs)` on a `Square` answered `0`
silently before this fix too — and the write now merely agrees with it. Which of the three
answers loft wants (refuse at compile time, check the tag at runtime, or allow a common
prefix) is a language decision, not a patch.

Guard: `tests/scripts/977-struct-enum-collection-field-write.loft` — eighteen cells on both
backends, 45 assertions over value, length, ordering and the neighbouring fields — plus
three unit tests in `src/database/structures.rs`: the resolver's ambiguity cell, its
identity cases, and the guard's message, which no loft program can reach any more.

### An accessor's returned record borrows its container (loft#974, 2026-08-18)


`fn get(b: Bag, k: text) -> Item? { b.items[k] }` declared
`optional(reference(Item, deps {}))` — no dep — so the caller typed the result OWNED and
emitted `OpFreeRef(it)` at scope exit, freeing a store the CALLER's `b` still owned. The
next unrelated allocation claimed the recycled slot and every later lookup answered out
of it: `2, 0, 0` where the same lookup written INLINE reads `2, 2, 2`. Silent, both
backends, no imports; it survived dryopea's 1361-test suite.

`LOFT_NO_SLOT_REUSE=1` reads correctly WITH the defect present (the freed bytes survive
while nothing claims them), which is why no poison or UAF sweep ever saw it — the
detectors all watch a freed store, and this one is freed and then legitimately
re-occupied.

Root cause: one selector answering two questions. `ret_promo_base()` decides DELIVERY
(does this return get a `__retbuf`?) and peels `Optional(Vector)` only — deliberately, a
nullable struct is loft#896's `__nullable<S>` with its own delivery. But the whole
promotion pass was gated on it, so the SIGNATURE fact went with it: *which parameter
does the returned view borrow?* — which is true whatever the delivery is.

- `Type::ret_dep_shape() -> (&Type, RetPeel)` answers the signature question, peeling `?`
  for `Reference` and struct-`Enum` too and marking those `SignatureOnly`.
- `ref_return` under `SignatureOnly` records the borrow and skips every placement verdict
  (`Rename` / `Bind` / `Grow`), so no second delivery is created and the ABI is unchanged.
- `generation/dispatch.rs`: a first-bind from a callee that `returns_borrowed_view()` now
  ALIASES instead of deep-copying **where the destination is one the emitter will not
  free** (`variables.skip_free`) — what the interpreter already emitted (`PutRef`).
  Without the alias the copy is a store the IR never frees (one leaked record per call,
  caught by the new `accessor_cells_leak_clean_native`); without the `skip_free` half a
  LIFTED call temporary (`__lift_1`, which the IR owns and frees) aliases the caller's
  store and loft#677's guard reports `USE AFTER FREE (write) … killed by the free of
  var___lift_1` on native. The copy decision now reads the same fact the free decision
  reads, instead of a proxy for it.

Widening the delivery peel instead was measured and rejected: it re-typed the return
non-nullable (`-> Item["b"]`) and diverged the backends on a missing key.

Guard: `tests/scripts/974-accessor-returned-record-borrows-its-container.loft` +
`tests/accessor_borrow.rs` (static: the signature names the parameter and keeps its `?`;
behavioural: both backends, strict-stores, native leak check; plus a harness control).
⚠ The script is calibrated against the defect on BOTH backends — its first version put
every read in one scope and passed against the bug, and a `churn()` helper made native
pass while the interpreter still failed.

### Manifest-less resolution: one scope function, no lockfile written by running (@PLN143, 2026-08-18)


`lib_path`'s registry legs were three probes that each re-derived their own lockfile
path — beside the script, at the project root, and in the **cwd** — and a disagreement
between them was silent: a different version loads and nothing errors.
`resolution_scope(script) -> Package | PinnedScript | Bare`
(`src/resolution_scope.rs`) answers it once, and the same value decides which lock may
be WRITTEN, so read and write cannot drift. Two other copies of the walk-up went with
it (`Parser::find_project_root`, main.rs's `find_project_root_from`).

What changed behaviourally:

- **The cwd leg is deleted.** A `loft.lock` in the directory you stand in governs
  nothing. A bare script's first run no longer writes one either
  (`skip_lockfile` is read off `lock_write_target`, one fact), so "latest" is
  re-decided every run instead of being pinned by a file the run produced.
- **`Bare` scope gains a cache fallback** (`registry_index::newest_cached_loadable`):
  newest extracted, prereleases skipped, and any copy this build cannot load filtered
  out through `manifest::check_version` / `check_contract` — the loader's own
  functions, so the filter cannot drift from what the loader accepts. `Bare` only: a
  declared scope has a constraint the fallback would answer past.
- **`loft install <pkg>` outside a package writes a minimal `loft.toml`** so its lock
  has a root that governs it, and prints `created loft.toml (package \`x\`)`.
- **A governing pin behind the cached index prints one line** naming the cure the
  scope takes (`loft install <pkg>` / `loft pin <script>`). Cache-only (never a
  fetch), once per package per run, never for a dependency's own `use`, silent under
  `LOFT_OFFLINE`, off-switch `LOFT_NO_UPGRADE_NOTICE=1`.
- **`[registry] resolving <pkg> from registry` is gone**; `[registry] downloading
  <pkg> <version>` prints where bytes are actually fetched. With nothing pinning a
  bare script both parse passes re-decide, so the old line printed twice per run for
  work a warm cache was not doing. The auto-install ANSWER is memoised per run for the
  same reason — two passes must resolve the same file.
- **The governing lock decides the INSTALL, not only the load.** `lock_path` and
  `skip_lockfile` are now separate questions — the lock that governs vs. whether this
  resolution may write it — because reading them as one meant arc C2 (a run writes
  nothing) also stopped a pinned script's sidecar from being read. A sidecar pinning
  0.1.0 loaded 0.1.0 when the cache had it and INSTALLED the newest when it did not;
  the same hole applied to a package whose locked version was not yet extracted.
  `install::constraint_for` states the rule: an exact pin outranks a range, except a
  pin the manifest has since excluded, which is a stale lock losing to what it derives
  from. `install::options_for_use` makes the whole posture of the `use` path one tested
  fact, and the sidecar now gets `check_against_lockfile`'s re-publish check as well.
- **Two index reads that trusted unchecked bytes are closed**: `load_index_inner`'s
  `offline` branch now verifies like the other three, and `locked_hashes` takes the
  `skip_lockfile` guard `held_versions` already had.

Arc A (2026-08-18) preceded all of it: `probe_auto_install` no longer passes
`allow_unsigned: true`. `loft install` keeps its own CLI default, and that asymmetry is
the point — waiving is defensible for a verb a person typed, not for the path a bare
`use` takes on its own.

### Every store access is bounded, on every target (loft#950, 2026-08-17)


`Store::addr` and `addr_mut` bounded their offset with a `debug_assert!`, and loft's
library build compiles those out (`[profile.dev.package.loft]`). So the only bound left
in a release build was `checked_offset`'s `isize::try_from` — which can fail **solely
where `isize` is 32 bits**, i.e. the wasm targets.

The consequence was an asymmetry that cost a day of diagnosis. One corrupt `DbRef`
trapped in a browser page as `RuntimeError: unreachable`, while the same corruption on
the interpreter and `--native` computed a representable offset and read whatever lay at
it — a silently wrong scalar, or, through `addr_mut`, a `&mut` handed out into arbitrary
process memory. "The browser traps and every other backend is green" therefore said
nothing about where the corruption was; it said where the guard could speak.

The bound is now a real check in `offset_in_bounds`, shared by `addr`, `addr_mut`,
`read_span`, `write_span` and `buffer`, and its message names the fault as a corrupt
reference and points at `LOFT_STRICT_STORES=1` for the free that produced it. `buffer`
gains a bound it never had: its length comes from the record's own header, and a freed
record's header is negative, which reading it as `u32` turned into a multi-gigabyte
slice.

Measured by instruction count on a loop that does nothing but read and write struct
fields — the worst case by construction: **+2.5 % on `--native`** (the default backend)
and **+9.4 % on `--interpret`**. loft#885's hoisted element reads derive their address
once per loop and do not come through here at all.

This is the report half of loft#950. The corruption that produced the reference is a
separate fault and is still open.

### `map` answers `vector<U>` for a callback `fn(T) -> U` (loft#945, 2026-08-17)


STDLIB.md has documented `map(v: vector<T>, f: fn(T) -> U) -> vector<U>` all along, and the
lowering already built the result from the callback's return type. Every `U != T` was refused
anyway, in three different places:

* the **argument hint** pinned the callback's return to the element type, so an inline lambda
  was type-checked against `T` and the error landed *inside the user's own lambda* — `map(xs,
  |x| { "n{x}" })` reported *"expected integer, got text on return from block"*;
* **pass 1** answered `vector<T>` while pass 2 answered `vector<U>`, so a named callback got as
  far as the binding and was refused there — *"cannot change type from vector<integer> to
  vector<text>"*, and declaring the destination only moved the report;
* the hand-built per-element **call** never supplied the hidden buffer a heap-returning callee
  takes, which crashed the compiler outright: *"Too few parameters on n_shout (got 1, need 2)"*.

That third one bit `U == T` as well, so this was never only about changing the element type.
On the released 2026.8.0, `xs.map(|s| { "{s}!" })` over a `vector<text>` is an internal compiler
error, and so is `xs.reduce("", |a, x| { "{a}{x}" })` — while the equivalent comprehension
`[for s in xs { shout(s) }]` was fine all along, because it goes through the ordinary call
machinery. The comprehension is the oracle this is measured against.

**The lambda's return type now comes from its own body.** A short `|x|` form cannot declare one
(`-> τ` is refused there by design), so when the hint names no return either, `block_result`
adopts the tail's type at the tail. That timing is the whole point: a text or collection return
mints a hidden buffer parameter *while the body is parsed*, and minting it on pass 2 alone is
the H5 two-pass contract violation the ICEs above were. Pass 1 also takes the hint's return
type now, rather than storing `Void` and letting pass 2 force it — the same divergence, one
step earlier. `filter` and `reduce` keep their own contracts: a predicate really is `-> boolean`
and a fold really answers its accumulator's type.

Three supporting changes fall out of it:

* **`.map(…)` is recognised on BOTH passes.** It used to be pass-2 only, so pass 1 parsed the
  callback with no element-type hint — the two passes disagreed about the callback's signature,
  and the method and free spellings of one program lowered differently.
* **A heap-returning LAMBDA gets its return buffer reserved between the passes.** Its return
  type is read off its body, so nothing could reserve one at signature time, and pass 2 grew the
  attribute instead of renaming onto it.
* **A literal RECEIVER is recorded**, so pass 2 does not build `[1, 2, 3]` against the type the
  chain left on the LHS (`d = [1, 2, 3].map(|x| { "n{x}" })` leaves a `vector<text>` there, and
  `total = [1, 2, 3, 4].reduce(0, …)` leaves an integer).

**`reduce` with a HEAP accumulator is now refused** rather than mis-folded. The fold lowers to
`acc = f(acc, x)` in a loop, and a callee answering text or a collection writes into one
caller-allocated buffer that it CLEARS on entry — so after the first turn `acc` IS that buffer
and the next turn erases the fold. It answered the LAST element on the interpreter and did not
compile at all on `--native`. The cure is the H7 two-buffer rotation, which today covers only a
vector-typed assignment target; until it reaches text, the diagnostic names the loop to write
instead.

Guard: `tests/scripts/945-map-changes-the-element-type.loft` sweeps four axes together — the
spelling (free vs method), the callback (inline lambda vs named function), `U`'s storage class
(scalar, text, collection, record, enum), and whether `U == T` — with `filter`, `reduce`, a
capturing lambda, a chained `.filter(…).map(…)` and the comprehension as controls.

### A nullable collection return can have a return buffer — four of five gaps closed (loft#938, 2026-08-17)


Still behind `LOFT_NULLABLE_RETBUF=1` and still off by default, but the switch now carries every
shape in `tests/probes/938-nullable-collection-return-buffer.loft` correctly and leak-free on
both backends. `Optional(Vector)` was blind at three more gates than the first pass at this
found:

* **the tail-delivery selector in `block_result`** — the type-keyed chain matched `t` directly
  where the Text arm above and the Reference arm below already peel with `.base()`. A nullable
  collection tail missed the arm entirely, so it never reached `ref_return` and
  `classify_ret_promotion` was never called for it. This is why a FORWARDING tail
  (`fn fwd(i) -> vector<T>? { a1(i) }`) compiled to `return null` on `--native` while the
  interpreter read the freed slot and only looked right.
* **the per-arm materialise** — `vec_match_candidate` excluded any tail with a direct `null`
  arm, because materialising it would have forced an owned-buffer return type onto a path that
  yields null. With the `?` re-wrapped around the buffer-dep'd base that is representable, so a
  function whose arms disagree (null / alias of a parameter / fresh store) now delivers.
  On the released binary that shape corrupts the caller's collection on `--native` — `base=2
  base0=39` where `3`/`71` is correct — which was never filed.
* **the mid-body `return <call>` site** — it preferred the callee's declared deps over the
  site's own hidden buffer args, so a nullable callee that already names its `__retbuf` left the
  site's `__ref_N` out of the candidate list. Nothing bound it, `unregister_work_ref` never ran,
  and it leaked one store per call, scaling with the loop.

One known failure remains and is what keeps the default off: **two call sites of a two-arm
dispatch share one return dep**. The second result is typed as a borrow of the first
(`gl(1):vector<integer>["gd"]`) and freed on its schedule. Every value is correct and nothing
leaks; the only witness is `LOFT_STRICT_STORES=1`. It needs both halves — one arm alone is
clean, one call site alone is clean — which is why every smaller probe passes.
`known_two_site_dispatch_reads_a_freed_store` is the 14-line repro.

`LOFT_TRACE_RETPROMO` now prints the VERDICT and the site beside the candidate, because "no line
at all" (a gate upstream of the classifier) and "a `Skip*` verdict" (the classifier said no) are
different bugs that the IR does not tell apart.

### The module-shadow advice reaches the run it explains (loft#948, 2026-08-17)


`Advice[module-name-shadowed]` (loft#912) fired where the collision was HARMLESS and stayed
silent where it broke the build — the one case a reader cannot diagnose from the output.

Three findings, in the order they were established:

**The advice was produced all along.** The measurement that settled it holds the shadowing
file fixed and changes only whether the dependency CALLS the shadowed function:

| dependency calls it? | build | advices printed |
|---|---|---|
| no | compiles | 4 |
| yes | **errors** | **0** |

So detection was never the problem. `test_runner.rs`'s parse-error branch printed
`unexpected_errors` and `file_result.warnings` — and not `file_result.advice`, which the
success path a few lines below chains in. A diagnostic that explains a build break is only
useful in the run that breaks; it now prints on both paths.

**The doubling was real.** Both parser passes emit each diagnostic, so every warning and
advice reached the reader twice. A line carries its own position, so two identical lines are
one finding said twice — deduped.

**And one false positive had to go with it.** `tests/<pkg>.loft` beside `src/<pkg>.loft` drew
a rename that fixes nothing: the `use` binds the file the author meant, and two of this repo's
own fixtures use that layout. The advice is about a name *"shared across the whole dependency
graph"*, so it is now suppressed when both files resolve to the same nearest `loft.toml`.
Making a diagnostic prominent and leaving it firing where there is nothing to fix is how one
teaches people to skip the cases where there is.

Guards: `tests/module_name_clash.rs` gains the fatal case and the same-package control. The
fatal one drives the BINARY rather than `Parser::parse`, and that is load-bearing — the
shadowing file is imported by nobody, so nothing reaches it by following `use` edges. It is
loaded because building the package reads every file under `src/`, which is both why the
collision happens and why it has to be tested at the `loft test` surface where the output was
dropped.

### A nullable collection return can have a return buffer — opt-in (loft#938, 2026-08-17)


**`LOFT_NULLABLE_RETBUF=1`, OFF by default.** Built and validated far enough to be a basis,
not far enough to default on. With the switch off every consulting site is byte-identical to
before it existed: `Type::ret_promo_base` is the identity and `ret_promo_peels` is `false`.

**What it does.** A `-> vector<T>?` gets the same hidden `__retbuf` the non-nullable form
already gets. That is the whole fix, and the reason it is the right shape is visible in the
non-nullable callee, which normalises EVERY arm into the caller's buffer:

```
if n == 1 { OpClearVector(__retbuf); OpAppendVector(__retbuf, v, 0);        __retbuf }
else      { …build…; OpClearVector(__retbuf); OpAppendVector(__retbuf, _vec_1, 0);
            OpFreeRef(__vdb_1);                                             __retbuf }
```

So ownership never varies at the call site — the ABI removes the variance rather than
deciding it, and the caller frees one buffer per call SITE, which is why a loop does not
leak. This corrects the earlier note on the issue, which argued from the nullable lowering
that ownership was undecidable in principle. It is not; `Optional` was blind at every gate.

**Five gates, each hidden behind the one before it:**

| gate | symptom while blind |
|---|---|
| the `__retbuf` signature gate (`definitions.rs`) | no buffer created |
| the top-level promotion gate (`control.rs`) | `classify_ret_promotion` never called at all |
| `Bind` eligibility | no delivery emitted |
| two `Vector` carve-outs | nullable took a different route |
| `ref_return`'s re-typing | *"Unexpected return type in ref_return: vector<integer>?"* |

The second one is why this took three sessions: it guards the WHOLE pass, so the symptom was
the ABSENCE of a trace line rather than a wrong decision. `LOFT_TRACE_RETPROMO=1` exists for
exactly that reading.

**The peel is narrow on purpose.** `Optional(Vector)` only. Widening it to
`Optional(Reference(S))` leaks one record per call — a nullable STRUCT return is loft#896's
synthetic `__nullable<S>` enum with its own delivery, and
`882-keyed-element-read-borrows-its-container.loft` catches it. The `?` is transparent only
where the storage under it is.

**Still open with the switch on**, both pinned as `#[ignore]`d tests in
`tests/nullable_ret_buffer.rs` so they are named and runnable:

* a function mixing a `null` arm, a parameter ALIAS arm and a FRESH arm leaks its fresh arms.
  The null arm forces the `__ret_N` merge before promotion sees the arms, so none is a return
  tail and none delivers. Value correct, container intact, `--interpret` only.
* `pln133-optional-unify.loft` miscompiles on `--native` — a method returning `vector<T>?`
  through different routes reads an element back as `0` where `14` is correct, while the
  interpreter passes. A silent wrong answer on one backend is why this is not defaulted on.

**How to finish it:** get those two green, then flip the default and delete
`default_path_is_unchanged`. `make ci` 4144/4144 with the switch off.

### A `?` on a collection field means what it says (loft#917, 2026-08-16)


A collection field is a 4-byte RECORD ID where `0` already means the EMPTY collection, so
`xs: null` and `xs: []` lowered to the identical `OpSetInt4(h, 8, 0)` and `xs == null` could
never answer true. The value-level null cannot help: `DbRef::NULL` says absent in `store_nr`,
and a field has no `store_nr` — `OpGetField` is pure pointer arithmetic (`pos + offset`) and
the collection ops read the id out of the slot themselves.

**Absence gets its own reserved id.** `DbRef::ABSENT_REC` (`u32::MAX`), the same move a store
already makes for `u16::MAX` (`database_named` asserts `slot != u16::MAX`). No layout change,
no width change — one value reserved out of a range a record id cannot reach, since a record
id indexes words within a store. Verified free before anything was built on it: the read
conversion alone, with no writer, passed the whole suite 4141/4141.

**The reserved id is read two ways, and keeping them apart is the design.**

| reader | sees | because |
|---|---|---|
| `vector::is_absent_collection` | the RAW slot | `== null` is the one question whose answer differs between absent and empty |
| `Store::collection_rec` | `MAX` mapped to `0` | every other reader asks "which record holds the elements?", and absent and empty answer that the same way |

That split is what keeps the change to one accessor instead of a decision at each of the
twenty-odd sites that dereference a slot — and a missed site is loud: taking `u32::MAX` for a
record number is `get_u32_raw(MAX, 4)`, a SIGSEGV rather than a wrong answer. Two matrix rows
(refilling an absent field, and `?? []` over one) caught exactly that when the writers landed
before the readers.

**Writers**, both gated on the declared `?` — without one the field's own type says it can
never be absent: the struct literal (`handle_field`, the sibling of the `__nullable<S>` arm
loft#896 added for structs) and the assignment (`clear_vector_field_as`). The RELEASE of the
records is unchanged and stays ungated, which is loft#922's rule; only the marker is new.

**`== null` now dispatches for every collection kind.** A keyed field is the same 4-byte slot
and carries the same marker, but `hash<K[k]>? == null` fell past the vector-only selector to
`OpEqRef` — a DbRef comparison against `DbRef::NULL` that a slot POINTER can never satisfy, so
it answered false however the field was written.

**Retired: the `nullable-collection-field` warning.** It existed to say the `?` was a promise
the storage could not keep; that is no longer true. Its pinned row in `tests/e1_code_set.rs`,
its DIAGNOSTICS.md entry and the `LOFT_NO_NULLABLE_COLLECTION` switch are gone with it.
`tests/scripts/917-…loft` used to ASSERT the wrong answers and said so in its own header —
"if a later layout change makes `== null` work, they fail and this file is the thing that says
the warning can go". This is that change; the assertions are inverted.

**Not loft#938.** That is the ownership half — who frees a returned `vector<T>?` — and is
untouched: absence says nothing about who owns the collection that IS there.

### A null test on a collection FIELD does not free the record it read (loft#920, 2026-08-16)


`vector == null` tests the null sentinel through `OpVectorIsNull`, which reads only the
sentinel and discards the vector — so a heap-owning TEMP consumed by the test would be
orphaned. `parse_compare` captures a non-`Var` operand in a work-ref precisely so scope exit
frees it (loft#938's caller half).

**"Non-`Var`" was standing in for "a temp that owns its store", and a FIELD READ is non-`Var`
too.** `h.vec == null` captured `OpGetField(h, …)` — a DbRef pointing INTO `h`'s record — and
freed it:

```
x = { __ref_1 = OpGetField(h, 0i32, 21i32); OpVectorIsNull(__ref_1); OpFreeRef(__ref_1); }
```

The type said so all along: that read is `optional(vector(…, deps { items: [0] }))`, and a
non-empty dep list means BORROWS. The work-ref is now skipped for an operand that positively
declares a borrow — strictly narrowing, so a call result keeps its work-ref and loft#938 is
untouched (its leak count is unchanged, and that is a control in the guard).

**This was the nightly UB gate's SIGSEGV**, red for three sessions. Under `LOFT_POISON=1` the
freed record reads back `0xDEADBEEF`, so the next read of the same field built a garbage
DbRef and `len()` dereferenced it. The whole-corpus run is now green under
`LOFT_POISON=1 LOFT_HASH_SEED=0x0123456789abcdef`: **1876/1876**.

**It was never only a UB-gate problem.** With no flags at all, letting the freed slot be
recycled makes the next read of that field answer *another variable's data*:

```loft
h = Holder { vec: [71, 82, 93] };
x = h.vec == null;
filler: vector<integer> = [11, 22, 33, 44, 55, 66, 77, 88];
len(h.vec ?? [])        // 8 before the fix — the FILLER's length; 3 after
```

Both backends, released 2026.8.0 included. That is what the guard asserts, because a crash
needs a flag and a silent wrong answer does not.

**How it was found, since the previous two attributions were wrong.** `tests/wrap.rs` gained
`LOFT_SCRIPT_FIRST`/`LOFT_SCRIPT_LAST`, which run a WINDOW of the sorted corpus — the
instrument a "one script corrupts, a later one dies" fault needs, and the one whose absence
made the last three sessions guess. A bisect over 739 scripts took ten runs and landed on a
single file; from there the reduction is six lines. Note the byte-vs-locale sort trap: `ls |
sort` and Rust's `PathBuf` sort disagree, and an off-by-one there briefly indicted the
neighbouring script.

### A forward reference inside a tuple resolves like one anywhere else (loft#944, 2026-08-16)


An in-file forward reference resolves by ADOPTION: the name becomes a `DefType::Unknown`
stub and the declaration upgrades it in place. Nothing rewrites the `Type::Unknown(stub)`
values already stored — `rewrite_unknown_refs` runs only for the cross-file import case,
whose list is empty for one file. It works because **pass 2 re-parses** every type position
with the declaration now visible. A tuple defeated that twice over.

**Neither the name nor the layout of `__tuple<…>` survives an unresolved member.**
`tuple_def` derives both from the members' spellings: an unresolved member spells
`"unknown"`, and `element_stack_size`/`element_stack_align` have no arm for `Unknown` and
fall through to **0 and 1** — a zero-width member, frozen, because the name lookup
early-returns and nothing recomputes it. Pass 1 minted `__tuple<integer,unknown>` and pass 2
asked for `__tuple<integer,Q>`, which the H5 guard reported as an internal compiler error.
`tuple_def` now refuses to mint one until the members are final, which makes the mint
pass-2-only for the same reason `map`'s output wrapper is — H5 gained the matching
exemption, on its own stated criterion (a name-keyed idempotent append that leaves pass-1
numbering untouched). Stabilising only the NAME would have been worse than the ICE: it
reuses the zero-width layout.

**Everything that stores a type froze the pass-1 stub.** `resolve_adopted_stubs` now points
those at the real type between the passes, driven by stubs RECORDED at adoption — after the
fact an adopted stub is indistinguishable from a generic's type VARIABLE, and a first
attempt that swept by shape rewrote `vector<T>` and took the whole stdlib down with
*"expected vector<text>, got vector<T>"*. It deliberately does NOT touch a function's
`returned`: pass 2 recomputes that in full, and patching the member in place leaves the
tuple-return PROMOTION undone while making the type look settled.

**Underneath both: `Type::is_unknown()` answers for a bare `Unknown` and a vector of one,
not for an unresolved member nested in a wrapper.** Six guards asked it and got "no" for
`(integer, unknown)` — the same shape `&unknown` (#375) and `Never` (#376) had each already
needed their own arm for. They now ask the recursive `Data::type_has_unresolved`, which
walks children through `Type::for_each_child`:

| site | what the user saw |
|---|---|
| `change_var_type` (scalar + `Vector` arms) | `cannot change type from (integer, unknown) to (integer, unknown)` — one type, printed twice |
| `objects.rs` field store | `Cannot assign (integer, unknown(0)) to field W.t of type (integer, unknown(708))` |
| `parse_vector` element convert / `declared` / adoption | `cannot store (integer, Q) elements in a vector<(integer, unknown)>` |
| `new_record` + `build_vector_list` | Fatal `cannot build this record — its type never resolved`, aborting pass 1 |
| `unbox_tuple_from_dbref` | `data.def(u32::MAX)` — an internal compiler error on a plain undefined name |

**One shape is refused rather than fixed: a tuple RETURN naming a type declared below it.**
A heap-carrying tuple return is boxed into `Reference(__tuple<…>)` and given a `__retbuf`,
decided by asking whether an element carries a lifetime concern — which an unresolved member
does not — and the return type is stored on pass 1 only. Promoting it between the passes
gets the signature right and still miscompiles: the synthetic struct is minted after pass
1's `fill_all`, so it has no layout while pass 2 parses the bodies that read it and every
offset lands as `u16::MAX` (`OpDatabase(__ref_2, 65535)` where the working spelling reads
`79`). Making it work means minting and laying out that struct during pass 1, from members
that resolve only at the end of it. It now reports *"move the declaration of `Q` above
`mk`"* — which replaces an ICE and, more importantly, the interpreter-runs /
`--native`-reads-`0` split that removing the ICE alone produced.

Guards: `tests/scripts/944-forward-reference-inside-a-tuple.loft` (8 rows + 4 controls, both
backends) and `944b-forward-tuple-return-is-refused.loft`. Found and filed while writing
them: loft#946, a call result used directly as a tuple member leaks the callee's store —
pre-existing, unrelated to declaration order.

### A coroutine's yielded record is borrowed, not owned (loft#920 partial, 2026-08-16)


`collect_iterator_subject` materialises a `match <iterator<T>>` subject by pulling the
coroutine into a buffer.  The pulled value lands in `stream_x`, typed with the ELEMENT type.
For a **struct-enum** element that makes it a record ref — and the ref points into the
coroutine's own frame, which lives in the STACK store.  The append below deep-copies it
(`OpCopyRecord`) so the buffer owns its copy, but nothing said `stream_x` does not, and the
loop-scope exit emitted `OpFreeRef(_stream_x_1)` on every pull:

```
loop {#stream pull_3
  _stream_x_1(3):ref(Tok) = OpCoroutineNext(_stream_gen_1(2), 12i32);
  …
    OpCopyRecord(_stream_x_1(3), OpGetField(_stream_elm_1(3), 0i32, 78i32), 78i32);
  OpFreeRef(_stream_x_1(3));          <-- whole-store free of a stack-record ref
}
```

Only the store-0 guard kept that from taking every live frame with it; what reached the user
was `BUG (#306)`.  The sibling `stream_elm` was already marked `skip_free` for the identical
reason one line below, with a comment saying so.

`skip_free` is ONE bit for every free kind, so the mark is gated on the record case: a `text`
element's `stream_x` holds a String the caller DOES own (`OpFreeText`), and marking it would
trade the wrong free for a leak of one string per yield.  A scalar emits no free at all.
Verified both directions — the leak detector fires on a known-leaking control and stays
silent here, on both backends.

**The previous attribution was wrong, and the instrument is why.**  loft#920 was recorded
against `tests/scripts/75-native-stub.loft`; the refusals actually came from
`35p-iterator-match.loft`, which reproduces **alone**, on `--interpret`, with no harness and
no `catch_unwind` — so the "mid-execution unwind leaves a live state" theory it was filed
under had nothing to do with it.  The message carried a bare `pc=`, which is a position in
the WHOLE bytecode stream (stdlib included) while `introspect` prints only the user file's,
so no reader could resolve it.  It now names `file:line:col` (via the published span table)
AND the dispatching opcode — the span map is sparse and can point several statements away,
the op is exact, so both are printed.  That pair turned three failed attempts into one
reduction.

**And the gate that let it live.**  `35p-iterator-match.loft` raised this on every suite run
for as long as it existed and scored PASS each time: the refusal keeps the store alive, so
nothing the script asserts can notice, and the report went to stderr where nothing read it.
`Stores::free_named` now counts refusals unconditionally (`keys::stack_free_refusals`) and
`tests/wrap.rs` fails any script that raises one.  Corpus run: 3 refusals before, 0 after,
and no other script trips it.

**Not closed.**  The nightly poison gate's SIGSEGV survives this fix — `OpLengthVector`,
reached during `917-nullable-collection-field-says-so.loft`, which is clean standalone.  That
is latent corruption from an earlier script surfacing there, a second mechanism, and it does
not reproduce under `LOFT_STRICT_STORES=1` (strict mode changes free/reuse and masks it).

### A tuple's record shape does not depend on how its type was reached (loft#943, 2026-08-16)


Two defects, one report, because a vector literal of tuples needs both fixed.

**1. An inferred tuple element type registered no `__tuple<…>` struct.**  A tuple is stored as
that synthetic struct, and every consumer needing its record shape — `type_def_nr`,
`type_elm`, `fill_database` — resolves it BY NAME.  Registration happened only where a tuple
appears in a TYPE position (`sub_type`, `parse_type_full`, the `__retbuf` rewrite), which is
every DECLARED spelling and no inferred one: `v = [(7, 8)]` names no type anywhere.
`type_def_nr` answered `u32::MAX` and `new_record` refused the literal outright with *"cannot
build this record — its type never resolved"*.

`Data::vector_def` now registers from the element type, which is the point that already needs
it — the vector def's own `parent` is `type_def_nr(tp)`, and it was being stored as `u32::MAX`
beside the refusal.  `ensure_tuple_defs` recurses inside-out (a nested tuple's members are
sized from their own defs) and SKIPS a tuple with an `Unknown` member: a forward-declared
member resolves only in pass 2, and registering the pass-1 shape would mint a second
`__tuple<…,unknown>` beside the real one.  `ensure_tuple_defs_for_capture` (P216, the same
walker written for closure captures) now delegates to it rather than keeping a second list.

**The filed scope was the tuple LITERAL; the axis is inference.**  `t = (7, 8); v = [t]`
builds its element from a tuple LOCAL and failed identically — c5 in the guard.

**2. A struct-literal tuple member kept its `Rewritten` marker.**  `Rewritten(Reference(S))`
says a value was built in place (#319) — a signal to the expression that parsed it, not a type
a member can HAVE.  The tuple literal (`vectors.rs:409`) recorded it verbatim in the member
list, so every consumer matching on the type constructor missed it:

| site | what the user saw |
|---|---|
| `set_field` | `Cannot assign to field '_0' of type S` |
| `get_val` | `Field access not supported on type S` |
| `emit_tuple_put_ops` | `internal compiler error — unsupported elem Rewritten(Reference(707, …))` |

That last row needs no vector at all: a bare `t = (S { … }, k)` was an ICE on released
2026.8.0.  The wrapper is now peeled at the producer through the new `Type::unrewritten()`,
which is the same move `parse_vector` and `parse_vector_for` already make for a vector's
ELEMENT type — a tuple member is that fact one level in.  A struct member reached through a
LOCAL or a CALL was always fine, which is what named the literal as the axis.

Guard: `tests/scripts/943-inferred-tuple-element-type.loft`, 9 rows + 4 controls, both
backends.  Out of scope and filed separately: a forward-declared type inside a tuple never
resolves (loft#944) — it fails on the DECLARED path too, so it is stub adoption, not
registration.

### A tuple vector element is not offered the element slot (loft#942, 2026-08-16)


`parse_item` seeds the element expression with `Value::Var(elm)` — the slot `OpNewRecord`
carved out of the container — so a struct element builds straight into it; `parse_object`
takes that in-place path whenever it is handed a `Var` that owns a store.  A TUPLE element is
not built as one record: `emit_tuple_set_ops` writes each member at its own offset.  Offering
the slot let the tuple literal's FIRST member consume it, so `[(S { … }, k)]` wrote S's fields
directly into the element and then handed that valueless statement list to `OpCopyRecord` as
its SOURCE:

```
_elm_1 = OpNewRecord(v, 81, 65535);
OpCopyRecord({ !! INSERT                      <-- a statement list, no value
    OpSetInt(_elm_1, 0, 11); OpSetInt(_elm_1, 8, 22)
  }, OpGetField(_elm_1, 0, 78), 78);
```

Only the FIRST member could fail this way: `(a, b, …)` parses member zero into the caller's
value (`vectors.rs:409`) and every later member into a fresh one, which is the whole reason
`vector<(integer, S)>` was correct while `vector<(S, integer)>` was not.

One defect, four filed symptoms — each construction path corrupts differently: reading an
element back panicked in `allocation.rs` using the tuple's SECOND member value as a record
index (`--native` refused the generated Rust with E0308), a 2+ element literal aborted the
compiler with `Incorrect var _elm_1[56] versus 40`, a single `+=` silently zeroed the struct's
fields, and a second `+=` SIGSEGV'd.

**The guard keys on the `(` token, not on the element type.**  A literal in RETURN position
infers its element type from itself, so the type is still `Unknown` when member zero is seeded
and only resolves by member one — instrumenting the seed showed the guard firing 4× for a
declared local but only 2× in return position.  A type-keyed attempt therefore fixed every
other row and turned the return-position abort into a SILENTLY EMPTY vector, which is why the
regression guard asserts lengths as well as values.  A `(`-leading element that is not a tuple
(`[(S { … })]`) gives up the in-place build and takes the allocate-then-copy path every
non-first member already takes; the unparenthesised `[S { … }]` common case is untouched.

Not fixed, and separate: a vector literal of tuples cannot have its element type INFERRED
(`v = [(7, 8)]` fails with "cannot build this record — its type never resolved" for every
tuple shape including ones with no struct at all, on the released binary too).

### A destructured tuple element is a value the binding owns (loft#941, 2026-08-16)


A tuple return wider than 8B lands in a synthetic `__tuple<…>` record held by a work-ref
belonging to the CALL SITE, so one site reuses one buffer.  Destructuring read each element
straight out of it — `OpGetField(tmp, offset, …)` answers a DbRef sharing `tmp`'s `store_nr`
and `rec` — and made that VIEW the binding.  Reassigning the work-ref frees the store it
named, and reassigning it is exactly what the next turn of a loop does, *before* the call it
feeds:

```
327: VarRef(__ref_2)              ; the tuple store from the previous iteration
330: FreeRef            [store-free]
337: Call(fn=n_passthrough)       ; xs, which VIEWS that store, is passed in here
```

So the binding dangled from the second iteration on.  One use-after-free, two symptoms:
reading a freed store answers its cleared contents, so `(xs, n) = passthrough(xs)` reported
`len` 0 while `--native` — which does not emit that free — answered correctly; and appending
onto a record the arena had recycled panicked in `vector_append`.

P250 had given a `Reference` element a DEPENDENCY on `tmp` so scope analysis would not emit a
second `OpFreeRef` for the binding.  That stops a double free, but a dependency cannot lengthen
the buffer's life past the reassignment — the binding still outlived the record it was read
from.  `materialize_tuple_element` copies instead, the same materialise-the-view move
`return <field>` (#306) and `&out = <field>` (loft#775) already make, which is why those two
directions were safe and this one was not.  A record goes through `OpDatabase` + `OpCopyRecord`,
a collection through `vector_db` + `OpAppendVector`; value-typed elements are read by value and
are untouched.

The filed scope was a third of it.  A plain STRUCT element fails identically, so the axis is any
element read back as a pointer, not `vector<T>`; and the `xs = f(xs)` spelling is not needed —
any binding that names the buffer and is read after the site runs again will do.  What IS
load-bearing is the SITE repeating: two distinct call sites alternating in one loop each own a
buffer and were always correct.

The result is emitted as a flat `Insert`, not as `Set(v, <block ending in v>)`: the allocation
writes through the binding itself, and the native backend renders a first binding as
`let mut var_v = <init>`, which rustc rejects when `var_v` appears inside it.

### `loft test` shares one library parse across the files that `use` it (loft#925, 2026-08-16)


`run_tests` builds one `Parser` per test file, deliberately — a shared one would let one
file's definitions leak into the next.  Each of those parsers loaded the `use`d library from
source, and **twice**: `Parser::parse` runs two passes, `Data::reset` clears `use_names`
between them, and an unnamed library is one `use` re-reads.  A suite therefore paid the
PRODUCT of its file count and its library's size — measured at 0.068 s/file against a
no-`use` control's 0.022 s/file, with the per-file cost proportional to the module count.

Three pieces:

- `Data::preloaded_uses` + `Data::freeze_uses`.  `reset` re-seeds `use_names` from it, so a
  `use` of an already-parsed library takes the `use_exists` branch — a pending import against
  definitions that are already present — instead of `switch_to_dep`.  This is the whole
  mechanism; everything else is plumbing around it.  Empty for every ordinary parse.
- `Parser::parse_as` — `parse` refactored so the entry can be a STRING claiming a filename,
  the two `lexer.switch` sites going through one `load_main_file`.  Not `parse_source`, which
  skips the between-pass promotions (`reserve_late_return_buffers` and friends): a base built
  that way would hand its libraries on in a state no ordinary parse produces.
- `Parser::seed_from` — `data` cloned, `database.install_schema`, plus `use_paths` and the
  native/placed-library registrations a manifest read queued.  The parse-time side maps
  (`complexity`, `field_read_counts`, the sandbox designations) deliberately do NOT travel:
  they drive diagnostics the base already emitted, and copying them would emit each twice.
  The runner carries the base's diagnostic LINES instead — minus the ones positioned at the
  base file itself, which every group member re-emits from its own `use` line.

`test_runner` groups files by `(directory, lib search path, leading use region)` — the region
VERBATIM, so the parser stays the authority on what those lines mean and one key is one
library set by construction.  The base is built when a SECOND file asks for it; a seeded file
skips the stdlib warm load (the base holds it, and decoding the bundle only for `seed_from`
to discard was most of what a seeded file still paid) and takes `start_def` from the base's
recorded stdlib boundary, so the native codegen range and the coverage tally keep counting
the library as part of the program under test.

Refused, falling back to the ordinary parse: under a `[sandbox]` policy (admission reads what
the parse recorded about designated functions), on a base parse that panics or errors, and on
any region that is not plainly an optional `#cwd` plus complete `use` statements.  `#cwd` is
IN the region rather than a reason to give up — all 81 of dryopea's test files open with one,
so refusing it made the change measure perfectly on a synthetic and do nothing for the case
that motivated it.

Measured: 20 files / 25 modules 1.32 s → 0.43 s, 40 files 2.68 s → 0.74 s, 20 files / 50
modules 2.44 s → 0.81 s; dryopea's 81-file, 1161-test suite 238 s → 209 s with byte-identical
output.  `loft test <one-file>` unchanged (0.07 s → 0.06 s).

`LOFT_NO_TEST_BASE=1` is the opt-out and `LOFT_TEST_BASE_REPORT=1` names the shared regions.
`tests/test_base_equivalence.rs` compares a whole run against the opt-out over a package with
four groups across two libraries — proved able to fail by dropping the carried diagnostics
(turns `@EXPECT_WARNING` and `--deny-warnings` green) and by dropping the region from the key
(a file resolves a library it never named).

Also here: the per-function `@EXPECT_ERROR` / `@EXPECT_WARNING` / `@EXPECT_FAIL` maps are
`BTreeMap`s.  They are iterated to REPORT the function names a file satisfied, and hash order
is randomised per process, so the same green run printed that list in a different order every
time — which makes a run's output undiffable, and this change is verified by diffing runs.

### `sizeof` and `type_name` answered null for an undeclared name (loft#933, 2026-08-15)


Both intrinsics read their argument the same way: take the identifier, look it up, and — when
the def exists but is still `DefType::Unknown` after pass 2 — mark the argument *found* and
return.  `*val` keeps its `Null` initialiser, so `sizeof(NoSuchType)` answered `null` and
`type_name(NoSuchType)` rendered `null` as if that were a type's name.  Neither said anything,
and the null flowed on as a value.

A name still unresolved after pass 2 is not a forward reference — those resolve in
`resolve_deferred_unknowns` and take the branch below with a real size (verified: a struct
declared after its use, and a type named only in an earlier signature, both answer correctly).
It is a typo, and it is the likeliest way to reach either intrinsic wrongly.  Both now report
`Undefined type <name> — sizeof/type_name needs a variable or a declared type`, still marked
found so the expression path adds no cascade.

`src/parser/operators.rs`'s bare `Unknown variable` also names its variable now, matching every
sibling site (`Unknown variable 'x' — did you mean 'y'?`).  Half of loft#934; the other half —
an undefined comparison operand whose ONLY report is `missing argument for parameter 'v1' of
`OpLtInt`` — is pinned in `36-parse-errors.loft` and left open.

### `--lib` is part of the program-cache key (loft#930, 2026-08-15)


`program_cache_paths` hashed the entry script's path and nothing else, so one script run
against two library trees shared a cache slot and the second run silently reused the first
tree's build.  Nothing downstream could catch it: the drift manifest re-validates the files
the FIRST run resolved, and those are still unchanged, so the freshness check passes and
hands back the wrong library's code.

That made the tool look responsive while ignoring the flag that selects which library it is
responding to — an in-place edit of the bound library DID rebuild, moving the tree away DID
force re-resolution, and only the `--lib` value itself changed nothing.  A consumer's A/B
harness (the Moros Economy planet-generator port, verifying loft against a compiled C# twin
by running one entry script against two library trees) compared an arm against itself and
read the byte-identical output as the strongest possible pass.

The search path now feeds the key, length-prefixed and in order — order matters because the
same dirs listed differently can resolve a name to a different file.  The cache still hits:
repeated runs against one tree keep one manifest and stay warm (0.04 s cold → 0.01 s), and
two trees now keep two.

`loft` built in a Cargo tree disables the program cache by default, which is why this
reproduces on an installed `loft` and needs `LOFT_PROGRAM_CACHE=1` to show up in a dev
build — worth knowing before concluding a cache defect is fixed.

### 56 of 167 `@EXPECT_ERROR` annotations never fired, and the suite reported nothing (loft#929, 2026-08-15)


`check_diagnostics` failed a file on *unexpected* errors and on unmatched `// #warn`
patterns.  It collected unmatched `@EXPECT_ERROR` and `@EXPECT_WARNING` substrings and then
DROPPED them, so an expectation whose diagnostic had been reworded, narrowed or removed kept
passing.  A third of that guard family was inert.

**The dominant cause was not message drift.**  `Parser::parse` runs pass 2 only when pass 1
finished clean, and a large share of loft's diagnostics are emitted by `!first_pass` code —
`Unknown variable`, the const/`&` checks, match exhaustiveness, the @PLN25 N-Store family, the
type-mismatch messages.  One pass-1 error therefore silences every pass-2 diagnostic in the
same file, and an annotation for one of those can never match however correct its wording.
Two files held 52 of the 56 for exactly this reason.  Error fixtures are now split by pass —
`102`/`102b`, `36`/`36b`, `35`/`35b` — and TESTING.md records the rule.

The rest triaged into reword (the argument/return/branch messages became `expected X, got Y
on …`; the `&`-vector concat refusal was rephrased), tier (the whole N-Store family reports at
`warning`, not error — `@EXPECT_WARNING` now), over-count (three `Undefined type V` signatures
earn ONE diagnostic), delete (a tuple in a struct field is supported since Plan-06, so its
refusal is gone), and blocked-by-a-neighbour (three tuple fixtures sourced their nullable from
`s as integer`, which now errors `text-parse-may-fail` on the line before, so the N-Store check
was never reached — they source it from division instead).

Two guards had gone inert without any annotation being wrong:

* `389-narrow-sentinel-rejected.loft` asserted the pre-@PLN25-F2 rule.  F2 made a plain narrow
  integer non-null and full-range, so `nullable_sentinel_hint` returns early and there was no
  error left to expect.  It now pins the rule from the value side, plus the half F2 left open
  (a NULLABLE narrow still spends its top value on the sentinel: `U8Q { x: 255 }` reads back
  null, silently).
* `894-lost-write-through-returned-struct.loft` expected a diagnostic **the harness could not
  produce**: `warn_lost_temp_writes` and its two neighbours ran only in `src/main.rs`.  They now
  run in `run_test` in the same window, so the suite can both confirm one of their warnings and
  catch a false positive from one.

Both holes are closed: unmatched expectations are fatal, and the check runs even when a file
produced NO diagnostics — the second way an expectation went unlooked-at.  `loft test`
(`src/test_runner.rs`) had the weaker form of the same hole, where any single matching error
satisfied every `@EXPECT_ERROR` in the file; each substring must now match one, the bar
`@EXPECT_WARNING` already held.

Three defects surfaced that the inert guards had been hiding, filed with repros: an `i32`
struct field silently truncates a 64-bit integer (loft#931 — `i32` is the one narrow alias
declared without a `limit(…)`, so the range-containment narrowing test cannot see it), the
reserved-`key` hash guard covers a struct field but not a local (loft#932), and
`sizeof(<undeclared name>)` answers null with no diagnostic (loft#933).  A fourth, an
unresolved comparison operand cascading into `missing argument for parameter 'v1' of
`OpLtInt``, is pinned in the fixture and filed as loft#934.

### A struct that contains itself says so, even when the program uses it (loft#929, 2026-08-15)


`Data::has_value_cycle` skipped recursing into a child struct that `def_referenced` marked.
That flag records that a struct has been CONSTRUCTED somewhere (`build_object_ops` and the
object literals set it) — it says nothing about whether the FIELD is a reference.  So the cycle
report fired only for a cyclic type nothing instantiates, and every cyclic type a real program
writes fell through to the layout validator instead:

```
Error: type layout: PENode: field 'next' has no position (u16::MAX)
```

in place of *"Struct 'PENode' contains itself (directly or indirectly) — use reference<PENode>
to break the cycle"*.  The field's own deps are what say "reference" (the `u16::MAX` share
marker), and that test was already there; the extra condition only suppressed the good message.
Verified across the shapes the rule separates: `reference<Self>`, mutual A/B, mutual broken by a
`reference`, plain nesting, `vector<Self>`, and a cyclic type never constructed.  The
`@EXPECT_ERROR: contains itself` fixture that should have caught this had itself gone inert.

### A `for` loop binds its own variable, so two loops may reuse a name (loft#915, 2026-08-15)


A loop variable was an ordinary function-scoped local: `add_variable` resolved it by name, so
a second `for` over the same name was handed the FIRST loop's slot — old var, old type, old
dep. That is why two loops in one function could not reuse a name at different element types,
and it is the mechanism behind loft#690's corruption (the second body read B's records through
A's layout, `m=8589934636` for a sum of 3), which had been answered with a diagnostic.

Each loop now binds its own variable. `Function::loop_binding` names them: the first loop to
use a name binds the name itself — so a program with no repeat spells every loop variable
exactly as it did before, dumps and debugger frames included — and a second binds `i#1`, a
third `i#2`. The suffix is on the NAME and not merely on a lookup key because the native
backend names a local `var_<name>` and two locals spelling one name declare it twice, the same
constraint loft#928 hit for a generator's fields.

**The cross-pass identity key is separate from the name** (`Function::loop_variable`, key
`<name>#bind`). A loop variable cannot key on its own name: the name is re-pointed at each loop
that binds it, so `names["i"]` ends pass 1 holding the LAST loop's slot and pass 2's FIRST loop
would be handed it — a text binding reusing an integer one, which is the shape the split exists
to stop. The occurrence counter reads no type and consults no table, so pass 2 regenerates the
same sequence and every slot number holds.

`i` after the loop still reads what the last loop left, so nothing that read it before changes.
The companions (`#index`, `#next`, `#count`, `#iter_state`) are keyed off the binding rather
than the spelling, and `iter_op` derives that base from the variable the name resolves to, so
`i#index` in the second loop finds the second loop's counter. `loop_nr` — which `#break` and
`#continue` jump on — matches the loop by BINDING instead of by name, since comparing names
would walk past a loop whose variable is `i#1` and answer the chain length.

**Two diagnostics folded into one.** loft#690's *"loop variable 'i' has type text but was
previously used as integer"* is gone: the corruption it reported is unreachable by
construction, and the local collision it also covered no longer needs a type comparison to
state — any non-loop binding of the name is the shadow, whatever its type. The C61 shadow
diagnostic now owns that case and fires on PASS 1 for every type pairing, where the type
diagnostic reached the differing-kind case only on pass 2.

Still rejected: a loop variable landing on a plain function local, and nested same-name loops
(the inner binding would take over the name for the rest of the outer body).

Guard: `tests/scripts/915-loop-variable-per-loop.loft` — 13 hand-computed cells on both
backends, covering the filed shape, the after-loop read, `#index` / `#count` / `#first` /
`#break`, loft#690's two-struct shape, three loops under one name, text loops, comprehensions,
`_`, nesting, and per-function independence.

### A loop that writes no store reads its vectors through one derived header (loft#885, 2026-08-15)


`v[i]` re-derives three facts per element on `--native`: which store holds the vector, which
record its elements live in, and how long it is. All three are loop-invariant in a loop that
writes no store, and `rustc` cannot lift any of them — every store load is guarded
(`if rec != 0 && valid(..)`) and LLVM will not speculate a conditional load out of a loop.
So the emitter lifts them: `let __vh_N = vector::vec_header(…)` lands before the loop and each
read becomes a bounds test plus address arithmetic. **~2× on the issue's kernel**, taking
`vector<single>` indexed reads from ~15× hand-written Rust to ~6.5×.

A scalar read of a hoisted element then fuses into ONE load (`vector::get_elem_hoisted`):
the bounds test, then the value, with no element `DbRef` built, no `rec == 0` test, no second
store resolution and no `rec != 0 && valid(..)` re-check between them — the bounds test
decided all of it. Covers `OpGetInt` / `OpGetSingle` / `OpGetFloat`, the getters whose `Store`
bodies are a plain guarded load; the masking / re-basing / decoding ones stay unfused. That
also meant teaching the pre-eval collector about the fusion: `OpGetVector*` is on
`op_uses_stores`, so it was being hoisted into a `let _pre_N` that the fused emission
ignores — which would have run the read twice. `hoist::fused_element_read` is the one
definition of the shape, and the emitter and the collector both ask it.

Only an index in range for the hoisted length takes the fast path — a negative index, an
out-of-range one, `i64::MIN`, a null or an empty vector all fall back into `get_vector` /
`vec_get_or_raise_runtime`, so those answers and the `IndexOutOfBounds` / `NegativeIndex`
raise keep one definition. The interpreter is untouched.

The gate (`src/generation/hoist.rs`) is an **allow-list**, deliberately: PERFORMANCE.md
§ Design: P8 catalogues five hand-maintained deny-lists of "which op mutates" that have
already drifted, and an omission in one of those would be a silent wrong read. Here an op
missing from the list costs the optimisation instead. An op qualifies by being named as a
reader/constant, or by declaring at least one parameter with **every parameter a plain
runtime scalar** — where `const` disqualifies, because a `const` parameter is a slot number
or type id, and that is the channel `OpDatabase`, `OpCoroutineNext` and `OpFreeText` reach
state through despite scalar signatures. The walk follows calls, so `for i in 0..len(v)`
still hoists; `CallRef`, `par` and `yield` decline.

Two switches, both read at generation time: `LOFT_HOIST_VERIFY=1` emits the checking form of
every hoisted read (re-derives the header, panics on a mismatch) and `LOFT_NO_VECTOR_HOIST=1`
emits the pre-885 form. Guards: `tests/hoist_gate.rs`, `tests/scripts/885-vector-hoist.loft`.

### A `τ?` struct field is representable as absent (loft#896, 2026-08-14)


A field declared `maybe: Inner?` was stored as a dense `Inner`, byte-identical to a
non-nullable one. `OpGetField` therefore handed back a `DbRef` into the PARENT record, whose
`rec` is never 0, so every reader saw a present-but-zeroed value: `??` never reached its
default, `== null` was always false, and `h.maybe = null` found nothing to clear. Both
backends, silently.

**The representation was already built.** The synthetic `__nullable<S>` enum (discriminant 0 =
absent) has shipped default-on for vector ELEMENTS for some time, and `vector<Inner?>` answers
every cell of loft#896 correctly — which is what made it the oracle. What was wrong is the
rewrite that assigns that type to a FIELD (`typedef.rs::synth_nullable_struct_fields`): it
matched a bare `Type::Reference`, and `Inner?` reaches the type table as
`Optional(Reference(Inner))`. A field written `Inner` — one that *cannot* be absent — is the
bare `Reference`. So the arm selected the exact COMPLEMENT of its intended set, rewriting every
dense field and never once firing for the `S?` it was written for.

That inversion is also why it sat behind `LOFT_E2_FIELDS`: the gate's stated justification was
that flipping fields tree-wide breaks stdlib field reads, which is what rewriting *dense* fields
does. The symptom was read as the representation being immature rather than as the selector
being backwards, and the read/construct glue the gate's comment called missing turns out to
work — a bare `h.maybe.z` auto-unwraps.

Fix: select on `Optional(Reference(S))`, drop the gate, and add the literal-`null` construction
path (`H { maybe: null }`) — assignment already had one, so both now route through a single
`Parser::build_nullable_set_null` and the two spellings of "absent" cannot drift.

Also fixed by the type change: a struct literal that merely OMITS such a field did not compile
under `--native`. The omitted default was `Value::Null`, which the interpreter tolerates as a
no-op `OpCopyRecord` of a null source and native lowers to `OpCopyRecord(cell, (), …)` —
`()` where a `DbRef` is expected. It reproduced on the released `loft 2026.8.0`.

**Costs.** A `S?` field carries a discriminant, so it grows 8 bytes;
`tests/multilib/fwd797_layout.loft`'s hand-computed sizes moved 44→52 and 52→60. A
`vector<T>?` FIELD is `Optional(Vector)`, a different payload shape, and is unchanged — still
wrong, and genuinely separate work.

Guard: `tests/issue_896_nullable_field.rs`, 16 cells on both backends, including a dense-field
control that fails if the selector ever again keys on anything but the `?`.
`tests/plan25_e2_layout.rs`'s fixture declared `item: Row` and asserted the rewrite fired — it
passed by encoding the inversion, and now declares `item: Row?` with a dense sibling beside it.

### A partial struct literal names the field it left out (loft#914, 2026-08-14)


New `advice[omitted-field-zero]`, default on, `LOFT_NO_OMITTED_FIELD` opts out. A literal that
names SOME fields and leaves another out gives the omitted one its type's zero, and nothing
distinguished that from an author writing the zero deliberately. It bites where zero is a
meaningful value of the field's domain — dryopea's palette index wanted `-1` for "nothing
selected", got `0`, and `0` is the entry that erases; the project carried a two-field workaround
and a CLAUDE.md rule for it, because the cure (a declared field default) was undiscoverable.

`advice`, not `warning`, per the two-tier rule: the zero is documented behaviour
(`tests/scripts/06-structs.loft` locks it), so ignoring it cannot produce a result the language
did not promise — and a warning would fail every library's own `LOFT_DENY_WARNINGS=1` CI on a
common idiom, which is the trap the tiers exist to avoid.

Quiet where the code already says what it means, or where the cure would be a no-op: a declared
default; a nullable field; `reference<T>` and fn-ref fields (their omitted default is a null
sentinel, and a fn-ref has no other default to declare); collections and `text` (their zero is
the identity, and `= []` / `= ""` IS that zero); and a bare `S {}`, which asks for the whole
default record and reads that way. Only the PARTIAL literal is ambiguous.

The last three exemptions came from the suite rather than from reasoning —
`issue_328_reference_field_pointer_semantics` and `p213_struct_field_default_init` each name a
field whose absence is already the declaration's promise. Swept before it spoke: 25 hits across
7 of ~400 corpus files, and one golden baseline moved
(`tests/error_messages/cases/33_struct_missing_field.loft`, which produced no output at all).

### `loft test` no longer reports a green for a file it did not run (loft#916, 2026-08-14)


`loft test <a> <b>` silently discarded everything after the first target. It ran `<a>`, printed
`test result: ok. 1 passed; 1 file`, and exited **0** — even when `<b>` held a failing test. The
file count was the only place it showed, and that reads as correct unless you already knew how many
you asked for. Naming two files is the natural move when a change touches two suites and the whole
run is slow, which is exactly when nobody re-reads the count: it cost a sabotage sweep whose second
half never executed and was reported green.

A second target is now an error naming both, not a drop. One target per run is kept deliberately —
the summary line is a single verdict over one scope, and looping would print a partial one per file,
which misleads in a new way rather than fixing this one. Only the CONSECUTIVE leading positionals
are examined, so a later flag's value (`--lib <dir>`) is never mistaken for a second target; that
row is in the guard, because a rule written as "no bare token after the target" would have broken
it. Guards: `tests/test_command_targets.rs`, asserting the EXIT CODE as well as the text — the exit
code is what a CI job reads, and it was the half that made this dangerous rather than merely
confusing.

### A module shadowed by a dependency's same-named file now says so (loft#912, 2026-08-14)


A module's basename is global across a consumer's whole dependency graph. Only the first file to
claim a name is loaded, so adding `src/catalogue.loft` to a package whose dependency already had
one made the LOSER's functions simply absent — reported as `Unknown function part_list` at a line
inside a package the consumer never edited and cannot fix. Nothing in the output mentioned a
collision, so the search went looking for a missing `pub`, a typo, or a version skew.

New `advice[module-name-shadowed]` names the collision and BOTH files: *"module 'catalogue' is
declared by two files — '…/pkg_top/src/catalogue.loft' and '…/pkg_dep/src/catalogue.loft' — … this
`use` binds the second one"*. Both load orders are covered, so the report does not move when a
`use` is reordered.

**The resolution itself is unchanged, and that is deliberate — decided against a measurement rather
than a preference.** The obvious fix, refusing the clash, was implemented first and then measured:
it breaks code that builds today. `graphics` ≤ 0.4.2 and `mesh3d` both ship `math` / `mesh` /
`scene`, and this repo's own `tests/fixtures/libs/graphics` depends on the registry `mesh3d` while
carrying its own copies of all three — three test binaries went red. A first sweep over
`~/.loft/registry` alone had said the clash was extinct; it missed the fixtures, which is the axis
that was held fixed.

**Scoping module names to their package is the fix this advice is a signpost for.** Two things
block it, both worth recording: `Data::use_add` derives a new source id from the SIZE of the name
map (so a second key per module happens to keep the counter right, but nothing says it must), and
`qualified_type_name` derives a DATABASE type key from a module's short name — a package-qualified
key has to stay machine-independent, so it cannot carry the package's path. Guards:
`tests/module_name_clash.rs`, which asserts the advice fires in both directions AND that the
`Unknown function` symptom still follows, so a test cannot silently claim the fix that has not
landed. It also pins the three neighbouring shapes that must stay silent: a distinct name, one
module used from two files of one package, and a file named like a declared dependency (which the
existing dep-shadowing guard already resolves).

### `loft doc <library>` documented nothing, into the directory you were standing in (loft#911, 2026-08-14)


`loft doc` reads as — and is used as — `loft doc <library>`, but its argument was a PATH only. A
library name is not a directory, so `loft doc graphics` took the empty-manifest branch: it created
`./graphics/doc/` wherever the user happened to stand, found no `src/` to read, and reported
"0 API sections" for a package with 119 documented `pub fn`s. The path it printed was relative, so
the stray tree looked like part of the project — one was swept into an unrelated repository by a
later `git add -A`.

The argument now resolves as a directory first and an installed package second, and a name that
resolves to NEITHER is an error that creates nothing. An installed library's docs go to
`~/.loft/doc/<name>-<version>`, because the registry copy is shared immutable cache content and the
working directory is not loft's to write to; `-o <dir>` overrides, and the reported path is
absolute. `loft doc graphics` now reports 1 guide and 19 API sections. Guards:
`tests/doc_command.rs`.

### A non-empty collection literal on a nullable field aborted the compiler (loft#909, 2026-08-14)


`struct S { m: vector<integer>?, t: integer }` with `S { m: [5], t: 3 }` aborted with
`Incorrect var _vec_1[65535]`, on both backends and on the published release. Whether a field
carries a record-pointer HEADER is a question about its storage, and `Optional(τ)` shares τ's
storage exactly — but `parse_object_field` asked it by matching the declared type against the
collection formers without peeling the marker. The field was therefore not recognised as a
collection: the literal built through a standalone temp instead of in place, and that temp, minted
with a dep on the struct it sits in, is exactly the case where `build_vector_list` skips
`vector_db`. Nothing assigned it, so it reached codegen with no live interval and no stack slot.

Both halves were needed — a non-nullable field took the in-place path, and an empty literal
returned before the temp was minted — which is why each of them rescued the program. A nullable
KEYED field failed one step earlier still, refusing the literal outright with "Cannot assign
vector<R> to field of type optional(hash(…))".

The peel is applied at the three sites that classify a field by its layout — `parse_object_field`'s
header prime, `handle_field`'s deep-copy dispatch, and `get_type`, whose sibling resolvers
(`type_def_nr`, `type_elm`, `rust_type`, `element_stack_size`) all peeled already while it answered
`u16::MAX`, the "no such type" sentinel, for every `Optional`. The nullability checks keep the
unpeeled type: those ARE about nullability. Guard:
`tests/scripts/909-nullable-collection-field-literal.loft`.

### A bare `if` statement swallowed the `[` of the line below it (loft#910, 2026-08-14)


`[` postfix-indexes a value, and a `Void` expression produced none. `if` is an expression in loft,
so a bare `if c { … }` STATEMENT reached the postfix chain that handles `.`, `[…]` and `(…)` — and
that chain consumed the bracket opening the next line. A function whose tail expression was a
vector literal had that literal read as an index on the `if` above it, and indexing a `Void` fell
to the catch-all: *"Indexing a non vector — keyed collections have no generic-constructor
expression"*, naming a feature the program does not use and a line that is correct as written.

The filed scope was a comprehension in tail-return position reading a local a bare `if` had
mutated. None of those three is the trigger: a plain `[1, 2]` fails identically, `else` makes no
difference, and the mutation is irrelevant. `for` and `while` never had it — they are statements
and never reach the chain. The guard reads the subject's TYPE, so an `if` that yields a value keeps
its index: `if c { [1,2] } else { [3,4] }[0]` is unaffected. An indexed `Void` CALL
(`voidcall()[0]`) is still an error, so no wrong program became a silently accepted one.

**A native-only defect surfaced while pinning that last row**, present on the published release
too: `if c { [1,2] } else { [3,4] }[0]` ran correctly under `--interpret` and failed native codegen
with "expected expression, found `let` statement". A pre-eval binding is emitted as
`let _pre_N = <text>;`, so the text has to be one Rust expression — and an `if` that pre-declares
its branch variables emits `let mut var__vec_1: DbRef = …; if …`. The wrap that made a statement
sequence an expression had been written into ONE of the two lowerings that produce that prefix, so
the other never got it. It is now enforced on the artifact instead: whatever lands in a `let`
binding is braced if it is a statement sequence, which no producer can drift away from. Guard:
`tests/scripts/910-statement-if-does-not-index-the-next-line.loft`, asserted on both backends.

### `loft test` refused the path it prints (loft#913, 2026-08-14)


`loft test` reports its files as `tests/<name>.loft` and rejected that exact string: the argument
was joined onto `tests/` unconditionally, so pasting a failing line back asked for
`tests/tests/<name>.loft`. Copying the path out of the output is the obvious way to iterate on one
file, and the tool not accepting its own output is re-discovered by every new user rather than
learned once.

Measuring every spelling before fixing turned up a second break the report did not mention:
`loft test draw::test_foo` — the selector form the code's own comment documents — resolved to
`tests/draw::test_foo`, whose path half has no extension and matches no file. The `.loft` was
appended to the whole argument rather than to the PATH half, so the documented form only worked
if the caller also wrote `.loft`.

`resolve_test_target` now splits the `::selector` off first, supplies the extension on the path
half, and joins `tests/` only when the path is not already under it (nor absolute, nor reaching
out with `..`). The doubled path could never exist, so every spelling that worked before resolves
to the same file; four spellings that used to be errors now work. Unit tests in `src/main.rs`.

**And the accidental guard it removed is replaced by a real one.** `loft test good::test_missing`
used to fail — for the wrong reason, on the mangled path — while the correctly-spelled
`loft test good.loft::test_missing` reported `ok. 0 passed; 0 files` and exited 0, on the published
release too. A filter that matches nothing left every file empty and each was skipped silently. A
selector naming no test function is now an error: it is the shape a CI job reads as "the tests I
asked for passed". Only an explicit selector is checked — a directory with no tests is a different,
legitimate zero. (A brace list with SOME matches still runs those and reports them; only a
completely unmatched selector fails.)

### An empty-text assignment pushed a value nothing consumed (loft#908, 2026-08-14)


Reported as "a function that reads a MISSING file and returns a struct double-frees and SIGABRTs
the interpreter" — `free(): invalid pointer`, `last op: OpFreeText`, on `--interpret` only, which is
the worst direction: a consumer's gates run interpreted and the shipped native build is correct.

Neither the file nor the struct is the defect. Appending the EMPTY literal to a text variable is a
no-op — the variable has just been cleared — so `set_var`'s put dispatch skipped `OpAppendText` for
it. It skipped only the OP, after `self.generate(value, …)` had already pushed the 16-byte const:
the value stayed on the eval stack with nothing to take it off, and `stack.position` ran high for
the rest of the statement. **A value is pushed if and only if an op consumes it**, and the two
decisions sat in different places.

Harmless until the statement is one ARM of an `if`/`else` — which `?? ""` over a CALL always is, the
nullable result going into a work-ref that the presence test branches on. The arms then disagreed in
height and `gen_if`'s arm-height equaliser (@PLN85 P2) "corrected" the taller one with an
`OpFreeStack` whose discard walked past the frame's eval base into the LOCALS, overwriting a live
text descriptor; freeing that at scope exit aborted. The aggregate return is what puts a live local
under the over-discard (the hidden `__retbuf` shifts the frame), which is why the reporter's matrix
found it needed a struct return — and why `-> integer` merely mis-tracked the stack silently.

Four axes had to meet: the nullable text from a CALL, the call answering null, an EMPTY default, and
an aggregate return. Moving any one made it correct, which is what kept it hidden.

The guard now returns BEFORE the push, so push and consume are one decision;
`gen_set_first_text` already skipped the whole `set_var` for this value, so the reassignment path
now agrees with the first-assignment path rather than diverging from it.

Guard: `tests/scripts/908-empty-text-default-does-not-strand-a-const.loft`, one axis per row and
every row asserting a VALUE — `--native` was always correct, so a row that merely ran would read as
a pass there. Verified to abort on the pre-fix build before shipping.

### `--native` linked a `#native` symbol by NAME, not by what implements it (loft#907, 2026-08-14)


`#native "sym"` is an API id, not the name of the Rust fn behind it. A library registers its
implementations by loft symbol — `loft_register_bridges! { "sym" => other__loft_bridge }` — and
that table is free to name an `other` different from `sym`. `--interpret` reads the table.
`--native` put the `#native` string straight into a `#[link_name]`, so it bound whatever else the
cdylib happened to export under that name, and a C-ABI link matches on name alone: no error, no
warning, a call marshalled into the wrong function.

In the published `graphics` that hit **ten** functions — every store-aware one. Each has loft's
`(LoftStore, LoftRef)` entry point at `n_<x>` and an older raw `(ptr, count)` fn under the
`#native` name, so the arguments arrived shifted by a register. `save_png` returned `false` and
wrote nothing under `--native` while returning `true` under `--interpret` (the reported symptom);
`gl_upload_vertices`, `gl_upload_canvas`, `gl_upload_indices`, `gl_upload_instance_buffer`,
`gl_update_buffer`, `gl_set_mat4`, `gl_texture_subimage`, `rasterize_text_into` and
`audio_play_raw` were mis-marshalled the same way and had no reporter because the WebGL
consumers run in the browser, whose `--html` host imports take the raw pair by design.

**One source for the answer, read by both backends.** `extensions::resolve_native_impl_symbols`
asks the loaded cdylibs' own registration which fn implements each symbol (`dladdr` on the
registered bridge names it; `X__loft_bridge` sits beside `X`), and records only the entries where
the two names differ, in `Data::native_impl_symbols`. `Data::link_symbol` is what codegen emits
through, on both the C-ABI `#[link_name]` and the rlib `krate::sym` path. A clean binding — what
`loft-ffi-build`'s generator produces, and the only shape it CAN produce — maps to itself and is
untouched.

Residual: a library whose cdylib is absent or predates the bridge registry cannot be resolved and
keeps the literal name. That is not a silent wrong answer — the interpreter reports it at load
(loft#886) and calling it panics rather than answering.

Guards: `tests/lib/native_remap_pkg` is a `[native] crate` fixture in exactly this shape, exporting
a DECOY under each `#native` name (-1000 / -2000) so a regression answers rather than fails to
link, and the answer names which resolution path was taken;
`native::remapped_native_symbol_resolves_to_its_implementation_on_both_backends` runs it on both.
`native_scalar_pkg` is the clean-binding control.

### Removing one entry of a linked collection group had no owner (loft#900, 2026-08-14)


loft#898 gave the CLEAR an owner for a linked group's shared records; removal never got
one, and was wrong in both directions. Through a VIEW it freed the record the primary
still held (the vector kept the entry and its key, the text read back `null`); through the
PRIMARY it never reached the views, which reported their old length over a freed record.
Both backends, and the published 2026.8.0.

**A removal spelled through any member removes it from the group** — the same verdict
loft#898 reached for the clear, and for the same reason: `h.view += [e]` has appended to
every member since loft#843, so an operation spelled through a view acts on the group. The
alternative has no coherent successor state — `h.by_k[1] = null` then
`h.by_k[1] = E{k:1,…}` would remove one index entry and then add to the whole group,
leaving the primary holding two records under one key with nothing able to repair it.

The ORDER is the mechanism. Every unlink reads the record's key out of the record, so the
free must come last and the record must stay reachable until then. The parser emits the
lookup ONCE into a work-ref temporary (marked `inline_ref`, since the record is the
collection's, not the temporary's), then one `OpHashRemove` per other member carrying the
`CLEAR_KEYED_VIEW` bit — the same `0x8000` convention `OpClearKeyed` and `OpSetKeyed`
already use on their `tp`, so arity and both emitters are unchanged — and finally the
ordinary removal on the member the source named, which frees. The temporary is also what
keeps the key expression evaluated once (@PLN102 F2); repeating `OpGetRecord` per member
would have re-run it.

The field site is resolved by walking the `OpGetField` chain (`keyed_field_site` /
`holder_type`) rather than by reading the base variable's type, so a group one level down
resolves too — reading only the base var is what left loft#898's nested case on the unsafe
path until its guard row a7 caught it.

Two supporting facts had to be repaired, both pinned by guard rows:

* `Stores::remove`'s `Parts::Array` arm computed its slot with BY-VALUE arithmetic
  (`(rec.pos - 8) / size`), which is 0 for every element of a record-backed container —
  the loft#719 defect, fixed then for `Ordered` and left for `Array`. The documented
  `vector<T>` + `hash<T[k]>` group has an `array` primary, so every unlink through it went
  to slot 0.
* `remove_owned` sent a grouped hash to `hash::free_entry`, which correctly declines to
  free a record a stride-0 table only borrows. Declining is right only while somebody else
  frees; when the removal is spelled through that member it IS the free, so the record and
  everything it claimed leaked. `Stores::hash_owns_entries` is the table's own answer to
  which case it is.

Matrix: 45 cells × both backends — every (primary, view, spelled-member) triple over the
four member kinds, three-member groups, an absent key, drain-and-refill, first/middle/last
of three, and ungrouped controls per kind.

Two PRE-EXISTING defects the matrix separated out and did not fix, filed with repros:
loft#902 (two `index` members share their red-black links, which live in fields of the
element record — the fill "works" because both fields then describe ONE tree, and the first
removal rebalances it into a panic) and loft#903 (`e#remove` in a loop maintains no
sibling, and over an `array<T>` removes two elements — no group involved).

### A `sorted` emptied by removal published the wrong slot on the next append (2026-08-14)


`sorted_new` hands the constructor a scratch slot and `sorted_finish` / `ordered_finish`
read the new record back out of it — at `length + 1`, except at length 0 where they take
the "first record needs no reordering" path and read slot 0. `sorted_new`'s existing-record
branch always answered `length + 1`, so the two disagreed at length 0.

Only one thing reaches that state: a collection EMPTIED entry-by-entry
(`coll[key] = null`), which keeps its allocation. `coll = []` drops the record, so the
next append takes the fresh-claim branch and lands in slot 0 as expected. The append
therefore wrote into slot 1 while `sorted_finish` published slot 0 — the bytes of the last
element removed, with its text already freed — so `s.a += [E{k:9,…}]` read back as
`2:null` and the new element was simply lost. `ordered_finish` inherits the slot from the
same call, and had the same failure with the rec-id.

Pre-existing on the published 2026.8.0, both backends, `sorted` and `ordered` only —
`hash` and `index` are unaffected. Found by the loft#900 matrix's drain-and-refill cell;
guard row b3 of `tests/scripts/900-linked-group-remove.loft`.

### A linked collection group's second route was silently under-populated (loft#901, 2026-08-14)


Filling one member of a linked group fills every member (loft#843). For three pair shapes
the second route never got the elements, with no diagnostic: `hash` + `index` kept ONE
element however many went in, `sorted` + `sorted` and `vector` + `sorted` stayed empty,
and — not in the filed scope — `hash` + `hash` built the right NUMBER of entries with
every one naming the first record. The filed table counted `len` only, which is exactly
what that last case does not disturb. Both backends and the published 2026.8.0.

**One fact explains all of them.** Every member names its elements by a 4-byte record id:
a hash slot encodes `rec.rec` (`hash::SLOT_RECORD`), an `array` / `ordered` slot stores it
raw and reads it back at a hard-coded payload start, and an `index` keeps its red-black
links in FIELDS of the record. None can express a position INSIDE a record. Two shapes
handed the siblings elements that do not own one:

* a hash **packs its entries into a shared chunk arena** (@PLN135 arc H), so an
  instrumented `record_finish` showed the two elements of `hash` + `index` arriving as
  `rec=(2,15,8)` and `rec=(2,15,32)` — one record, two positions. The index's b-tree links
  then collided in that record and it kept the first; a sibling hash encoded both slots as
  record 15 and read both back as its payload start.
* a `sorted` **stores its elements inline**, so as a view it has no record to name at all:
  `insert_record`'s `Parts::Sorted` arm never receives `rec` and sorts the view's own empty
  buffer.

Both disappear once the group's element type is record-backed. `record_new` already
refuses the arena for an element type flagged `linked`, with a comment describing this
exact failure, and `finish_type` already promotes `vector` → `array` and `sorted` →
`ordered` for one. The flag was only ever **set as a side effect of that promotion**, so a
group whose members are all keyed never set it. `Stores::finish` now seeds it from group
membership directly — a field with a non-empty `other_indexes` — which is the same
predicate `types.rs` used to form the group, so the two cannot drift.

This also removes an action-at-a-distance: whether a `sorted<T[k]>` was record-backed used
to depend on an `index<T[..]>` declared anywhere else in the program (the loft#719 /
loft#891 conversion), so the same source line lowered differently per file. That is what
made loft#898's `vector` + `sorted` matrix cell vacuous rather than correct, and it is why
`tests/scripts/901-linked-group-fill.loft` gives **every row its own element type** —
written over a shared `E` the guard printed `901 ok` on the unfixed published build.

Scope held: a collection that is not in a group is untouched, so a lone hash keeps its
arena (guard row c3 — a fix that made every hash allocate one record per entry would pass
every other row and silently give back @PLN135 arc H's win).

Matrix: 70 cells × both backends, covering all 16 primary/view pairs in isolation, `=` vs
`+=`, the fill spelled through the view, three-member groups, the contaminated-file
confounder, key lookup through the view, element counts 0/1/3, `trie` in both declaration
orders, and a clear after the fill. Gate: 3974/3974 curated + 57/57 on the four excluded
binaries a schema change can reach, fmt + clippy clean.

### A linked collection group had no owner for its records (loft#898, 2026-08-14)


Two or more keyed collections over one element type in one struct are auto-linked into
several routes to a SINGLE record set (`Field.other_indexes`, loft#843). Nothing said which
of them OWNED that set, so `remove_claims` freed the element records through whichever was
cleared and left the others naming freed memory — a length that still read 2 over bytes
answering `4294967296:null`.

The filed scope was wrong in three ways, all measured on a 12-cell matrix against the
published 2026.8.0:

* **`vector<T>` + `hash<T[k]>` is affected and was not in it** — the pairing DATABASE.md
  documents by name, and the one with an unambiguous owner.
* **Both directions are broken, one was filed.** Clearing a VIEW leaves the primary over
  freed records; clearing the PRIMARY never resets the views, which keep their old length
  over the same freed records. A fix for one does nothing for the other.
* **`Parts::Array | Ordered` is not the producer the report named.** `vector` + `sorted`
  is not a counter-example either: it never links at all, so that cell is VACUOUS rather
  than correct, and it is recorded as such rather than counted as coverage.

The ownership fact already existed in the schema and had exactly one reader. `types.rs`
marks every member after the first with a leading `u16::MAX` on `other_indexes`; only the
JSON default-init asked. Three pieces make it load-bearing:

1. `Stores::borrowed_spine` — what a VIEW owns, per kind: the hash table record, the
   `Ordered` slot list, and for `index` nothing at all (a b-tree's nodes ARE the element
   records, so zeroing the root is the whole teardown). It rides the SAME per-`Parts`
   match as `for_each_owned_child` rather than sitting beside it, because the spine a view
   drops is the `container_rec`/`extra_recs` that walk already names — a layout change
   cannot move one and miss the other. `OwnedChild` gained a `borrowed` flag so the
   struct-teardown arm can mark a view field from the schema.
2. A `0x8000` bit on `OpClearKeyed`'s `tp`, the convention `OpSetKeyed`/`OpReplaceKeyed`
   already use, so the op's arity and both emitters are unchanged. Both backends decode it
   in ONE place — `Stores::remove_claims_keyed` — so the interpreter's `#rust` template and
   `codegen_runtime::OpClearKeyed` cannot drift.
3. `Parser::keyed_group_clear`, emitted by the KEYED assign and the VECTOR assign alike:
   the documented `vector<T>` + `hash<T[k]>` shape has the vector as record holder, so a
   fix living only in the keyed branch would have closed half the matrix.
   `clear_group_primary` picks the op the owner's kind needs (`OpClearVector` for a plain
   vector, `OpClearKeyed` otherwise), because a clear may be reached from either member.

**The semantics question the report left open**, and what settled it: a clear spelled
through ANY member empties the group. Not a preference — `h.view += [e]` already appends
to every member (loft#843), so an operation spelled through a view acts on the group, and
`=` must match or `h.view = []` followed by `h.view += [x]` is incoherent. The filed
report asked for view-only emptying, which cannot be made coherent for a NON-EMPTY
literal: the elements still enter the group, so `h.view = [e]` would leave the view
holding `e` and the primary holding `e` plus everything it had. A model that works only
for the empty literal is not a model, and its output — an index silently not indexing its
records — has no repair operation. Rows d1/d2 of the guard pin the `+=` fact the model
rests on, so a future change to it fails here rather than silently invalidating the clear.

The parent struct type comes from `lhs_parent_tp`, which the assign already holds. Reading
it back off the base EXPRESSION only resolved a bare `Value::Var`, so a group one level
down (`o.inner.by_k`) read as "not a group" and kept the unsafe clear — the cell that
caught it is a7 in the guard.

loft#895's exclusion is gone with it: the multi-index field was kept on the append
specifically to avoid this use-after-free, so `=` now replaces on a group like every other
keyed field. `895-keyed-assign-replaces.loft` row c15 pinned that append deliberately and
is updated rather than left to flip silently.

**Not fixed, filed:** removal (`coll[key] = null`) has the same two directions and neither
is right (loft#900) — `Stores::remove_owned` takes no `secondary` flag, unlike the sibling
`dedup_keyed` that already makes exactly this distinction. And a group's view is silently
under-populated for three pair shapes (loft#901), which is why the `vector` + `sorted` cell
above is vacuous.

### A field store had no type check, and two of them corrupted the heap (loft#893, 2026-08-13)


A field store is the one assignment form with no variable to re-type, so
`change_var_type`'s rejection — the one that refuses `v = make()` for a local — never saw
it. The checks that DID cover fields are further down `parse_assign_op`, behind an early
return that a `text` or collection target takes first, so the class went unreported.

Three symptoms, one missing assertion:

* `h.v = make()` on a `vector<float>` field stored nothing and leaked the source store;
* `h.s = 3` on a `text` field carried the integer into `OpSetText` as a text handle and
  took SIGSEGV;
* `h.v += make()` reached the same op pair and panicked writing into the read-only
  `CONST_STORE`.

So the hole was memory safety, not only a dropped write.

Enforced at the point every store form still reaches (`parse_assign_op`, where `s_type`
settles, before any of the early returns) — which is why one check closes all three. The
predicate is `convert`, the same one the constructor path (`handle_field`) and the
scalar-target check already ask, plus one named carve-out: a keyed collection BUILT from a
vector of its elements (`h.m = [E{…}]` for `hash<E[k]>`) is the supported idiom, is
deliberately not `is_equal`, and has no `convert` arm.

`convert` is a `&mut self` emitter, so it is asked in the shape-only form it already
understands — a `Value::Null` expression, which every rewriting arm guards against and no
verdict depends on — and `conv_owned_result` is saved and restored around the call, since
a cast arm sets it to mark an allocating conversion and the next real conversion `take()`s
it. A probe that left it set would hand its answer to an unrelated expression. Adding the
diagnostic therefore cannot move codegen.

**Method note.** The predicate was run as a silent probe over all 2188 `.loft` files in
the tree before it was allowed to speak. Exactly one file hit, and it was a true positive:
`tests/docs/13-file.loft` read a sized `f#read` straight into a `vector<single>` field,
which LOFT.md's conversion table documents as needing an explicit `as`. It had been
storing an EMPTY field, and the example asserted nothing about the result, so nothing
caught it — the doc stated a rule the code never ran. Fixed to the documented spelling
and given the two assertions that would have caught it.

Known and NOT fixed here: the documented `as vector<single>` cure leaks its store when
consumed directly by a field store or call argument (loft#897), so the doc example binds
it to a local first.

### A write through a returned struct is now reported (loft#894, 2026-08-13)


`hurt(first(s), 10.0)` writes into a temporary discarded one instruction later, while
`hurt(s.es[0] ?? E {}, 10.0)` writes through — same types, no diagnostic on either. This
is the shape `lost-write` exists to catch and it was silent, so the analysis now covers
its second shape. Semantics unchanged.

Two facts must meet, and requiring both is what separates a LOST write from a merely
pointless one:

* the callee WRITES THROUGH the parameter — read off its own body with
  `find_field_written_vars`, the same walk `check_ref_mutations` uses to decide whether a
  `&` parameter was really mutated, so the two cannot disagree about what such a write is;
* the argument COPIES A PLACE THE CALLER CAN STILL REACH — read off the return type's
  deps, since `first(s)` returns `E["s"]` while a value built from nothing is dep-free.

The second condition is the one that matters. Without it the lint fires on
`hurt(fresh(), …)`, where a write into a freshly built value loses nothing that existed
before the call, and on the write-then-return builder idiom, where the write is delivered
through the return value. A dep is believed only when it names a parameter the call site
filled with a REAL variable: a function building into a caller-supplied return buffer
carries a dep too (`alloc_canvas(w, h, fill)` returns `Canvas["cv"]`), and that copy is
nobody's data — the `_`-prefix test tells them apart, the same convention
`warn_dead_stores` uses.

Both exclusions were found by sweeping all 2188 `.loft` files with the lint as a probe
before letting it speak: the first cut had two hits, both the builder idiom, and the final
one has zero. Runs from `main` beside `warn_dead_stores` / `warn_double_move`, reusing the
`lost-write` code rather than minting one (same fact about the same C86 copy);
`LOFT_NO_LOST_TEMP_WRITE` opts out.

Deliberately an under-approximation, per the two-tier rule: binding the result to a local
first stays silent, because that copy is still readable and belongs to `warn_copies`.

### `=` to a keyed collection appended, because only the EMPTY literal cleared (loft#895, 2026-08-13)


A collection literal lowers to element-construction ops that APPEND, so the assignment has
to put a clear in front of them. `parse_assign_op`'s vector-field arm does. The keyed arm
did it only for `Value::Insert(ls) if ls.is_empty()` — `s.h = []`, the @P307 clear — and
said so in place: *"Non-empty / non-literal keyed-field reassignment is a separate (harder)
case left to its current path."* So `s.h = [a, b]` added to what the field held, and `=`
meant `+=`.

The filed scope was a struct with two keyed fields, where the second assignment read length
4 for two elements. The matrix says that is not the boundary. Assignment ORDER is
irrelevant — the row filed as correct fails too, 4 the other way round — and a SINGLE keyed
field assigned twice is equally wrong, as is a keyed LOCAL, which has no struct at all. The
pair is just the loudest witness, because `Field.other_indexes` makes two keyed fields over
one element type two views of one record set (loft#843), so filling either fills both.

Two arms now carry the clear: the field one prefixes any literal with `OpClearKeyed`, and
the local one prefixes `Set(v, Null)` — the lowering `s = []` already takes (P193
`create_keyed`), which codegen turns into the `OpDatabase` store reset, and which also
gives the slot its init when a literal is the local's first assignment.

A MULTI-INDEXED field is excluded and keeps the append (loft#898). `OpClearKeyed` →
`remove_claims` frees the element RECORDS, not just this route to them: `Parts::Array |
Ordered` hands every slot back with `owning_elem: Some(elm)` unconditionally, and a
borrowing `Parts::Hash` does the same whenever `owns_entries` is false. So both members of
a group free the shared records and whichever is cleared first takes the other's elements
down with it — `h.ordered = []` leaves `h.keyed` reporting length 2 over freed memory. That
is a use-after-free, and emitting the clear there would trade #895's wrong length for it.
`allocation.rs:2921` already carried the marker: `// TODO prevent removing records twice via
secondary structures`. The exclusion reuses `keyed_field_is_linked`, the same predicate
@P305 uses to route `coll[key] = value` away from the group for the same reason.

The empty literal keeps its unconditional clear either way — making that one conditional
would restore the silent no-op @P307 fixed, so the change is strictly additive.

### A field-store RHS temp was typed as the destination, so it never owned anything (loft#897, 2026-08-13)


`s.v = <expr>` lowers to `Set(tmp, expr); Clear(s.v); Append(s.v, tmp)`. `tmp` was built
with `f_type` — the destination FIELD's type, deps included. scopes.rs frees a var only
when its deps are EMPTY (*"`dep` empty → the variable owns the value → emit `OpFreeRef`"*),
so a temp carrying the field's dep read as a borrow of the struct and no free was ever
emitted. Any allocating RHS then leaked for the life of the program.

Nothing about the `as` cast was involved, which is what the filed scope named. A local was
clean only because a user local carries no such dep — `LOFT_VAR_TABLE` shows both temps
marked `OWNS`, and the difference is entirely whether something BINDS the value. The
borrowed-Var arm two branches up already builds its temp from a dep-free
`Type::Vector(elm, Deps::none())` for the #320 aliasing reason; this is that same choice on
the general arm, which is the one an allocating RHS reaches.

The other half of the filed scope — the same expression consumed with NO binding — is not
an ownership question and is loft#899. `#reading file`'s temp DECLARATION is lifted into an
expression slot there (`{ !! INSERT _read_1(5):vector<single> = null … }`), which
`--interpret` evaluates against the wrong header (`len` answers 1; `for e in` yields the
second element alone) and `--native` emits as a `let` inside an expression, so rustc
rejects it. The emitted `OpReadFile(…, db_tp=78)` is byte-identical between the working and
broken programs, so the read op is not what differs. It is also order-sensitive: an
unrelated `vector<single>` local elsewhere in the file flips the answer, which is what a
type-registration side effect looks like. Fixing the leak on a path whose value is wrong
would have been polishing, so this fix stops at the field store.

### An unbound `f#read(n) as vector<T>` failed three ways, from two causes (loft#899, 2026-08-13)


The order-sensitivity was the tell, and it named the first cause. `gen_set_first_vector_null`
resolves its store type by NAME — `data.name_type("main_vector<single>")` — and the read's
temp is the one vector local that never reaches an assignment, so nothing registered the
wrapper: every other vector local gets it from `Parser::change_var_type`, and the
`typedef.rs` sweep that catches the remaining producers reads struct and enum-value FIELDS
only. The lookup returned `u16::MAX`, and the emitted `OpDatabase(var, db_tp=65535)` created
the store with no type at all. Wrong header width, so `len` answered 1 and the data started
one element in — and any OTHER `vector<single>` in the file registered the wrapper as a side
effect and made the same read correct, which is why a line elsewhere changed the answer.
`objects.rs` now calls `data.vector_def` for a vector read type, the same call its
`OpCastVectorFromText` sibling makes 800 lines down.

The `debug_assert_ne!` guarding exactly this `u16::MAX` sat one line below the lookup and
has never run: `[profile.dev.package.loft] debug-assertions = false` strips it from the
library in both profiles. An env-gated `eprintln` in its place, swept over all 2190 corpus
`.loft` files, found this temp to be the ONLY producer — and found no corpus file that
covers it, which is how it shipped.

The other two failures are one mechanism. The `Value::Block` arm returns a value block that
yields an owned temp as `Insert([Set(v, Null), block])`, and `scan_args` hoists that `Set`
into the enclosing statement list (`is_a56_hoisted`). But `scan`'s `Value::Span` arm rewraps
the scanned argument, and its unwrap predicate recognised only the `Set(__lift_N, …)`
preamble — so a span-wrapped null-init preamble never reached the `if let Value::Insert`
that would hoist it, and the declaration stayed inside the argument expression. Native
emitted it there literally: `expected expression, found let statement`, plus an E0425 for
the `var__read_1` that no longer scoped. That arm's own comment already gave the reason the
lift shape is unwrapped — *"the native backend would emit `Set(__lift_N, …)` inside an
enclosing expression and fail to compile"* — for a sibling shape it did not cover. The two
sites now share one predicate, `is_null_init_preamble`, so they cannot drift again.

Hoisting alone left the store unfreed, because the hoist MOVES the owner: the declaration
now stands in the enclosing statement list, and an argument is only read, never adopted the
way `v = <block>` adopts. `scan_args` re-registers the temp at the current scope for
`get_free_vars` and runs `mark_lift_handoff` on it, so an argument the callee MOVES from
(`OpCopyRecord` with the `0x8000` flag) still does not drop twice. `return f#read(…)` is the
other side of that and must NOT be freed; it transfers, and the guard's c8 row pins it.

Element type is an axis here, not a detail. `main_vector<integer>` is registered by the
stdlib whatever the program does, so an integer-element probe sees the leak and the native
failure but never the wrong value. The same masking bites the regression guard itself: any
control row that binds the read to a local registers the wrapper and disarms the very cell
it pins, so the `vector<single>` case needs a file that declares no other vector at all
(`899-unbound-file-read-only-vector.loft`, deliberately minimal for that reason) while the
main guard carries the remaining seven shapes plus a `vector<P>` row.

### The last local-gate flake: a well-known port on a shared machine (2026-08-13)


`engine_host_udp::probe_server_poses_ride_the_fastest_path_per_client` connected to a
hardcoded **18084**, because the fixture it drives —
`tools/audience-demo-50/probe_server_kernel.loft` — binds that constant. Its own comment
said so, and named the fix: *"this one test can still collide with a concurrent
sibling-checkout run; fixing that needs a port-arg on the fixture."*

On this machine 18084 was held by **five** long-lived processes from other checkouts
(`planet_server-a`, `planet_server-e`, `loft_native_bin`), so the test failed for someone
else's run — every run, not intermittently.

The fixture now honours `LOFT_PROBE_PORT`, defaulting to `PORT` when unset
(`env_variable` answers `""`), so the documented demo invocation is byte-identical. The
test passes a port from a new `free_port()` helper.

`free_port()` checks **TCP and UDP**: the kernel listens on both for one number, and the
OS picks a TCP port knowing nothing about the UDP table — a TCP-only probe would hand
back a port whose UDP half is taken, and the fast-lane assertions would then fail for a
reason unrelated to the code under test. `SO_REUSEADDR` is deliberately not set, since a
port that only looks free because of address reuse is not free.

Candidates come from **20000–29999 keyed on the pid**, not from `bind(":0")`. The first
attempt did use `bind(":0")` and a full-suite run then failed
`engine_host_placed::the_engine_host_serves_the_same_client_from_either_placement`, which
passes 6/6 in isolation on both this tree and the preceding commit. `bind(":0")` draws
from the OS ephemeral range (32768–60999 on Linux) — the same pool every other test's
port probe draws from, including that one's TCP-only `free_port` — so it traded a
collision with a *well-known* port for a collision with a *sibling test*, which is harder
to recognise when it bites. A pid-keyed number in a quiet range separates concurrent
checkouts and stays out of that pool.

`engine_host_placed` still has its own TCP-only probe. It is latently exposed to the same
UDP half-taken hazard; left alone here because nothing has been measured failing on it
once the ephemeral contention is removed, and a speculative rewrite of a second test's
networking is churn.

Verified by binding: with `LOFT_PROBE_PORT=19731` the fixture listens on 19731 for both
UDP and TCP. The test's own timing is corroboration — it now connects to the port
`free_port()` chose, so a fixture that ignored the variable would sit on 18084 and
`ws_connect` would spin to its 15 s deadline; it completes in ~0.12 s instead.

That closes the third and last of the session's flakes, all one shape — **a fixed-name
shared resource plus parallelism**: a process-global overwritten per compile, one temp
path for four callers, and a well-known port. Local gate: **4079 passed, 0 failed**.

### A test helper's temp file was keyed on the pid, so its four callers shared one path (2026-08-13)


`tests/introspect.rs::resolution` wrote its program to
`temp_dir()/loft_res_<pid>.loft`, ran `loft introspect` on it, and deleted it. The pid
is the same for every test in the binary, so all **four** call sites shared one path —
and `why_reports_where_a_name_is_defined_and_reachable_from` alone calls it twice. On 8
threads one call's `remove_file` landed while another's subprocess was still opening the
file; the subprocess printed nothing and `section()` panicked with ``no `=== resolution
===` in:``.

Measured at 4/6 failing runs of the binary, and 3/6 on the preceding commit — pre-existing,
and the second of the two flakes behind the local gate's "4077 passed, 2 failed". It passes
100 % in isolation, because the race needs a second caller in flight.

Fixed by making the path per-CALL (an `AtomicUsize` counter alongside the pid) rather than
per-process; the pid still separates concurrent `cargo test` invocations. 8/8 clean after.

The wider pattern is worth knowing when writing a test helper here: **a fixed-name shared
resource plus parallel tests**, the same family as the hardcoded ports in
`engine_host_udp.rs` (18084) and `multiplayer` (18099). 70 test files call
`std::process::id()`; most already add a per-test discriminator (`{name}`, `{tag}`,
`{port}`), and a pid-only name is only safe where exactly one caller exists.

### The `#native` stub set was a process-global that every compile overwrote (2026-08-13)


`compile::byte_code` recorded which `#native` symbols it registered a panic stub for —
the set `wire_native_fns` consults to know which stubs it may replace with an
auto-marshalled wrapper — by *overwriting* a `static STUB_SYMBOLS`:

```rust
pub fn set_stub_symbols(syms: HashSet<String>) {
    *STUB_SYMBOLS.lock()… = Some(syms);   // wholesale, on every compile
}
```

In any process that compiles more than one program — a test binary, the REPL loading a
second file, an embedder — a sibling compile landing between one program's compile and
its wiring replaced the set. `wire_native_fns` then hit `!stubs.contains(sym) → continue`
in **both** phases, skipped resolution for its own symbols, and left the panic stub in
place. The failure surfaces much later, at the first call, as *"native function not
loaded: its library's native cdylib is missing or stale"* — a message that sends the
reader after a build problem that does not exist. Diagnosing this one burned time on
exactly that: rebuilding cdylibs, and checking `nm -D` for undefined `libloft` symbols
to rule out staleness (there are none — the fixture links only `loft-ffi`).

**Fix: the set lives on `State`.** It describes the program that was just compiled, and
`register_native_stubs` already had the `State` in hand (`state.static_fn(sym, stub)` on
the line above). `STUB_SYMBOLS` and `set_stub_symbols` are deleted rather than kept
beside the new field, so there is one home for the fact. A lock around the global would
have been the wrong fix — it serialises the writes and still lets the last writer win.

Cost measured before the fix, by worktree A/B against the preceding commit, alternating
single RUNS with both arms pre-built (binaries *and* test binaries) so no build sat
between them:

| | `repl_session` not-pass |
|---|---|
| A — preceding commit | **5 / 10** |
| B — same tree + the vector-leak and panic-hook commits | **5 / 10** |

Identical rate, identical failure mode: `file_debugger_can_call_into_a_native_library`
fails about half of all full-binary runs and passes 100 % in isolation, because it needs
~52 sibling compiles in the process to lose the race. That is almost certainly the single
failure behind earlier "3972/3973 curated" gate reports.

`tests/native_loader.rs` already carried a `TEST_LOCK` whose comment named
`STUB_SYMBOLS` as shared global state — a workaround that could only ever cover tests in
that one file, and `repl_session` is a different binary.

Guard: `native_loader.rs::a_sibling_compile_does_not_take_over_this_program_s_stub_set`
reproduces the interleaving **deterministically** — compile B, compile a sibling
declaring a *different* `#native` symbol, then wire and run B — so it fails outright on
the old code instead of depending on thread scheduling.

### A buffer-bound vector fn delivered only its TAIL when the tail borrowed an argument (2026-08-13)


`dispatch_vector_delivery` is the one place that decides how a vector-returning function's
result reaches the caller's `__retbuf`. `Delivery::Rename` routes through `ref_return`, which
delivers the tail AND rewrites every mid-body `return <fresh local>` into the buffer.
`Delivery::CopyBorrow` routes through `copy_borrow_tail_into_retbuf` — a **tail-only** funnel,
by design and by its doc comment. So a function with an early `return <fresh local>` and a tail
that borrows an argument delivered the tail and left the early return handing back a store of
its own: the caller adopts the buffer, the fresh store orphans. One leaked store per
undelivered return, every value correct.

The invariant, now asserted in both arms: **a buffer-bound vector fn delivers EVERY return site
into the buffer, not only its tail.** The `CopyBorrow` arm calls `deliver_mid_vector_returns`
before the tail copy; `copy_borrow_tail_into_retbuf` keeps its narrower tail-only contract. The
walk is idempotent by construction (it rewrites `Return(Var(v))` only for `v != buf_var`, and
its own rewrite yields `Return(Var(buf_var))`), so the existing fallback — which delivers again
via `ref_return` when the work-var allocation fails — cannot double-deliver.

Boundary, mapped on a 14-cell matrix before the fix (values hand-computed, each cell asserting
value + length + leak, both backends):

| tail shape | mid-body payload | pre-fix |
|---|---|---|
| borrows a whole vector argument | fresh local / inline literal | **leak** |
| borrows a struct FIELD of an argument (#415) | fresh local | **leak** |
| a call / a fresh local / the same var | fresh local | clean |
| branch arms (`Delivery::Materialize`) | fresh local | clean |
| borrows an argument | a param / a call result | clean |

Both leaking rows are the two sub-shapes the funnel's own comment says route to it, so the
boundary is the `CopyBorrow` arm exactly. The leak count scales with the number of undelivered
returns — a two-early-return function leaked ×2 — which is what pinned the mechanism rather
than merely correlating with it. `Delivery::ForwardCopy` needs a `#native` heap-returning
callee and is **not** covered by the matrix; it shares the tail-only shape and is the place to
look if this recurs.

Guard: `tests/scripts/midbody-return-into-borrow-tail-retbuf.loft` — both sub-shapes, a
two-early-return function (guards the COUNT), a loop-nested return, and a caller loop proving
the delivery clears before filling. Proven to fail on the released binary: ×9 leaked stores
while still printing `ok`, so only `wrap.rs`'s exit-time gate catches it.

Found while probing whether `OpReplaceVector`'s absence from `find_written_vars` /
`find_field_written_vars` could give a wrong answer. It cannot today — all 9 of its occurrences
across the stdlib and the dump corpus are masked by an `OpClearVector` or a `Value::Set` on the
same target — but the masking is incidental, so the op is now listed in both walkers as
hardening. That is a no-op on current behaviour, kept because the two lists are hand-maintained
twins and nothing compares them (PERFORMANCE.md § Design: P8).

### Two nightly gates that measured the environment, not the diff (loft#888, 2026-08-13)


**The leak gate went red on our own fix.** loft#876 gave a field's declared default a home on
the schema `Field`, and a TEXT default has to intern its spelling: `Content::Str` is a raw
`{ptr, len}` with no owning variant, so `fold_declared_default` reaches for the same
intentional `Box::leak` that `ir_read` / `ir_schema` / `snapshot` already use for the same
type. That leak is bounded by the SOURCE — one allocation per field that declares a text
default, decided once at type registration, never one per read — so it belongs in
`.github/lsan_suppressions.txt` beside its three siblings.

What kept it out was purely a symbolization detail: a suppression matches by FRAME NAME, and
the function inlined into `typedef::fill_database`, so the only name on the stack was that
one. Suppressing `fill_database` would have blinded the gate to every allocation in the whole
type-registration path — a real loss, since that is where schema construction allocates. So
`fold_declared_default` now carries `#[inline(never)]` FOR the suppression, and the two must
be kept in step: drop the attribute and the suppression silently stops matching.

Both halves are measured rather than argued. Without the suppression the frame is now named
(`#2 loft::typedef::fold_declared_default`, `#3 fill_database`); with it, the run is clean and
LSan reports the template it used. The per-file scan over the whole corpus is **0 leaking files
of 721**. And the gate is still live where it matters: a deliberate `Box::leak` injected into
`fill_database` itself is still reported, with the suppression file active, owner
`loft::typedef::fill_database` — so the new line suppresses exactly one deliberate interner
and nothing else.

**The toolchain matrix failed before running a loft op.**
`a_private_scope_end_hook_in_a_library_runs` spawns loft to BUILD a library cdylib, which links
`libloft.rlib`. The `Suite under <toolchain>` job only ever runs `cargo test`, which builds the
lib into `deps/` for the test binaries and never produces the rlib
`native_lib::find_loft_rlib` looks for — so the spawned build died on "libloft.rlib not found
for this build". That is an environment result, and this matrix exists to detect toolchain
drift in loft's own code.

The obvious repair does not work, which is why it is recorded here rather than tried again:
adding `cargo build --release --lib` clears "not found" and then fails `E0463: can't find crate
for libloading`, because `cargo test` and `cargo build --lib` unify features differently, so
the uplifted rlib's dependency set is not the one sitting in `deps/`. Both cells were run in an
isolated target dir; cell A reproduces the CI message verbatim. The test is therefore skipped
in that job, which is the exclusion the asan and asan-leak jobs already carry for it and for
the same reason (loft#855). Its sibling `a_delegating_producer_binds_its_companion_cleanly`
passes there and is deliberately NOT skipped.

The third leg needed no change: the `LOFT_POISON` gate was red on
`877-index-a-call-result-in-return-position.loft`, which is loft#882 / loft#889 / loft#890, and
the poison sweep now runs **1870/1870** on this branch.

### Two stores freed at the wrong time (loft#889, loft#890, 2026-08-13)


**loft#890 — a lift freed what its consuming op had already released.** `br = mk_hash(n)`
lowers to `__lift_1 = mk_hash(n); OpReplaceKeyed(__lift_1, br, tp | 0x8000)`. The bit means
"nobody else owns this store", which is true of a bare call result and false the moment
`scan_args` lifts it into a temp the scope sweep frees. `free_named` is a no-op only while
the slot is still free, so the second free steals whatever store the allocator handed that
slot in between — and the record return allocates its buffer in exactly that window, which
is why the filed shape needed a call, a keyed container AND a record return together. With
an integer return nothing is allocated there and the double free is invisible. The
interpreter was right for no better reason than not reusing the slot.

`scan_args` already carried the lift-site half of this rule for `OpCopyRecord` (@PLN139
stage C), but only for the DROP — the store was "left to the ordinary sweep, which is
null-tolerant either way", which is the part that is not true. `Scopes::mark_lift_handoff`
now records the FREE hand-off too, and `get_free_vars` consults it. Only `OpReplaceKeyed`
answers `moved_source_arg`: it is the one `0x8000` op whose source is a whole store a lift
can own. Answering for `OpCopyRecord` took the free away from @PLN85's Join-return
machinery — 3 of 54 fuzz cells SIGSEGV'd and `elem_accumulate` doubled its own source
vector. The marker is a `Scopes` set rather than a `skip_free` stamp for the same class of
reason: that flag is read at ALLOCATION time too, so stamping it made the lift borrow
instead of own.

**loft#889 — a collection reached through a field of a call's result.** `mk_bag(n).b_vec[0]`
reads an element living in the bag's store, and the bag is an inline call result with no
name, so the element typed as OWNED and nothing copied it out before `OpFreeRef`.
`keyed_container_dep` (loft#882) is now `container_dep` and reaches THROUGH field
projections via `projection_root_mut` to bind the ROOT call — the bag, not the `b_vec`
projection, because that is whose store the element is in. `parse_index`'s VECTOR arm asks
too; it had relied on the container type's own deps, which a fresh call has none of.

The SUBSCRIPT asks, not the field read. `return make().rows` returns the field itself,
which the delivery machinery already copies out (loft#877 / zt12), so binding a container
there only adds a holder nothing releases — five of them in that one file.

The binding now happens on BOTH parser passes. That is load-bearing: this dep is what tells
`ref_return` the binding borrows, and a verdict that differs between passes is worse than
none. Skipping pass 1 read the binding as owned and renamed it ONTO the return buffer; pass
2 then saw the view and materialised into the buffer the binding now WAS, so
`materialize_return_into` emitted `OpDatabase(e); OpCopyRecord(e, e)` — a copy from the
record it had just re-minted. `e = mk_hash(n)[k] ?? d; e` answered an empty record for that
reason before this issue existed, so loft#882's own shape had the hole one bind away.

`return_projects_into_local` gained `projection_base`, which peels the binding block to its
var: a base that is neither a var nor a call read as "rooted at nothing", and the field was
delivered as if it owned what it points at.

Guarded by `tests/store_lifetime_890_889.rs` over `tests/scripts/{889,890}-*.loft`: value,
`LOFT_POISON=1` and `LOFT_NATIVE_LEAK_CHECK` on both backends, plus a harness control. The
poison oracle because freed bytes are usually still intact; the leak oracle because "never
free it" ends both use-after-frees while passing every value cell. loft#888's nightly
poison gate was red on `877-index-a-call-result-in-return-position.loft::i877_field_of_call`
— loft#889's cell, recorded there and invisible because the suite does not run that file
under poison — and is green with this.

`723-ncc-loop-element-bind.loft`'s leak check now measures round-over-round inside ONE
frame: a container work-ref is one slot per SITE for the life of its frame, so two snapshots
taken at different sites differ by that constant and say nothing about the round.

### A sorted collection dropped every insert once an index existed elsewhere (2026-08-13)


`s[k] = v` on a `sorted<T[k]>` local inserted nothing — `len(s)` answered 0 and every lookup
its fallback — whenever ANY struct in the program declared an `index<T[…]>` field over the
same element type. The struct is never constructed; declaring it is the whole input. Both
backends.

A `sorted<T[k]>` becomes an `ORDERED<T[k]>` — the by-reference twin — under exactly that
condition, so the same source line lowers differently because of a declaration somewhere
else entirely. `towards_set`'s insert arm listed Hash, Sorted, Index, Radix and Trie, so an
`Ordered` collection fell past `OpSetKeyed` to the update-only `OpCopyRecord`, which copies
into the lookup's result and therefore no-ops when the key is absent.

This is loft#719's omission one function over: that issue gave `Ordered` to the REMOVAL arm
(`towards_set_hash_remove`, directly above), where its absence had made `coll[key] = null` a
silent no-op interpreted and a compile failure natively. Nothing compares the two lists.
`Stores::set_keyed` has always handled `Ordered`; only the routing to it was missing.

Found while building loft#889's boundary matrix: a five-collection bag promoted its own
`sorted` field, and that one cell answered the fallback on both backends while every
neighbouring kind was right — a lopsided matrix that is evidence of a missing arm rather
than of the bug under investigation. Guarded by
`tests/scripts/891-sorted-promoted-to-ordered.loft`, which fails at its first assertion on
the previous commit.

### A keyed element read never said it borrows its container (loft#882, 2026-08-13)


`v[i]` on a vector types its result with a dep naming the container, and that dep is the
whole reason the vector shape is safe: `return_views_local` sees a borrow from a local and
`materialize_view_return` copies the element into the return buffer before the container is
freed. Every keyed read — hash, index, sorted, trie, any key arity — carried none, so
`return make_hash()[k]` handed back a pointer into a store the same function freed on the
way out.

`parse_index` propagates the container TYPE's deps (`for on in t.depend()`), and a freshly
built collection has none to propagate. `Parser::container_dep`
(`src/parser/fields.rs`, then named `keyed_container_dep`) now names the container at the one place keyed element reads are
typed: a local, parameter or field is depended on directly; an inline call that MINTS a
container is bound to a pass-2 work-ref first, because `scopes.rs` lifts it into a
`__lift_N` long after the materialisation decision has been made. A parameter's dep
resolves to a function attribute, so the element correctly stays a borrow.

The two backends disagreed, which is why it survived: an EMPTY dep list reads as OWNED by
`--native`'s assignment lowering, so it inserted a defensive `OpCopyRecord` and the program
was right, while the interpreter aliased and read freed bytes. Under `LOFT_POISON=1` the
boundary matrix scored `--interpret` 6/17 and `--native` 14/17; both are 16/17 now.

The filed cause (`parse_key`'s no-prelude branch) was not the boundary: the prelude branch
attaches `dep.clone()` — the container type's deps, which are empty — so it named the
container no more than the other branch did, and BOTH spellings were broken. The two cells
still red are older and separate: loft#889 (a collection reached through a field of a call's
result) and loft#890 (a bound keyed container on `--native` when the function returns a
record — the workaround the issue was filed with).

Guarded by `tests/keyed_element_borrow.rs`, which runs under `LOFT_POISON=1` on both
backends plus a static oracle (the container must be NAMED and the return MATERIALISED), a
leak check and a harness control. It needs its own binary because freed bytes are usually
still intact — the ordinary suite was green over this.

### A registered native with no bridge was only found by calling it (loft#886, 2026-08-13)


A cdylib can export a `#native` symbol and register no marshal bridge for it. The symbol
resolves, wiring succeeds, and `native_auto_dispatch` panics — but only when something
calls it, so a library can ship, pass its own suite, and carry a function that is dead for
every consumer exercising a path its tests do not.

`wire_native_fns` now collects those symbols and reports them at load
(`report_bridgeless_natives`), separately from `report_unresolved_natives` because the fix
differs: the library is not stale, its registration is incomplete. The message names the
library and each dead function and points at
`loft_ffi_build::generate_register_from_loft_with_bridges`, which derives both the register
list and the bridge list from the `#native` annotations and cannot drift — a hand-written
`loft_register_bridges!` lives in a different file from the declarations and nothing
compares the two.

The issue's stated cause — a non-`pub` `#native` taking a vector gets no bridge — does not
reproduce: a 9-cell package varying visibility against parameter kind, call site and symbol
binding is correct in every cell on both backends, and `parse_register_symbols_from_loft`
strips an optional `pub ` and never looks at it again.

### A repeat literal walked off the store on a negative count, and lost its text (2026-08-13)


Two further defects in `[x; n]`, found while reading the bulk-fill path before routing a
constant comprehension into it (loft#884).

A NEGATIVE count cast `as u32`, so `[7; -1]` became 4 294 967 295 `copy_block`s that walked
off the store until glibc aborted — the same failure `n == 0` had. A count is a TOTAL and a
negative total is no vector at all, so it now answers empty. `--native` already clamped with
`count.max(0)` and the interpreter did not: a heap-corrupting input on which the twins
disagreed.

The claim copy took the VECTOR HANDLE as its source instead of the template element, so a
`text` element re-interned whatever the handle's four bytes decoded to: `["abc"; 4]` gave
"abc" at index 0 and junk at 1, 2 and 3. Structs and nested vectors carry claims too and
were wrong the same way. Length and element 0 were both correct, which is what made it
invisible.

The twins are also back in step on the record re-read: growing the vector can move its
backing record and both ends of the copy live inside it — `--native` re-read it for the
destination only, the interpreter not at all.

### `[x; n]` built n+1 elements, and n=0 corrupted the heap (2026-08-12)


`OpAppendCopy` receives the TOTAL a repeat literal asks for, and the template element is
already appended by the time it runs — so it needs `n - 1` more. It added `n`:
`vector_set_size(&data, multiply, size)` grew the vector one past the request while the
copy loop wrote only `multiply - 1` slots, leaving the last one never initialised. `[7; 3]`
read back as **length 4 with garbage in the last element** — a wrong length and an
uninitialised read, silently, on both backends.

`n == 0` is the same off-by-one taken to its end: `for i in 0..(multiply - 1)` on a `u32`
wrapped to 4 294 967 295 and walked `copy_block` off the end of the store until glibc
aborted the process (`Fatal glibc error: malloc.c:2599 (sysmalloc): assertion failed`).
The template also has to be dropped, or a zero-length request answers length 1.

The op's contract is now "the vector ends with exactly `count` copies of its last element",
which is what the literal means, and it is total: 0 removes the template, 1 is already the
answer, n adds `n - 1`. Fixed in BOTH twins — `State::append_copy` (`src/state/io.rs`) and
`codegen_runtime::OpAppendCopy` — which carry separate copies of the loop.

Both halves reproduce on the published `2026.8.0`. Found while measuring loft#884: the
repeat literal is the bulk-fill path a constant comprehension would be lowered into, so it
was read before being built on. Guarded in `tests/scripts/886-repeat-literal-count.loft`,
including a RUNTIME count and a runtime zero — the rows no const-fold can reach — and a
`float` element so a stride error shows as a wrong sum rather than a wrong length.

### A declared field default now reaches a cast, when it is a constant (2026-08-12)


`height: float = 1.5` was honoured by a struct literal and ignored by `text as Struct`,
which wrote the type's zero — the same field with two absent values depending on how the
record was made. Invisible before loft#870, because the cast answered `null` for all
three cases and the value was wrong in a louder way.

The default lives parser-side as a `Value` IR node and the JSON walker sits below the
parser with no evaluator, which is what made this `needs-design`. Of the three possible
answers, the one taken is folding a CONSTANT default into the schema `Field`
(`typedef::fold_declared_default`) — it needs no evaluator, and it comes with a contract
to state rather than a hole:

* a LITERAL default (`= 1.5`, `= 7`, `= "hi"`, `= true`) is part of the type: it answers
  a missing key, an explicit `null`, and a struct literal alike;
* any other default is computed, is already lowered parser-side into a function the
  CONSTRUCTION site calls, and keeps exactly its previous reach — the constructor, not a
  cast. Documented in LOFT.md § struct fields, and pinned by a probe cell rather than
  left implicit.

Deposited at the one parse-time site that knows (`typedef::fill_database`), beside the
`nullable` deposit and for the same reason. Three details carry the weight:

* **The value goes in through `walk_parsed_into`** — the same writer the cast uses for a
  key the document DID carry — so a default lands exactly as if the JSON had spelled it,
  and every field encoding (ranged `u8`/`u16`, text interning, the `Parts` dispatch) is
  handled in one place instead of restated. A literal that does not fit its field writes
  nothing and the previous absent value stands.
* **It is written only for `Absent::Final`.** A `Prefill` is overwritten by whatever
  follows, so honouring a default there would pay back the per-record cost loft#875 split
  the enum apart to avoid.
* **It is carried, never RENDERED.** A default changes no width and no offset, so
  `layout_dump` must not see it — otherwise `height: float = 1.5` becomes a different
  layout from `height: float` and adding a default would refuse an existing store. The
  dump's default branch is removed; it never fired, because every field held the `Str("")`
  placeholder. Same call `nullable` made (@PLN127 arc D).

`Field::default` becomes `Option<Content>` (it was `Content`, set to `Str("")` at every
site and read by nothing). The snapshot and IR-store formats are unchanged: `None` is
written as `Str("")` and read back as `None`, so an existing schema round-trips
byte-identically. `--native` needed its own half — the generated `init()` replays the
schema, so `emit_field` now emits `set_field_default`, folded by the same function, which
is why the two backends cannot disagree about which defaults are constant.

Matrix: 9 cells on both backends, 7 failing on the published `2026.8.0`; the negative
controls (a present key beating the default, a field with no default keeping loft#870's
answers, a literal override) pass on both. Guarded in
`tests/scripts/876-declared-field-default-in-cast.loft`.

### An optional return was a shape the lift never recognised (2026-08-12)


`inline_struct_return` (`src/scopes.rs`) is the one predicate that answers "does this
call hand back a store the caller must own?", and every arm matched the callee's
return type UNPEELED. `Optional(τ)` is a compile-time wrapper over τ's own runtime
layout (@PLN25), so `-> C?` allocates and delivers exactly what `-> C` does — but it
read as "not liftable", and the result got a bare stack-pop (`FreeStack`) instead of a
`__lift_N` temp with a scope-exit `OpFreeRef`. One leaked record per call, unbounded in
a loop, interpreter-only (native frees through its own drop path).

Filed as loft#879, a `??` bug. The `??` is incidental: a discarded `pick(1);` leaks the
same store with no `??` anywhere, `takeopt(pick(1))` leaks it as an argument, and an
optional VECTOR return leaks too. The deciding axis is the optional aggregate return
whose result stays a temporary — not the spelling that produced it.

The `??` half is a second arm. A null-coalesce lowers to an `ncc` value-block that
assigns the subject to a `__ncc_N` temp and yields either that temp or the default arm's
`__ref_N`. The temp is `skip_free` — the block's result ALIASES it, so freeing at the
block would dangle the value the consumer reads — which leaves the subject owned by
nothing when the block is used inline. Text ncc temps were already covered by the
@PLN85 skip_free-orphan pass and vectors by their own delivery path; only the
`Reference` result leaked, and only that arm was added.

Both halves emit what the hand-correct bound form has always emitted: `x = pick(1)`
binds an `optional(reference(C))` local and frees it at scope exit. The lift rewrites
the inline spelling into that bound form, so the fix adds no new delivery path — which
is also the soundness argument for the borrowed case (`fn kid(h) -> Cell? { h.child }`),
since binding one has always been clean.

Boundary matrix (11 cells, both backends, `scripts/probe-matrix`): 7 cells fail on the
published `2026.8.0` and pass here; the 4 negative controls — the default arm, a
borrowed optional view, a non-optional return, and store-free optional scalars — pass
on both, so the matrix is not green by having stopped checking. Guarded in
`tests/scripts/174-inline-temp-free.loft`, the file that already owns this class.

### One call emitter re-derived the Rust fn identifier (2026-08-12)


Emitted Rust is one flat namespace, so two same-named fns from different files get a
file-hash suffix on the DEFINITION (`disambiguated_fn_ident`, #305). `Output::fn_ident`
is the chokepoint, and its doc says every site writing a definition OR a call must go
through it. `dispatch.rs`'s adopt-or-copy bind — `{ let _dst = …; let _src =
<callee>(cell, …)` — wrote `callee.name()` instead, so the call named a `fn` that had
been emitted under another: `error[E0425]: cannot find function n_defaulted in this
scope`, on a package whose interpreter suite was green.

The trigger is narrow enough that both the reporter's minimisation and my first one came
out GREEN, which is what the guard's comment records. A FIRST bind of a call result goes
through `calls.rs`, which was always right; it takes the adopt path — the callee returning
a LOCAL bound from another call — to reach this emitter at all. Reproduced by
reconstructing the consumer's pre-fix state from its current source (the failing state was
never committed), then reduced to a 20-line package.

Swept the siblings: `dispatch.rs:665` delegates to `output_code_inner`, and the other
`def_fn.name()` reads are dispatch predicates on `Op*` builtins, which are never mangled.
loft#878.

### A work-ref mint landed on the return-buffer ARGUMENT (2026-08-12)


`ref_return` promotes a body work-ref to the function's hidden return-buffer argument on
pass 1, and the variable tables persist across passes BY NAME while the counter restarts —
so on pass 2 `Function::work_refs` re-minted `__ref_N`, found the argument, and
`set_type`'d it to whatever the new site asked for. `work_refs`'s own doc already stated
the rule ("a name that resolves to an argument is STEPPED OVER rather than reused") and
the body never implemented it.

The site asking was a CALL's out-param buffer: the callee was handed the caller's record
buffer as its `vector<T>` destination, cleared it as a vector and built into it. The value
came back empty and the write that followed landed out of bounds — silently on the
interpreter, as `store_nr == 65535` reaching `allocations[…]` on native.

The step-over is keyed on the TYPE, not on argument-ness alone: pass 2 re-minting the same
name for the same ROLE is how the return buffer is re-found, and a blanket step-over grew a
lambda a second return-buffer attribute on pass 2 ("grew a pass-2-only attribute", the H5
two-pass contract). `Function::retypes_argument` = argument AND `without_deps()` differs, so
only a mint that would RE-TYPE the buffer steps on. `LOFT_NO_WORKREF_STEPOVER` opts out
(`keys::work_ref_stepover_enabled`).

It subsumes half of @PLN90 W1's A1b collapse: with `LOFT_NO_A1B=1` alone the known-wrong
plan is now correct, so `oracle_flags_the_a1b_wrong_plan` disables both gates to still have
a defect to catch. loft#872.

### A container in `ls` was renamed onto a RECORD return buffer (2026-08-12)


`classify_reference_delivery`'s fallback renames the return's dep candidates onto
`__retbuf`. A tail that indexes a call — `make(n)[0] ?? d` — leaves the indexed CONTAINER
in `ls`, and that container has no further deps of its own, so `return_views_local` reads
it as owned and the rename fires: a `vector<Cell>` became the `Cell`-shaped buffer the
caller allocated. Same promotion `moros H12` hit through a field projection, reached here
through an index, which is why the guard is on the buffer's SHAPE (`ls_can_be_record_buffer`)
rather than a fourth tail walker. Both delivery arms carry it — the block tail and the
`RetSite::MidReturn` explicit `return`, which fails identically.

Only a COLLECTION blocks the rename, and that narrowness is load-bearing: a first, wider
spelling refused every non-record candidate, so a `-> Dialect?` whose value came from a
call was MATERIALISED — and materialising a null-valued record fabricates an empty one.
`registry_pure.loft`'s "a refusal speaks no dialect" caught it. loft#877.

### `ShowDb::write_hash` walked a layout that had moved (2026-08-12)


Formatting a struct with a `hash<…>` field SIGSEGV'd the interpreter in `OpFormatDatabase`
and exited 1 with no output on native; `to_json()` on the same record shared the walker and
so shared the crash. `write_hash` carried a bucket loop of its own that read each slot as a
bare record number at `pos: 8` — the layout before @PLN135 arc H moved entries into an
arena, where several share one record and `(rec, pos)` identifies an entry.

Nothing caught the drift because nothing reached it: a BARE hash is refused at compile time
(`append_data`: "Cannot format type hash<…>"), so a hash FIELD of a struct is the only way
in, and the method was marked `#[allow(dead_code)]`. It is `hash::records_sorted` now — the
module that owns the layout — which also gives the render a stated order (key order, like
`index`/`sorted`) instead of bucket order. The `max_elements` cap the debugger's glance
relies on came along with it. loft#873.

### A not-found key field was used as an attribute INDEX (2026-08-12)


`Data::attr` answers `usize::MAX` for a name it cannot find, and `set_mutable` /
`set_mutable_directed` (`src/typedef.rs`) handed that straight to
`definitions[..].attributes[a_nr]`. Any key field a keyed collection names that its
ELEMENT type does not have therefore panicked — "index out of bounds: the len is 6 but
the index is 18446744073709551615" — with a Rust source location and a caret on
whatever line the layout was reached from, which is correct as written.

Wider than the report, which arrived as the two-argument `hash<integer, At>` spelling.
The same sentinel is reached by an ordinary MISSPELLING (`hash<At[ca_kye]>`), which is
both well-formed and far more likely, and by all five keyed kinds — `hash`, `index`,
`sorted`, `spatial`/radix and `trie` — since every one of them routes through these two
helpers. Sweep of the five spellings: all ICE'd before, none does now.

The name is recorded (`Data::record_unknown_key_field`) rather than reported in place
because `fill_database` has no lexer; `fill_all` has one and drains it
(`report_unknown_key_fields`) — the same record-here / report-there split
`actual_types_deferred`'s `defer_unknown` uses. The caret lands on the FIELD that
declared the collection, which is where the name was written.

The message corrects the MODEL rather than just naming the symptom, because the way in
is a user who believes `hash<K, V>`: it says the key must be a field of the element and
shows the spelling. A did-you-mean rides `suggest_similar_capped`; failing that it lists
the element's fields, except when the element is not a struct at all (`hash<integer,
At>` puts the key in the element slot) — there it says so, because listing what
`integer` answers to would offer its METHODS as candidate keys. loft#874.

### A struct field's absent value is the FIELD's question, not the type's (2026-08-12)


`integer`(0), `long`(1), `single`(2) and `float`(3) spell absence with a SENTINEL and
share one content type between their `T` and `T?` spellings, so
`Stores::set_default_value` — which sees only `tp` — had to pick one, and picked the
sentinel. Writing that into a field declared plain put a null in a slot DN1 says cannot
hold one: the reader answered `null`, the declared type said otherwise, and
`redundant-coalesce` then advised deleting the `?? 0.0` doing the work. A ranged field
was always right (`Parts::Byte`/`Short`/`ShortRaw`/`Int` carry `nullable` in the Part),
which is what made the defect read as a float/integer oddity rather than a rule about
FIELDS.

`Field::nullable` (@PLN127 arc D) already existed and already documented why it must be
DEPOSITED rather than derived — "`text?` and `integer?` share their non-null type and
spell absence with a SENTINEL, so nothing in the store implies this". It simply was not
being asked. Three sites default a struct field and all three dropped it:
`set_default_value`'s `Parts::Struct | EnumValue` arm (recursed on `f.content` alone),
`walk_parsed_struct`'s missing-key loop, and `walk_parsed_into`'s `Parsed::Null` arm.
All three now route through `set_default_value_nullable`, which differs from the
type-only answer in exactly four arms; `field_declared_nullable` resolves `(rec_tp,
field)` and answers `true` — today's behaviour — wherever the question does not apply
(`field == u16::MAX` for a top-level or array-element target, a non-struct `rec_tp`), so
every non-struct path is byte-identical.

Wider than filed on three counts. A key the JSON OMITS is the same question as one
written `null` and had the same wrong answer. So did a FAILED parse — a syntax error or
a leaf type mismatch abandons the record at its pre-defaults, and those were sentinels
too, which is why `tests/docs/24-json.loft` and `tests/scripts/57-json.loft` both
asserted `== null` after a bad parse. And a non-null field at DEPTH follows its own
declaration, not its parent's.

Consequence worth knowing: `ShowDb::write_fields` skips a field iff `is_null`, so fields
that were wrongly null were invisible in a dump and are now printed. Parse-then-show
NORMALISES rather than echoes, and the normalised form round-trips to itself —
`tests/data_structures.rs::record` now asserts that second parse directly instead of
asserting `show(parse(x)) == x`, which only held because the omitted fields were null.

`tests/scripts/298-multi-return-site-ref-buffer.loft` reached its third return site
through `result.v == null` on a plain `integer`; `v` is `integer?` now, because the
@PLAN59 site under test would otherwise have gone unexercised while every assertion
still passed.

Still open, filed: a DECLARED default (`= 1.5`) is ignored by the walker (loft#876)
because it lives parser-side as a `Value` IR node, not in `Field::default`. The fix
attaches at the same missing-key loop below and needs an answer for a non-constant
default first — fold a constant one into the schema, refuse a computed one on a
JSON-castable struct, or give the walker an evaluator. loft#870.

### The `text` half: what an absent text field costs (2026-08-12)


`text` was left out of the fix above because a text handle of 0 IS null (`Store::get_str`),
so its empty value has to be INTERNED — and `set_default_value`'s struct arm runs per
record on the allocation path. Interning there measured +78 % wall and +91 % peak heap over
400 000 rows with three text fields, every one of them overwritten by the literal that
followed.

So the call was split by what the value is FOR (`structures::Absent`): `Prefill` — a fresh
record, whose fields the literal or the walker will write — keeps the cheap 0, and `Final`
— the value a reader actually sees — interns. Three sites are `Final`: `walk_parsed_into`'s
`Parsed::Null` arm, `walk_parsed_struct`'s missing-key loop, and `db_from_text`'s pre-parse
fill (`set_final_default_value`), that last one because a parse that FAILS reaches neither
walker and leaves the record exactly as the fill left it.

A wide first spelling — intern in every arm — also leaked: `set_text` claims a fresh record
and overwrites the handle without freeing the old one, so a prefilled empty leaked once per
text field per record (`removal-frees-what-the-element-owned.loft`: 323 → 773).

The literal keeps @PLN25's base-zero rule for a NULLABLE field (`text? → ""`), which is a
decision, not a divergence: a constructor omitting a field asks for the zero, while a
document omitting a key did not say anything. `875-json-absent-text-field.loft` pins both.

Three assertions in the suite were the demonstration and are now updated: `57-json.loft:65`,
`tests/docs/24-json.loft` and `tests/docs/23-safety.loft` each asserted `!x` on a plain
`text` — passing only because of this defect, on a line where `redundant-null-negation` said
it never could. loft#875.

### A narrow vector element got the wide store op from the comprehension (2026-08-12)


`narrow_elm_set` (`src/parser/vectors.rs`) picks the store op for an element's own width,
and its contract is that every site BUILDING a vector routes through it — its own doc
names the failure mode: "a site that misses it emits the wide 8-byte `OpSetInt` into a
1-byte slot, so one write covers eight element slots" (the slice half of #624).
`build_comprehension_code` was a third such site and went straight to `set_field`, which
dispatches on the element DEF. A narrow integer is an ALIAS of `integer`, so a 4-byte
slot got the 8-byte op.

Each element overwrote its successor; once the writes passed the initial allocation they
reached the vector's own bookkeeping and `vector_add` stopped terminating. Hence a
BOUNDARY rather than a slowdown — `[for i in 0..13 { i as i32 }]` hung where `0..12`
returned instantly, that being where the overrun first reaches the header. Measured
boundaries: i8/u8 at 17, i16/u16/i32/u32 at 13, `integer` never (8 into 8 is correct).

Two things narrowed the filed scope. `+=` was already routed through the helper, so the
append loop was clean to n=5000 and the defect read as comprehension-specific rather than
width-specific. And `r.len()` cannot see the damage BELOW the boundary: a store that
clobbers its neighbours leaves the count right and the values wrong, so the guard
(`tests/scripts/869-narrow-vector-comprehension.loft`) checks elements and a
hand-computed sum. loft#869.

### A text→heap cast was typed as a view of its source (2026-08-12)


`OpCastVectorFromText` (`State::db_from_text`) interns text into a store of its OWN, but
the `as` handler (`src/parser/operators.rs`) grafted the source's deps onto the result.
@PLN99 arc C had already established the rule for `convert`'s allocating conversions —
"the result is not a view of the source, so grafting would mark it a borrow" — and `cast`,
which has the same property, never reported it. `Parser::cast_allocates` now answers it
from the two TYPES (text source, heap target), which is what makes the verdict
pass-stable: the return buffer is chosen on PASS 1 and freezes the signature, so a
pass-2-only correction lands after the text local has already become a parameter.

A freshly allocated record therefore read as a borrow of the text it parsed, and the
return-buffer machinery delivered it as one. The symptom was decided entirely by what the
source expression was:

| cast source | interpreter | native |
|---|---|---|
| text LOCAL (incl. a literal) | renamed onto `__retbuf` — one slot both `text` and record buffer: #306 guard, then SIGSEGV | 4 × rustc E0308 (`"".to_string()` into a `DbRef`) |
| LIFTED call temp | correct | bound as the buffer, cast emitted as a bare STATEMENT, untouched buffer returned — an EMPTY vector, silently |
| PARAMETER | correct | correct |

`file()` was never part of the trigger: a text literal reproduces it, which is what says
the defect is the cast. rustc had been reporting the vector half from the other side all
along ("unused return value of `db_from_text`").

With the deps corrected, the struct target still answered null on native:
`classify_vector_delivery` has a #409 forward-copy leg for a `#rust` callee that delivers
its own store, and `classify_reference_delivery` had none — it classified `AsIs`, which
claims the tail already wrote the buffer. It gets the record twin
(`emit_forward_copy_ref_409`), and "does this tail forward its own store"
(`tail_forwards_own_store`) now has ONE home instead of two that disagreed.

Matrix: 13 probes over {vector, struct} × {tail, non-tail, bound-local, inline, no-return}
× {parameter, text local, literal, lifted call, const}, every cell value-checked by hand on
both backends plus `LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`. The spurious `File×1`
leak those shapes reported went away with the borrow that caused it.

Residue: the interpreter still prints the #306 guard twice for
`t = <text>; m = t as Struct; m` — value correct on both backends, native clean. Two
mechanisms were tried and reverted (rerouting the delivery to `MaterializeView` returned
a null; a `SkipReassigned` rung in `classify_ret_promotion` changed nothing), so the
rename is reached from a site neither covers. loft#866, loft#867.

### A member access recovered only over an identifier (2026-08-12)


`Parser::field`'s Unknown/Never recovery skips the member token so parsing can continue
past a receiver with no type yet. It consumed an IDENTIFIER only, so a numeric tuple
index stayed in the stream and the statement parser tripped on it as `Expect token ;` —
on PASS 1, and a pass-1 error means pass 2 never runs.

Two failures from that one gap. A forward reference to a tuple-returning function could
not be tuple-accessed AT ALL (`v = later("x"); a = v.0;` with `later` declared below is
legal, and Unknown on pass 1 by design). And an unresolved call was never reported:
"Unknown function" is a pass-2 diagnostic — pass 1 cannot tell a typo from a forward
reference — so the aborting `Expect token ;` left the real cause unmentioned. The
named-field and `[i]` spellings always recovered, each consuming its own member token;
only the tuple form had no consumer.

Indexing a `Never`-poisoned receiver is now silent too, matching the @P376 recovery in
`field()`: the receiver's own error is already on screen, so "Indexing a non vector" only
named a second correct line. loft#868.

### The op-sets are a property of the program, not of each question (2026-08-12)


`use_analysis` consults four op-number sets — `projection_ops`, `value_reader_ops`, the
arg-0 writers (`is_first_arg_write_name`) and the collection `len` methods
(`is_length_op_name`). Each is a pure function of the definition NAME table, so each is
constant for a given program. Each was rebuilt per question, and the walks
(`dead_store_accesses`, `collect_uses`, `Ownership::new`, `classifies_structurally`) ask
once per FUNCTION — two of the four being a full scan of every definition doing string
prefix matches. O(functions × definitions).

Measured on a `println`-sized program: **9 000 rebuilds over 708 definitions**. `perf`
put `first_arg_write_ops` at **22.7 %** of a warm-cache run — the hottest symbol in the
process — and with the `HashSet<u32>` inserts, rehashes and sip-hashing it drives, ~40 %
of startup. Warm 18.3 → 9.0 ms/run, cold 53.7 → 30.0 ms/run, against a HEAD build of the
same tree.

Cached on `Data` as `OpSetCache`, **keyed by definition count**. The keying is the whole
correctness story, and the first attempt (a plain `OnceLock`) was a measured no-op: the
first question arrives while the definition table is still growing, so pinning the answer
to that moment made every later question a miss — **7.5 M rebuilds** across the
`tests/scripts` corpus, i.e. the cache never once answered. A `debug_assert` would not
have caught it either: `[profile.dev.package.loft]` sets `debug-assertions = false`, so
such a guard is compiled out of the library in every standard build. Hence a checked key
rather than an asserted invariant.

Shape and clones-empty rationale follow `LazyDriverCache`, the existing precedent
directly above it. Deliberately NOT the loft#854 shape: those facts derive from
`Definition::code`, which `scopes.rs` rewrites, so a `Data`-lived cache would answer from
a body that no longer exists; these derive from def names, which never change once a
definition exists. `rebuild_indices` (which can REMOVE definitions) drops the cache.

Behaviour-preservation gate: `loft introspect` over all **672** `tests/scripts` programs
byte-identical before and after, IR and bytecode.

Closes loft#864 (per-invocation floor). The issue proposed a warm session or a `loft
serve` daemon; the measurement said the floor did not need one. Two facts settled it.
An INSTALLED loft was already at ~20 ms, not the 80–220 ms reported: the whole-program
cache is default-on but deliberately disabled for a dev build
(`cache::running_a_dev_build` — any `debug`/`release` path component), and the report
measured a from-source binary, which is the one configuration where it is off. And the
floor that remained was over a third pure waste, which is what this removes. A
persistent-session daemon stays available as future work; it is no longer the answer to
this issue.

Two unrelated reds cleared alongside:

- `cargo build --no-default-features` did not compile (already red on HEAD): `loft_home`
  sat behind the `registry` feature while `cache_areas` — the unconditional `loft cache`
  command — resolves the build cache through it. `dirs` is an unconditional dependency,
  so the gate was simply wrong.
- `AtomicU64::fetch_update` is deprecated; now `update`, not `try_update` — the
  saturating subtraction cannot fail, so there is no `None` case to report.

### The heap ledger is per-linkage-unit, and a store outlives the one that made it (loft#862) (2026-08-12)


`make ci` aborted at `exit_codes::moros_glb_cli_end_to_end` with `store_budget.rs:219:
attempt to add with overflow` — an *add* overflowing a `u64`, which can only happen if
`TOTAL` is already near `u64::MAX`, which can only happen if a `fetch_sub` already went
below zero and wrapped. So the reported site was the victim; the cause was a release.

The instrument said the rest in one run. Asserting `bytes <= TOTAL` inside `release`
rather than waiting for the next allocation gave:

```
PROBE release underflow: kt=125 bytes=4344 born_at=99218 TOTAL=0
  store_budget::release ← codegen_runtime::OpDatabase ← extensions::shared_store_dispatch
```

`TOTAL=0` with 4344 bytes being released: that `OpDatabase` runs INSIDE the library's
auto-native cdylib, which links its **own** copy of libloft and therefore its own
`store_budget` statics. The store was counted by the host's ledger and released against
the cdylib's, which starts empty.

`TOTAL` now saturates at zero, because below zero is not a quantity of heap — it is the
ledger being asked about bytes belonging to another one. What that costs is stated rather
than hidden: the HOST still counts those bytes as live, since its own ledger never sees
the release either, so the ceiling reads high for a program that frees inside a library.
That direction is the safe one (it can refuse early, never late) and it is what the
behaviour already was; making the two ledgers one needs a `loft_ffi` hand-off and is not
this change. Guarded by `releasing_more_than_was_added_stops_at_zero`, which asserts the
VALUE — a saturating floor and a wrap both survive a debug `fetch_sub`, and only the
number tells them apart.

Worth noting where it hid: `binary(exit_codes)` is outside `find_problems.sh`'s curated
selection, so a green `--wait` never covered it, and a release build wraps silently rather
than aborting. Both were true of `main` as well — verified on a clean worktree at
`00db858b`, not inferred.

### A refusal that returned `u32::MAX` became a syntax error (loft#863) (2026-08-12)


`fn sum(v: vector<integer>) -> integer { … }` collides with the stdlib's
`pub fn sum <T: Addable>` and is correctly refused — but it answered with a bare
`Cannot redefine 'sum'` and then `Syntax error: unexpected '->'` against a signature that
is perfectly well formed. Reporting the collision returned `u32::MAX`, `parse_function`
reads that as "this was not a function" and returns `false`, and the top-level loop then
resumed with the lexer parked between the parameter list and the `->`.

The sibling branch ten lines above — a free function shadowed by a METHOD on its first
argument's type, which `len` takes — never had the second message, because it reports and
falls THROUGH to the registration. This is the same fall-through: the rejected definition
registers under a `#dup` name no call can spell, so the rest of it parses, the real error
stands alone, and the winner keeps the real name. The position is printed too, which is
what says `sum` is the stdlib's rather than a duplicate of the reader's own.

### A tuple element read was a cursor typed as an owner (loft#857, loft#858) (2026-08-12)


Two issues, one line. `v[i]` on a `vector<(…)>` unboxes the element through a work-ref
that holds a DbRef into the vector's store, and `unbox_tuple_from_dbref` minted it with
`Deps::none()` — which in this codebase means OWNS, not "unknown".

Read as an owner, the cursor was freed at scope exit. Reading out of a `vector<(…)>`
**parameter** therefore destroyed the CALLER's vector store on return; the slot was
recycled, and the next call's `+=` appended through a handle that named another record
entirely (loft#857 — `vector_append: field 1.8 lies outside its own record, which claims
-99 words`). The filed scope was three axes too wide: the hash parameter, the outer loop
and the call count are all irrelevant, and one indexed read plus two calls reproduces it.
A DEAD read does too, so it is the read and not the dataflow.

Read as an owner, the cursor also had to hold its own store, so `__ref_1 = <foreign
DbRef>` lowered to `OpDatabase` + `OpCopyRecord`: every `v[i]` **allocated a store,
deep-copied the element into it and freed the previous one**, then read the elements back
out of the copy (loft#858). That is the ~14× against `vector<struct>`, whose cursor has
carried a dep on the vector all along and just copies a pointer. The reporter's verbatim
benchmark goes **379 ms → 12 ms**, against 11 ms for the struct — 14.6× to 1.09×.

So the cursor names the vector in its deps when the receiver is a variable, which is what
the struct path always did. Two measurements say the earlier readings of #858 were wrong,
and both are worth recording: the `??` join is **not** the cost (`v[i]?` measured the
same, 159 ms vs 155 ms), and it is **not** per-element unboxing either (arity 2 cost
139 ms against arity 3's 155 ms — the allocate/free dominates, so the gap barely moves
with arity). A `vector<float>` indexed read is only 2.2× its iteration, which is the
raising `OpGetVector` (a length read on top of `get_vector`) and is a separate, small
thing left alone.

The free still needs suppressing separately: the scope-exit sweep frees a `__ref_N` on
its NAME ahead of asking whether it owns anything — a rule written for the work-refs that
back ref-returning calls, which do own what they hold. And the borrow verdict is per-DbRef,
not per-call-site: the same helper unboxes heap-carrying tuple RETURNS, which ARE owned,
and a blanket skip leaked the four `pair(…)` return buffers in
`822-vector-tuple-spellings.loft`. A vector element read is the positive, checkable case;
everything unrecognised keeps the owning treatment. The mark is pass-2 only — the variable
table persists across passes by name while the `__ref_N` counter restarts, so a pass-1
mark could land on whatever temp pass 2 gives that name (loft#848).

`857-vector-tuple-element-read-borrow.loft` pins both, including the cells a borrow newly
has to survive: a read and an append in one statement, and 40 rounds of grow-then-read so
a cursor left pointing at a moved allocation shows up as wrong values rather than a crash.

### `τ` → `τ?` was refused with advice that could not work (loft#859) (2026-08-12)


Storing `x / g` back into a non-null `g` is refused correctly — a variable divisor can be
zero, so the quotient is `τ?` (C80). But the message offered `as` and a new variable name,
and on this shape **both fail**: the cast checker refuses `as τ` for the very reason the
store was refused, `as τ?` lands back on this same error — a closed loop between two
diagnostics — and a fresh name still has to store the value somewhere. The cures that
work, `?` and `?? <default>`, were named only by the cast checker.

`reject_retype` now picks the advice by which property changed. Only the ADVICE half
differs: the diagnosis, "cannot change type from τ to τ?", is right as it stands and is
kept word for word, so the two messages remain one diagnostic to anyone reading, grepping
or testing for it. A genuine type change (`integer` → `text`) keeps the `as` advice, which
is what it was written for.

`bench/06_newton_sqrt/bench.loft` was the reason @PLN140's oracle corpus had no row for
it — it did not run on either backend. It discharges at the division now and runs.

### The runtime rebuild retried against the state that motivated it (loft#855) (2026-08-12)


`--native`'s post-compile heal rebuilds loft's runtime rlib and retries the compile with
the SAME `Command` — whose `--extern loft=` / `-L dependency=` args were chosen from the
rlib that was on disk when the command was built. With no rlib at all they were never
added, so the retry re-ran a rustc that still named no crate: the heal reported its own
success and an identical `E0463: can't find crate for loft` in one breath, which reads as
a broken toolchain rather than a missed refresh. The runtime args are re-asked after a
successful rebuild.

The nightly ASan legs were red for a different reason and no leak: the per-file scan
reported **0 leaking files of 701**. Two tests spawn loft on both backends, and the
`--native` leg makes the spawned run BUILD native artifacts, which cannot resolve loft's
proc-macro deps under `-Zsanitizer` + `--target`. The ASan sweep already excluded them;
the leak gate did not, so it now does — a gate that reds on its own toolchain teaches
readers to ignore it.

### The ownership oracle is asked once per function, not once per question (loft#854) (2026-08-12)


`use_analysis::ownership_of` is documented as *"the ONE fact every own-vs-borrow
chokepoint READS instead of re-deriving"*. It re-derived it on every read:

```rust
pub fn ownership_of(data: &Data, d_nr: u32, value: &Value) -> Own {
    let def = data.def(d_nr);
    collect_defs(&def.code, &FillOps::of(own.data), &mut defs);  // the WHOLE function
    own.classify(value, &def.variables, &defs)                   // …to answer about ONE value
}
```

`scopes::scan_set` asks once per assignment, and a vector literal is one assignment per
element — so an n-element literal walked the function n times, **cloning every defining
right-hand side** on each walk. Quadratic: 2 000 elements 0.68 s, 4 000 2.34 s, 8 000
9.42 s, a clean 4× per doubling. crawler's generated terrain file (one 86 400-element
`vector<integer>`) took **over 13 minutes** at 99 % CPU with no output, which reads as a
hang; five of its `make` targets imported that module, so none of them were ever run.
It is now **0.99 s**.

**The fix is a memo, and where it lives is the whole design.** `function_defs` is split
out of `ownership_of` and memoised on `Scopes` — created per scan phase, holding the
`d_nr` being scanned. Correct *by construction* rather than by convention: `data` is
borrowed `&Data` for the entire traversal and `run_scan_phase` installs the rewritten body
only after `scan` returns, so the borrow checker is what keeps the memo from going stale.
It is the same value the old code recomputed — `data.def(d_nr).code` — computed once.

**Not cached on `Data`**, which is where it would naturally go. `Data` must stay `Sync`
(tests park it in a process-wide `OnceLock` and parallel workers read `&Data` across
threads), and the existing precedent there — `caller_index` — is a write-once `OnceLock`
that is never invalidated, which is sound only because the caller graph is stable after
parse. `Defs` is not: `scopes.rs` rewrites `definitions[d_nr].code` at four points, so a
`Data`-lived cache would answer from a body that no longer exists — silently, and in the
direction that mis-classifies ownership. The other 14 `ownership_of` call sites are
unchanged and keep recomputing; none of them is hot.

**Verified behaviour-preserving, not just green:** `loft introspect` over 60
`tests/scripts/*.loft` programs is **byte-identical** before and after, IR and bytecode.
Guarded by `tests/compile_scaling.rs`, a new home for complexity bounds, whose margin is
measured in both directions — 69.9 s against the reverted fix, 0.33 s with it.

Two things the filed report had wrong, worth recording because both would misdirect a
reader: it is **not parsing** (parse is linear: 12 → 17 → 34 ms while `scopes::check` went
705 → 2 271 → 9 323 ms), and the suggested **chunking workaround does not work** — eight
1 000-element literals in one function cost the same as one of 8 000, because the axis is
per-FUNCTION, not per-literal. Splitting across functions is what helped.

### A method lookup asks the type's OWN source only to replace a foreign candidate (loft#850 follow-up) (2026-08-11)


loft#850 taught `find_fn` that the mangled key `t_<len><Name>_<fn>` spells a type's NAME,
not a type: two packages may each declare a `Thing`, both register `t_5Thing_go`, and the
caller's name table holds whichever import landed first. The fix checks each candidate
against the receiver it declares and, when the candidate is foreign, re-asks in the type's
OWN source — where the right package's method lives.

The re-ask was written as a plain second search source:

```rust
for from in [source, own_source] {
    let d_nr = self.source_nr(from, &key);
    if self.method_receives(d_nr, type_nr) { return d_nr; }
}
```

**Every type has an own source, and for a builtin it is the stdlib.** So this did not just
resolve collisions — it added the whole stdlib method surface as a fallback for any call
whose first argument is a builtin type, ahead of the free-function lookup below it. A
library's free `split(pattern: text, input: text)` lost its own qualified call to the
stdlib's `split(self: text, separator: character)`: `regex::split("[,;]", "a,b;c")` stopped
compiling with *expected character, got text on argument 2*. The published `regex` package
had shipped that call since 0.2.0, and a language change retro-breaking a shipped library is
what the freeze forbids — `revalidate-libs` is the gate that caught it, green on `main` and
red on the branch.

The re-ask is now what it was meant to be: a REPLACEMENT for a candidate this scope answered
with and that proved foreign, never a second place to find a method the caller's scope does
not have. No candidate under the key means nothing to disambiguate, so the search falls
through to the free function exactly as it did before loft#850.

Pinned by `issue853_a_library_free_fn_outranks_a_stdlib_method_of_the_same_name`
(`tests/imports.rs`, both backends), whose control line asserts the stdlib's own free
functions and text methods still resolve from the same scope — a fix that reached the free
function by losing the methods would satisfy the subject and break the language.

### A binding position mints a local, whatever else carries that name (loft#852, loft#756) (2026-08-11)


A library's public function occupied the CONSUMER's variable namespace: with
`use engine_host;` in scope, `turn = 0` was a compile error anywhere in the consuming
program. So one public verb added to a library broke every consumer already using that
word as a local — crawler's gate went red across 109 rows, on a commit crawler did not
make. [C97](DESIGN_DECISIONS.md)/[C98](DESIGN_DECISIONS.md) make this impossible for
the *stdlib*; this is the same hazard one level up, where nothing weighs each new name
and nothing announces which words a release claims.

**The refusal was one code path, not a rule.** It fired in three binding forms —
`name = …`, the typed local `name: T = …`, and a tuple-destructuring element — all
three routing through the bare-function-reference fallback in `parse_var`
(`src/parser/objects.rs`). The other three never refused, for the stdlib either: a
parameter, a `for` variable and a struct field could all be called `chr` while
`chr(65)` in the same scope answered `"A"`. So loft already keeps values and functions
in **separate namespaces**, and the parentheses already pick between them.

**The fix** is that the function-ref fallback yields at a binding position, exactly as
the type/enum branch beside it already did (`def_nr(name) != MAX && !at_binding_name()`);
the name then takes the ordinary new-local path. The pass-2 rescue that re-resolves an
untyped pass-1 placeholder to a forward-declared function takes the same guard — without
it a binding whose type pass 1 could not infer (`turn = a_forward_fn();`) would hand
`parse_assign` a function-ref where its target belongs.

What the refusal was really reporting was a **recovery** problem, not a namespace one:
@P335/@P392 found that the function-ref left the `:` or `=` unconsumed and the author
saw a confusing `Expect token ;`. Binding supplies the `Value::Var` that recovery
wanted, so those forms parse rather than diagnose. `tests/scripts/repro_p392.loft`
changes from an `@EXPECT_ERROR` case to asserting the typed local binds and `now()`
still answers — the mis-parse it exists for fails it either way.

**Pinned by** `tests/scripts/852-local-shadows-a-function-name.loft`, which carries the
parameter / loop / field cells as CONTROLS so a change that frees the three refusing
forms by breaking the three that already worked fails there, and
`pln102_c98_a_local_may_shadow_a_library_function` (`tests/imports.rs`) for the library
half on both backends. Register entry: [C112](DESIGN_DECISIONS.md).

**Residual, unchanged by this:** a bare `use lib;` still wildcard-imports every public
name into the unqualified namespace (`src/parser/mod.rs`,
`None => Some(ImportSpec::Wildcard)`), so `turn(3)` is callable unqualified after
`use mylib;`. C98 rules it must bind only the `lib` handle. That is a breaking
resolution change needing a pre-freeze migration (`use lib;` → `use lib::*;`), so it is
owner-timed rather than folded in here — and C112 is forward-compatible with it.

### `--html` binds a filesystem, over raw `loft_io` imports (loft#851) (2026-08-11)


`--html` bound no filesystem. The loft-side file calls compiled anyway — the
wasm-bindgen feature that routes them to a JS host is off for this target, so each
one took its inert branch and answered "absent" — and the build reported success.
A page could draw and could not save, and the consumer discovered it by grepping
the emitted bundle.

**The transport.** `--html` cannot reuse the `loftHost.fs_*` bridges: those are
`js_sys::Reflect` lookups needing wasm-bindgen, and this target builds its rlib
`--no-default-features --features random` and refuses a page whose wasm imports
anything beyond `loft_gl` and `loft_io`. So the file functions are raw `loft_io`
imports declared in `src/lib.rs`, and reads use the `len`-then-`copy` shape
`loft_host_input_len`/`copy` already proves, sharing one host stash — safe because
each is a synchronous loft call, so the next read cannot begin before this one has
copied. `usize::MAX` is absent, which is not a length of 0.

**One chokepoint.** `src/wasm.rs::host_fs_*` is the only place that picks a
transport (`js_sys` under the `wasm` feature, raw imports otherwise). Every call
site asks one question instead — the new `host_fs` cfg from `build.rs`, set for the
wasm-bindgen bundle and for `--html`, and deliberately NOT for `wasm32-wasip2`,
which has a real WASI filesystem. That replaced 40-odd hand-written
`feature = "wasm"` gates across `state/io.rs`, `database/io.rs` and
`codegen_runtime.rs`, and the two `--html`-only inert stubs they had grown beside
them. `codegen_runtime.rs` is the one that mattered: `--html` runs generated code,
and its browser arms were stubs returning nothing while the interpreter's were real
bridges — so the two backends had different filesystems and only one of them was
documented.

**The cursor.** A page has no OS file handle, so the host keeps the read/write
position per path. This is where the first working version was still wrong, and the
matrix is what caught it: every whole-file cell passed while `read_bytes` answered
zero bytes for a file that had just been written. `fs_read_bytes` / `fs_write_bytes`
in `database/io.rs` are WHOLE-file operations and were calling the cursor-relative
bridges — so a preceding write left the cursor at the end and the read started
there. They call `read_binary` / `write_binary` now, which also fixes the same wrong
result in the wasm-bindgen build, where it had been latent.

**The host half** is `doc/loft-fs.js`: an immutable base tree the page supplies
(`globalThis.loftBaseFS`) plus a delta holding every write, persisted to
`localStorage` — the `LayeredFS` shape, which is what a page needs. Both page
shells bind it, so a program that only stores still gets the minimal engine-less
page. Every node harness that instantiates an `--html` wasm imports the same module
rather than restubbing it, because a stub answering 0 means "an empty file that
exists" where the contract says absent.

**Guards.** `tests/html_wasm.rs` drives a real page through the whole surface and
through the cursor, with every expected value taken from what `--interpret` and
`--native` print for the same program; `tools/loft_fs_unit.mjs` (run from the same
test binary) covers the base tree and the reload, which a node-hosted page cannot
reach. The build-time warning added earlier for this issue is gone — its premise
was that the target binds no filesystem.

Also here: the wasm32 rlib would not compile at all. An entry inserted into
`src/native.rs`'s natives table landed between a `#[cfg(not(target_arch =
"wasm32"))]` and the item it guarded, so `n_kernel_listen` lost its gate; `--html`
reported "install the target" — blaming the environment for a source error — and
linked the previous rlib.

### `store_release` — a working-set hint, and the per-record shape it replaces is measured dead (@PLN126) (2026-08-11)


@PLN126 opened on a measurement rather than an API: *does ordered insertion leave a
finished record contiguous?* `src/database/spans.rs` answers it by painting every word
of a built arena with the record that owns it, through `for_each_owned_child` — the same
ownership walk `remove_claims` frees by, so the measurement cannot disagree with the
runtime about who owns what. On `routing`'s generator shape (`hash<TTile[tkey]>`, two
grown-by-append vectors):

| | span/live mean | exclusive 4 KB pages | droppable below the finish frontier |
|---|---|---|---|
| strict key order (W=1) | **356×** | **0.0%** | **98.7%** |
| 16 records open (W=16) | 391× | 0.0% | 93.0% |
| 64 open | 540× | 0.1% | 86.9% |

**Contiguity is false by two to three orders of magnitude, and the cause is not vector
reallocation.** The outer `hash` keeps entries in a chunked arena claimed early, while a
record's vectors are claimed at the frontier much later — so a record's own bytes sit
either side of the whole store. With ONE record in the store and nothing else alive, its
5-word slot and its 28 words of vectors are separated by 318 words of the collection's
own spine.

That kills the per-record release outright (0.0% exclusive pages is the granularity
problem in a number) and, contrary to the plan's reasoning, does **not** kill the
frontier release: that one needs the region below the mark not to be written again,
which is a property of the allocator on an append-only workload, and it holds. So the
plan was re-scoped onto the claim that measured true, and built.

`store_release(collection) -> integer` (`Store::release_resident` → `Stores::release_store`)
does `msync(MS_ASYNC)` + `madvise(MADV_DONTNEED)` over the whole pages below the mark
that have not been released. Peak RSS 44.3 MB → **2.2 MB** on an 89 MB build, at 1.0×
wall, one call per record. Content, references and file length are all untouched: the
mapping is `MAP_SHARED`, so a released record re-reads from the file one fault later.

Three costs the design could not have predicted, each found by an instrument:

* **Deriving the frontier cost the whole point.** `Store::usage` walks the block chain,
  one header per block, touching every page — so asking it what to drop faulted the
  entire store back in first, and peak RSS became the whole file (80.9 MB against 44.3
  for making no call at all). `Store::claimed_end` now carries the mark forward at
  `claim_block`. It is a monotone UPPER bound on `live_end_words` — the safe direction
  here, the wrong one for `shrink_to` / `reclaim_tail` / `bind_path`, which still read
  the chain because truncating to an upper bound cuts live data.
* **Each call must be bounded to what is new** (`Store::released_bytes`). Flushing from
  zero every time re-syncs a region that grows with the run: 208× wall.
* **`MS_ASYNC`, never `MS_SYNC`** — same resident set, and waiting costs ~1.5 ms a call.

**It pays only for an ordered build** (1.01× at W=16), and the attribution is the free
block count rather than the layout: 10 free blocks at W=1 against 3 691 at W=16 for the
same data. The LLRB free-space tree's nodes live inside the freed blocks, so a scattered
arena gives the allocator its own scattered working set — exactly the region the release
just dropped.

Gates: `tests/scripts/126-store-release-keeps-everything.loft` +
`store_release_keeps_every_record_and_reference_both_backends` (a reference held across a
release is checked by VALUE, because a re-faulted page returns a plausible number either
way), `database::spans::one_tile_footprint_is_the_blocks_it_owns`, and the two `#[ignore]`
measurements. Full workings: `doc/claude/plans/126-record-frontier.md`.

### A hash's entries move into a chunked arena, and the lookup win it was built for does not exist (@PLN135 arc H, #809) (2026-08-10)


`hash<T[k]>` stops claiming one store record per entry. Entries are slots at a fixed
stride in a chunked arena (`src/arena.rs`) whose bookkeeping lives in the bucket table,
and a bucket slot holds a 1-based arena INDEX rather than a record number.
`placement::HASH` bumps 1 → 2, so a store written before this REFUSES instead of being
misread — the reason Q2 shipped ahead of H.

Measured against the installed `v2026.8.0` as before-oracle, alternating A/B on a quiet
box, 1M `integer` keys, `--native-release`:

| | before | after | |
|---|---|---|---|
| insert (reserved) | 330 ms | **258 ms** | **1.28x** |
| store bytes / entry | 27.67 B | **18.6 B** | **−33%** |
| claimed records, 2000 entries | ~2000 | **9** | table + directory + 6 chunks |
| random lookup | 184 ns | 183 ns | **unchanged** |

**@PLN135 predicted 2.3x on lookup and that is wrong, from correct measurements.** Q1's
ablation (the record read is 82% of a random lookup) and Q5's shapes (a dense
`vector<Entry>` at 80 ns against the hash's 200) are both sound; the inference is not.
80 ns is ONE random read and a hash lookup makes TWO — bucket, then entry — so packing
the entries moves where the second miss lands without removing it. Density pays for
locality and a lookup has none. What the arena removes is the per-entry `Store::claim`,
which is what #809's title names, and that lands on insert and on bytes.

Three things the build turned up that the design did not:

* **A hash has TWO kinds of entry.** A secondary index (a sibling field's
  `other_indexes`) reaches records the PRIMARY collection owns and must neither move nor
  free them. The discriminator is the recorded stride — a table that allocated its
  entries knows their width, one that borrows records has none — so `stride == 0` means
  borrowed, and every decode, free and teardown reads it.
* **Chunk sizes must stop doubling** (`arena::CAP_CHUNK`). Uncapped, the tail waste is
  proportional and measured 27.33 B/entry — the whole saving — and it made a store's size
  depend on construction order. Capping bounds it at one partly-filled chunk: the
  difference between −1% and −33%.
* **Creation and freeing are one change.** `record_new`'s keyed arm allocates from the
  arena; `for_each_owned_child` stops returning each entry as an `owning_elem` for
  `Store::delete` and returns the chunks and directory through a new
  `OwnedWalk::extra_recs`. Half of that alone hands interior arena bytes to the free tree.

**A latent store-lifetime bug came with it, and it is the sharper half.**
`free_iteration_scratch` decided what to release by reading its scratch header's
fields, guarded only by `Store::is_claimed_record` — which says the BLOCK is live, not
that it is still ours. A released scratch leaves its block on the free list and the
next claim takes it; every field read after that is somebody else's bytes. One of
those reads decides *"the elements live in another store, so free the whole store"*,
and it fired: the arena's first chunk is exactly the claim that lands on a released
2-word header, so a hash captured by a closure lost the entire store it lived in
between one invocation and the next — every entry gone, no error anywhere. It surfaced
as `multiplayer_v2::v2_two_clients_with_spectator_routing` hanging: the tictactoe
server keeps its client table in a struct its `server::run` closure captures, so every
lookup after the first iteration missed and `handle_click` returned at its first guard,
leaving both clients to wait out their budget. The scratch header now carries a marker
(`vector::scratch_tag`, and the same word holds the element width), and the free path
refuses a record that does not present it — refusing costs at most the scratch's own
two blocks once, acting on a foreign record cost the store. Guard:
`data_structures::a_released_iteration_scratch_is_not_acted_on_twice`, which re-claims
the released header's block deliberately and asserts on `Store::is_free` rather than on
a value read back, because a freed store keeps its buffer until the slot is reused.

Four `store_persist_loft` tests changed with it, none by loosening a bound. Two fixtures
(`store_compact_slack_730`, `store_load_density_729`) were building their vectors in a
LOCAL and copying the finished thing into the entry, which acquires none of the in-place
growth slack they are named for — the entry's copy is claimed at its exact length. What
they actually acquired was the blocks the local abandoned on the way up, i.e. free space
BETWEEN records, and the arena made the allocator pack those better (same digest, file
1 729 032 → 1 091 112 B). Both now append through `h[i].data`, which grows the record the
collection HOLDS. `persisted_size_tracks_content_not_construction` now compares the two
construction orders AFTER a rebind, which is the comparison loft#710 is about — as built
they differ by interior free space that only compaction reclaims, as that test's own
header says. `reclaim_and_compaction_refuse_a_sealed_store…` grows its control 10x
instead of 2x, because 3000 extra entries are now ~48 KB of slots rather than 3000 claims
and no longer push the store buffer past its bound size.

### Three answers that were derived from a proxy instead of the fact (#829, #830, #831) (2026-08-09)


Three consumer-filed defects with one shape: a decision read a stand-in for the
fact it needed, and the stand-in was true when the fact was not.

- **#829 — `content()` answered `""` for bytes it could not decode.** `""` is
  what an empty file says, so a caller could not tell the two apart, and a
  round-trip gate over binary data passed vacuously (`0 == 0 * 2`). `content()`
  is `text?` and already answered null for a missing file (@PLN102 H4); it now
  does the same for non-UTF-8 bytes and for a directory, with `""` reserved for a
  file that really is empty. The decision sits in the loft-level `content()` —
  one home, so both backends get it from the same place. The read beneath it also
  had two homes (`State::get_file_text` and `codegen_runtime::OpGetFileText`) and
  the stderr warning lived in only one of them, so `--native` read binary in
  silence; both now call `read_file_text_into`. Guards:
  `tests/binary_io_matrix.rs::c829_*` (four `cross_mode!` cells, including the
  empty-file cell that keeps null and `""` apart) and a both-backend
  `p166_content_on_binary_file_warns`.

- **#830 — `loft update` resolved the lockfile, not the project.** A dependency
  declared in `loft.toml` and absent from `loft.lock` was never looked up, and
  the summary counted lock entries, so the omission printed `all N packages
  up-to-date`. The work list is now `lockfile::update_worklist(lock, declared)` —
  the union, as a pure function with unit tests, so "which packages" has one
  testable home. A declared-but-unresolvable package is named and turns
  `--check` red (that check asks whether the lock describes the manifest);
  `loft update <pkg>` on a non-dependency refuses instead of claiming it is
  up-to-date; a project with declared deps and no lockfile gets one written.

- **#831 — a cdylib that built was assumed to be one this process can use.**
  Marking a function for cdylib dispatch makes `byte_code` emit `OpStaticCall`,
  so an unwirable symbol reaches the `compile.rs` panic stub and kills the run.
  Marking was gated on the BUILD succeeding; an artifact can build and still not
  load (different `libloft.rlib`, missing system library, replaced by a
  concurrent `loft`), and an artifact declaring no layout is adopted outright
  because that is what a hand-written cdylib looks like.
  `native_lib::probe_and_mark_exports` now `dlopen`s the artifact and `dlsym`s
  each bridge before marking, marks only what resolves — partial is a valid
  outcome — and KEEPS the handle, so a later prune or rebuild cannot invalidate
  the decision. Unresolved functions interpret, which is what the auto-native
  model always promised. This is why crawler's suite lost a different test on
  each parallel run: processes share `<pkg>/native-auto/`, and the loser got the
  panic stub instead of the interpreter. Guards:
  `tests/n3_use_native.rs::an_unwirable_cdylib_interprets_instead_of_panicking`
  and `::a_partially_exporting_cdylib_marks_only_what_resolves`, both driven by a
  real artifact replaced with a cdylib that loads and exports no bridge — the
  shape every freshness check accepts. `--help` now names `LOFT_NO_NATIVE_LIBS`
  and `LOFT_REQUIRE_NATIVE`, which the report searched for and could not find.

  **Residual half, found by the same suite:** `prune_artifacts` bounded
  `native-auto/` by sweeping every `.so` in it by age, and the directory is not
  exclusively its own — a `[c] shim` cdylib lives there too, content-keyed and
  built ONCE, hence permanently the oldest file and the sweep's first victim.
  That does not cost a rebuild, it deletes the only definition of the package's
  `#c` symbols; the run then dies at `c_call.rs` with *"symbol not found … or
  check the spelling"*, and nothing can interpret in its place because a `#c`
  binding IS the implementation. Reproduced deterministically against
  `tests/fixtures/sqldb/sqlite` (saturate, run once, shim gone, exit 101) — it
  had been living in the suite as the "known flaky"
  `native::a_lazy_read_gives_one_answer_down_rust_and_down_loft`. The sweep now
  takes only the `loft_auto_<pkg>_` family it built; guard
  `::a_foreign_library_in_native_auto_survives_pruning`.

### The block-tail `expected` push learns a third shape: the interpolation target (#837) (2026-08-10)


@PLN124's target is read off the one `⇐` channel, and `parse_block` pushed the block's
result type into that channel only when the result was an **enum** (@PLN22 phase 1) or a
**collection** (@PLN90 W8). A struct is neither, so `fn q(name: text) -> Query { "hi
{name}" }` parsed its tail with `expected = Unknown`, took the ordinary text path, and
failed the tail conversion — *"expected Query, got text on return from block"*. The gate
now also fires when `interpolation_target(result)` resolves, which is a pure lookup
(`Type::Reference` → `DefType::Struct` → defines `t_<len><name>_lit`), so the cost is one
def-table probe on block tails whose result is a struct.

One gate covers all three reported spellings — block tail, explicit `return`, and an `if`
tail threading into both branches — because they share `parse_block`'s tail.

The issue asked which of doc and code was wrong, on the reading that the **call-argument**
position had been closed deliberately by #776. It has not: the argument position builds
correctly on both backends on current `main` (verified `parts == ["hi "]`, `values ==
["ada"]`, not merely that it type-checks), so the doc's list was accurate except for the
return. #776's narrowing was of the HOLE channel, not the argument channel, and that gate
still holds — `q: Query = "{"seed"}"` passes `"seed"` to `hole_text` as a value rather
than building a second accumulator.

Guards: `tests/scripts/interpolation-hook.loft` grows `built_as_tail` / `built_as_return`
/ `built_in_branch` beside the existing `seq_of` argument-position case, asserting the
call SEQUENCE (`lit(t)>int>lit(u)`) rather than the result — a target that only checked
the final string could not tell the hook from ordinary formatting. Both backends.

### A tuple match arm that consumes nothing is a parse that never ends (#832) (2026-08-10)


`parse_tuple_match`'s arm loop could iterate without consuming a token, and then it
never stopped. `(first, ..)` reached the element loop's literal branch, where
`expression` took the `..` and left the `)` unclaimed; `expect_match_arm_arrow` then
found no `=>` and called `recover_to(&[",", "}", ";"])`, which **resynchronises** and
returns WITHOUT consuming when the cursor already sits on a stop token or an unmatched
closer. The arm re-parsed the same token forever — 2.1 million iterations in four
seconds — and silently, because first-pass diagnostics are suppressed. loft is
unbounded by default, so nothing bounded it.

The filed scope was `..`; the matrix widened it. **An over-arity pattern hangs
identically** (`(a, b, c, d)` on a three-element tuple): the element loop stops at the
subject's arity and leaves the cursor on the surplus `,`. Junk arm heads (`1`, `"x"`,
`[1,2,3]`, `{ }`) recover fine, because `recover_to` scans forward from them and does
consume — which is what made the boundary look narrower than it was.

Three changes, one invariant — *every arm-loop iteration consumes at least one token*:

- `..` / `..=` is refused **by name** in an element position, with the supported form in
  the message, then skipped to the closing `)`. Arity is fixed by design (TUPLES.md
  § "What is NOT supported"), so a rest has nothing to stand for.
- A pattern longer than the subject reports the subject's **arity** rather than a bare
  "expected ')'", and the surplus is skipped so the arm reaches its `=>`.
- A `bad_pattern` flag keeps a refused arm from being classified as a **wildcard**. A
  refusal binds nothing and tests nothing, which reads exactly like `(_, _, _)` — and a
  wildcard arm ends the arm loop, so a rejected FIRST arm swallowed every arm after it
  and reported a missing `}` instead of the refusal. This is why `(.., last)` and `(..)`
  behaved differently from `(first, ..)`, which binds and so escaped the misclassification.
- The element loop's missing-comma `break` is no longer gated on `!first_pass`. Both
  passes must walk an arm the same way, or the first wanders into positions the second
  never visits.
- A backstop compares `lexer.at()` across the whole iteration and breaks if nothing moved,
  so an unknown shape ends in a diagnostic rather than a stuck build.

Two adjacent defects found by the first tuple-element-pattern coverage the corpus has
ever had (`28-tuples.loft` carries no `match`, which is why the hang shipped), both
filed rather than fixed here:

- **#839** — an `if` guard never parses on a vector or tuple arm: those two loops call
  `has_keyword("if")`, which matches only `LexItem::Identifier`, while `if` lexes as a
  token; the three working match kinds use `has_token`. Swapping it in was tried and
  reverted: the guard then parses and the arm silently does not match, because captures
  are assigned in the arm BODY and the condition runs first, so the guard reads an
  unassigned variable. A clean refusal beats a silent wrong answer; both call sites
  record why.
- **#840** — a tuple **parameter** with a `text` element fails rustc on `--native` when
  it is the match subject: the `match_tuple` temp is spelled with the owned type
  (`String`) and initialised from the borrowed parameter (`&str`).

Guards: `tests/scripts/832-tuple-pattern-refused.loft` (every `..` position plus
over-arity, asserting the REJECTION — a timeout-only test would pass for the wrong
reason) and `832-tuple-pattern-elements.loft` (the positive twin; a fix that rejected
every tuple pattern would satisfy the first alone). Both backends.

### One SQL boundary closes: a table loft made and a table loft found are the same value (@PLN133) (2026-08-08)


@PLN133's gate passes, on **four database backends and both loft backends, with
byte-identical output in all eight cells**. Write a struct graph through the derived
`INSERT`, bind a collection lazily to the SAME connection string, traverse it, and
get back the values, the identity across two paths, and the trip counts laziness
predicts. **Run twice** — once into an empty database where loft writes the schema,
once into a table made by hand with a different column order, the float kept in a
`VARCHAR`, and an extra column loft knows nothing about. Only the second run proves
requirement 3; the first passes even against a `reconcile` that always agrees.

- **S11 + S12 are ONE call.** `ensure(d, dial, want)` is the whole absent-or-present
  decision, because it is one decision — splitting it puts the test in every caller,
  which is where two callers eventually disagree about what absence means. Absence is
  decided by ASKING THE CATALOGUE, never by an `IF NOT EXISTS`: the rule *loft never
  touches a table it did not find missing* belongs in loft's code where it can be read,
  not in an engine's tolerance for a repeated `CREATE`. After creating, it reads the
  table BACK and reconciles against what the engine actually stored — mariadb turns
  `BOOLEAN` into `tinyint`, so reconciling against the derivation would assert the
  round trip instead of testing it.
- **`introspect` now reads all four catalogues**, which is what S12 needed and what S6
  had deferred with a scope statement. The columns half unifies on
  `information_schema.columns` (scoped by an expression the `Dialect` carries); the
  INDEX half does not and is not pretended to — `information_schema` has no index view,
  so PostgreSQL answers from `pg_index`, mariadb from `information_schema.statistics`,
  and duckdb hands back the `CREATE INDEX` TEXT rather than a row per column. Every
  query was RUN against a live server of its engine before it was written down.
  Two things a guess would have got wrong: PostgreSQL's `indkey`/`indoption` are
  **0-based** `int2vector`s, so `indoption[ord-1]` reads every direction as NULL; and
  the type mapping is a WHITELIST rather than sqlite's substring test, because sqlite
  has affinity — a rule the engine itself applies — while PostgreSQL's `point`
  merely contains `INT`.
- **S13's statement is derived, its values are not.** `insert_row` renders the writer's
  `INSERT` from the same `TableDef` the reader's `SELECT` comes from, so they cannot
  drift. The generic walk from an arbitrary struct's fields to those values is NOT
  built and cannot be here: loft's reflection reports types, not values.
- **S10's deletion is REFUSED, and that is the finding.** Deleting core's Rust sqlite
  path makes a driver mandatory for `sqlite:`, a driver names a concrete element type
  so it cannot be generic, and `store_bind_lazy(c, "sqlite:x.db")` needing no user code
  is a shipped promise. The alternative — core synthesising a driver that calls the loft
  library — makes a fixture a dependency of core. So S9's precedence rule IS the answer:
  a demotion, not a deletion. What it buys is not deletion but a stopped clock, which
  was the plan's actual complaint: N=4 backends now and **+1 forever**. The +1 is gone.

Found on the way and filed rather than absorbed: **[loft#813](https://github.com/loft-lang/loft/issues/813)** —
a value whose static type is a struct-enum VARIANT (`x = AsA { … }` rather than
`x: Any = AsA { … }`) is accepted where a bounded generic wants the ENUM and then
answers the type's empty value. Silent on `--interpret`, a `todo!()` panic on
`--native`, a SIGSEGV with two generic hops.

### A buffer that is already a reference is handed over, not wrapped (loft#806) (2026-08-08)


`return t.m(i) ?? "x"` SIGSEGV'd the interpreter while `--native` answered correctly.

`OpCreateStack(v)` is how a variable that OWNS its text hands out a reference to it.
The call site that fills a callee's hidden `&text` return buffer picks that buffer BY
NAME (`__work_cN`, so the two passes agree on it), which means it never looked at the
variable's TYPE — and the name can already belong to a `&text` PARAMETER of the calling
function, because `text_return` promotes a text local the return value depends on into a
caller-allocated buffer (loft#662). Wrapping it again built a DbRef pointing at the
reference SLOT; the callee's single deref then read that slot as text. `--native` passes
the reference by the Rust ABI and is immune, which is why the two backends disagreed
instead of both crashing.

This is the rule #266 already states for non-text references at the argument-coercion
site. That site compares TYPES, so a variable already holding the wanted reference never
reaches its conversion at all — which is why only this one, keyed on a name, could reach
the double wrap.

**The filed boundary was a tenth of the defect.** A 20-cell matrix over the composition
axes put 8 cells on the crash, and two of the three conditions the report listed as
required are not:

| axis | filed | measured |
|---|---|---|
| enclosing function returns `text` | required | `text?` crashes too |
| fallback is a non-empty literal | implied | `?? ""` crashes too — the buffer-append is skipped, so the wrap alone is the fault |
| receiver is a plain variable | implied | a field read (`h.inner.m(i)`) crashes too |

The passing cells are not incidental: a free function, an intermediate local, an extra
text parameter and interpolation all avoid it, and each does so by changing which
variable the promotion picks. An attribution pass over the IR — `OpCreateStack` applied
to a var the same function declares `&text`, keyed on the (name, scope) PAIR because
`n_main`'s plain local and a callee's promoted parameter routinely share a `__work_cN`
name — flags exactly the 8 crashing cells and none of the 12 passing ones, before and
after.

The fix hands over the BARE variable. Both backends already forward a `RefVar` argument
into a `RefVar` parameter with no deref (`codegen.rs` `OpVarRef`; `generation/calls.rs`
`var_x`), and both recognise that shape only as a literal `Value::Var` — wrapped in a
block or an `Insert` the generic path runs instead and re-derefs, which is a second wrong
read one layer down rather than a fix. The per-call clear is not lost: a promoted buffer
is cleared by the function preamble once per invocation, and promotion only happens when
the RETURN VALUE depends on the buffer, which puts its call in tail position. Restricted
to the no-default case, so a `&text` parameter carrying a `= "…"` default keeps its
existing lowering rather than trading this crash for a dropped default.

`tests/scripts/issue-806-retbuf-double-reference.loft` carries the axes as value
assertions — a wrong read here is as likely to answer `""` as to fault, so "did not
crash" is not the bar — plus the forwarded-buffer control that says the fix did not
simply delete the wrap everywhere.

### A method's return was adopted by one half of the compiler and freed by the other (loft#810) (2026-08-08)


Filed as a SIGSEGV needing a library, a `vector` local, a foreign package's record type
and a loop-body local — six ingredients, drop any one and it ran. None of them is the
defect. It is **one word in a predicate**, it needs no library at all, and its ordinary
outcome is a WRONG VALUE rather than a crash.

Binding a call's result to a heap local asks one question: may the caller ADOPT the
returned store, or must it COPY into a store the binding owns? Cluster A already collapsed
the ANSWER into one carried fact (`Def::return_adopts_fresh_store`). What had drifted was
the GATE on it — which callees the question is even asked about:

| site | decides | accepted |
|---|---|---|
| `scopes.rs` (`scan_set`) | strip the binding's deps → emit its scope-exit `OpFreeRef` | `n_` **and** `t_` |
| `state/codegen.rs` (first-Set) | deep-copy, or adopt | `n_` only |
| `state/codegen.rs` (reassign) | deep-copy, or adopt | `n_` only |

So a `t_` METHOD returning through the caller's hidden `__ref_N` buffer — the shape every
`q: Acc = Acc { }; …; return q` compiles to, dep `["q"]` — fell past both copy arms to the
plain-adopt fallthrough, and was then freed at scope exit as if the binding owned it. The
buffer's store went back to the pool while the caller still named it. Next iteration the
callee's own work-ref drew that slot, the retbuf `OpDatabase` re-`claim`ed it at the
original name, and one record had two owners.

`Def::is_loft_defined()` is now the single home for that gate, next to the two facts it
guards, and the six spelled-out copies of it (scopes ×3, codegen ×2, parser ×1) read it.

**What the boundary actually is**, measured one axis at a time
(`tests/scripts/810-method-return-buffer.loft`):

- No library, no foreign package, no vector: a single file reproduces it.
- The callee needs ONE competing allocation between the free and the next iteration's
  re-adoption — otherwise the freed slot is handed straight back and nothing shows.
- Whether it crashes at all depends on what lands in the recycled slot. The reported case
  hit a record header and died in `memcpy`; the plain shape silently loses a vector and
  answers a plausible count. **The value cells are the test**, not "did not crash".
- Three cells, not one: first-Set in a loop body, reassignment of an outer local, and
  assignment inside a nested block — the last two go through the OTHER codegen site, so
  fixing only the first leaves two thirds of the defect standing.

`--native` was never affected and needs no change, for a reason worth writing down rather
than trusting: its Reference-typed `__ref_N` is a `DbRef::NULL` sentinel passed BY VALUE,
so the callee always allocates fresh and the caller's copy stays null. The alias the
interpreter formed cannot form there. Its vector retbuf (`__vdb_N`) IS caller-allocated,
and that path was already right — pinned as a control.

The store guards that came out of the first pass at this stay, and earn their keep: a
non-positive size word makes every whole-payload walk compute `size * 8 - 4`, which wraps
to ~18 exabytes and dies inside `memcpy`, naming the copy — which is innocent. `assert!`
and not `debug_assert!` on purpose: a debug build already catches the underflow, and it is
the RELEASE build, where the wrap is silent, that needed one. `Store::copy` /
`Store::zero_fill` route through one `payload_bytes` helper; `vector_append` asks first
whether the field is inside its own record at all, and now says what that means — a slot
with two owners, with `LOFT_NO_SLOT_REUSE=1` as the one-run test — rather than the layout
fault it first read as. `Store::valid` bounds a field with `fld <= size * 8`, admitting a
read that starts exactly AT the record's end, and is a `debug_assert!` compiled out of the
profile the loft library builds under; that is why nothing caught this earlier.

`LOFT_TRACE_DB` now prints from the native runtime too. It had existed only in the
bytecode VM, so it went silent exactly where a call crossed into a package's shared
library — which is where the slot adoption it exists to show was happening. Both backends
read one cached key.

### A persisted trie is laid out so it can be paged (@PLN134) (2026-08-08)


`trie<T[k]>` ships whole-image only: a prefix query over `routing`'s 220 032-word
vocabulary is 5.9 MB gzipped, downloaded once. @PLN134 asked whether a paged reader could
answer it in a few range reads instead, and opened on the measurement that decides it —
**pages touched, not nodes**.

The first answer killed the cheap design. A PATRICIA descent reads ~330 bytes of nodes and
spreads them over **27 pages of 64 KB**, because node ids are handed out in INSERTION
order and a root→leaf path visits nodes created at wildly different times. Renumbering
breadth-first halves it and stops there. 1.7 MB to answer a keystroke is worse than the
download by the fourth one.

The second answer is the plan's own declined branch, reached on evidence: it is not "page
a trie", it is **lay a trie out so it can be paged**. Same tree, same walk, same touch
sets, five numberings:

| node order | pages @ 64 KB | @ 4 KB |
|---|---|---|
| as built | 27.1 | 36.4 |
| breadth-first | 15.4 | 26.0 |
| key order | 8.7 | 14.5 |
| depth-first pre-order | 4.2 | 7.2 |
| **van Emde Boas** | **2.8** | **3.8** |

The 4 KB column identifies the mechanism rather than the number: vEB barely moves where
every other order inflates by half. That is what cache-*oblivious* means, and it matters
here because the page size is not ours to pick — a local file, an HTTP range read and a
browser cache disagree about it.

The records matter more than the nodes, and step 1 had not measured them at all: the 20
records a query RETURNS sit on ~20 distinct pages when claimed in insertion order, and on
**1** when written in trie key order. Together a cold query is ~3.8 pages / 250 KB against
the 5.9 MB image, and the second keystroke of a session costs ONE page.

- **`radix_tree::rtree_relayout`** renumbers a tree van Emde Boas and compacts the free
  list. Node ids are internal, so nothing observable moves — `r11` holds it to the same
  walk and the same record for every key, which is the gate that matters: rewriting the
  array in place produces a structurally valid PATRICIA tree holding the wrong records,
  and `rtree_validate` alone passes that. Idempotent, and it refuses a tree whose walk
  does not account for `n-1` nodes over `n` records.
- **`store_persist_bind` runs it before writing the image** (`Stores::relayout_tries`),
  because that image is what a reader pages. Stores whose SCHEMA cannot hold a trie skip
  the data walk entirely, so no other kind pays for it.
- The measurement lives on as `trie_db::pages` (`#[ignore]`, three tests: the layouts, the
  record placement, the warm session) and `r10` asserts every candidate order is a
  permutation of the live nodes — not decoration, since a duplicate-emitting order reports
  a BETTER page count.

Paging a trie is still unwired: `store_bind_lazy` refuses one, and `store_load_key_text`
reads a `hash`. The layout is the prerequisite that made those worth building.

### sqlite down the loft path, measured against the Rust one (@PLN133 S9, 2026-08-08)


Core drives sqlite in Rust (913 lines across `sql_source.rs` and `sql_query.rs`)
and the loft library drives four backends behind one `SqlDb` interface. The step
is *"switch sqlite to the loft path"*, and taken literally it cannot preserve what
it must: every @PLN129 test binds `sqlite:` with NO user code, and
`store_bind_lazy(persons, "sqlite:people.db")` needing no loading step is a
shipped promise.

So it is an opt-in with a measurement:

- **A declared driver WINS**, including over a source core drives in Rust. A
  program moves its sqlite reads onto loft one element type at a time; every type
  with no driver keeps the Rust source. `Stores::lazy_loft_source` now takes the
  caller's answer to *"is there a driver for this element type"*, because the two
  backends learn it differently — the interpreter asks `Data`, `--native` asks the
  table generated `init()` filled — and neither is reachable from `Stores`.
- **The two paths are proven indistinguishable.**
  `tests/fixtures/sqldb/s9_two_paths.loft` puts two element types of one shape
  over two identical tables in ONE program bound to ONE connection string. Same
  values, same float, same identity, same residency counts, same absence handling
  — and the trip count, which is the only thing a value check cannot see: three
  lookups reach the driver and the repeat of a resident key reaches none. Both
  backends, byte-identical.
- **`select_by_key`** — the `select(TableDef, key)` the design table always listed
  — derives the statement from the same `TableDef` a writer would `render` into
  `CREATE TABLE`, wrapping a float column in the dialect's read expression. The
  driver names no column.
- **`Data::lazy_fetch_drivers` is cached** per definition count. It walks every
  definition and sits on the MISS path, which is the one place @PLN129 measures in
  queries per lookup. Keyed on the count rather than answered once, because the
  REPL parses fresh sources into a live `Data` and a driver can appear after a
  lookup has already asked.

**The cost, attributed rather than assumed.** A loft driver has nowhere to keep a
connection — loft has no process-level state a library can hold — so it connects
per missed row where core caches a handle per target. Release build, 400 fetches
each: **67 µs** per fetch through Rust, **140 µs** through loft. ~2.1×, because a
local sqlite file reopens cheaply. What that does NOT cover is the case that
matters most: for a client-server backend the same shape is a TCP connect and an
auth per row, and those are precisely the backends core has no Rust driver for.

**S10 is not unblocked by this.** Deleting the Rust path makes a driver
mandatory, and a driver names a concrete element type so it cannot come from a
library — a program binding `sqlite:` with no user code would stop working. That
needs a generated driver (making the sqldb library a dependency of core) or a
demotion rather than a deletion, and it is a decision about what loft's
distribution contains.

**Filed on the way past:** [loft#810](https://github.com/loft-lang/loft/issues/810)
— a library function that both holds a `vector` local and returns a record of
another package's type SIGSEGVs on the second call when the caller binds the
result to a loop-body local. `Store::copy` computes `size * 8 - 4` from a record
whose size word reads `0`. Six axes were moved one at a time to find the
boundary; the driver takes the passing cell (a fresh `derive` per fetch).

### A lazy driver serves ONE element type (@PLN133 S9 prerequisite, 2026-08-08)


S8 let a program declare one `lazy_fetch`, which reads as a limit on how many
collections may be lazily bound. It was not only that: **nothing checked that the
driver a miss reached was declared for THAT collection.** S8's shape check was
about the driver's signature and never about its subject, so a program with two
lazily-bound element types ran the first type's driver against the second
collection — measured on both backends, inserting a `TdcPerson` into a
`hash<TdcOrder[id]>` and reading `.what` back as `person-9-postgres://db/people`.
One type's field through another type's offset: a plausible value, which is the
class @PLN129 arc C exists to keep out.

One mechanism does both jobs — the driver is looked up by the collection's
ELEMENT TYPE, so several drivers become possible and reaching the wrong one
becomes impossible.

- **`Data::lazy_fetch_drivers`** answers `(element type name, def_nr)` per driver
  and is the single home both backends ask. What a driver serves is read off its
  declared collection parameter, never guessed from its name.
- **The key is a NAME**, because the two sides count types in different spaces (a
  parse-time `Definition`, a runtime `Stores::types` entry) and a name is the one
  key both hold without a mapping to keep in step — `LOFT_STRICT_SCHEMA_IDS`
  exists because that kind of mapping drifts.
- **Membership needs more than the name.** `lazy_fetch` exactly is THE driver
  name, so a wrong shape there is named; `lazy_fetch_<anything>` additionally
  requires a keyed collection as its first parameter. The first version of this
  rule keyed on the name alone, and a plausible helper (`lazy_fetch_row`) was then
  read as a malformed driver and poisoned every lookup in the program, including
  the working driver beside it.
- **Two drivers for one element type are refused, naming both.**
- **`--native` installs one pointer per driver** under the same key, and every
  driver is a reachability ROOT — a driver left out of the walk is S8's quiet
  failure arriving once per type instead of once per program.

**A backend divergence had to be closed to gate the refusals, and it was S8's.**
The interpreter asks `Data` at every miss and reports the sentence it wrote;
`--native` cannot ask, registered nothing, and said *"needs a loft driver"* — the
same program naming a different mistake depending on which backend ran it, and
the one naming the real mistake was the one you did not get if you compiled. The
refusal now travels as data (`register_lazy_fetch_refusal`) and the no-driver
sentence has one home (`database::lazy::no_lazy_driver`).

**The emission diff is one line.** `loft introspect` over the two-driver corpus
before and after differs only in the registration — one
`register_lazy_fetch(n_lazy_fetch)` becoming two keyed calls — with nothing else
in the IR, the bytecode or the generated Rust moved. Corpus and both captures:
`doc/claude/plans/133-sql-one-boundary/bytecode-comparisons/two-drivers-*`.

Gated by `tests/fixtures/133-lazy-driver-dispatch.loft` (three element types over
`hash` and `index`, a fourth bound with no driver, a prefix-sharing helper,
absent-vs-unreachable) plus two refusal programs, through
`tests/lazy_sql_source.rs`, both backends with the whole output compared. The
`orphan` cell asserts a driver-call COUNT rather than a value: a collection whose
type no driver serves must reach none, and a value check alone would pass on a
driver that happened to answer nothing.

### One connection string, four C libraries (@PLN133 S7, 2026-08-08)


Requirement 1 is *one configuration string switches every SQL consumer in the
process*. S5 delivered the parser; this is the half that hands back a connection,
and it had no obvious spelling because **loft interfaces are static dispatch** —
`SqlDb` is satisfied by four unrelated types and no function can return "one of
them".

- **`tests/fixtures/sqldb/registry/`** — `AnyDb`, a struct-enum over the four
  backends plus a refusal variant, satisfying `SqlDb` itself. `connect(spec)`
  parses the string, asks whether that backend's library is on this machine,
  opens it, and runs the dialect's session setup. Shape (1) of the three the plan
  named; the decision and the two it was chosen over are recorded in the file's
  header, because (2) is cheaper and the difference is visible to every consumer.
- **The method must be on the ENUM, not the variant.** A per-variant method
  dispatches correctly and does not satisfy an interface for the enum — the
  compiler says so: *"'AnyDb' does not satisfy interface 'SqlDb': missing
  db_exec"*. Fifteen `match self` forwarders, none of which decides anything.
- **A refusal is the fifth variant, not a null** — every operation false, every
  column null (not `""`, which is a value), `db_last_error` saying why. The idiom
  `TableDef`, `Binding`, `Conn` and `SqlText` already use in that package.
- **The connection string is not one string.** What a driver's own `db_open`
  wants is a fact about the driver: sqlite and duckdb take a path, libpq reads a
  URI itself (so it must arrive WITH its scheme), and mariadb's client takes
  keywords, so a `mysql://…` URL is translated. A PORT in that URL is **refused
  rather than dropped** — the driver connects on 3306 and reads no port, so
  honouring the string would reach a different server than it names.
- **The session setup finally has somewhere to run.** `Dialect.setup` has carried
  PostgreSQL's `SET extra_float_digits = 3` since S3 with no caller. @PLN133 P3
  measured 1887 of 2000 random doubles inexact without it and 0 of 2000 with it,
  and it is a SESSION setting — so the connect is the only place that can make a
  float read back exactly, and nothing downstream can see that it did not.

Gated by `tests/fixtures/sqldb/registry_pure.loft` (unconditional — it opens no
library, so it cannot skip into a green that asserted nothing) and
`registry_live.loft`, through `tests/native.rs`, both loft backends with the whole
output compared.

### `τ?` is one type however it was handed back (2026-08-08)


`Type::is_same` compared `Optional(τ)` with derived `==`, which reaches the inner
`Deps`. Every dep-ignoring rule below that comparison — a text's deps, an
integer's range, a vector's element buffer — was therefore unreachable for a
nullable, so two `text?` differing only in which local they came through read as
different types. It presents as a refusal quoting the same name twice:

```
error: cannot unify: text? and text?
```

Peeled on BOTH sides only, so a `τ?` and a bare `τ` stay different kinds — that
distinction is the whole of DN1. Found by @PLN133 S7, where a `match` forwards
`db_col` to four backends and one of them returns through a local; the same
comparison also gates interface satisfaction and the @P344 loop-variable reuse
check, both of which wanted the dep-insensitive answer all along. Guarded by
`tests/scripts/pln133-optional-unify.loft`.

### A returned text is owned by the return, not borrowed from a buffer (2026-08-08)


A branch in tail position delivers each arm into the return accumulator. An arm
whose text is not a bare variable is first built into a work buffer and handed
back as `OpCreateStack(buf)` — a REFERENCE — and `push_text_arms_into` wrapped
that reference in the delivery, while the enclosing scope frees the buffer on the
next statement.

- **The interpreter answered `""`** — a wrong value, silently, exit 0.
- **`--native` emitted `*var_acc = ().to_string()`**, which is not Rust.

The shape is ordinary: `return x ?? "fallback"`. Binding to a local first
(`y = x ?? "fallback"; return y`) avoided it, which is what made the failure look
like it was about `??` rather than about delivery. The leaf rewrite now delivers
the BUFFER, so the accumulator copies the bytes it is about to own. Guarded by
`tests/scripts/pln133-text-tail-delivery.loft`, whose every cell has a
deliver-through-a-local twin.

Found not by the registry — which does not contain that shape — but by the
regression test written for the `Optional` fix above, whose helper happened to
spell `return got ?? "<null>"`.

**Still open:** [loft#806](https://github.com/loft-lang/loft/issues/806) — a
METHOD call coalesced in return position (`return t.m(i) ?? "x"`) SIGSEGVs the
interpreter while `--native` is correct. The caller-retbuf promotion makes the
callee's work buffer a `&text` PARAMETER and the `#default ref` site then wraps an
already-borrowed variable in `OpCreateStack`, building a reference to a reference.
Workaround: one intermediate local.

### `#c` on a wasm target, and under the sandbox (@PLN24 arcs E–F, 2026-08-08)


Closes @PLN24. Both remaining arcs plus the plan's last open design question.

**Arc E — the two wasm targets get a defined answer, and it is a refusal.** The
plan had recorded wasm as having "no C ABI to bind to at all". It has one:
`wasm32-wasip2` links a libc, so a `#c` binding to `strlen` resolved, LINKED with
a `rust-lld` warning, and then TRAPPED at the call — `signature_mismatch: strlen`,
`(i32) -> i64` against the sysroot's `(i32) -> i32`, because wasm32 is a third
data model (ILP32) while the extern carried the host's widths from
`CTarget::host()`. That is this plan's counted `N × silence` risk arriving at a
re-assertion site nobody listed: one of the targets is not the host.

Two further cells, both measured on one tree: a symbol the sysroot does NOT export
gave a raw `rust-lld: undefined symbol` naming neither package nor library, and a
package declaring `[c] optional-libs` gave `E0433: cannot find c_call in loft`
once per symbol — **for bindings the program never called**, because the lazy
resolver is emitted per declaration rather than per call.

- **Nothing `#c` is emitted on a wasm target** — no `extern "C"` block, no lazy
  resolver. `Output::no_c_abi()` is the single reader of the two target flags, so
  the three sites that consult it cannot drift into different answers.
- **The refusal sits at the CALL** (`output_c_direct_call`), which scopes it to
  reachability for free: an unused `#c` declaration still builds for wasm, the
  rule `#native` already follows for a routeless browser symbol (@PLN26 / P269).
  It names the loft function, the C symbol, the declaring package and the target.
  The PACKAGE, not the library: a `#c` annotation never names the library it came
  from (arc G), and one of a package's `[c]` entries is the shim loft built itself.
- **`__C_LIBS` / `__C_LIB_SYMS` moved OUT of the target gate.** They were emitted
  only on non-browser targets, so `c_library_available` — the query a library is
  told to ask before calling into an optional backend — failed to compile under
  `--html` with `E0425`. A refusal that names a cure has to leave the cure
  reachable. It now compiles on both wasm shapes and answers `false`, which is the
  true answer rather than a stub.
- The static-`clang --target=wasm32-wasi` route stays unbuilt and is recorded as
  such: no C cross-compiler was available to prove one cell, and it reaches only a
  pure-computation shim. `@PLN119` (out-of-process) is the route the message names.

**Arc F** was already closed by @PLN23 S1 (`libmariadb.so.3` through a versioned
soname, both backends identical, zero rustc); the plan's status table said
otherwise.

**Open question 3 — a `#c` binding is gated by `native_ffi`, not by `#cap`.** The
question asked whether an effect declaration could make `#c` admissible under the
sandbox. Measured first: a sandboxed script reaching a `#c` binding tagged
`db#read`, under a profile granting `db#read` with `native_ffi` at its default
false, was **admitted and ran the C**. Both the external-FFI ban and
`reachable_ffi_bridges` key on `def.native()`, which arc A leaves EMPTY on a `#c`
definition on purpose so the Rust dispatch path cannot claim one — the inverse of
arc D's three defects, where paths matching on *body-less* wrongly CLAIMED a `#c`
def. `CapViolation::CBinding` is the new arm; `allow_libs` still admits it, which
is the host vetting the library as a unit exactly as for `#native`.

Guards: `a_c_binding_is_refused_by_name_on_a_wasm_target` (emission-level, so it
runs without a wasm toolchain, and it calibrates against the host emission so a
refusal that fired everywhere could not read as a pass),
`pln24_a_reachable_c_binding_is_refused_end_to_end_on_wasm` (both shapes, asserts
exactly ONE message and that loft's own feature gates never reach the author),
`pln24_html_c_library_available_compiles_and_answers_false`, and
`a_c_binding_is_gated_by_native_ffi_not_by_a_capability_grant` (three cells:
granted cap rejects, `native_ffi = true` admits and calls, `allow_libs` admits).

### A module may name the entry's type in an EXPRESSION (loft#801) (2026-08-07)


Companion to loft#797, which fixed the LAYOUT half of the same load-order story. This is
the resolution half.

A forward reference resolves through a **stub**, not a lookup: an unresolvable type name
becomes an `add_def(name, …, DefType::Unknown)`, `use` imports it into the importer along
with the module's other names, and the importer's own `struct` / `enum` / `type`
declaration upgrades it IN PLACE so both files share one def. `Data::def_nr` is keyed on
`(name, source)` with only a source-0 fallback, so there is no cross-source lookup at all —
adoption is the entire mechanism. Documented in COMPILER.md § How a forward reference
actually resolves, because nothing said so.

The consequence was that only a spelling which LEAVES a stub could be forward-referenced.
Written types go through `parse_type`, which leaves one; expressions did not. So
`r: Roofs = Roofs { … }` compiled and the identical `r = Roofs { … }` did not — the same
name, the same file, decided by whether an annotation happened to be written.

- **Two sites in `parse_var` now leave the same stub** — the `Name { … }` construction
  branch and the bare-name branch. Both pass 1 only. They are tracked in
  `speculative_type_refs` so `resolve_deferred_unknowns` stays quiet about an unadopted
  one and the construction site still reports in pass 2 with the author's own spelling and
  its suggestion — reporting both is the one-typo-two-errors cascade #376 removed.
- The bare-name branch also stops creating a placeholder VARIABLE for such a name. A
  function's variable table survives into pass 2, so the pass-1 placeholder was still
  there when pass 2 looked the name up, and it shadowed the type the declaration had
  meanwhile produced.
- Its name test is deliberately NOT `is_camel`, which answers "not lower_case and no
  underscore" and so accepts `FOO`, `N`, `X`. Treating those as types took the placeholder
  variable away from every misspelled constant — the `upper-case-local` advice and
  `Unknown variable 'N'` are written against it. A type name carries a lowercase letter.
- **`parse_typedef` adopts a stub**, which `parse_struct` and `parse_enum` already did. It
  was reporting the waiting stub as a name clash, so a typedef was the one declaration
  kind a module could not forward-reference.
- **`parse_file` drains `todo_files` on a plain Error**, stopping only on Fatal. That list
  holds the files SUSPENDED at a `use` — the importer, waiting for the module it pulled in
  — and they had not been parsed at all, so abandoning them did not avoid a cascade, it
  invented one: the definitions they carry never registered, and one error in a module
  produced a second saying a type was undefined while it was declared two lines away in
  the importer.

Fixed spellings (both backends): a local built by construction alone, a vector literal,
iterating that vector, the type as a value argument (`sizeof(T)`), and a typedef.

**Not fixed, and deliberately excluded: a bare name qualifying a VALUE (`Colour.Green`).**
It fails in a single file too, so it is not a module problem, and it is loft#803. Leaving
the stub there makes the program COMPILE and evaluate to `unknown` for every variant — a
wrong answer where there had been an error. That issue records two further attempts (an
enum-aware `layout_blocked`; registering enum stragglers from `fill_all`) and exactly what
each broke. Read it before patching.

Guards: `module_names_the_entry_type_in_an_expression` (`tests/issues.rs`) over the
`fwd801` fixture, and golden case `47_module_error_keeps_importer`, whose baseline pins the
ABSENCE of the invented second error.

### A field whose type another module declares gets a slot (loft#797) (2026-08-07)


A package entry that `use`s a module before declaring the types that module names
suspends itself at the `use`. The module was then parsed to completion — layout
included — while every such type was still a `DefType::Unknown` stub. `fill_database`
skips a field whose `type_elm` is `u32::MAX`, so the field got no slot; the stub was
upgraded in place moments later when the entry resumed, so the DECLARATION ended up
correct and the LAYOUT kept the hole, and nothing revisits a registered type. Only
the load ORDER decided it.

`fill_all` now defers a layout until every field's type is known, and re-asks on each
call. Three sites had to agree:

- **`layout_blocked`** (was `has_nameless_unknown_attr`) — covers `Unknown(stub)` as
  well as `Unknown(0)`, and is TRANSITIVE: an inline field stores its content's bytes,
  so a host whose field type is waiting cannot be laid out either. The loop is keyed on
  `known_type == u16::MAX`, so this defers rather than drops.
- **A sweep at the top of `fill_all`** re-runs `copy_unknown_fields` over everything
  still unlaid. Without it the deferral never ends — `actual_types_deferred` sweeps only
  the file it is finishing, and nothing was asking again.
- **`Type::Optional` is peeled, not matched.** `S?` and `S` name the same forward
  reference, and three places that peel `Vector` had all forgotten the `?`:
  `copy_unknown_fields`, `Data::rewrite_type_opt`, and the native `init()` generator's
  field-hoist match. The last one emitted `db.field(t_host, "f", t_content)` ahead of
  `let t_content` — the generated crate did not compile, so a nullable forward field
  broke the library build where a plain one worked.

Also from the same matrix, both now diagnostics rather than panics: a keyed collection
whose content was a stub indexed `attributes[usize::MAX]` in `set_mutable`, and a vector
literal of a stub element tripped `new_record`'s `assert_ne!` as an internal compiler
error. The first is gone with the deferral; the second reports `type 'X' is not defined
here — use the module that declares it`.

Not closed, and out of scope: a type named in a function BODY rather than a field
DECLARATION is not deferred, so a module naming a type it cannot see still fails with
`unknown type 'X'`. That is resolution, not layout — filed as loft#801, together with the
cascade that makes it expensive (`parse_file` returns on error before draining
`todo_files`, so the suspended parent is never re-parsed and the type it declares is then
reported undefined).

Guard: `forward_module_type_gets_a_slot` (`tests/issues.rs`) over the `fwd797` fixture,
asserting sizes as well as values — a read follows whatever offsets the layout ended up
with, so reading a field back cannot by itself prove the field has storage.
`tests/field_without_storage.rs` (loft#796's guard) changes with it: the hole it used as
a trigger no longer exists, so both its tests now assert the ANSWER.

### Lazy stores — the fault is the collection's MISS path, and a SQL source drives it (2026-08-06)


`store_bind_lazy(c, source)` binds a collection to a store image or to
`sqlite:<path>`; a lookup that misses consults the source, materialises the record
and retries. Both backends. The model, the derivation and what is refused are in
[LAZY_STORES.md](LAZY_STORES.md); what matters here is where the hook went and why.

**Not at `Store::addr`, which counted better.** All 14 typed getters funnel through
it behind `valid()`, and the native `#rust` bodies call the same accessors, so one
site would have served both backends. But `valid()` is unconditionally `true` in
release — every check inside it is a `debug_assert!` — so the "one site" does not
exist yet, and creating it puts a branch on the hottest path in the language, paid
by every program. The hook is `Stores::find`'s miss path instead, which already
spells a miss as `rec: 0` and already has exactly two call sites (`State::get_record`
and `codegen_runtime`'s lookup), both holding `&mut Stores`.

**Residency needs no representation.** It is absence from the collection. No third
block state, no cost in `valid()`, and the resident set doubles as the cache — which
is what makes identity fall out of the ordinary lookup instead of a `(type, key) → rec`
map that could diverge from the store.

New modules: `database/sql_query.rs` (the derivation + `Mapping`),
`database/sql_source.rs` (the connection, through `c_call::resolve` with typed
`extern "C"` pointers — no rustc, no loft frame, no re-entrancy),
`database/lazy.rs` (the `LazySource` seam and the materialiser, which reuses
`record_new` + `record_finish` so a SQL arrival and `coll += [x]` end in the same
place).

Three findings worth carrying:

- **The derivation quotes every identifier**, because `from` is an ordinary loft
  field name and a reserved word everywhere. Quoting removes the class rather than
  one word of it.
- **SQLite reads an unresolvable double-quoted name as a STRING LITERAL** —
  `SELECT "naam" FROM "person"` returns the text `naam` once per row, so a renamed
  column would have been materialised into the record. The connection disables it
  (`SQLITE_DBCONFIG_DQS_DML`/`_DDL`); versions before 3.29 do not know the option,
  which is why the schema check is a requirement rather than a guard.
- **An `index` element carries its own red-black links** (`#left_1`, `#right_1`,
  `#color_1`), and `#color_1` is an ordinary boolean — so a column filter written on
  field TYPE named a column no table has. `LayoutField::is_data` now has one home,
  shared with `read_via_descriptor` and the browser delivery.

The tests need `libsqlite3` at runtime and self-skip without it, which is a skip
that reads as a pass. CI now installs it and sets `LOFT_REQUIRE_SQLITE=1`, which
turns the skip into a failure; elsewhere the skip is recorded in the
environmental-skip ledger and surfaced as an annotation. `tests/lazy_sql_source.rs`
also serialises on one mutex: `c_call::register` REPLACES the declared-library list
with the running program's own, so a test that merely runs a loft script was wiping
its neighbour's sqlite declaration.

### The IR store holds a block BY REFERENCE — `Node` shrinks 48 → 28 bytes (2026-08-04)


`NdBlock` / `NdLoop` inlined a whole `Block`, and `NdParFor` a whole
`ParForBody`, so a `Node` record was as wide as its largest variant: 48 bytes,
paid by every node in the image including a 12-byte `NdVar`. They hold a
**box-of-one vector** now — the idiom the schema already uses for `Block.result`
and `DbField.default` — and the stride is 28.

A box is a 4-byte handle. `reference<Block>`, which `ir.loft` had drifted to,
generates a 12-byte `Parts::DbRef`: the same indirection for three times the
width, with no other reader of one in this store and no existing helpers. The box
reuses `field_recvec` / `push` / `get` unchanged.

What moved together, because a half-done version of this is a store that reads
its own records at the wrong offsets:

- `ir.loft` → regenerated `ir_schema_gen.rs` (the field is a vector handle now).
- `data_store.rs`: `NDBLOCK_BLOCK` / `NDPARFOR_BODY` are the HANDLE offsets, and
  the sub-struct constants became the sub-record's own — `PARFOR_X_VAR` is 0, not
  `body_base + 0`. New `Node::block_rec` / `Node::par_for_rec` reach the record,
  and `write_block` / `write_loop` / `write_par_for` push the box and hand it
  back so a caller fills it without a second lookup.
- `ir_store` / `ir_read` / `ir_node` read and write through those records.
- `CACHE_FORMAT_VERSION` → 3: every offset in a `Node` moved.

The layout guard in `data_store.rs` is what made this safe to do — it asserts
each baked constant against the registered schema, so the migration was a
conversation with a failing assertion rather than a hunt for silent corruption.

### `ir_schema_gen.rs` regenerates byte-identically again (2026-08-04)


The IR store-schema generator had been unusable, so schema edits were HAND-ADDED
to the generated file — which is how it drifted out of sync with `ir.loft`
without anyone seeing. Two independent defects and one wrong declaration:

- **`tN` labels were absolute type ids.** `generated.rs` numbers types after the
  whole stdlib and `extract.py` copied those names verbatim, so adding ONE stdlib
  type renumbered every label and a fresh regen differed in ~1300 lines. They are
  only Rust locals, so the extractor now relabels ours in declaration order from
  `t7` (after the `t0..t6` base prelude). Proven: a binary WITHOUT
  `default/07_reflect.loft` and one with it now produce byte-identical output.
- **Named locals were dropped.** The keep-rule listed `byte_enum` and `vec_*`
  only, so a field whose storage local was `dbref_*` referenced a name nothing
  bound and the regenerated file did not compile — which is what forced the
  hand-adds. Every `let <name> = db.…` is kept now.
- **`ir.loft` described `src/data.rs` instead of the STORE.** It had drifted to
  `NdBlock { block: reference<Block> }` because `data.rs` boxes it; the store
  INLINES the block and the hand layer reads it that way
  (`NDBLOCK_BLOCK + BLOCK_SCOPE`). Regenerating from that produced a schema
  nothing could read — SIGSEGV in five IR round-trip tests. `ir.loft` says
  `Block` again, with the reason written beside it: making that field
  by-reference is a real store migration (schema, `ir_store`, `ir_read`, the
  baked offsets, `CACHE_FORMAT_VERSION`), not a transcription change.

The committed schema's CONTENT is unchanged — the regenerated file matches the
previous one registration for registration. What changed is that it is
reproducible, so the next schema edit is a regen rather than a hand-edit.

### @PLN127 arc D: reflection reports field nullability (2026-08-04)


`FieldInfo.nullable` — and the line it draws is the contract decision the plan
asked for: **reflection reports what a VALUE can be, not what CODE may do to it.**
Nullability is the first kind; `const` is the second and stays out.

Neither was a storage fact. `text?` and `text` share a content type and spell
absence with a SENTINEL, so nothing in the stored bytes implies either. (A NARROW
int is the exception — it registers a distinct content type per nullability, which
is why the descriptor reported nullable for those and only those.) The fact
therefore had to be DEPOSITED: `Field.nullable`, set at the one parse-time site
that knows (`typedef.rs`, where `Optional(τ)` is peeled before layout), carried by
`LayoutField`, and read back by `reflect_type_into`.

Nullability is deliberately **not RENDERED**. `layout_algo_hash` hashes
`layout_dump`, and `LayoutDesc::layout_hash` hashes `render_dump` — neither
mentions it, so the @PLN97 layout identity is untouched. That is measured, not
argued: a store written by the pre-arc-D binary loads under the arc-D binary
through both the whole-image and keyed paths with `ok=true`, and the same gate
still REFUSES a genuinely reshaped layout, so the check is not vacuous.

**`--native` reported `nullable=false` for every field** until the generator
emitted the deposit too — it rebuilds the schema by REPLAYING `init()`, so a fact
the parser deposits and the generator does not emit is simply absent there. The
parity probe caught it. The emission wraps `emit_field` rather than sitting in
either caller, because there are two call sites and the one that mattered was not
the obvious one.

The setter is keyed by NAME rather than field index for the same reason: the
generated `init()` writes it beside the `db.field` it belongs to, and one spelling
for both backends is what stops them disagreeing.

It also had to reach the @PLN11 IR-store round trip (`ir_store` / `ir_read` /
`DbField` in `tools/ir_schema/ir.loft`), or a schema read back from a store
answered "not nullable" for every field — caught by
`read_stdlib_schema_round_trips`. That grew `DbField` by a byte (stride 28 → 29),
which needed **`CACHE_FORMAT_VERSION` bumped to 2**: the stdlib cache key does not
fold in the binary's mtime the way a program bundle does (`BUILD_ID` is the git
HEAD hash, unchanged across uncommitted edits), so a cache written at the old
stride was read at the new one and panicked in `ir_read` on a shifted
discriminant — 25 LSP tests, all one cause. A layout change is exactly what that
byte is for. The registration is HAND-ADDED to
`src/ir_schema_gen.rs` beside the existing `ty_optional` hand-add rather than
regenerated: a clean `extract.py` run reorders that whole file today, so a regen
would fold an unrelated drift into this change. `ir.loft` carries the field, so
the source of truth is right and the regen cleanup stays its own task.

Arc E's generator is what made this concrete — written before arc D it was
complete, correct, and could not emit `NOT NULL`, which does not make a DDL less
detailed, it makes it accept rows the loft type would refuse.

### @PLN127 arcs C + E: `type_named`, and the consumer that used the API as a gate (2026-08-04)


`type_named(name) -> TypeInfo?` is reflection with no value in hand — the shape an
ORM needs when the type name arrives from a config file or a catalogue. No parser
intercept, because the name is a RUNTIME value; it works on `--native` because the
generated `init()` replays the type registrations, names included, and
`Stores::name` is a TOTAL lookup that answers absent rather than minting a type
for a typo. Both entry points reach ONE filler, so they cannot disagree.

**That is the plan's Q1 answered rather than worked around.** It expected a
runtime name→id lookup to be impossible under `--native`'s replayed type table;
the replay includes the names.

Arc E is the dogfood gate: `tests/scripts/pln127-reflect-consumer.loft` generates
`CREATE TABLE` from a loft struct through the API only, with the table name as a
runtime value. It passes on both backends, and used as a gate it found two limits:

- **It cannot emit `NOT NULL`.** Nullability is not in the answer because it is
  not in the STORE — `Field` carries a name, a content type-id, a position and a
  default. A narrow scalar records a nullable flag; `text` and a record reference
  spell absent with a SENTINEL instead (`text?` is stored as `"\0"`, the fact arc
  A repaired in the JSON writer). `const` is the same.
- **It had to be a schema generator, not a serialiser.** Reflection describes a
  TYPE; a value's field cannot be read by name, and a serialiser needs both.

So arc D's question changed shape: not "grow the descriptor by two fields" but
"does reflection report facts that exist only in the SOURCE?". One measurement
bears on the cost — `layout_hash` hashes `render_dump()`, so a fact the descriptor
CARRIES but does not RENDER leaves the @PLN97 layout identity untouched. The
carrying is cheap; the depositing is the decision.

### @PLN127 arc B: `type_of(x)` — the declared shape of a type, as data (2026-08-04)


loft had VALUE reflection (`{x:j}`, `Type.parse`) and FRAME reflection
(`stack_trace`) reachable from loft code, and a SCHEMA level only Rust and a
foreign JavaScript reader could see. `default/07_reflect.loft` brings the third
one across: `TypeInfo` / `FieldInfo` / `VariantInfo` / `TypeKind`, filled from
@PLN105's `LayoutDesc` — the descriptor the browser bridge already reads, pinned
byte-for-byte against the @PLN97 layout dump. Reading THAT rather than walking
`Parts` afresh is what stops reflection becoming a second, drifting description
of the same layout.

`type_of(x)` is intercepted in `parse_call_extra` and lowered to
`n_reflect_type(<type-id>)`, so the id is a parse-time constant — the mechanism
`to_json` already uses. One filler (`native::reflect_type_into`) serves both
backends: the interpreter through `src/native.rs`, `--native` through a
`codegen_runtime` wrapper onto the SAME function.

**The argument is not evaluated.** Nothing about the answer depends on the value,
and evaluating it would mean discarding a result — the operation loft's ownership
model gets wrong most easily (loft#771). The contract is C's `sizeof`, and the
doc comment says so.

Three things the build settled:

- **The plan's Q1 dissolves for `type_of`.** `--native` REPLAYS the type table
  rather than minting it, which is why a runtime name lookup was the plan's one
  load-bearing question — but a parse-time id is replayed with the table. Q1 is
  arc C's question, not arc B's.
- **Q3 answers itself for exactly two scalars.** `get_type`, the one existing
  storage derivation, reports `integer` for a `character` (which is how it is
  stored) and has no entry at all for a `boolean` (`#65535`). Those two are named
  directly; everything else keeps the single derivation, because a second one is
  a second thing to drift. Narrow ints still report storage, and `size` shows it.
- **Reflection inside a generic is not reachable this way.** A generic body is
  parsed ONCE against its type variable, so `type_of(v)` there answers
  `__typevar_T` — the same mechanism that makes `"{v:j}"` in a generic body render
  `{}`. Stated in the doc comment rather than left to be discovered.

Arc A was a prerequisite in fact, not just in order: a `TypeInfo` holds an enum in
a struct field, the exact shape that made `json_parse` reject a whole document, so
`"{t:j}"` on a `TypeInfo` renders complete JSON only because arc A landed.

`tests/scripts/pln127-reflect.loft`, both backends: a record with hand-checked
byte offsets, an enum whose tags start at 1 (0 is how the store spells ABSENT), a
struct-enum variant, a nested record, a vector's element, all five scalars, and
the `TypeInfo` itself serialising.

### @PLN127 arc A: the JSON form is the only field enumeration loft has, and two shapes broke it (loft#768, loft#769) (2026-08-04)


`{x:j}` + `json_parse` is what a generic serialiser, an ORM or a schema walk
reaches for, and both defects were WHOLE-DOCUMENT failures rather than a wrong
field — a struct holding either shape could not be read back at all.

**An enum-typed field wrote its tag bare** (`{"kind":Circle {"r":2}}`), an
unquoted token in value position, so the text was not JSON and `json_parse`
returned null for everything. Two writers render an enum and only one knew about
JSON: `Parts::EnumValue` (an enum-VALUE position, the bare `Circle{…}` form)
wrapped as `{"Circle":{…}}`, while `Parts::Enum` (an enum-TYPED position — a
struct field, a vector element) did not. The typed position now wraps the same
way, and `walk_parsed_into` already accepted that shape as a tagged variant, so
writer and reader name one shape between them rather than two. A fieldless
variant gets a body (`{"Dot":{}}`) so every variant reads back through one path;
an absent discriminant and one naming no variant this schema has are both `null`,
which is what the reader already degrades an unknown tag to.

**An absent `text?` is stored as the sentinel `"\0"`, not as a null pointer**, so
it reached the JSON escaper and came back as the one-character string
`"\0"` (a one-character string holding a NUL) — a present, corrupt value where the program meant nothing. It is the
same absence the null-pointer branch beside it already rendered as `null`, so it
renders the same way. That is the distinction the type exists to carry: SQL NULL
and `''` stay different answers across a round trip instead of collapsing.

The debug form (`{x}`) is deliberately unchanged — it shows the representation,
and only the `json`/`loft` re-parseable forms make a claim about round-tripping.

Seven cells in `tests/scripts/57-json.loft`, both backends, each proven able to
fail first: the field form, its round trip through `.parse`, a fieldless variant,
an enum inside a vector, the bare form unchanged, null-text as JSON null, and
absent-versus-empty surviving as themselves.

### @PLN124 H6/H7: an interpolation hole may be a value of a NAMED type (2026-08-04)


`format_hole` read a hole's kind off the value's type and accepted six scalars; a
struct or enum was a compile error. It now derives the kind from the type's own
NAME in the case a loft method is spelled in — `SqlIdent` asks for
`hole_sql_ident`, `Level` for `hole_level`, and an acronym run breaks at the last
capital (`SQLIdent` → `sql_ident`). Derived rather than chosen, so a target and
the parser cannot disagree about what a type's hole is called and the diagnostic
names the exact method to add. The refusals are unchanged: a kind the target does
not define, and a spec on any hole, are both errors.

That is what lets a target hold something apart from BOTH a literal and a bound
value. The motivating case is @PLN23 H6: a SQL table name is genuinely syntax, so
`SqlText` puts it in inline — and the safety rests on the TYPE, because nothing
builds a `SqlIdent` but its validating constructor.

**A second leak of the expected-type channel, into the HOLE.** A hole is not the
destination, so a string literal inside one must not inherit the destination's
type; without that, `q: SqlText = "{"seed"}"` checked the inner literal against
`SqlText` and it took the BUILD path. The same leak the arc closed per call
argument, one level in, and found only once a consumer wrote a text hole inside a
built statement.

The fix is narrower than the call-argument one deliberately. Clearing `expected`
for the whole hole broke `store_load_layout_gate` on `--native`: the hole
`"{(h[42] ?? Tile { … }).name}"` is a KEYED LOOKUP, and a keyed lookup resolves
its record type through that same channel, so blanking it silently changed the
schema the generated `init()` replays. Only the TARGET derivation is gated now
(`in_format_expr` in `constant`), and only its `expected` source — `var_tp` still
applies, since a declaration written inside a hole does name a destination. Cost:
a format string in argument position inside a hole is plain text, which is a
visible type error at the call rather than a silent difference.

Inertness re-proved after both changes: the 104-site corpus is byte-identical in
IR and in generated Rust.

@PLN23 H6/H7 rest on it — `SqlIdent`, and procedures as named parameterised
statements (`CREATE OR REPLACE PROCEDURE` + `CALL` on postgres/mariadb, a shared
process-side registry for sqlite/duckdb). Two findings from that build:

- **Identifier quoting is chosen at ASSEMBLY time**, not when the hole is filled:
  mariadb reads `"loft_p"` as a string literal and wants a backtick, measured as
  a syntax error when given the ANSI quote the other three use.
- **A procedural body is refused on all four backends**, not just the two with no
  procedural language. mariadb writes them in SQL/PSM and postgres in plpgsql or
  `BEGIN ATOMIC`, and neither reads the other's, so there is no such body a
  uniform API could carry. One statement per procedure, refused where the author
  can see it.

### `chr(cp)` names the code-point constructor that already worked (loft#748) (2026-08-03)


`cp as character` already produced the right character and interpolating it
produced the right text, so what was missing was the ENTRY POINT, not the
capability — the same shape as this issue's byte half, where `text_from_bytes`
had existed for two releases and was reported missing because the generated
reference filed it under Environment.

`chr` is a loft-level definition in `default/03_text.loft` beside
`text_from_bytes`, not a new `#rust` native: the mechanism is already proven on
both backends, and loft covering it is exactly when not to add a dependency.

The refusal set is `""`, never a crash (C80): a surrogate, past `U+10FFFF`, a
negative number — and `0`, which is the one that needed deciding. `character`
uses 0 as its null, and text ITERATION STOPS at an embedded NUL (measured: a
3-byte `"A\0B"` reports `len` 3 and slices at 3, and yields ONE character to
`for`), so a NUL built by `chr` could not be read back by the loop `chr` is the
inverse of. The byte route still carries one. That iteration/`len` disagreement
is filed separately.

`doc/claude/STDLIB.md` gained a **Bytes and code points** section: it listed
neither `byte_at` nor `text_from_bytes` either, so #748's discoverability defect
was live in the agent-facing doc as well as the generated one.

### A tail expression that is a place read no longer reaches the Return as null (loft#754) (2026-08-03)


A function body ending in `w.items[i].bytes` returned an EMPTY vector under
`--native` and the right one under `--interpret`; putting `return` in front of
the identical expression fixed it.

A tail with pending scope frees is hoisted to a `__ret_N` temp so the frees run
between the read and the `Return` (`scopes.rs`, the B5-L3 rule). That rule named
only the SCALAR return types and a later branch only text, so a `vector` / record
/ struct-enum tail fell through to a fabricated `Return(Null)` with the
expression left as a discarded statement. The interpreter read the value off
eval-stack top; native emitted `let _ = expr; …; return DbRef::NULL`.

The hoist now covers the heap return types, bounded to a PLACE READ
(`Value::is_place_read` — a bare accessor chain over a variable, now the one home
for the question `Parser::is_addressable` already asked). The bound is
load-bearing in both directions: only a place leaves its value on the eval stack
alone, so only a place can be dropped; and `Set(tmp, place)` is a bare `DbRef`
copy, so the hoist adds no ownership. A CALL tail already delivers through its
hidden buffer, and hoisting one engaged the store-transfer machinery
(`protect_store_frees` + `CopyRefOrNull`) around a borrowed argument and
over-froze the caller's store.

### A vector's element WIDTH is part of its type at a call (loft#751) (2026-08-03)


`Type::is_equal`'s vector arm compared elements with the scalar-integer rule —
kind only, ignoring width — so a `vector<integer>` was accepted wherever a
`vector<u8>` was declared and its 8-byte elements were re-read as bytes. Silent:
the element COUNT is stored, so `len()` still agreed and every length-based check
passed while only the bytes were wrong. The same mismatch written as a literal
was already refused, so the two spellings of one mistake disagreed.

In a register an `integer` and a `u8` are one type; in a vector the element width
IS the stride. `Type::same_element_storage` states that, keyed on the canonical
`IntegerSpec::byte_width` (so `integer(0,100)` and `u8` are correctly the same
layout) plus the sign of the lower bound (so `i8` and `u8` are not). Refuses at
the call, the assignment, the field init and the return — all four went through
`is_equal`. The suite needed no change, so nothing in tree relied on it.

### Compilation is reproducible again (loft#750) (2026-08-03)


`store_confinement` answered a `HashMap`, and its caller relocates each confined
`__vdb`'s null-init; a relocation that cannot reach its block puts the init back
at body position 0, so visiting several confined stores in Rust's per-process
hash order PERMUTED the null-inits at the head of the body and moved the slots
under them. Compiling one file twice with one binary produced different bytecode
and different slots. It is a `BTreeMap` now, so the visit order is declaration
order. Over the 645-file script corpus, self-differing files: 3 → 0.

Program output was never affected. The cost was that a `--native` artifact could
not be bit-reproducible (#711), and that "prove this change emits byte-identical
IR" — the standing gate for every inert-first plan step — could not tell "my
change did nothing" from "the hash seed moved".

### The whole File surface works through a `&File` parameter (loft#753) (2026-08-03)


`&File` was accepted and then nothing you can do with a `File` worked through it:
`f#read` reported "Unknown loop attribute '#f'", `f += v` reported "No matching
operator 'Add' on '&File' and '&File'". A `&File` is `RefVar(Reference(File))` and
`is_file_var` / `is_file_var_type` both matched a bare `Reference(File)`, so every
File path fell through to the generic operator / attribute code and reported what
THAT code saw. Codegen had always dereferenced such a slot (`OpVarRef` +
`OpGetStackRef`). The peel now lives in the one predicate both callers share — the
loft#740 shape, two guards deciding one question with one of them peeling.

### Releasing a bound store hands its file tail back (loft#752) (2026-08-03)


loft#710 decided a persisted store's size must follow its content and fixed the
IMAGE-write path. A store bound with `store_persist_bind` FIRST never goes through
it: its file IS the live arena, which grows by 7/3 and never shrinks by itself, so
the file left behind was a rung on a ladder — up to 57% above its content, with
40 000 and 60 000 features writing a byte-identical file.

`free_named` now calls `reclaim_tail` on a file-backed store before marking the
slot free. That placement is the fix: `usage()` early-returns on a freed store, so
after the flag the chain walk reports mark 0 and `shrink_to` (rightly) refuses.
It does not contradict @PLN123 A3's "the program says when" — that rule is about
the middle of a run, where a reclaim is paid back at 7/3 by the next claim; at
release there is no next claim, which is the one moment the runtime can tell a
permanent drop from a lull. `shrink_to` still declines a read-only store, an
incomplete walk, and a durable `.dmeta` sidecar.

### A bytecode dump survives a partial schema (2026-08-03)


`introspect` aborted mid-dump on a file that failed to compile: a position still
carried a type id the (partial) schema did not have, and the dump indexed the
type table raw. It prints the bare id now — a dump must never panic.

### @PLN124: a format expression hands its parts to the type being built (2026-08-03)


`parse_string` gains a target: when the type a format string is assigned to
defines `lit`, the string lowers to `lit` / `hole_<kind>` method calls on an
accumulator of that type instead of appending into a text work buffer. The
literal/hole boundary already existed at parse time and was erased only because
every branch appended into the same buffer.

`Parser::interpolation_target` is a fifth SHAPE read off the one `⇐` expected-type
channel, beside `lambda_hint` / `enum_hint` / `vector_hint` / `read_target_type` —
not a sixth side-channel. `var_tp` supplies the target for a typed local, a typed
reassignment and a field init; `expected` for a call argument (free function and
method) and a return body.

Three constraints the build settled:

- **The mint must be pass-stable.** Taking the branch mints an accumulator, and
  the variable tables persist across passes BY NAME (loft#662), so the branch
  keys on method defs (collected on both passes, as the `to_text` hook already
  relies on) and the accumulator draws from its own `__fmt_N` counter
  (`Function::work_format`) rather than sharing `__work_N` or `__ref_N`.
- **The expected-type channel leaked across a nested call.** Each hint SET it when
  it applied and none CLEARED it when it did not, so in `take(build_one("arg"))`
  the literal — `build_one`'s `text` parameter — was checked against `take`'s
  parameter type. Latent while only the enum / collection / function shapes read
  the channel; immediate once a `text` parameter could be shadowed by a struct
  target. Now cleared per argument at both call sites.
- **Nullability is not a kind.** `format_hole` peels `Optional`, so a `text?` hole
  is a text hole whose value may be absent and the target's own parameter type
  decides whether it takes one.

An unsupported hole kind and a format spec on a hole are both compile errors —
never a silent fall back to rendering, which would put a value back on the text
path.

Inertness is the gate:
`doc/claude/plans/124-interpolation-hook/bytecode-comparisons/format-corpus.loft`
covers 104 format sites (specs, `text?` holes, the `OpTagFault` path, an inner
fault that must not tag, JSON/pretty, a custom `to_text` spec, three `for` forms,
backtick, escaped braces, `+=`, argument position) and `loft introspect`
before/after is byte-identical.

### Format hook: a boolean cannot carry a count of hidden work buffers (2026-08-03)


`to_text(self, spec)` with a conditional early `return` was an internal compiler
error — *"Too few parameters on t_5Money_to_text (got 3, need 4)"* — on both
backends, for every interpolation of the type, and on the released binary.

Each formatted `return` promotes its own hidden text work buffer, so two of them
make the hook `(self, spec, __work_1, __work_2)`.  `try_bound_to_text_call`
recorded the buffers in a BOOLEAN, which cannot carry "two": it appended one work
argument and `generate_call`'s arity assert fired.

#533 hardened this same site by classifying parameters by TYPE rather than by
count — correct, and still not enough, because it left the COUNT unrepresented. A
one-buffer body worked, and that working member hid the omission.

The fix fills one argument per attribute, walking the definition's own order, and
refuses to emit a call whose argument list does not fit the definition — so a
signature this hook cannot spell falls back to the generic field dump, which is a
defined answer where a short call is a crash. Guard:
`tests/scripts/format-hook-early-return.loft`.

### `LOFT_UAF_GEN`: a stamp keyed by offset cannot say which store it is about (2026-08-01)


Detector (c) reported a use-after-free on 25 of the 548 corpus scripts, all of them
clean. Any loop calling a struct-returning function drew one — an ordinary shape, so
the noise landed on exactly the programs the detector exists to clear.

The shadow stamped each pushed DbRef's slot generation, keyed by eval-stack byte
offset. `put_stack` is the only writer that keeps that shadow in step, and it is not
the only writer OF the eval stack: `copy_result` slides a return value down with a raw
`copy_block`. The destination offset kept whatever stamp its previous occupant left,
and the next pop compared a returned DbRef against a generation belonging to some
earlier value. A trace showed a push of store 2 at offset 648 answered by a pop of
store 3 at the same offset, checked against store 2's stamp.

Two changes, because either alone leaves the class open. `copy_result` moves the stamp
with the bytes — the source stamp is the real one, written by the callee's `put_stack`
— which covers the SAME-store case no identity check can reach (the slot is recycled
between iterations, so the stale stamp and the fresh ref name the same store at the
same rec and pos). And the shadow now carries the STORE alongside the gen, so a
leftover from any bypassing writer, found or not, is inert rather than evidence.

Ground truth for calling the reports false: `LOFT_NO_SLOT_REUSE=1` with
`LOFT_POISON=1`. With no slot reuse a genuine stale read must land on poisoned bytes,
and every reporting script stayed clean and correct.

`LOFT_UAF_GEN_INJECT=1` is the other half. Silencing a detector and fixing one are
indistinguishable from a test that asserts only "no reports", so the injection ages
every ref just after its push stamps it: 471 of 548 scripts report under it against 0
without. `tests/uaf_gen_detector.rs` pins both directions. Recorded because it changes
what the tool's silence means — and note the detector never actually caught #723: its
report on the broken binary was this same false positive. Detector (c) sees only the
window between a push and its pop, and #723's genuine stale read is a ref going stale
in a FRAME slot.

### #722 / #723: an ownership proxy and the carried fact disagreed at the pre-Set free (2026-08-01)


`x = f().items[0] ?? Fallback {}` bound a dangling reference into the temporary `f()`
returned — correct on the first read, zeroes once the store was reused. The `??` is the
whole difference: it lowers to a value-producing BLOCK, and the temp holding `f()`'s
result was registered at that block's scope and freed on the way out while `x`, bound
in the enclosing scope, still pointed into it. A lift temp minted inside a value block
is now re-registered at the ENCLOSING scope with its block-exit free dropped, and only
the temps the RESULT points into move — hoisting every lift in a value block instead
made frees go missing (27 leaked `File` stores), so `borrow_root` walks the getter
chain to decide.

The loop form was a second fault with a different mechanism. `generate_set`'s
`owned_ref` decides whether a re-assignment must free the store it drops, and asks the
TYPE: empty deps means owned, by loft's convention. That is a PROXY, and it reads
"owned" for a borrow whose dep list was never populated — exactly what a `??` subject
of Reference type is, since the parser materialises it into `__ncc_N` and marks it
`skip_free` while leaving deps empty. Outside a loop the variable is assigned once, so
the free ran on a null slot; inside one it ran on the previous iteration's borrow, by
then dangling into a store freed at the body exit whose slot the next call had already
recycled — so it released a store that had just been allocated and was about to be
read. The @PLN118 sentinel reset cannot cover it: that hangs off a block-exit free,
which a `skip_free` var by definition does not have.

The fix is a subtraction — `skip_free` vetoes the proxy — and not a new rule:
`generate_call` already suppresses any IR-level `OpFreeRef` naming a `skip_free` var
(the S34 guard). `generate_set` emits its pre-Set free as raw ops and bypassed that
chokepoint. A sentinel over the remaining raw emission sites found 0 further `skip_free`
frees corpus-wide against a proven-live control of 1831 hits, so the class is closed.
Interpreter-only: the generated Rust is byte-identical, and `--native` derives the
borrow correctly in `generation/dispatch.rs`.

### #687: a mutated text capture's STORAGE is decided per binding, not per function (2026-07-30)


#685 fixed mutated scalar PARAMETER captures for every type but one and refused the
remainder by name: a `text` parameter inside a function that itself returns `text`. Plan-22
phase 02d-vii skipped text boxing whenever the parent returned text, so mutable text
travelled as a hidden `&text` out-parameter that pass 2 cannot add without growing the
signature (the H5 two-pass contract catches it).

**`parent_returns_text` was a proxy for one real case, and wrong in both directions.** The
case it protected is a text local that is the function's RETURN SOURCE, which the return
machinery has already given its own hidden `&text` out-parameter — that binding cannot
*also* be a shared cell, and does not need to be, since the record stores the value inline
and the existing per-call write-back propagates the closure's writes. As a proxy it was too
WIDE (it also skipped a text local the function does not return, which boxes cleanly) and
no help at all for a PARAMETER, which has no indirection of its own to reuse. A boundary
sweep separated all four combinations, and the discriminator turned out to be a fact
already used elsewhere in the same function: **`RefVar` means "this binding already has an
indirection"**.

The two halves that must agree are the record's attribute type
(`box_captured_names_for_outer_scalars`, at the LAMBDA's epilogue) and the binding's own
type (`flip_scalars_to_box_types`, pass 2) — and measurement showed the epilogue simply
cannot be right: a to-be-returned text local is still a plain `Text` there and only becomes
`RefVar(Text)` later in the body, which is exactly why the two disagreed. So the epilogue
boxes PROVISIONALLY (the common case, and it must write something because pass 1 freezes
the record's storage — leaving the raw scalar lays the field out 8B inline instead of a 12B
shared DbRef), and `Parser::finalize_capture_storage` corrects it at the parent's **pass-1
body end**: the first moment the fact is final, still before `fill_all` lays the record out,
and the same hook `reject_shared_mutable_scalar_captures` already uses for the same reason.
It runs first — the rejection consumes the parent's lambda list.

Net effect is a SUBTRACTION: both `parent_returns_text` guards and #685's refusal are gone,
replaced by asking the binding. One text-returning function can now need both answers at
once (`keep` returned → inline, `side` not → cell), which is what no per-function condition
could ever get right.

Two measurements worth recording, because both contradicted the obvious reading:

- Removing only ONE of the two guards leaves them disagreeing and SIGSEGVs — they are one
  fact re-derived twice, so they move together.
- `box_captured_names_for_outer_scalars` and `synthesize_closure_record` run in **pass 1
  only**: in pass 2 the lambda records zero captures (its pass-1 placeholder vars are
  restored into its own table, so the capture branch is never reached). The record's
  attribute types are a pass-1 decision, full stop — which is why the correction has to be
  a pass-1 hook and not a pass-2 repair like #686's.

Guards: `tests/scripts/687-mutated-text-param-capture.loft` (values, both backends — the
capture's source, whether it is the returned value, the parent's return type, cardinality,
by-value, repeat calls) and `issue_687_mutated_text_capture_storage_is_per_binding`, which
asserts the STORAGE for three bindings whose only difference is what claims them. Both
verified RED with `finalize_capture_storage` disabled. #685's
`issue_685_text_param_in_text_returning_fn_is_refused` is replaced by it — that test pinned
the refusal, which was always a placeholder for this issue.

### #686: a capture of a FORWARD-declared type was mis-typed, then mis-positioned (2026-07-30)


A lambda capturing a local whose type came from a struct declared LATER in the file read
that capture as `text` — `Unknown field text.cells`, on a program with no `text` in it.
Two faults composed, and the first hid the second.

**Fault 1 — a sentinel read as a def number.** A capture's type is the type of an
EXPRESSION (`ch = w.chunks[1]`), so with the struct not yet declared it is `Unknown(0)`:
the codebase-wide "no type known" marker, which names nothing. `copy_unknown_fields` read
the `0` as a forward-reference stub and set the field to `data.def(0).returned` — whatever
the first definition in the program happens to return, in practice `text`. The `Vector`
arm of that same function already guarded `was != 0`; the bare arm did not. This is a
LYING fact, not a missing one: the field looked resolved, so nothing downstream
questioned it. Guarded now, which turns the symptom honest (`unknown`, not `text`) and is
what exposed the second fault.

**Fault 2 — a struct laid out while a field was unsized.** `fill_database`'s field loop
SKIPS an attribute whose type it cannot size (deliberately — so the user sees the parser's
diagnostic rather than a panic), but the struct is still registered and `finish` still
sizes it, leaving the field at `position == u16::MAX` forever: `finish_type` never
revisits a sized type. The closure then read and wrote its capture at **offset 65535** —
an INTERMITTENT crash, which is what made the readings during investigation worthless
until a repeat-run harness replaced them (two byte-identical probe files disagreed;
single runs had been "confirming" three different stories).

The invariant: **a struct is laid out only once its fields are sized.** Enforced at the
one place that lays them out — `fill_all` skips a def carrying a NAMELESS unknown
attribute (`has_nameless_unknown_attr`). Narrow by construction: `Unknown(stub)` names a
type and `copy_unknown_fields` resolves it before the loop, so only `Unknown(0)` — an
expression-typed field, and the closure record is the sole producer of those — can reach
the layout unsized. The loop is keyed on `known_type == u16::MAX`, so deferring costs
nothing; a field that never resolves leaves the struct unregistered, which is harmless
because the parser has already reported the error.

Pass 2 then re-types the attribute from `capture_context` (`resolve_forward_captures`, at
lambda entry — NOT at the record-synthesis epilogue, which runs after the body) and lays
that one record out on demand via `Stores::lay_out_record`. The on-demand layout is
required, not a shortcut: the body bakes field offsets into its IR as it parses, so the
end-of-pass `finish()` is too late, and a full `finish()` mid-parse re-appends keyed-index
bookkeeping. `lay_out_record` is the sibling of `lay_out_synth`, which solved exactly this
for a forward-referenced synth enum — same deferral, same reason, same empty `linked` set
(a closure record holds scalars and 12-byte DbRefs, never an inline keyed collection).

The pass-1 storage-encoding `match` moved into `closure_attr_type` so the synthesis and
the repair cannot drift on which captures store as a shared DbRef.

Guards: `tests/scripts/686-forward-declared-capture.loft` (values, both backends — field /
element / whole-vector / scalar projections, cardinality, and repeat calls) plus
`issue_686_forward_declared_capture_is_typed_and_positioned` and
`issue_686_nameless_unknown_is_not_resolved_against_def_zero`. The two facts are asserted
separately because they fail separately, and each half of the fix was verified to break
its own: with the sentinel guard off both fail; with only the layout deferral off, the
type is right and the POSITION is still 65535. A value test alone cannot see that — a
positionless field only crashes when the bytes at offset 65535 happen to be fatal.

### #685: a mutated scalar capture sourced from a PARAMETER corrupted the frame (2026-07-30)


Two producers of one fact disagreed. `box_captured_names_for_outer_scalars` gave the
closure record a 12-byte `__cell_<T>` `DbRef` field for a mutated scalar capture, while
`flip_scalars_to_box_types` skipped arguments outright — so the parameter stayed an
8-byte stack scalar and `emit_lambda_code`'s `OpSetDbRef` read 12 bytes out of an
8-byte slot, corrupting the 16-byte fn-ref being built beside it. The interpreter then
dispatched a garbage `d_nr` (`fn_call_ref: … out of range`) or SIGSEGV'd; `--native`
emitted field access on a bare `i64` and would not compile.

**The filed scope was one cell of each axis; the boundary is a single fact.** The
trigger is only "the mutated capture's source is a parameter": every boxable type
fails (integer / float / boolean / character, and text — the last as a SIGSEGV, via a
different lowering), the closure need not be CALLED, the enclosing function need not
read the value back, the closure may sit in a nested block, and a set before the
lambda does not help. **That last cell falsified the filed hypothesis** ("the cell is
never allocated because allocation happens on first set"), which is why the fix does
not touch allocation timing. Two or more captures in one frame crash at a *different*
site (`allocation.rs`) — the same corruption seen through slot reuse.

The argument-skip could not simply be dropped: flipping a parameter's own type to a
12-byte cell reference changes the call ABI. The fix promotes it instead —
`promote_boxed_scalar_arg` mints a shadow local of the same type, `set_promoted_from`
+ `remap_name` point the name at it before the body parses, and the existing
promoted-argument preamble in `parse_code` seeds it at function entry. Every read,
write and capture then routes through the shadow and the emitted IR is byte-identical
to the LOCAL case that already worked — which is why all five types are covered with
no per-type work. It is the hand-written workaround (`acc = n;`) done by the compiler,
and it follows the mutated-text-argument promotion the codebase already had.

Reusing the local path exactly is also what preserves by-value semantics: the
parameter slot is untouched, so the caller cannot see the closure's writes.

Supporting changes:

- `boxed_cell_alloc_and_set` extracted as the ONE home for "a boxed scalar comes into
  existence" — the first assignment to a boxed local and the parameter's entry seed
  need the identical `Set(v,Null)` + `OpDatabase` + `OpSet<T>` trio, and the seed has
  no assignment of its own to hang it on. The shadow is marked `defined` at creation
  so the body's first write does not prepend a SECOND allocation, which would replace
  the seeded cell and lose the argument's value.
- `Type::Boolean => "OpSetBoolean"` added to the cell-write table. It had been
  deferred on the premise that a boolean cell needs a 4-arg `OpSetByte`; the premise
  was wrong — the working boxed-boolean lowering emits the 3-arg `OpSetBoolean`. The
  unit test that pinned the fall-through now pins the write.
- A value-const parameter mutated through a closure is now rejected at the promotion
  site. The closure-side write never reaches `validate_write`'s guard (inside the
  lambda the name is a capture, not a binding carrying the flag), so without this the
  fix would have quietly handed the closure a writable cell for a read-only parameter
  — a silently accepted contract violation in place of a loud crash.
- `RefVar` arguments are excluded from both branches: a user `&T` out-parameter's
  writes must reach the caller, and a mutable text local the compiler already promoted
  to a hidden `&text` out-parameter is itself the working path. The first attempt
  omitted this and refused `local = n; … local = local + k;` — code that worked.

**Residual, refused by name rather than corrupted (#687):** a mutated `text` parameter
inside a text-returning function. There `flip_scalars_to_box_types` skips text boxing
(plan-22 02d-vii) and mutable text travels as a hidden `&text` out-parameter instead,
which cannot be added from pass 2 without growing the signature after pass 1 fixed it.
The **H5 two-pass contract caught that attempt** — the assert doing exactly the job
#662 showed it had been blind to. The diagnostic names the working alternative.

Guards, all four verified RED with the promotion disabled:
`tests/scripts/685-mutated-scalar-param-capture.loft` (values, both backends — every
type, both non-trigger axes, cardinality, and the by-value edge) plus
`issue_685_mutated_scalar_param_is_boxed_like_a_local` (the invariant: the record's
field type and the frame's binding for that name are the same cell, and the arity is
unchanged), `issue_685_text_param_in_text_returning_fn_is_refused`, and
`issue_685_const_scalar_param_mutated_by_closure_is_rejected`.

### #682: the closure-record cascade freed captures the record never owned (2026-07-30)


A reference / collection capture is stored in `__closure_N` as a 12-byte `DbRef`
(P260), and `free_named`'s cascade freed every one of them when the record died.
That is correct for a store the defining frame OWNED and handed over — `get_free_vars`
suppresses the frame's own `OpFreeRef` for a captured reference, and the cascade being
the sole free is exactly what lets an escaping factory closure outlive its frame
(#323). The pairing only holds where a frame free existed to suppress, and for two
common capture sources it never did: a **parameter** is excluded from the scope-exit
sweep entirely (`variables()`: "never return function arguments"), and a **projection
local** (`ch = w.chunks[1]`, a `for` element) is `owns == false`. Both were cascaded
anyway, so the caller's store was freed under it.

**The filed scope was a lambda handed to a library as `fn(float,float)->float`;
neither the hand-off nor the call is a trigger.** The minimal cell captures a struct
parameter and never invokes the lambda. The axes that matter are the ones deciding who
owns the capture — capture SOURCE (parameter / projection / for-element / owned local)
and KIND (struct reference / vector / hash / boxed `__cell_`) — and the class covers
every store-backed capture of a borrowed binding, not just the reported struct.
The symptom was three steps removed: a freed-but-unreused store still reads correctly,
so the fault surfaced when the next allocation recycled the slot, in an unrelated
function ~900 lines from the closure.

One dep marker had to carry two facts, which is the encoding bug behind it:
`Deps::share_sentinel()` meant both "store a 12-byte DbRef" and "the record owns the
target". It is now a pair — `share_sentinel()` (adopted, `dbref`) and
`borrowed_share_sentinel()` (borrowed, `dbref_borrow`) — two type-table entries of the
same 12-byte / align-4 shape, so no position, size, read or write path moves; only
`free_named`'s filter (`Stores::dbref_is_adopted`) reads the difference.

**The verdict cannot be computed at record synthesis, which is why it is not.** A
capture's ownership is not final at parse time: `ch = pick(w, 1)` parses as "borrows
`w`" from the callee's declared return, and only `scopes::check`'s call-result rewrite
(`make_independent`, the `!adopts_fresh_store` arm) turns it into OWNED once it knows
the return ABI deep-copies into a fresh store. A first attempt read the parse-time dep
and leaked that copy. The decision is therefore `scopes::mark_borrowed_captures`, run
after every dep rewrite has settled, reusing `get_free_vars`' own `owns` test so the
two cannot drift. It reaches each record through the defining frame's `___clos_N`
LOCAL; the lambda's hidden `__closure` PARAMETER has the same type, and reading it too
flipped verdicts by definition order.

`--native` picks the marker up from the attribute type directly (its schema is emitted
after scope analysis); the interpreter's schema is laid out during parse, so
`typedef::sync_capture_ownership` re-points the field from `compile::byte_code_from` —
the one funnel every `byte_code*` entry point passes through. A `__cell_<T>` capture
(plan-22's boxed mutated scalar / text) is always adopted: the cell is minted for that
closure alone, so the record is its only possible owner however the binding was
reached — including from a parameter.

Both `dbref` shapes register **together** from either entry point. Type numbers are
positional and `--native` replays the registration sequence to rebuild its schema, so
a shape appearing in only some programs shifted every id after it — `505-collection-
capture` failed native with `Cannot add to none-structure 'State'` until the pair
became unconditional.

Guards: `tests/scripts/682-closure-capture-borrow.loft` (values, both backends — every
borrow cell called twice so the recycled slot shows, every adopt cell three times so a
wrongly-borrowed verdict shows up as a leak) and
`tests/issues.rs::issue_682_closure_capture_ownership_marker` (the marker itself, both
directions). Both verified to go RED with the pass disabled. Reproduced and fixed
against the consumer's real `hex_world::World`, which the pre-fix binary corrupted from
the second tick on.

### #654: jump displacements were 16-bit — a body past 32 KB jumped somewhere arbitrary (2026-07-28)


`OpGotoWord` / `OpGotoFalseWord` carried a `const i16` displacement, computed with an
unchecked `as i16` at every emission site. Past ~32 KB of emitted body the value wrapped
and the jump landed at an arbitrary address; for a `while true` that meant the body ran
ONCE and control fell out of the loop, with `main` returning 0 and no diagnostic.

**The filed scope was the backward `while` jump; a boundary matrix showed the real one.**
All five jump classes truncate, because they share the encoding: `while` and `for`
(backward, `gen_loop` / `gen_continue`), `break` (forward, patched in `Stack::end_loop`),
and the forward skips of `if` and `else` (`gen_if`). `--native` was correct in every cell
— it emits real Rust control flow and never reads these operands — which made it the
positive control the matrix was read against.

Both ops now carry `const i32`, which covers the whole `code_pos` (`u32`) space, so the
threshold is removed rather than moved. A fixed-width slot also means the forward-jump
patch sites need no branch relaxation: `code_put` writes an `i32` into a slot whose size
was already reserved.

Getting a 4-byte constant emitted required two places to stop deriving an integer's width
from its RANGE and start reading its declared `size(N)`:

- `variables::size(_, Context::Constant)` — a 1 / 2 / 8 ladder with no rung for 4.
- `Data::rust_type` — the same ladder, deciding what the generated reader in `fill.rs`
  reads. Left alone it would have READ an `i64` while codegen WROTE 4 bytes.

Both are inert for every integer alias that predates the change (`u8` / `i8` force 1 and
range to 1; `u16` / `i16` force 2 and range to 2; plain `integer` forces nothing), proven
by a byte-identical `loft introspect` over a corpus exercising all of them before and
after the `variables::size` change alone.

Displacement arithmetic that measured from after-the-operand moved from `- 2` to `- 4`
(`gen_loop`, `gen_continue`, `Stack::end_loop`); the disassembler's jump-target scan
(`compile.rs`) and both renderers in `state/debug.rs` follow the same width. `tests/dumps`
is gitignored, so no golden output churned.

Guard: `tests/issues.rs::issue_654_jumps_survive_a_body_past_the_16_bit_displacement`, one
case per jump class, each asserting an accumulated VALUE rather than mere completion —
the failure mode was silent fall-through, which a runs-to-completion check would have
called a pass. Verified non-vacuous against the installed pre-change 2026.7.2 binary: all
five cases produce NO output there (execution falls past the asserts) and pass here.

### @PLN108 "Share read-only parent stores across par workers" — interpreter core (2026-07-17)


- A par worker whose captured parent state is read-only (@PLN102 C93) now **BORROWS** the parent
  stores read-only instead of `clone_for_worker`'s per-worker byte-copy, for `run_parallel_discard`
  and `run_parallel_queue`. Copy-elision, no semantic change.
- **Auto-selected by heap size:** the borrow engages only when the copy it would save is ≥ 2 MB
  (`Stores::active_clone_bytes`), so small/frequent pars keep the cheap rayon-pool clone (no
  regression) while a par over a large read-only structure goes flat instead of copying the session
  heap per worker (measured ~53× on a 122 MB shape). `LOFT_PAR_SHARE=0`/`=1` force off/on.
- Safety is compiler-carried (the dispatcher's `&Stores` signature proves parent-unwritten) +
  the `read_only` write-panic; **ASan + TSan clean** on the flag-ON path (positive control fires).
  `--native` par still copies (a native analogue is deferred). Decision recorded as C99.

### `loft fmt` — parser-driven formatter, written in loft


- New `loft fmt [--check|--write] <file…>` (`-` = stdin): a canonical formatter (`tools/fmt/whole.loft`)
  driven through the new host-call API. Default prints; `--write` rewrites in place; `--check` is a
  CI gate (non-zero if unformatted). One canonical style — struct/enum/interface defs + fn bodies
  expand; struct-literal/control-flow VALUES stay inline; number vectors pack, object vectors break;
  trailing comments stay at end of line. Coexists with the older Rust `--format`.

### `loft::host` — Rust→loft call API


- `Program::from_source(src)` → `prog.call("fn", &[Value::…])` → `Result<Value, LoftError>`: call any
  loft `pub fn` by name with typed args, typed return, errors as a `Result`. Routes through the
  interpreter's stack ABI (`State::execute_host`). Phase 1 supports text / integer / single / boolean
  / void; struct/vector returns are a clean `Unsupported`. Consumed by `loft fmt`.

### @PLN28 "Better error messages" — closeout (2026-07-07)


Phases 5, 6, 1, and 7 landed, completing the plan (0/2/3/4 shipped earlier).
Commits `a77afaec` (P5+P6), `6e9b6c5f` (P1 resolution), `<this>` (P7).

- **P5 suggestions** (`src/diagnostics.rs`, `parser/objects.rs`): all seven
  candidate-scoped `did-you-mean` sites live. Relaxed `suggest_similar_capped`
  from `min(2, n/4)` to distance-2 for names ≥ 4 chars (catches transpositions
  like `naem`→`name`); wired the struct-literal unknown-type site; a qualified
  `Enum::Typo` now reports + suggests instead of silently nulling (exit 0 → 1).
- **P6 type-mismatch** (`parser/mod.rs`, `parser/control.rs`, `parser/objects.rs`):
  call-arg mismatch names the argument index; a `match` pattern whose type can
  never match the subject now errors instead of compiling to a silent dead arm;
  struct extra-field recovers past the orphaned value (6-error cascade → 1).
  The spec's phrasing-only rewrites were skipped (messages already name both
  sides + the operation); missing-field and format-spec-on-wrong-type left as
  designed behaviour (zero-default fields / freeform specs).
- **P1 spans**: verified the 5 "remaining" wraps (Set / Iter / Return / struct-
  lit / narrow-cast) unnecessary — their diagnostics already capture positions
  via `diagnostic_at!` and none faults at runtime, so wrapping would attach a
  position no consumer reads while risking `unspan()` churn. Status → done.
- **P7 closeout**: COMPILER.md § Diagnostics rewritten (spans → runtime C66 →
  renderers); user-facing CHANGELOG entry; CLAUDE.md `LOFT_ERRORS` + diagnostic
  toggles; CAVEATS native-no-source-map entry. No opcode changes; no runtime
  path touched (bench flat).

Golden `error_messages` baselines 06-10, 30, 34 regenerated + locked; full suite
green on both backends. Deferred (non-blocking polish, tracked in the phase docs):
phase-4 `4e.3 slice 2` (finer format-null tokens) and the `= note:` secondary-line
renderer.

### #497: reassignment-path adopt-vs-copy — a borrowed call return freed the lender's store (2026-07-04)


A struct-returning call REASSIGNED into an owned Reference local took the
plain-adopt path whenever the callee had no visible Reference/struct-Enum
param — the old `has_ref_params`-style proxy missed a callee borrowing from a
visible VECTOR param (`return cs[i]`). The local then aliased the borrowed
element, and its owned pre-Set free whole-store-freed the LENDER's backing
store: crawler's `build_walls` lost `cs` mid-function — silent wrong data
first (writebacks vanished; the #496 face), SIGSEGV once store recycling
reused the number (the #497 face; the scale-dependence and heisenbug were
pure visibility artifacts — `LOFT_LOG=poison_free` reproduces it in the small
hand-built level deterministically). The one-axis trigger: the call-assignment
sitting inside a nested `if` (its first-set is the hoisted init, so the Set is
a REASSIGNMENT; the first-Set path already read the carried fact and was
correct).

- Fix at the A.3 chokepoint (OWNERSHIP_MODEL row 102/270): the reassignment
  gate in `state/codegen.rs` now reads the ONE carried adopt-vs-copy fact,
  `return_adopts_fresh_store()`, exactly like the first-Set path — and gains
  the #429 struct-Enum parity the first-Set path already had.
- The raw path is preserved behind `LOFT_NO_REASSIGN_COPY` (the
  `LOFT_NO_JOIN_OWN` preservation pattern) so the ownership fuzz gate's
  crash-channel positive control stays non-vacuous; the self-test's buggy
  config now disables both gates (control re-pinned).
- Guards: `tests/scripts/497-reassign-borrowed-elem-copy.loft` (the minimal
  if-wrapped shape + the two-captures build_walls composition, both backends);
  the 54-cell fuzz map is 0/54 flagged on the default gate.

### Nightly registry validation — published packages vs loft@main (2026-07-04)


New `registry-validation.yml` workflow (04:30 UTC + `workflow_dispatch`): one
matrix leg per non-yanked registry package, each installed from the LIVE
registry exactly as a user gets it and validated against loft built from main
on the runner's current stable rustc — `loft install` (tarball + sha256 +
deps), `cargo build` of the shipped `native/` crate, and the package's own
tests on both backends via the new `scripts/registry_validate.sh` (also
runnable locally). Closes the gap where a released tarball rots unnoticed
after loft moves (the loft-libs-core#14 class); the first live sample caught
cbor 0.1.0 (DN1 type error) and crypto 0.3.4 (machine-local `path =` deps in
the published `native/Cargo.toml`). See PKG_REGISTRY.md § Nightly toolchain
validation.

### `#native` boundary: nullable scalars marshal, C-ABI externs are i64 (2026-07-04)


Found via loft-libs-core#14 (`random.rand` declaring the honest `-> integer?`
contract under the @PLN25 null/dense model). Two related fixes, one invariant:
*marshal/ABI classification is layout-based, and `Optional(τ)` shares τ's
sentinel layout — every judgment classifies the peeled type* (`Type::base()`).

- **`Optional(τ)` in `#native` signatures now wires and dispatches.** The
  marshal classifiers (`extensions::compute_sig` / `compute_shared_sig` /
  `marshal_arg_t`, `native_gate::is_scalar_type` / `is_bridge_type` /
  `classify_bridge_attr` / `shared_store_dispatchable`, the shared-bridge
  read/write emitters, and the `--native` direct-call + extern emitters) all
  fell through to their unmarshallable/default arms on `Optional`, so a
  `#native` fn declared `-> integer?` was never wired — the interpreter call
  hit the stale-cdylib panic stub, and `--native` mis-emitted. All sites now
  peel via `Type::base()`, extending the @PLN25 slice-(b) pattern already used
  by `rust_type`.
- **The C-ABI extern block now declares i64 for plain `integer`.** The emitter
  decided width by `IntegerSpec::is_wide()` (value range — false for a
  declaration's template spec) while the interpreter marshal and the package
  cdylibs use the @P370 judgment (plain `integer` = i64; only `forced_size`
  narrows). The `i32` extern against an i64 cdylib silently truncated i64
  traffic — the null sentinel (`i64::MIN`) arrived as `0`, and beyond-i32 /
  negative values corrupted. Both judgments now key on `forced_size`.
- Regression guards: `native_loader::wires_optional_integer_return_and_wide_values`
  (end-to-end null + 2^40 round-trip through the fixture cdylib) and
  `n2_cdylib::cabi_extern_declares_i64_for_plain_and_optional_integers`
  (emit-shape); fixture patterns 10/11 (`ext_maybe`, `ext_echo`).

### @PLN22 — enum-scoped variants, prelude shadowing, `use … as …` aliasing (2026-06-14)


All four phases of the namespaces initiative (`loft-lang/plans#22`), built
chokepoint-first and verified on both backends.

- **P1 — enum-scoped variant definitions.** Variants are resolved through one
  `Data::variant_of(enum, name)` chokepoint (plus `variant_in_source` /
  `enums_with_variant`) instead of a global bare key, so two enums may share a
  variant name. A bare variant used as a *value* resolves only via context
  (match subject, typed decl, typed reassignment / `rec.field`, parameter,
  return incl. an `if`-branch tail, struct-field type & default, `==` LHS,
  `Enum::`/`Enum.` qualifier); defining a new untyped variable from a bare
  variant (`x = Red`) is a hard error even when the name is unique. Mixed-enum
  unit-variant field defaults no longer clobber a sibling field. The variant
  name stays usable as a TYPE / constructor (`Circle { … }`, `s: Circle`).
- **P2 — prelude shadowing.** `STD_SOURCE = 0` (stdlib + global synthetic
  wrappers) and `MAIN_SOURCE = 1` (user main) are named explicitly; the user
  main file gets its own source so a user `enum E` / `struct File` / `pub PI`
  shadows the stdlib name in bare lookup while `std::Name` still reaches the
  prelude. Built-in type-keywords (`integer`, `vector`, …) stay sacred —
  non-shadowable — via the `DefType::Type @ STD_SOURCE` guard.
- **P3 — `use … as …` aliasing** for libraries (`use lib as m`), types
  (`use lib::Type as T`), and functions (`use lib::fn as f`).
- **P4 — grouped selective imports** `use lib::(a as x, b, c)`; the flat comma
  list `use lib::a, b` is dropped (hard error). Design decision C76.
- **Reserved-keyword hardening (commit `c383a25c`).** `struct iterator` was
  silently adopted (the struct adopt-branch swallowed the `type iterator;`
  forward-decl); `enum hash` / `type sorted` panicked in `complete_definition`.
  Guarded the adopt branch on `DefType::Unknown`, gated the enum/typedef
  completion calls on `!conflict`, and forward-declared `type radix;` /
  `type spacial;`. All builtin type-keywords now reject cleanly across
  struct/enum/type. Regression: `tests/scripts/102-expected-errors.loft`.

Regressions: `tests/scripts/369-pln22-shared-enum-variants.loft`,
`370-pln22-prelude-shadowing.loft`, `tests/imports.rs` (phase 3/4 aliasing +
grouped + flat-list-rejected). Resolves INC#34.

### engine_host: `run_local` — the standalone windowed host (#343) (2026-06-12)


A windowed program with no server could not run on the @PLN18 kernel: `run`
(listener) never returns and has no frame yield; `run_client` bails without a
connection.  `run_local(tick_interval_us, on_event, on_tick)` is the connector
loop with **no transport** — drift-free ticks (one tick = one frame for a GL
host), the per-turn frame yield, the loop's own idle backoff (kills the
consumer's busy-spin), swap machinery (08-S5 `LOFT_SWAP_READY` included) and
the debug control endpoint, all without a peer.

Mechanics: `ClientKernel.conn` became `Option<TcpStream>` (the two `Some`
sites — frame read, masked write — are behavior-identical; `None` reads
nothing and `send` reports false).  `kernel_local(tick_interval_us)` landed on
all three calling conventions: bytecode native, browser (`ws:-1`, guarded
pump/send), and the `--native` typed twin (`CODEGEN_RUNTIME_FNS`).  The loft
side factors `run_client`'s body into one shared `client_loop`; `run_local` is
local boot + the same loop (no third copy).  Going online later means swapping
`run_local` for `run_client` — handlers never change.  Regression (both
backends): `tests/engine_host_kernel.rs::run_local_ticks_and_stops_without_a_server`.
Driven by the crawler consumer (#343); design note: plan-18 ENGINE_HOST.md
§ Update 2026-06-12.

### engine_host: `post`, listener `stop()`, listener frame yield (the crawler K2 trio) (2026-06-12)


The three flow-back asks from the crawler consumer's K2 (observer slice):

- **`post(msg) -> boolean`** — enqueue a LOCAL event on the running kernel
  (any role): window input becomes an ordinary events-class message with
  `cid: -1` (local origin).  The connector loop previously hardcoded `cid: 0`
  when constructing events (the server was the only source); the new
  `kernel_client_event_cid` accessor carries the real origin.  Registered on
  all three calling conventions; surface fns with a `#native "sym"` alias
  register their DEF name in `CODEGEN_RUNTIME_FNS` (`n_post`, like `n_send`).
- **Listener `stop()`** — `Kernel.alive` + `kernel_stop`/`kernel_alive`;
  `run` loops on `kernel_alive()` and returns after a handler calls
  `engine_host::stop()` (the windowed listener's window-close exit, mirror of
  `client_stop`).
- **`kernel_frame()`** — the per-turn yield in `run`'s loop (no-op native,
  frame-yield browser; twin of `kernel_client_frame`), so a windowed
  LISTENER frames correctly.

Regression (both roles × both backends):
`tests/engine_host_kernel.rs::post_and_stop_in_both_roles`.

### rpc debugger: `verified` flag on setBreakpoints + string-form tracepoint log (#342) (2026-06-12)


Two silent-failure footguns in `loft debug --rpc`, found while verifying the
loft-debug skill against the implementation:

- `setBreakpoints` answered `{ok:true}` with no per-breakpoint feedback, so a
  breakpoint that can never fire (no breakable code on the line, or a file the
  program doesn't use — matching is by **basename**) just never stopped.  The
  response now carries the PROTOCOL.md-documented `breakpoints:[{line,
  verified}]`, resolved eagerly via `breakable_lines_in_file`.
- A tracepoint's `"log"` given as a plain string was silently ignored (only
  the array form worked).  A string is now sugar for a one-element array;
  entries are expressions rendered `expr = value`.

PROTOCOL.md's request table is corrected to match the implementation: `launch`
LOADS only; the previously undocumented `run` request starts execution (set
breakpoints between them).  Liveness note for clients: conditions and trace
expressions see only the locals live ON that line — an out-of-scope name
evaluates null and a condition on it silently never matches.  Regressions:
`tests/rpc.rs::rpc_set_breakpoints_reports_verified`,
`rpc_tracepoint_log_accepts_plain_string`.

### Multiple materialised par loops no longer corrupt each other (#282) (2026-06-06)


Several **materialised** par loops (range / `iterator<T>` / text inputs) in one
function, with **different element types**, silently corrupted an earlier loop:
its materialised input (`__par_mat`) was read at the wrong stride (e.g. an
`integer` range loop's input came back as `vector<character>`), so a worker saw
garbage elements.

Root cause (var-table / scoping level, not IR-structure): `materialise_iter_for_par`
builds its body **pass-2-only**, so naming its temps via the global `create_unique`
counter advanced that counter only on pass 2 — desyncing two-pass numbering for
sibling materialise loops, whose `__par_mat` vars then **collided on one name**.
`add_variable` merges by name, so the merged var took one element type; the other
loop read its store at that type's stride. (Same family as the result-var
two-pass fix.) Keyed materialise was immune only because all its loops share one
element type.

Fix: name the materialise temps by the stable `loop_nr` (`_par_mat_l<loop_nr>` …)
via `add_variable` — unique per loop and identical across both passes, so no
collision and no counter advance. Verified on both backends
(`tests/scripts/22e-par-many-materialise.loft`).

### `for … par(…)` accepts every iterable source; hash skips its sort (#270) (2026-06-06)


The parallel for-clause now runs over **any iterable**, not just a flat vector.

- **Parser hang fixed (#270).** `for i in 0..3 par(r = i, 2) { … }` infinite-looped
  the parser: `skip_to_parallel_body` (the par-clause error-recovery drain) had no
  comma consumption and no forward-progress guard, so it spun on the `,`.  Added a
  no-progress guard mirroring `consume_call_args`; recovery can no longer hang.
- **Range / `iterator<T>` / text sources now work.** A non-vector, non-keyed source
  is materialised into a flat `vector<T>` (via `materialise_iter_for_par`, reusing
  `build_comprehension_code` for correct per-kind element append) before the queue
  dispatcher partitions it.  Keyed collections (hash/sorted/index/spacial) keep their
  existing `materialise_keyed_for_par` path.
- **Hash skips the sort for par.** `for x in h par(…)` builds its iteration scratch
  from `hash::records()` (raw bucket walk via the new `hash_unsorted` / `n_hash_unsorted`)
  instead of the key-sorting `hash_sorted` — the parallel queue has no use for a hash's
  order.  Sequential `for x in h` stays key-ordered; only the par form differs.
- **Two pre-existing native-codegen par bugs fixed (surfaced here, untested before —
  no keyed/range/primitive-vector par script reached `--native`):**
  - keyed/range materialise wrapped its temp var in a `v_block`, which native lowers
    to a Rust `{ }` scope, so `__par_mat` died before the dispatch used it (E0425).
    Now spliced as `Value::Insert` (inline), like the vector path.
  - a by-value scalar worker (`fn(x: integer)` over `vector<integer>`/range) got the
    element `DbRef` instead of the read-out value (E0308 `expected i64, found DbRef`).
    `tuple_arg_prep` now reads scalar element types out of the record, the 1-element
    case of the existing tuple-worker path.

  Verified on both backends across range/vector/hash/sorted/index sources and
  integer/float/boolean/single worker returns (`tests/scripts/22c-par-sources.loft`).
- **Interpreter text-return par fixed.** A text-*returning* par worker over a
  non-`DbRef` element (a `vector<integer>` / range → primitive input; a
  `vector<text>` → text input) produced garbage or a SIGSEGV: `run_parallel_text`
  always pushed the element as a 12-byte `DbRef`, unlike the integer path's
  input ladder.  `execute_at_text` now takes a `WorkerArg` (Ref / Primitive /
  Text) and `run_parallel_text` selects it by the worker's first-arg kind —
  the same ladder `run_parallel_queue` applies.
- **Interpreter ref-return par over a primitive input fixed.** A struct/
  reference-*returning* par worker over a `vector<integer>` / range fed the
  worker the element `DbRef` instead of the primitive value (`run_parallel_queue_ref`
  → `execute_at_ref` had no input ladder) → garbage results.  `execute_at_ref`
  now takes the same `WorkerArg`; `run_parallel_queue_ref` reads a primitive
  element by value.  Text / struct inputs keep the `DbRef` path (already correct).
- **Par result-var two-pass instability fixed.** The fused-par result var was
  named `_<name>_<global-counter>` via `create_unique`.  Across many par loops
  with mixed result types the `create_unique` count diverged between parser
  pass 1 and pass 2, so the pass-2 `b_var` failed to reuse its pass-1 entry —
  the user name then aliased to a wrong-typed var (`r.len()` on a `text` result
  rejected as `integer`).  The `b_var` is now keyed on the stable `loop_nr`
  (`_<name>_par<loop_nr>`), identical across both passes.  Guarded by the
  intentional `r`-reuse in `tests/scripts/22c-par-sources.loft`.
- **Native text-input par fixed.** A par worker with a `text` parameter (over a
  `vector<text>` source) failed `--native` compilation: the worker closure passed
  the element `DbRef` where the worker wants `&str` (E0308).  `tuple_arg_prep` now
  emits `loft::codegen_runtime::par_read_text_input(cell, elm)` (reads the row's
  text into an owned `String`) for a text first-arg — the text-input sibling of
  the scalar-element read.
- **Native literal-returning text-return par fixed (#273).** A par worker that
  returns text via literals (the @P205 no-work-buffer / owned-`String` shape) has
  no `&mut String` work-buffer param, but the worker closure unconditionally
  passed one → `E0061`.  The Text closure now branches on
  `generation::returns_owned_string(worker_def)`: owned-`String` workers are
  called `worker(cell, arg)` (no buffer); buffer-building workers keep the
  `let mut _w = String::new(); worker(cell, arg, &mut _w); _w` form.  Both par
  emitters (For + Queue) updated; verified on both backends over range / vector /
  text inputs (`tests/scripts/22c-par-sources.loft`).
- **Native fn-ref-returning par implemented (#281).** A par worker returning a
  function reference had no native lowering — the emitter fell through to a
  wrong-arity call to the interpreter stub (`E0061`).  Added the `QueueStitch::Fn`
  native path: `ClosureShape::Fn` (closure returns the native fn-ref tuple
  `(u32, DbRef)` verbatim) → `n_parallel_queue_fn_native` +
  `n_parallel_buf_get_fn_native` / `_drop_fn_native`, buffering one `(u32, DbRef)`
  per row in the new typed `Stores::par_fn_native_buffer_stack`.  Non-capturing
  fn-ref returns now compile + run on `--native`, matching `--interpret`.
- **Capturing closure from a par worker → clear diagnostic.** A par worker that
  returns a *capturing* closure used to hit a raw `index out of bounds` panic on
  both backends (the worker-local captured store is dropped at join, leaving the
  fn-ref dangling).  It is now rejected at parse time: "a parallel worker cannot
  return a capturing closure …".  The check (`worker_returns_capturing_closure`)
  flags only `FnRef` with a set closure-var in return/tail position, so a
  non-capturing `return add5;` and closures used only internally are never
  rejected.  Supporting capture would mean copying each captured environment
  across the thread boundary — deliberately not done.
- **Native narrow-integer-vector par fixed.** par over a `vector<u8>` / `vector<i32>`
  (or any narrow-Integer element) read garbage on `--native`: the worker-element
  closure used `get_int` (8 bytes) regardless of the element's 1/4-byte stride,
  over-reading across rows.  `tuple_arg_prep` now reads a scalar `Integer` element
  at the vector's stride (`element_size`, threaded in) — `get_byte` / `get_i32_raw`
  zero-extended to `i64`, matching the interpreter's `read_primitive_at`.  Other
  scalar kinds keep their fixed-width reads.  Verified both backends
  (`tests/scripts/22d-par-narrow.loft`).

### Program-relative asset loading — relative paths resolve against the program (#255 / @PLN9) (2026-06-04)


Relative file paths now resolve against the **program's own directory** (source
dir under `--interpret`, exe dir under `--native`, host cwd under wasm) instead of
the launch cwd — so bundled assets (fonts, images) load regardless of where the
program is started, unblocking the native games.  `Stores::resolve_path()` is the
single resolution home; absolute paths untouched.  Opt back into cwd-relative with
the `#cwd` file directive (the repo-root tools that operate on the working dir —
the doc generators, the tracker indexer/scanner, the branch-review viewer, the GLB
exporters — declare it); override globally with `LOFT_PATHS=program|cwd`.  Shipped
both backends + a 13-file corpus migration; the wasip2 print path stays gated on
#268.  (PR #269.)

### Coroutine native yield codec — per-shape spray → one layout-driven flatten-walk (@PLAN16) (2026-06-04)


The `--native` coroutine value channel had a per-shape codec: a hand-written
producer+consumer template per yield shape, gated on a runtime tag.  New composite
shapes fell through to the wrong arm — `(integer,float)`, `(integer,boolean)`,
`(vector,integer)` failed to compile.  Replaced with one **flatten-walk derived
from `T`'s slot kinds** (`src/coroutine_layout.rs`): each scalar slot inline as an
`i64`, each reference slot as a full `DbRef`; the per-slot kind list rides as extra
`OpCoroutineNext` args so producer (from `T`) and consumer (from the transmitted
kinds) agree by construction — no runtime shape tag.  Three previously-broken
composite shapes now compile + run via the single walk, zero per-shape code;
`coroutine_matrix` 18/18 green on both backends.  `(text, …)` tuples remain the one
excluded cell (a text element's `&str` repr needs a store intern).  @PLAN16 closed
→ `finished/`; the build was the *with-arm* that graduated DESIGN_VERIFICATION C1
into [Design Protocol 1](DESIGN_PROTOCOL.md).  (PR #269.)

### @PLN11 G2 Track 1 — program cache default-on + binary-mtime invalidation (2026-06-04)


`src/cache.rs`.  `cache::program_cache_enabled()` now returns **true by
default** for real (non-Cargo) invocations — the whole-program startup cache
(~3–3.6× warm-run speedup) is no longer hidden behind `LOFT_PROGRAM_CACHE`.

Precedence order (first match wins):
1. `LOFT_NO_CACHE` set → off.
2. `LOFT_PROGRAM_CACHE` set → on (explicit force; cache tests use it).
3. `CARGO_MANIFEST_DIR` present → off (inside `cargo run` / `cargo test`).
4. else → **on**.

`build_signature()` now folds `binary_signature_tag()` — the running exe's
mtime — so an uncommitted compiler rebuild invalidates bundles.  `BUILD_ID`
(git HEAD) alone did not change across uncommitted edits.

`cache::prune_program_cache()` evicts the oldest `(.store + .manifest)` pairs
after each cold save to keep the cache dir under `LOFT_CACHE_MAX_MB` (default
512 MiB).

Full design + E1/E2/E3 arc: `doc/claude/plans/11-data-as-store/README.md`.
Benchmark detail: `doc/claude/PERFORMANCE.md § Startup cache`.  Commit `77da481`.

### @PLN11 G2 — `read_data` breakdown: allocation-bound, E2 is the only lever (2026-06-04)


`src/ir_read.rs`.  `bench_read_data_breakdown` (`#[ignore]`; run with
`cargo test --release --lib bench_read_data_breakdown -- --ignored --nocapture`)
profiles `read_data` on the real stdlib bundle.

Results:

| Component | Time | Share |
|---|---|---|
| def-fields (attribute + return-type `Type` trees) | 453 µs | 65% |
| body trees | 142 µs | 20% |
| variable tables | 98 µs | 14% |
| **total** | **693 µs** | — |

Variable-table cost is **~0.39 µs/variable** — linear in allocation count, not
in variable count alone.  The hot work is native-graph materialisation (each
variable = a `String` + a boxed `Type`; each def = its attribute/return `Type`
trees).  No targeted `read_function` optimisation can beat this: the cost IS
the allocation.  E2 (zero-copy store-backed reads) is the only structural lever.

Corrects the earlier "~2.2 ms variable tables" figure, which measured a
whole-program bundle (~5–6 k vars) rather than the stdlib slice (~251 vars).
E2 startup prize sized at **~0.7 ms on the stdlib** (scales with def + var
count).  Commit `41835b2`.

### @PLN11 G2 M1 — `Definition` read-accessor seam completed codebase-wide (2026-06-04)


`src/data.rs`, `src/state/`, `src/generation/`, `src/parser/`, `src/compile.rs`.
All immutable `Definition` field reads across the four subsystems now go through
accessor methods (`name()`, `native()`, `source()`, `position()`, `attributes()`,
`code()`, `returned()`, `op_code()`, `known_type()`, `variables()`, `def_type()`,
`rust()`, `parent()`, `closure_record()`, `mutated_captures()`,
`scalars_to_box()`, `synthetic()`) instead of touching public fields directly.

The three milestones:
- M1a — `state/` — landed earlier in the arc.
- M1b — `generation/` — commit `c2741e2`.
- M1c — `parser/` + `compile.rs` — commit `69f0c6e`.

Derived fields (`attr_names`, `const_ref`, `code_position`, `code_length`)
stay as direct reads — they are cheap computed values, not layout-sensitive.

Pure refactor; no behaviour change, no test delta.  The seam is the
precondition for swapping the `Definition` backing representation to
store-based reads per subsystem (E2 arc in @PLN11).

### Nested-vector layout — four corruption/crash clusters fixed (plan-58) (2026-06-03)


Closed the `vector<vector<…>>` stability class across depth × element-type ×
context (plan-58, now `finished/`).  `vector<vector<…>>` is load-bearing (map
tiles, matrices, adjacency lists, comprehension results); a stride/type-id
investigation found four independent defects beyond the one filed crash:

- **Single sentinel (#262, `tests/scripts/183`).**  A freshly-created
  vector-of-vectors element is a 4-byte rec-id HANDLE, but `OpNewRecord` default-
  inits it with the inner scalar's null sentinel.  For `single` the NaN
  (`0x7FC00000`) is a non-zero garbage rec-id → SIGSEGV.  Generalized the `@P380`
  `OpSetInt4`-zero from the copy path to every construction path.
- **Narrow-int literal (`184`).**  `vector<vector<i32>> = [[1,2]]` typed the
  inner literal wide (`integer`, stride 8) while the read used `i32` (stride 4) →
  silent corruption.  `parse_item` now propagates the declared element type into
  the inner literal.  Width-general (i32/i16/u8).
- **Boolean handle stride (`185`).**  The outer vector strided handles by the
  inner scalar size — fine for ≥4-byte scalars, but a 1-byte `boolean` made the
  4-byte handles overlap (corrupt→crash).  Parse-time fix: pass the outer vector
  type as `OpNewRecord`'s type when the inner content is <4 bytes (so
  `record_new` strides by the handle), plus a read-stride clamp to ≥4 — no type
  classification change.
- **Nested comprehension (`186`).**  `[for i { [..] }]` wrote a 12-byte handle
  via the scalar `OpSetInt4` path → eval-stack skew → CONST_STORE write panic;
  and its `known` over-wrapped one level vs `+=` (off-by-one).  Deep-copy
  (`OpCopyRecord`) + element-type `known` (`vector_of`).  Distinct from #248.

Adjacent fix: **`vector<character>` element reads** (`v[0]` / `for c in v`)
errored "Field access not supported on type character" — `get_val` had no
`Type::Character` arm (only the write side did).  Added the symmetric
`OpGetCharacter` read (`tests/scripts/187`).

All fixes verified on `--interpret` and `--native`.  The temporary `--vec4`
investigation lever was added then retired (−109 lines).  Remaining
nested-vector matrix red cell is `#263` (call-returned fn-ref into any
collection — a general closures bug, out of scope).  Benign residual:
≥4-byte inner scalars still over-reserve the outer slot stride (no
corruption/leak) — accepted; a future stride guard (sanitizer) is noted.

### `loft-libs-core` first all-green chunk under strict CI (2026-05-30)


Landed `loft-lang/loft-libs-core` PR #2 (omnibus): canonical
`library-ci.yml` refresh, `cargo build --release --lib --bin loft`
infra fix (closes the `mmap_storage` blocker that broke every
package's native step), Phase 6r random re-clean (bare `#native`
+ source-scan `build.rs`), `arguments` warning sweep (zero
warnings under `LOFT_DENY_WARNINGS=1`, no `.allow_warnings`
opt-out).  All three packages — `arguments`, `crypto`, `random`
— now green on interpret + native under strict warnings.  Pre-
landed @P385 (parser type-inference asymmetry) + @P386 (native
codegen `Str/&str` mismatch) via #231 — both bugs surfaced
during the warning sweep.  Established three warning-clean
idioms now documented in `.claude/skills/loft-write/SKILL.md`:
`not null` on safe-to-default-`[]` vector fields,
capture-into-local before indexing (skip-pattern 5 needs bare-Var
vec), capture-and-null-check.  Plan-12 README gained a
"Bringing a chunk to all-green CI" checklist; REFERENCE.md
records the per-symbol re-clean decision rule (redundant vs
genuine override).  Remaining chunks: `loft-libs-net`,
`loft-libs-graphics`, plus the registry `pr-validate.yml`.

### @P321c native dimension closed + 8 harvested fixes (2026-05-26)


Dogfood pass against the `../personal/training` Loft port surfaced and fixed a
batch of native-codegen, interpreter, tooling, and library bugs.

**@P321c `imaging` native direct-call ABI — FIXED, commit `8095f4ba`.**
`src/generation/mod.rs::output_native_direct_call` now forwards a `LoftStore`
(built from the struct `Reference` arg's own `store_nr`, not the null store) and
marshals each `Reference` arg as a `LoftRef` (`to_loft_ref` + `transmute_copy`,
no `loft_ffi` type named → no dual-crate StableCrateId collision), so a
store-MUTATING package `#native` fn like `load_png(path, image)` gets its full
4-arg ABI.  Return-conversion (`from_loft_ref`) split from the store-handle need
(`returns_loft_ref` vs `needs_loft_store`).  `loft generate` (`src/main.rs`) now
reads field offsets from the canonical schema (`Stores::position`/`size`) instead
of a separate layout calc that treated plain `integer` as 4 bytes (real layout:
`width@0`/`height@8`/`name@16`/`data@20`); `lib/imaging/native` corrected to those
offsets + `set_long`/`get_long`.  imaging un-skipped from `LIB_PKGS_NATIVE_SKIP`;
`native_library_suite` 53/53.  Only the browser-WASM half of @P321c remains.

**@P347 text ordering compare — FIXED, commit `a3e2e269`.**
`< <= > >=` between a `vector<text>` element (`&str`) and another text (`&String`)
failed `--native` compile (`PartialOrd` has no cross-type impl; `==` worked via
`PartialEq`).  `OpLtText`/`OpLeText` (`default/01_code.loft`) now route through
`ops::op_lt_text`/`op_le_text` (`AsRef<str>`), coercing both to `&str`.  `make
fill` regenerated.  Regression `tests/scripts/repro_p347.loft`.

**@P338 vector-index `&mut stores` double-borrow — FIXED, commit `a3e2e269`.**
`v[n / 2]` (checked-div guard `raise_runtime` + vec-get receiver) → E0499.  The
`OpGetVector`/`OpVectorRef` templates now bind `@index` to a local after `@r`.
Regression `tests/scripts/repro_p338.loft`.

**@P346 empty-text `Set` to a `RefVar(Text)` — FIXED, commit `ed47892c`.**
A string interpolation used as an if-branch result over a vector-indexed value
in a loop accumulated text on the interpreter (`[2.5][2.58][2.581]`).
`State::set_var` (`src/state/codegen.rs`) treated `Set(refvar_text, "")` as a
no-op; the buffer kept the prior iteration's content and `OpFormatStack*`
appended.  Now emits `OpClearStackText` (deref-clear), matching native.
Regression `tests/scripts/repro_p346.loft`.

**@P339 `lib/graphics` text kerning — FIXED, commit `29315f20`.**
`measure_text`/`rasterize_text` (`lib/graphics/native/src/text.rs`) summed bare
advance widths.  Both now apply fontdue `horizontal_kern` (rasterize via a float
pen).  `gl_measure_text("AV",40)` = 59.20 < `A+V` 61.91.  Regression
`lib/graphics/tests/kerning.loft`.

**@P341 native-test cache key — FIXED, commit `a3e2e269`.**
`native_cache_key` (`src/native_utils.rs`) now folds each native-package rlib's
mtime, so rebuilding a lib cdylib invalidates the cached `_bin`.

**@P345 typed loop-var diagnostic — FIXED, commit `a3e2e269`.**
`for i: T in …` now emits one clear "loop variable is type-inferred — remove the
annotation" message + recovery (`src/parser/collections.rs::parse_for`), not a
3-error cascade.  (Syntax intentionally unsupported.)

**@P342 `loft generate` method-as-field — FIXED, commit `a3e2e269`.**
The `u16::MAX` schema-position skip is the correct field/method discriminator;
generated `*_fields` no longer emit bogus constants for methods.

Also filed (open): @P343 (vector<fn-ref> for-loop mis-dispatch — partial
diagnosis recorded, P214-class).

### Open-bug design pass — 4 fixes + 5 grounded designs (2026-05-26)


A focused pass over the remaining open P-issues: each was carried to a
code-grounded fix design, then implemented + verified where the dev
environment allowed.

**@P348 GL golden HiDPI — FIXED.** `tests/graphics_gold.rs::crystal_editor_gl_matches_gold`
degraded the exact-dimension `assert_eq!` to a graceful skip when the captured
framebuffer differs from the gold (a HiDPI/display-scaled environment can hand
a scaled framebuffer even under `xvfb-run`).  CI + `make test-gl-golden`
(controlled size) still compare pixels.

**@P332 Windows install → 0 installed — FIXED.** Root cause: the install/extract
home resolves via `dirs::home_dir()`, which reads `$HOME` on Unix but
`USERPROFILE` on Windows — so the e2e test's `HOME=<tmpdir>` isolation leaked
into the real profile and cross-run caching routed everything to
`skipped_cached`.  `registry_index::cache_dir()` now honours a cross-platform
`LOFT_HOME` env var first (`HomeGuard` sets it); both `#[cfg(not(windows))]`
gates removed; `registry_e2e` 5/5.  Production unchanged (var unset →
`dirs::home_dir()`).

**@P333 Windows `/tmp/` fixtures — FIXED.** `moros_render/geometry.loft` +
`moros_sim/persistence.loft` ported to cwd-relative filenames + `delete()`
(the `scene_glb.loft` convention); Windows skips removed from `wrap.rs` +
`native.rs`.  moros_sim 137/137 + moros_render 155/155, no artifacts left.

**@P340 baseline metric — PARTIAL FIX.** New `gl_font_ascent(font, size) -> float`
(fontdue `horizontal_line_metrics`) lets callers baseline-align mixed-size
text; additive, so the text golden is untouched.  Needed a new
`(I32,F64)->F64` auto-marshal arm in `src/extensions.rs`.  `lib/graphics/tests/font_ascent.loft`,
66/66 both backends.  The `size*1.2`/`size*0.8` rasterization constants are
deliberately unchanged (switching them needs a `gold-text.png` regen).

**Designs recorded, implementation deferred (blocker noted in each PROBLEMS.md row):**
@P334 (`lib/world` wasm trap —
needs `wasmtime`, not installed here), @P343 (all three interp layers now
precisely located incl. the termination-test third layer; native E0600 half
separate), @P344 (doc-fix recommended; skill-checklist edit permission-blocked;
per-loop-scoping rejected as a core-model change for a Low bug), @P331 (cdylib
i64→i32 truncation site found; fix is an M-effort ABI-width alignment touching
the 53-cdylib gate — not blind-patched).

### @P349 — browser WASM playground: refresh bundle + JSON + file I/O (2026-05-26)


Refreshing the `doc/pkg` browser bundle (stale since 2026-05-18) against the
`../personal/training` port's `.field()` routine syntax surfaced a chain of
three gaps that left the gallery/playground unable to run file-reading or JSON
programs.  All fixed:

1. **Stale bundle.** `make wasm` rebuilt `doc/pkg/{loft.js,loft_bg.wasm}` from
   current source (`loft_bg.wasm` 2211894→2260122→2262xxx bytes across the
   three rebuilds).  The in-browser parser now accepts the JsonValue method
   syntax it rejected before (`Expect token ;`).
2. **`06_json.loft` not bundled.** `DEFAULT_FILES` (`src/wasm.rs`) embedded
   `01_code`..`05_coroutine` but not `06_json.loft`, so `json_parse` was an
   `Unknown function` in-browser (native JSON fns were already compiled in —
   no wasm cfg-gate).  Added the embed.
3. **Runtime `file()` ignored VIRT_FS.** `State::get_file_text`'s
   `#[cfg(feature="wasm")]` branch (`src/state/io.rs`) read only via the JS
   `host_fs_read_text` bridge (absent in the playground) → `file().content()`
   returned `""`, so `json_parse(file(...).content())` → `JNull` → `NaN`.  Now
   consults `wasm::virt_fs_get` first (where `compile_and_run` puts passed
   files), falling back to the host bridge — live-FS hosts unaffected.

Verified under Node (`initSync`+`compile_and_run`): `file().content()` →
`HELLO123`; `json_parse(file).field("activities").item(0).field("duration_s").as_number()`
→ `3600`, matching native.  Remaining minor caveat (in the @P349 PROBLEMS.md
row): `run_pipeline` picks `main` as the alphabetically-first user file
(`.min()`), so a data file sorting before the program is mis-compiled as main.

`doc/brick-buster.html` is a self-contained `--html` bundle (base64-embedded
wasm) — independent of `doc/pkg`.  (Earlier note here said its embedded wasm
"runs `loft_start: OK` under `tools/wasm_repro.mjs`, no @P337 trap" — that was
a FALSE NEGATIVE: the stub harness's `loft_gl_create_window` returns 0, so the
program bails before drawing and never reaches the render path.  See the @P337
correction below and @P351.)

### @P337 — Brick Buster browser bundle: corrected diagnosis + pipeline hardening (2026-05-26)


@P337 ("Brick Buster broken on the site / page times out") had been recorded as
a `vector<float>` length divergence on wasm32 (`panic_bounds_check` in
`build_mvp_2d`).  **That diagnosis is DISPROVEN.**  A minimal repro (16-elem
`vector<float>` literal in a struct field, index `[15]`) AND a faithful copy of
`build_mvp_2d` (computed-expression projection passed as `const vector<float>`,
indexing `proj[0..15]` + building a new 16-float vector) BOTH read back
`len==16` and `[15]` correctly on the wasm32-unknown-unknown `--html` build —
identical to interpreter + `--native` — even after `wasm-opt -O1 --asyncify`.
The committed `doc/brick-buster.html` renders cleanly in real headless Chromium
(WebGL via SwiftShader), rAF ~60fps.

**Actual root cause — a build-pipeline / toolchain hazard, not a runtime bug.**
`make wasm` (wasm-pack, `feature=wasm`) and `loft --html` write the SAME
`target/wasm32-unknown-unknown/release/libloft.rlib` with incompatible feature
sets (the Makefile has long warned of this).  Two independent break modes, both
passing the old size/DOCTYPE sanity check:

1. **rlib STOMP** — after `make wasm` the rlib carries `feature=wasm` →
   `wasm-bindgen`/`js_sys`, so `--html` emits a wasm importing
   `__wbindgen_placeholder__` (35+) that the embedded `loft-gl-wasm.js` glue
   (raw `loft_gl`/`loft_io` externs only) does not provide → the page fails to
   instantiate.  A correct `--html` bundle imports ONLY `loft_gl` + `loft_io`.
2. **MISSING `wasm-opt`** — without binaryen the `--asyncify` pass never runs,
   so there is no frame-yield; the HTML driver runs `loft_start()`
   synchronously and brick-buster's `for _ in 0..10000000` render loop blocks
   the browser main thread forever ("page times out").

Today's doc/pkg `make wasm` stomped the rlib; the working-tree
`doc/brick-buster.html` had separately been rebuilt without `wasm-opt`.

**Landed (toolchain + hardening — diagnosis-only on the runtime, no codegen
change):**
- `tools/check_html_bundle.mjs` — static gate: fails on non-`loft_gl`/`loft_io`
  imports (stomp) or a missing `asyncify_start_unwind` export (no frame-yield).
  Wired into `make game` step 6 so a broken bundle fails the build.
- `loft --html` (`src/main.rs`) — now warns LOUDLY, in plain language, when
  `wasm-opt` is absent (the page will hang, not merely be larger).
- `scripts/doctor.sh` + `make doctor` — full wasm/native toolchain report with
  plain-language consequences and package-manager-specific install commands;
  finds cargo/wasmtime-installed tools regardless of shell PATH.
- `doc/claude/WASM.md` — new "Build Toolchain Dependencies" section + the
  rlib-stomp build-order rule.
- `doc/brick-buster.html` regenerated via `make game` (correct rlib + asyncify),
  verified in headless Chromium.

**Follow-ups filed:** @P350 (a DIRECT `loft --html` after `make wasm` still
ships a broken bundle silently — the gate is only in `make game`; needs an
isolated rlib `--target-dir` or `--html` self-validation), @P351 (the
`tests/html_wasm.rs` Node gate cannot exercise the GL/render path — the exact
coverage gap that let this ship + be misdiagnosed).

### Native codegen — eliminated the `output_call_inner` match (2026-05-22)


`src/generation/dispatch.rs::output_call_inner` no longer contains a monolithic
`match` of per-Op emission arms — it is now just a registry-first guard
(`emit_op`) plus the template/user-fn fallback.  The 14 remaining arms were
relocated VERBATIM into `OpEmitter`s: the text/format/buffer family into one
`ops::text_ops::TextDispatchEmitter` (reproducing the @P283 refvar→`Stack`
rewrite internally), and the pass-throughs (`OpConvRefFromNull` / `OpGetTextSub`
/ `OpDatabase` / `OpStep` / `OpRemove`) into `ops::misc_ops`.  No `#rust`
template changed, so `src/fill.rs` (the interpreter) is byte-identical and
native emission matches the deleted arms byte-for-byte.  The
`dispatch_op_arm_budget` test is repurposed as a 0-ratchet that fails if a
`"Op…" =>` match arm is ever re-introduced.

### @P321 native library gate — 16/17 packages green (2026-05-23)


Seven native-codegen root causes (@P321a–g) that blocked `tests/native.rs::native_library_suite` from reaching
full green.  Splits were fixed and un-skipped independently; the gate now covers 16/17 packages.  Only `imaging`
remains skipped (`LIB_PKGS_NATIVE_SKIP`) pending @P321c (design-level, M+).

**@P321d `moros_map/serial` — FIXED 2026-05-23, commit `93a43051`**

`default/01_code.loft` / `src/fill.rs`.
Nested vector index `m.a[0].b[2]` emitted two live `&mut stores` borrows (E0499).
`vec_get_or_raise_runtime` is `&mut self` (may call `raise_runtime` on OOB); the outer
`stores.vec_get_or_raise_runtime(&<inner>, …)` held its receiver borrow across argument
evaluation while `<inner>` expanded to a second such call — Rust E0499.
Fix: the `OpGetVector` / `OpVectorRef` `#rust` templates in `default/01_code.loft` bind
`@r` to a local first (`{{let __vr = @r; s.vec_get_or_raise(&__vr, …)}}`), so the inner
borrow is fully evaluated (owned `DbRef`) before the outer call starts.  `src/fill.rs`
regenerated via `make fill`; the interpreter gets the same harmless local binding — single
source of truth for both backends.
Regression: `tests/scripts/repro_p321d.loft` (both backends).

**@P321e `moros_editor` — FIXED 2026-05-23, commit `da75dc67`**

`src/generation/emit.rs`.
A text-returning fn whose body is a `match` of format strings `.to_string()`'d the match
result into a `__ret_N` LOCAL `String`, then returned `Str::new(&local)` — a borrow of a
fn-local that drops at return → runtime `ptr::copy` panic in the caller's `.to_string()`.
A `RefVar<Text>` work-buffer arg existed but `output_set`'s P205 scratch guard fires only
when the returned value is a `RefVar` — the fn was returning a DIFFERENT local.
Fix: the text-`Return` path in `output_block` also routes through `stores.scratch` when the
returned value is a text LOCAL var (not already a `RefVar<Text>` work buffer).  moros_editor
5/5 files native + interp.

**@P321g `moros_ui` — FIXED 2026-05-23, commit `69f4ec3b`**

`src/generation/dispatch.rs`.
A `&`-ref-param call on an assignment RHS (`ec_hit = route_click(p, st.es_tools, …)`)
emitted `let` in expression position — `error: expected expression, found 'let' statement`.
The `&`-ref arg `st.es_tools` (an addressable field) materialises a
`Set(__ref_N, OpGetField…)` statement ahead of the call, so the RHS is
`Insert([Set(__ref_N, …), Call])` wrapped in `Value::Span` for source-position tracking.
`output_set`'s S35 hoist — which lifts all-but-last Insert ops to statements — matched only
a *bare* `Insert`, so `Span(Insert)` fell through to the brace-less `Insert` arm of
`output_code_inner`, producing `let x = let __ref_N = …; call()`.
Fix: S35 unspans `to` before the Insert check.  moros_ui 4/4 files native + interp.
Regression: `tests/scripts/repro_p321g.loft`.

**@P321c `imaging` — DIAGNOSED, needs design, NOT fixed** *(status as of
this 2026-05-23 entry — closed three days later; see the 2026-05-26
"@P321c native dimension closed" entry above, commit `8095f4ba`)*

`output_native_direct_call` (`src/generation/mod.rs:2181`) cannot express a
store-MUTATING `#native` fn: `load_png` decodes a PNG, allocates the pixel vector, and
writes `name`/`width`/`height`/`data` into the `Image` struct.  The ABI only marshals
text → `(ptr,len)`, vector → `(*const ELEM, count)`, and scalars; no `LoftStore` path
and no struct-ref marshalling → emits a 3-arg call to a 4-arg fn (E0061).
Recommended route: `codegen_runtime + Abi::Cell` (the crypto pattern, with store access)
reusing `src/png_store.rs::read`, with new `(text, struct-ref)` call-marshalling, a dual
interpreter(cdylib)/native(codegen_runtime) split, and `png`-feature gating.
Full diagnosis in PROBLEMS.md @P321c.  `imaging` stays in `LIB_PKGS_NATIVE_SKIP`.

---

### @P274 closed 2026-05-14 — heap-typed tail return + text-concat type-dispatch


Two coordinated codegen + parser fixes for native-only crashes
that surfaced when @PLN42 viewer added the
`render_md_table_row` / `parse_md_row` / `find_table_headers`
helper trio (commit 89fd2767).

**Bug A — `OpFreeRef` hoisted before tail-call argument use**
(`src/generation/pre_eval.rs`).  `patch_hoisted_returns` Pass 2
collapsed `[Call(parse_row, …, var___ref_1), OpFreeRef(var___ref_1),
Return(Null)]` (emitted by `scopes::free_vars`'s else-branch for
heap-typed tails — Vector / Reference / Enum-ref bypass
`is_value_return_type`'s primitive-only check) into
`[OpFreeRef(var___ref_1), Return(Call(parse_row, …, var___ref_1))]`,
giving native code `OpFreeRef(var); var.store_nr = u16::MAX;
return n_parse_row(…, var)` — callee derefs `stores[65535]` and
panics at `src/keys.rs:249`.  Two-part fix: (1) Pass 2 now
detects when `expr` references any var that an intervening free
op will free, and skips the hoist; (2) `detect_ref_tail_capture`
now accepts `Type::Never` blocks and looks up the enclosing
function's return type — so the original `[Call, free,
Return(Null)]` pattern in early-return arms gets the
`let __native_tail_ret = call(…); free; return __native_tail_ret;`
wrap that orders the use BEFORE the free.

**Bug B — `text + integer` concat misrouted** (`src/parser/vectors.rs`).
`parse_append_text` only dispatched parts on Text / Character;
integer / float / boolean / vector / enum etc. fell through to
`OpAppendText` with the wrong operand type → SIGSEGV in interp,
rustc E0614 "type i64 cannot be dereferenced" in native (the
`+= &*(...)` deref).  Fix routes non-text/non-character parts
through `append_data` (the same dispatch the `"…{x}…"` format-
string interpolation path uses), unwrapping `RefVar(inner)` so
`&text` parameters keep the existing OpAppendStackText / OpAppendText
fast path.

Pinned by `tests/scripts/100-p274-tail-return-with-cleanup.loft`
(walked through both backends by `tests/native.rs::native_scripts`
and `tests/wrap.rs`).  `make view` reverted to `--interpret`
default in 5dae80cc, restored to `--native` once @P274 closed.

### Plan-35 (branch-review viewer) closed 2026-05-14


Plan-35 ran 2026-05-13 → 2026-05-14.  Goal: a browser-accessible
doc + code review surface for the current loft branch, served by a
loft-script binary against the host loft binary as a frozen pair.

**Per-phase summary** (all shipped 2026-05-13 unless noted):

- **00** Skeleton + binary build.  `tools/viewer/` package layout,
  `make view-build` + `make view` + `make view-refresh` Makefile
  targets, `BUILD_NOTES.md` records the loft commit the viewer was
  built against.
- **01** HTTP routes.  Server skeleton via `lib/server`, `/`, `/tree/<path>`,
  `/raw/<path>`, `/static/style.css`, 404 fallback.  Originally
  blocked from `--native` by @P262 + @P263; fixed in the seven-bug arc.
- **02** Code-file rendering.  `/file/<path>` renders any text file as
  line-numbered HTML with `<a id="L42">` anchors for fragment scrolling.
  HTML escape + tab-to-4-spaces + binary-extension skip-list.
- **03** Markdown subset (later extracted to `lib/markdown`).  Headings
  with GH-slug ids, paragraphs, fenced code blocks, inline formatting
  (bold/italic/code/strikethrough), links with relative-path resolution,
  images, autolinks, blockquotes, lists with continuation merging,
  GFM tables with alignment, task lists, setext headings, hard line
  breaks, backslash escapes, HTML escaping.  Extracted as standalone
  `lib/markdown/` library + `lib/markdown/tests/01-render.loft` (~30
  in-library assertions, one per construct).  Two follow-up extensions
  shipped 2026-05-14: `extract_headings(source)` returning
  `vector<Heading>` for TOC building; `tag_url_prefix` and
  `image_url_prefix` parameters wiring `@P-id`/`@PLAN-id` autolinks
  + relative image rewriting (caller chooses prefixes).
- **04** Git state via wrapper script.  `tools/viewer/refresh.sh` dumps
  branch + changed-files + recent-commits + uncommitted state to
  `tools/viewer/state/*.json` (uses `git` + `jq`).  Viewer reads JSON
  via the (now fully-wired) JSON natives from P54 sprint completed
  the same day.
- **05** Diff + commit views.  `/diff/<path>` and `/commit/<sha>` use a
  shared `render_diff()` helper that classifies each line and wraps it
  in `.diff-add` / `.diff-del` / `.diff-hunk` / `.diff-head` / `.diff-meta`
  / `.diff-ctx` / `.diff-noeol` spans.  Top-right `[Rendered ¦ Diff vs main]`
  toggle on every `/file/` page; the diff link hides when no per-file
  diff exists.  `breadcrumbs()` fix: parent dirs always link to
  `/tree/<dir>`; only the leaf segment uses the page's kind, so
  `/diff/<path>` doesn't generate broken `/diff/<dir>` parent links.
- **06** Full GFM tables — alignment + headers + body + nested formatting
  in cells (via `render_inline`) shipped via the `lib/markdown` table
  renderer.  Multi-line cells + escaped pipes deferred (rare in loft
  docs; promote when a downstream consumer needs them).
- **07** Closeout (this entry) — DEBUG.md § Branch review viewer +
  CHANGELOG.md user entry + this technical retrospective + plan moved
  to `plans/finished/35-branch-review-viewer/`.

**Loft drivers — features matured by building this**:

- `lib/server` proven well beyond test fixtures (lib's first big
  consumer outside the test suite).
- The seven-bug native arc @P262→@P269 (closed 2026-05-13) was
  surfaced by trying to compile the viewer to `--native`.  Each bug
  was a real loft-codegen issue that was invisible until a real
  consumer walked the path — `lib/web` + `lib/server` integration,
  text-returning fn inline calls, fn-ref dispatcher work-buffers,
  cross-crate native fn routing, JSON parser UTF-8, JSON natives
  todo-stubs.  Closed via DESIGN_DECISIONS.md § C67 ("fail at startup,
  not at runtime — internal-bug runtime panics caught at compile time").
- P54 (JsonValue ecosystem) native side completed via @P268 + the
  16-fn follow-up wiring all 23 JSON natives in
  `src/codegen_runtime.rs`.
- New `lib/markdown` library spun out as a reusable single-file loft
  module, ~720 lines, with comprehensive in-library tests; first
  pure-loft library born from a real consumer.
- Surfaced gaps not blocking the ship: subprocess primitive
  (workaround: `refresh.sh`), regex (workaround: char-by-char
  parsing), HTML escape lib (workaround: `html_escape` in `lib/markdown`
  exposed publicly).

**Plan moved to `plans/finished/35-branch-review-viewer/`.**

---

### Plan-37 phase 04b — viewer per-doc sidebar shipped 2026-05-14


`tools/viewer/src/main.loft` gains two sections at the
bottom of every `/file/<path>` page:

- **Referenced by** — reads `index/tags.json`'s `links`
  bucket (phase 09 backlinks).  Lists every doc that links
  inbound to the current file, with file:line + context.
- **Tags on this page** — walks the tag buckets, surfaces
  any tag whose ref list contains the current file.  Each
  tag is a clickable chip → `/tag/<bare>` (phase 04a's tag
  detail page).

Both render to empty strings when `index/tags.json` is
missing or the file has no associated entries — pages
degrade gracefully on a fresh checkout that hasn't run
`make index` yet.

CSS additions: `section.sidebar` for the section wrapper,
`ul.tag-list` for the chip-style tag pills (flex-wrap, dark
mode covered).

Only the welcome-landing half of phase 04b remains open;
that one depends on @PLAN35 phase 08 which is unstarted.

Verified end-to-end 2026-05-14:
`target/release/loft --interpret --lib lib/ tools/viewer/src/main.loft`
starts the viewer; `curl localhost:8765/file/doc/claude/PROBLEMS.md`
returns 295 KB of HTML containing both sidebar sections
("Referenced by 59" + "Tags on this page" with @P198…@P204
chips, all populated from `index/tags.json`).  An earlier
report of a `--interpret` extension-loading panic (filed as
@P273) turned out to be a stale `target/release/loft`
binary predating the cdylib's last build — once rebuilt
fresh, the `apply_manifest_side_effects` path picks up the
dep cdylibs correctly via `auto_build_native`.  @P273
closed as no-bug; lesson recorded for the next "missing
native" symptom.

---

### Plan-37 phase 09 follow-up — broken-link cleanup shipped 2026-05-14


The 61 broken markdown links surfaced by phase 09's
`broken_links` bucket are all cleaned up.  CI gate enabled
(`tests/index_hygiene.rs::index_hygiene_clean` checks both
`.broken` and `.broken_links` are empty).

**What landed:**

- `tools/indexer/fix_broken_links.py` — auto-fix script:
  for each broken target, tries the `try_extra_dotdot`
  heuristic (pop intermediate path segments and check
  whether the result exists).  Catches the dominant
  off-by-one `../` case where a plan README in
  `plans/<dir>/` cites a top-level doc as `../X.md` but
  needs `../../X.md`.  Manual override map for the
  plan-22 / plan-35 closeout drift.
- Scanner tightened in `tools/indexer/scan.sh` — link
  extraction now uses per-file awk that tracks fenced
  code-block state; example links inside `\`\`\`markdown`
  blocks no longer count as real refs.  Cost: ~1.5 sec
  added to the scan (was 1.5s, now 3s; still under the
  5-sec budget).  Without this fix, the auto-fixer
  rewrote example links to bogus paths in several files
  (caught + reverted via the validate-after-fix step).
- 106 of the 61 distinct broken-target refs auto-fixed
  (61 distinct targets, but 106 individual ref sites).
  Remaining ~20 manually patched: missing-doc citations
  (`DX.md`/`LSP.md`/`WEB_SERVER_LIB.md`) redirected to
  the corresponding `lib_plans/future/` dirs;
  `BYTECODE_CACHE.md` and similar not-yet-written sibling
  docs converted to plain-text mentions; intentional
  template / test-fixture / inline-backtick examples got
  `<!--noindex-->` markers.
- `tests/index_hygiene.rs` rewritten as a single
  `index_hygiene_clean` test (was two parallel tests
  racing on `make index`; corruption surfaced as
  intermittent failures).

**Numbers:**

- Before: 61 broken markdown links, 309 link targets, 1297
  inbound links.
- After fence-aware scanner: 19 broken (42 false positives
  from fenced examples removed), 264 targets, 1277 links.
- After cleanup: 0 broken, 245 targets, 1267 links.

The phase 09 follow-up section in
`plans/42-tracker-index/09-backlinks.md` is updated to
mark this closed.

---

### Plan-37 phase 06 — retroactive `@`-tagging shipped 2026-05-14


`tools/indexer/migrate.py` rewrites bare-name tracker
references to `@`-prefixed form across `doc/claude/**/*.md`:

- `P259` → `@P259` when 259 is a valid PROBLEMS.md row ID.
- `plan-22` → `@PLAN22` when 22 is a valid plan dir number.

The migration is **conservative on purpose** — false
positives in 154 files would be expensive to clean up by
hand:

- Skip `P\d+-R\d+` (phase-N risk-M notation in COROUTINE.md
  / CHANGELOG_TECHNICAL.md / SAFE.md).
- Skip single-digit `P[1-9]` — overloaded with PERFORMANCE.md
  design IDs ("Design: P1") and plan-N phase-M shorthand
  ("P5.2").  Two-digit `P54` and longer are unambiguous
  enough.
- Skip refs preceded by `/` (`/tag/P259` URL routes
  shouldn't break).
- Skip refs inside fenced code blocks.
- Skip refs inside same-line backtick spans (so `` `P259` ``
  examples explaining the convention survive).
- Skip lines containing `<!--noindex-->`.

Multi-line backtick spans (rare but present in CLAUDE.md's
"## Tracker tags" section) need explicit `<!--noindex-->`
markers per line.

**Numbers**:

- 1500+ refs migrated across ~150 `.md` files.
- Legacy bucket dropped from 2643 → 1917 refs (the residue
  is single-digit `P1`-`P9` skipped on purpose, refs to
  closed P-issues no longer in PROBLEMS.md, and code-file
  refs which don't migrate).
- New form: 783 `@P-id` refs + 753 `@PLAN-id` refs.
- `tests/index_hygiene.rs::no_broken_tracker_tags` still
  green.

Phase 06 was originally framed as "retroactive tagging +
closeout" — the closeout half is DEFERRED to after phases
7+8 (loft-native scanner + multi-project deploy).

---

### Plan-37 phase 09 — backlinks (links bucket) shipped 2026-05-14


Indexer now extracts every `[text](path.md)` markdown link <!--noindex-->
across the repo, resolves the target relative to the source
file's directory, and groups inbound refs under a top-level
`links: {target: [{file, line, anchor, context}]}` bucket
in `index/tags.json`.  Path resolution handles `..`, `./`,
repo-root `/...` paths, anchors, and skips http(s)/mailto
schemes.

Two new CLI surfaces on `./scripts/idx`:

- `incoming:<path>` — inverse of `file:`; lists everything
  that links TO the given path.  Trailing `/` resolves to
  `/README.md`; bare basename matches against any key
  ending in `/<name>` (returns `{ambiguous: [...]}` when
  multiple paths match).  `incoming:@PLAN35` delegates to
  the existing `tag:` lookup so the same query shape works
  for both file paths and `@`-tags.
- `broken-links` — sibling of `broken`; lists links
  pointing at non-existent files.  Initial scan surfaced
  62 stale references on the loft tree (mostly off-by-one
  `..` counts in `doc/claude/plans/<dir>/README.md` after
  files were moved to `finished/`).  No CI gate yet — the
  cleanup is a follow-up before tightening
  `tests/index_hygiene.rs` to fail on broken links.

**Bug fixes during the work:**

- **awk match() clobber** — the link-extraction inner
  loop's `resolve(base, target)` helper called `match()`
  internally, clobbering `RSTART`/`RLENGTH` for the outer
  loop's substr-advance step.  Effect: every link emitted
  twice (the second emit re-walked the same content from
  a stale offset).  Fixed by capturing `RSTART`/`RLENGTH`
  into local `rs`/`rl` before any helper call.
- **awk single-quote in shell-quoted block** — a comment
  containing `loop's` broke out of the bash single-quoted
  awk script, surfacing as "syntax error near token `('"
  at the apparent line of the next awk statement.  Comment
  rephrased to drop the apostrophe.
- **jq `--argjson` ARG_MAX overflow** — the assembled
  `LINKS_JSON` (~150 KB on the loft tree) exceeded the OS
  argv limit.  Switched the merge step to `--slurpfile`
  reading from a temp file (no argv pressure).

**Performance:** scanner runs in 1.5 sec on 953 files (was
1.0 sec without the link pass).  No CI gate impact.

`tests/index_hygiene.rs` continues to enforce zero broken
`@`-tag refs (phase 03 contract).  The `broken_links`
bucket is detection-only for now.

---

### Plan-22 (mutable closures) closed 2026-05-13


Plan-22 ran 2026-05-10 → 2026-05-13.  Goal: make closures whose
bodies mutate captures work intuitively without user-visible
annotation — implicit-by-body classification into cases A/B/C.

**Per-phase summary** (all SHIPPED 2026-05-13):

- **00** Matrix freeze + harness wiring.  `tests/mut_closure_matrix.rs`
  scaffolded (44 cells cross-mode); Case A baseline cells green.
- **01** Mutated-captures detection.  `walk_for_mutations` walker marks
  captures as `mutated: bool`; no behaviour change.  Known gap: first-
  pass GetField in `src/parser/vectors.rs:2498-2512` is non-load-bearing
  post-@P260 (cells handle both sides).
- **02** Case B (co-scoped mutating) + all sub-phases:
  - 02b: auto-Reference attribute emission in `synthesize_closure_record`.
  - 02c: Reference-type capture routing in `typedef.rs::fill_database`.
  - 02d-iii.a: `scalars_to_box` type-flip helper (outer local → cell).
  - 02d-iii.b: read auto-deref hook in `parse_var`.
  - 02d-iii.c: boxed-scalar assign-rewrite helper + `change_var_type` guard.
  - 02d-iii.d: `cell_alloc_prepend` helper for first-set rewrites.
  - 02d-vii: text-return crash fix (cell encoding + return routing).
- **03** Case C (factory / escaped closure).  Liveness check + @P259 fix
  (4 commits — OpIncRc + cascade-free cell ownership for multi-factory
  pattern).
- **04** DECOMMISSIONED 2026-05-13.  The cell + auto-Reference from
  phases 02-03 already gives Case D correct shared-state semantics;
  outer + closure share the same cell automatically.  No rejection code
  shipped.  See `04-case-d.md § Major finding`.
- **05** DEFER-BY-DEFAULT.  `Mutable<T>` helper unnecessary: the cell IS
  the shared-ownership mechanism.  Revisit only if a concrete use case
  surfaces that cells can't handle.
- **06** Doc closeout (this entry).  DESIGN_DECISIONS.md C38 updated;
  CAVEATS.md C38 cross-reference updated; ROADMAP.md @PLAN22 row
  removed; plan moved to `finished/`.

**Bug yield — P-issues filed during @PLAN22:**

- **@P256** — vector-capture into closure crashed both backends (no clean
  rejection).  Closed 2026-05-12 with parse-time rejection in
  `src/parser/objects.rs::resolve_name`.  Pinned by
  `tests/parse_errors.rs::p257_vector_capture_in_closure_rejected`.
  *(Filed as part of @PLAN15 closeout probing, attributed to @PLAN22's
  scope — collection capture is a closure-record layout issue.)*
- **@P257** — same as @P256 (duplicate tracking number; see PROBLEMS.md).
- **@P258** — native + interp layout divergence for cell-encoded scalars.
  Closed in phase 02d-iii.b.
- **@P259** — multi-factory cell ownership crash (OpIncRc missing +
  cascade-free teardown).  Closed 2026-05-13 via 4 commits
  `9f00afec` / `29ee04fd` / `cfb65e8b` / `0711973b`.
- **@P260** — closures captured `Type::Reference` by deep-copy; mutations
  silently no-opped.  Closed 2026-05-13 via `cfad6274` (one-line
  architectural fix: drop `is_mutated` gate in `synthesize_closure_record`).
  6 new cross-mode cells in `tests/mut_closure_matrix.rs`.
- **@P261** — vector-field literal-assign appended instead of replacing.
  Closed 2026-05-13 via `a1cf258a` (prepend `OpClearVector` in
  `towards_set`'s vector-literal path).  Pinned by
  `e_d3_struct_vector_assign_in_closure`.

**Final test surface**: 44 `mut_closure_matrix` cells + 22
`closure_matrix` (@PLAN15 regression net) + 633 issues suite + 26
leak guards — all green, interp/native byte-identical.

Active plans remaining after close: 1 (07-error-messages).
Plan moved to `plans/finished/22-mutable-closures/`.

### Plan-15 (closure validation matrix) closed 2026-05-12


Plan-15 ran in one session 2026-05-12 — promoted to current,
shipped all 6 phases, and closed.  Final matrix: 22 cells in
`tests/closure_matrix.rs` across 6 capture shapes (C0
non-capturing, C1 single-int, C2 text, C3 Reference, C5 multi-
basic, C6 nested) and 4 destinations (D1 local, D2 direct
stack, D3 struct field, D4 vector element — D4 only for C0).
Plus 5 leak guards in `tests/leak.rs::p15_phase0[345]_*_no_leak`
covering text / Reference / nested-closure capture surfaces
under 100-iteration tight loops.

**Bug yield: 0** new P-issues filed.  All gaps the plan was
designed to surface (closure-DbRef leak, "move-vs-copy
semantics gap analogous to T1.8c") turned out to be non-
issues — the underlying support landed earlier through @P213
(2026-05-04 — `Parts::ChildRec` layout-widening for struct-
field captures), @P214 (2026-05-05 — vector-of-non-capturing
closures), @P215 (2026-05-05 — nested closure name resolution),
and @P227 (2026-05-05 — text-returning fn-ref calls).

**Per-phase summary** (all SHIPPED 2026-05-12):

- **00** Harness wiring + smoke (3 cells).
- **01** C0 (non-capturing) × D1/D2/D3/D4 (5 cells, pins @P214).
- **02** C1 + C5 (basic-type captures) × D1/D2/D3 (6 cells,
  pins @P213 + @P215).
- **03** C2 (text capture) × D1/D2/D3 (3 cells + 2 leak
  guards) — disposed LIFETIME.md "Type::Function NOT YET
  HANDLED" annotation as documentation drift.
- **04** C3 (Reference capture) × D1/D2/D3 (3 cells + 2 leak
  guards) — no read-after-free, no DbRef-in-closure-record
  leak.
- **05** C6 (nested closures) × D1/D2/D3 (3 cells + 1 leak
  guard) — D3 included (matrix's "deferred" was conservative).
- **06** Doc closeout — LIFETIME.md "Implementation path"
  trimmed; ROADMAP.md / USER_FACING.md / plans/future/36-
  audience-generative-art cross-refs updated; plan moved to
  `plans/finished/15-closure-validation/`.

Active plans now: 2 (07-error-messages + 22-mutable-closures).

### Plan-14 (tuple validation matrix) closed 2026-05-11


Plan-14 ran 2026-04-30 → 2026-05-11.  Final matrix: 40 cells
across 5 element types (E1 scalars, E1n integer-not-null, E2
text, E3 nested, E4 closure, E5 struct reference) and 3
destinations (D1 local, D2 direct stack, D3 struct field), all
cross-mode-validated under `tests/tuple_matrix.rs`'s
interp/native byte-identical assertion.  Plus E6 (struct value)
folded into E5 by C65 design decision and E7 (collections in
tuples) closed-by-non-goal.

**Phases shipped (00, 01, 02, 03, 04, 05, 07):**
- 00: cross-mode harness in `tests/common/cross_mode.rs`;
  `find_loft_rlib`, `compile_native_job`, `run_native_job`
  exposed `pub(crate)` from `tests/native.rs`.
- 01: 12 E1/E2 cells (D1 + D2: local, arg, return, inline,
  match-subj, if-arm).  T1.8a closed via the lighter "rust_type
  Context::Result recursion" fix instead of new opcodes.
- 02: 5 E3 cells.  Closed @P247 (nested-tuple text move in
  format-string interpolation) + @P248 (element-of-element
  assignment `t.0.1 = 99`).
- 03: 5 E4 cells (closure-typed tuple elements).  Closed @P249
  (20-byte fn-ref layout extended into 6 tuple codegen sites +
  `__fn_ref_tmp` postfix-call temp marked skip_free).
- 04: 6 E5 cells (struct references).  Decision: MOVE
  semantics (recorded as C64).  Loop-iteration aliasing bug
  parked as @P250.
- 05: 6 D3 cells (tuples in struct fields).  Decision: LIFT
  was already shipped by Plan-06 phase 4d; phase 05 was a
  verification pass.  E4_d3 (closure-element tuple as struct
  field) parked behind @P251.
- 07: @P234 runtime — lifetime-bearing tuple returns route
  through `Reference(__tuple<…>)` synthetic struct.

**Phase deferred:**
- 08: @P234 runtime extended to LOCAL tuple-with-lifetime-concern
  variables — friction with P189b's vector-of-tuple index access
  meant the rewrite needed broader changes than the original
  Phase 08 scope.  Phase is a uniformity refactor, not a bug
  fix; juice not worth the squeeze.

**Bugs filed during validation:**
- @P247, @P248 — closed in phase 02.
- @P249 — closed in phase 03.
- @P250 — open: ref-tuple loop-iteration aliasing.
- @P251 — open: native projection for fn-ref-tuple-in-struct-field.

**Design decisions recorded in DESIGN_DECISIONS.md:**
- C64: Tuple struct-ref elements use MOVE semantics.
- C65: E6 (struct value) folded into E5 — no inline value-struct
  type in current loft.

Reference content moved to TUPLES.md (Known limitations + Non-
goals + Deferred work updated).  Plan moved to
`finished/14-tuple-validation/`.

### Plan-06 (typed-par redesign) closed 2026-05-09


Plan-06 ARC ran from 2026-04-30 to 2026-05-09.  All 11 sub-steps
shipped or formally deferred with rationale.

**Shipped (A1–A7 + A5b + A8.b + A9 superseded by A4 + A11):**
- A1: parallel workers — extra args + text/ref returns under one
  fused-for-par codegen.
- A2: per-thread slot cap stress test + structural fix
  (`worker_slot_dispenser` atomic counter replaces 8d.3's fixed
  16-slot cap).
- A3: Queue dispatch for narrow-primitive returns (Boolean,
  Character, Enum-no-payload, narrow Integer, Single, Float).
- A4: retired the light Concat path entirely
  (`n_parallel_for_light` and `n_parallel_for` panic if invoked).
- A5: Stitch::Reduce runtime + `par_fold(items, init, fn fold,
  threads)` parser builtin (interp + native).
- A5b: `par_fold` native runtime mirror.
- A6: closed 4 fn-ref / vector / keyed-collection canaries
  (`par_struct_to_vector_t4`, `par_struct_to_fn_t4`,
  `par_vec_of_fns_input_t4`, `par_struct_to_keyed_collection_t4`).
- A7: closed the par-tuple canary surface — A7.1 (size-based
  gate widen + work-ref unification — closes
  `par_tuple_return_int_int` / `_three_arity` / `_nested`),
  A7.2 (@P235 par half — synthesized wrapper-worker — closes
  `par_tuple_destructure_in_for`), A7.3 (@P234 lexer + runtime
  for tuple-of-struct member access).  Companion fix @P236
  (heap-owned reference returns from if/else native data
  corruption — broader than tuples) landed alongside A7.1.
- A8.b: stitch_id consolidation in `src/native.rs` — 5
  `n_parallel_queue*` fns collapsed to thin wrappers around
  `parallel_queue_dispatch(stores, stack, QueueStitch)`.
  Saves ~150 LOC.  Targets the interp-bridge layer (different
  from A8's `src/parallel.rs` target which deferred for sound
  reasons).  Codegen-runtime mirrors stay separate (closure
  types differ per stitch).
- A9: superseded by A4 (light path retired entirely; no `.loft`
  file uses `par_light`).
- A11: this entry + ARC.md status header DONE + acceptance-
  criteria final tally + THREADING.md dispatcher inventory
  section.

**Deferred with rationale:**
- A8 (Queue dispatcher trait collapse in `src/parallel.rs`):
  divergence is structural, not boilerplate.  The 4-5 dispatchers
  differ in `&Stores` vs `&mut Stores` access, worker primitive
  (`parallel_workers` vs raw rayon), per-row execute call,
  per-thread state, and merge step.  A unifying trait would
  relocate complexity rather than remove it.  Full audit in
  ARC.md A8 deferral section.  Codegen side is already collapsed
  (`ParallelQueueEmitter`); buffer stacks per-type are
  intentional (perf).  Commit `ada917d`.  A8.b stitch_id retry
  delivered consolidation at a different layer (see Shipped).
- A10 (browser parallel via wasm-bindgen-rayon): out-of-scope
  for @PLAN06 closure.  S2 strategic showcase; ships as its own
  multi-session arc when scheduled.

**Acceptance criteria — final tally:**
- #1 (≤ 3 dispatchers in `src/parallel.rs`): revised to ≤ 5;
  consolidation delivered instead at native.rs layer via A8.b.
  Documented in ARC.md acceptance section.
- #2 (par_light removed from user surface): MET by A4.
- #3 (zero ignored par canaries): 8 → 1 over the arc.  Final
  remaining ignore is heterogeneous-vec-of-fn (D11a row 8),
  outside @PLAN06 scope (different surface — vector
  construction, not par).

Three closure commits land 2026-05-09: `f974770` (closeout
docs + A8 deferral marker + A9 superseded), `15a7aab` (@P235
par half via wrapper synthesis), and the A8.b commit (this
change).

### @PLAN09 phase 07: close @P205 — bounded-generic text return scratch routing


Closes @P205 (1 of 4 native sub-failures retired).  The bug:
bounded-generic dispatch `fn f<T: Trait>(x: T) -> text` produced
native code that emitted `Str::new(&local_String)` whose pointer
referenced a stack-local that dropped at function return,
dangling the returned `Str` into freed memory.

Fix in `src/generation/emit.rs` at TWO emit sites (the dangle
isn't tied to a single Op — it's emit.rs's `Str::new(...)` wrap
choice for text-returning functions):

- **`Value::Return(val)` text-wrap path** (line 188+): detects
  "function returns Type::Text but has no
  `Type::RefVar(Type::Text(_))` attribute" and routes the value
  through `stores.scratch`.
- **Block-tail `wrap_result` path** (line 887+): same detection,
  same routing.

Detection logic:
```rust
let needs_p205_scratch = wrap_text && {
    let def = self.data.def(self.def_nr);
    matches!(def.returned, Type::Text(_))
        && !def.attributes.iter().any(|a| {
            matches!(a.typedef, Type::RefVar(ref t) if matches!(**t, Type::Text(_)))
        })
};
if needs_p205_scratch {
    write!(w, "{{ stores.scratch.push((")?;
    self.output_code_inner(w, val)?;
    write!(w, ").to_string()); Str::new(stores.scratch.last().unwrap()) }}")?;
}
```

`stores.scratch` is the same Vec<String> that
`n_parallel_buf_get_text_native` uses — lifetime stable for the
caller's use of the returned `Str`.

Detection nuance: `text_return` doesn't set `hidden=true` on the
attributes it adds (only `ref_return` does, at
`parser/control.rs:2452`).  Initial detection used
`a.hidden && Type::RefVar(...)` and filtered out every
text-returning function.  Fix dropped the `hidden` filter.

Probe finding (Outcome B from phase 07's diagnostic step):
removing the `DefType::Generic` skip at `parser/control.rs:375`
makes `text_return` run for generic specialisations but doesn't
help — text_return promotes locals to hidden RefVar(Text)
parameters, but bounded-generic specialisations have no local
text vars to promote.  The fix had to move from parser-side to
codegen-side.

Verified:
- `repro_p205.loft` exits 0 under native (was: panic on assert)
- `86_interfaces` no longer in run failures
- `native_scripts`: 89/93 → 90/93
- threading 43/43, threading_chars 35/35, issues 540/540 unchanged
- p09 fast gate: byte-identical (baseline refreshed for
  25-generics)
- fmt + clippy `-D warnings` clean

Regression tests in `tests/codegen_emitter.rs`:
- `p205_repro_passes_under_native` — runs the reproducer under
  native, asserts exit 0.
- `p205_no_str_new_of_local_in_corpus` — greps the doc-test
  baseline for `Str::new(&var___ret_*)` and fails if reintroduced.

Commit `6151231`.

### @PLAN09 phase 06: close @P202 — n_parallel_queue family in native


Closes @P202 (3 of 6 native sub-failures retired: 19_threading,
22_threading, 40_par_ref_return).  Native compilation of
`for ... par(...)` for-loops now works.

Three components ship together:

1. **Runtime fns** in `src/codegen_runtime.rs`:
   - `n_parallel_queue_native` / `_text_native` / `_ref_native`
     — queue dispatch (closure-based, mirroring
     `n_parallel_for_*_native`).  The ref variant adopts a
     single result store (simpler than the interpreter's
     per-worker dispenser); buf_drop_ref frees it.
   - `n_parallel_buf_get_native` / `_text_native` / `_ref_native`
     — per-row reads from the active par buffer.
   - `n_parallel_buf_drop_native` / `_text_native` / `_ref_native`
     — end-of-loop cleanup.
   - All 9 take `&UnsafeCell<Stores>` per phase 01's ABI;
     registered in `CODEGEN_RUNTIME_FNS` with `Abi::Cell`.
   - `# Panics` sections added to satisfy
     `clippy::missing_panics_doc`.

2. **Emitters** in `src/generation/ops/parallel.rs`:
   - `ParallelQueueEmitter` mirrors phase 03's
     `ParallelForEmitter` but routes calls to
     `n_parallel_queue_*_native` (returning i64 row count
     instead of DbRef).  Reuses `closure_shape` /
     `queue_helper_name` / extra-arg let-binding scaffolding.
   - `ParallelBufRenameEmitter` is a pass-through name
     rewriter for the 6 buf-get / buf-drop names — no closure
     transformation, just appends `_native` to the call site.
   - All 9 names registered in
     `src/generation/ops/mod.rs::build_registry`.

3. **Reachability fix** in `src/generation/mod.rs::collect_calls`:
   - The worker-fn-via-d_nr-arg detection (originally for
     `n_parallel_for*`) extended to the queue family.  Without
     this, the emitter's closure refers to a worker fn that
     never gets emitted (E0425 "cannot find function").  Caught
     during validation, not pre-flight — the lesson is captured
     in `feedback_forwarding_first_recipe.md` (extend reachability
     when emitter synthesises calls by name).

Trait-reuse decision (per phase 06 doc § Implementation notes):
Phase 09's plan-doc projected 3 thin wrappers around a
`ParQueueShape` trait following the for-par pattern.
Implementation revealed each variant pushes to a different
`par_*_buffer_stack` field with a different value type — a
trait would have ~80% conditional branching.  Kept queue
runtime fns flat (~90 LOC vs ~120 with a trait).  Codified in
`feedback_phase_doc_trait_drafts.md` ("if 3 impls share <50%
of method bodies, prefer flat over trait").

Verified:
- native_dir: 29/30 → **30/30** (19_threading compile fix)
- native_scripts: 87/93 → **89/93** (22_threading +
  40_par_ref_return compile fixes)
- threading 43/43, threading_chars 35/35, issues 540/540 unchanged
- p09 fast gate: 9/9 byte-identical (baseline refreshed for
  19-threading + 22-threading — emission shape changed
  intentionally as queue calls now route through emitter)
- fmt + clippy `-D warnings` clean

Regression tests in `tests/codegen_emitter.rs`:
- `p202_parallel_queue_runtime_fns_registered` — pins the 9
  runtime-fn names with `Abi::Cell`.
- `p202_parallel_queue_emitter_registered` — pins the 9
  build_registry entries.

Commit `8cf0676`.

### @PLAN09 phase 00a: introspection findings + downstream updates


Fired late (after phases 00, 01, 03, 04, 09 all shipped) so the
introspection retroactively covers the simplification cluster.

Findings populate `doc/claude/plans/finished/09-native-runtime-rewrite/00a-introspect.md`:

- **Effort vs estimate**: phase 00 landed in 6 commits (under
  7-9 budget) + 4 follow-on infrastructure commits (smoke test,
  evaluation gates, CI cleanup, fmt followup).  Future plans
  should budget post-step infrastructure explicitly.
- **EmitCtx surface area**: 3 planned helpers (`w` / `def_fn` /
  `output`) + 2 surfaced during phase 03 (`emit(v)`,
  `emit_i32_slot(v)`).  Phase 05's int-width helpers not added
  yet — phase 05 itself is misaligned (see below).
- **dispatch.rs op match arms**: 26 → 24 (phase 03 retired
  `n_parallel_for`; phase 04 retired `OpGetRecord` + `OpIterate`).
  Wart-budget gate `dispatch_op_arm_budget_not_exceeded` enforces
  shrink-only.
- **`Value::RawExpr` wart**: accepted with budget gate; future
  codegen synthesis must avoid extending it.
- **Byte-identical contract**: held across all 6 hoist commits +
  every subsequent simplification phase.
- **Two surprises caught and fixed**:
  - Forwarding-first recipe trap (initial 16-Op forwarding list
    included Ops in dispatch.rs special-case match) — caught by
    fast gate, list pruned to 9, recipe documented in NATIVE.md
    + phase 00 findings.  Phase 03/04 used the recipe as planned
    pre-flight.
  - Phase 09's `ParShape` sketch was too narrow (single `WorkerOut`
    type would have erased text's per-worker slot mechanism and
    ref's cross-store deep-copy capability) — implementation
    extended trait with `Self::Batches` second associated type +
    `store_results` method.

Hidden assumptions surfaced (drove downstream doc updates):

1. **Phase 05's WRITE-side scope was wrong** — actual @P200 bug is
   READ-side block-tail comparison-emission (`(_var as u8) ==
   (0_i64)` E0308), not the `f += val` template.  Plan rewrite
   required.  Documented in 05-file.md § Diagnosis findings;
   PROBLEMS.md @P200 entry updated with the surveyed shape.
2. **Phase 02 demoted from "@P200 prerequisite" to "optional
   simplification"** — phase 02 splits `narrow_int_cast`'s
   param-narrowing role (role #2); phase 05's actual bug is the
   block-tail role (role #1).  02-param-adapter.md now carries a
   "Status reassessment" block.
3. **Pre-existing CI breakage accreted** across phases 00-04 — 1
   fmt drift (hand-aligned `RuntimeFn` table from phase 01 step
   1.7) + 5 clippy errors (`map().unwrap_or()`, `borrow as raw
   pointer` ×3, lifetime elision, `let _stub = …` patterns,
   unused `Write` import).  Closed in commits f4d288a +
   97d17cc.  CI gate memory updated to "before each commit on
   hot-path edits."

Decision: **Continue with updated plans** (per the introspection's
decision criteria table — "2-3 surprises that updated downstream
phases").

Memory entries saved (durable beyond @PLAN09):

- `feedback_forwarding_first_recipe.md` — pre-flight pattern.
- `feedback_phase_doc_trait_drafts.md` — trait sketches in plan
  docs are drafts; expect to extend on first contact.
- `feedback_actual_error_survey.md` — bug-fix phases need
  `--native-emit` survey BEFORE writing implementation steps.

### @PLAN09 CI cleanup: fmt + clippy + no-default-features green


Pre-existing breakage that accumulated across phases 00-04:

- `cargo fmt --check` rejected the hand-aligned `RuntimeFn` table
  in `src/codegen_runtime.rs` introduced by phase 01 step 1.7
  (commit `2005f6e`).  Resolution: `#[rustfmt::skip]` on the
  const preserves the alignment.  Rest of the file (and 9 other
  files) reformatted to `cargo fmt` defaults.
- `cargo clippy --tests --release -- -D warnings` failed with 5
  errors (lib) / 9 (lib + tests).  All fixed:
  - `src/generation/ops/mod.rs:178` unused `Write` import — removed.
  - `src/generation/ops/mod.rs:54` explicit lifetimes on
    `EmitCtx<'a, 'b>` — elided to `EmitCtx<'_, '_>`.
  - `src/generation/ops/mod.rs:216-218` `let _stub = StubEmitter`
    pattern (binding to `_`-prefixed without side-effect) —
    rewritten as `let _: &dyn OpEmitter = &StubEmitter` (also
    strengthens the trait-impl-able assertion).
  - `src/codegen_runtime.rs:88` `.map(...).unwrap_or(...)` —
    collapsed to `.map_or(default, fn)`.
  - `src/codegen_runtime.rs:2049/2120/2209` `&mut ws.stores as *mut`
    borrow-as-raw-pointer — rewritten as `&raw mut ws.stores`
    (modern reference-to-raw idiom; clearer + clippy-clean).

Verified: fmt + clippy + no-default-features all clean; behavioural
baselines preserved (codegen_emitter 10/10, threading 43/43,
threading_chars 35/35, issues 540/540).

### @PLAN09 phase 09: parallel runtime consolidation


`src/codegen_runtime.rs`'s three `n_parallel_for_*_native` public
fns (scalar / text / heap-ref) collapsed to thin wrappers around a
generic `n_parallel_for_native_core<S: ParShape, F>(...)` core.

Mechanism:

- New `ParShape` trait with `WorkerOut: Send` + `Batches`
  associated types, `return_sz()`, `run_workers(...)` (static),
  `store_results(&self, ...)`.  Three impls: `ScalarShape`
  (carries `return_size`), `TextShape` (unit), `RefShape`
  (carries `struct_size` + `known_type`).
- The shared core sequences allocate (`alloc_par_result`) → run
  workers → store results → finalise (`finalize_par_result`).
- The three existing `run_native_workers_*` free fns stay as
  internal worker dispatchers; each `ParShape` impl's
  `run_workers` calls the appropriate one.
- Public fn bodies shrank from 36 / 24 / 39 lines to 20 / 13 / 24
  lines (full pub-fn span including signature; body is ~3 lines
  for each).  Pinned by `parallel_runtime_consolidated` test in
  `tests/codegen_emitter.rs` (≤ 15 body lines + must call
  `n_parallel_for_native_core`).
- Phase 06 (@P202 — adds `n_parallel_queue_*_native` queue variants)
  will add 3 thin wrappers (~10 lines) instead of 3 full ~80-line
  fns; cumulative saving ~240 lines.

Emission stays byte-identical (codegen calls the same public fn
names with the same ABI).  Behavioural baselines unchanged:
threading 43/43, threading_chars 35/35, issues 540/540, native
29/30 + 87/93 — same pre-existing failures (85_yield_resume,
86_interfaces, 87_store_leaks; compile failures in
19_threading / 20_binary / 22_threading / 40_par_ref_return).

### ARC.md A2: unbounded per-thread slot dispenser (8d.3 cap retired)


Replaces the spine-8d.3 fixed `SLOTS_PER_THREAD = 16` per-worker
reservation with a shared `Arc<AtomicU16>` dispenser.  Workers that
allocated more than 16 fresh stores per batch hit a hard `assert!`
in `database_named` ("worker exhausted reserved slot range
[N, N+16) at local_count=16"); the new design grows unbounded.

Mechanism:

- `Stores::worker_slot_dispenser: Option<Arc<AtomicU16>>` carries the
  shared counter into each worker's clone.  Initialised at
  `parent.allocations.len() + 1` by `Stores::make_worker_slot_dispenser`
  (the `+1` skips the parent-namespace index where each worker's
  `prog.new_state(ws)` push-at-ends a 1000-byte stack store, so the
  dispenser never collides with a worker's own stack-store slot).
- `Stores::worker_allocated_indices: Vec<u16>` per-worker list of
  parent-namespace indices the dispenser yielded.  After
  `run_parallel_queue_ref` joins, each entry triggers a
  `mem::swap` between parent and the worker's clone at that index.
- `database_named` in worker context now: pulls a fresh index via
  `dispenser.fetch_add(1, Relaxed)`, pushes
  `Store::new(100)` placeholders into the worker's own `allocations`
  Vec to fill any skipped indices owned by other workers, then
  initialises at the yielded index.  The placeholders stay
  `free=true` and are never swapped to parent (each worker only
  swaps its OWN allocated indices).
- 3 fields removed (`worker_slot_offset`, `worker_slot_limit`,
  `worker_slot_local_count`); 2 added (above).
  `reserve_worker_slots` / `release_worker_slots` removed.
- `database_named`'s `if slot == self.max { self.max += 1 }` widened
  to `if slot >= self.max { self.max = slot + 1 }` — the dispenser
  yields indices that can be > current max (it skips ahead in
  parent-namespace), so the strict-equality check missed cases and
  left max stale.

A2.3 invariant: an always-on `assert!` in `database_named` fires if
a worker has a dispenser attached but `disable_slot_reuse` was
cleared mid-call (the bypass would push to a parent-namespace index
unrelated to the dispenser, silently corrupting the swap-back at
thread join).  Always-on rather than `debug_assert!` because the
loft library compiles with `debug-assertions = false` in the test
profile per `[profile.dev.package.loft]` — a `debug_assert!` would
be a silent no-op in `cargo test`.

Tests:

- `tests/threading.rs::par_queue_ref_unbounded_allocations_per_element`
  exercises the allocator directly (bypassing the bytecode
  pipeline's `execute_at_ref` calling-convention mismatch with
  inline-struct-return functions): performs 50 named allocations
  via the dispenser, asserts strictly increasing parent-namespace
  indices and dispenser high-water = `parent_len + 1 + N_ALLOCS`.
- `tests/threading.rs::par_queue_ref_dispenser_bypass_assertion_fires`
  pins A2.3's invariant: a synthetic worker with dispenser attached
  but `disable_slot_reuse=false` panics with the documented message.

Bench-11 ±5%: ~101ms median post-A2 (vs ~98ms `main`, ~101ms
post-A1) — within gate.  All 37 `tests/threading.rs` tests + 31
`tests/threading_chars.rs` tests stay green.

ARC.md A2 status flipped to DONE.

### P196: tuple-of-fn-ref native codegen — `(u32, DbRef) as i32`


Fixes E0605 (`non-primitive cast`) + E0308 in native codegen when a
struct field of type `(fn(...) -> ..., int)` is assigned from a Var
or function-call source rather than a literal `(name, n)` tuple.

The bug: `set_field_check::Type::Tuple` non-literal path stashes the
RHS into a work-ref local, then for each element `i` emits
`OpSet*(ref, pos, TupleGet(tmp, i))`.  For a fn-ref element, native
codegen substitutes the template body's `@val` with `var_tmp.0` —
which has Rust type `(u32, DbRef)` (the fn-ref runtime
representation).  `OpSetInt4`'s template wraps `@val` with both
`@val == i64::MIN` (E0308 — comparing tuple to i64) and `@val as
i32` (E0605 — non-primitive cast on tuple type).

Fix in `src/generation/calls.rs::output_call_template`: when the
template parameter is `Type::Integer` and the IR value is a
`Value::TupleGet(var, idx)` whose tuple element type is
`Type::Function`, wrap the substituted expression with
`(i64::from(({with}).0))` — projecting the `u32` d_nr from the
fn-ref tuple's first element and widening to i64.  The template's
null-check (`== i64::MIN`) becomes tautologically false (a u32
can't equal i64::MIN) but compiles cleanly, and the `as i32` cast
narrows from i64.

The literal-tuple path was unaffected — `set_field_check::Type::Function`
already reduces `Value::FnRef(d_nr, _, _)` to `Value::Int(d_nr)`,
sidestepping the tuple shape entirely.

Tests:

- `tests/issues.rs::p4d_tuple_field_with_fn_ref` covers the literal
  case end-to-end through the interpreter (same shape as
  `p4d_fn_ref_as_struct_field`, but with the fn-ref nested inside a
  tuple field).
- `tests/exit_codes.rs::p196_native_codegen_projects_fn_ref_d_nr`
  pins the codegen-text invariant: a script with a non-literal
  fn-ref tuple source must emit `i64::from((var___ref_1.0).0)` and
  must NOT contain the buggy `(var___ref_1.0) == i64::MIN` shape.

PROBLEMS.md P196 entry retired.  ARC.md A6.c no longer gates on
P196 — closes independently of the 4d.C closure-storage redesign.

### P195: chained tuple-index lex (`n.v.0.0`)


Fixes the lexer's greedy `<digit>.<digit>` → float read when the
previous emitted token was `.` (field access).  Before: `n.v.0.0`
lexed as `n`, `.`, `v`, `.`, `Float(0.0)` — the parser then saw a
type mismatch on assignment and a stray `.` it could not place.
After: lexes as 7 tokens — `n`, `.`, `v`, `.`, `Integer(0)`, `.`,
`Integer(0)` — which is the correct chained tuple-index access.

Mechanism (`src/lexer.rs::number`):

- At entry, capture `prev_was_field_dot = self.peek.has ==
  Token(".")`.  `self.peek` holds the previously-emitted token at
  this point (the parser flow uses `cont()` which sets `peek =
  next()`-result; inside `number()`, `peek` is still the
  before-the-current-number token).
- After consuming a `.` and confirming it is **not** the start of a
  `..` range token, peek the next char in `iter`.  If
  `prev_was_field_dot && next.is_ascii_digit()`, push `Token(".")`
  onto `memory` (so the next `cont()` returns it) and return
  `Integer(val)` immediately — the trailing digit is then re-lexed
  as a fresh number on the call after that.
- The `..` range branch is unchanged: `0..5` still lexes as range,
  not tuple index.  Stand-alone floats like `0.0`, `1.5e3`, and
  expression-position floats like `x = 0.0` are unaffected because
  their preceding token is not `.`.

Test: `src/lexer.rs::test::p195_chained_tuple_index_does_not_glue_into_float`
exercises 5 cases — chained tuple index, stand-alone float,
expression-position float, mixed expression, range — using a new
`cont_array` test helper that drives the lexer through the same
`cont()` API the parser uses (the existing `array()` helper bypasses
`cont()` and would not catch context-aware lexing).

### `--show-types --trace` per-expression type tape


Adds a per-expression tape to the `--show-types` introspection
section.  Where the existing variable-level table catches dep loss
in *stored* values (the function's args, locals, return type), the
trace catches dep loss in *intermediate* sub-expressions of a
chained access.  Specifically, for a nested expression like
`a.v.0`, the tape shows the type at each step:

```
4:7        ref(A)["a"]              <- a
4:9        (text["a"], text["a"])   <- a.v
5:2        text["a"]                <- a.v.0
```

If P197 had been a regression today, line 4:9 would have rendered
`(text, text)` (no host dep) and the bug would have been visible
without reading any code.

Mechanism:
- `Parser::trace_types: bool` flag enables recording.
- `Parser::trace_types_lines: Vec<String>` accumulates entries
  formatted `<fn>\t<line>:<col>\t<type>`.
- `parse_part` calls `record_type_trace(&t)` after the initial
  `parse_single` and after each chaining step.
- Only fires on the second pass (first-pass types are placeholders
  that would emit thousands of meaningless lines).
- `main.rs` enables the flag for the user's file (not for the
  `default/*` stdlib parsed by `parse_dir`).
- `emit_types` in `introspect.rs` reads `opts.trace_lines`,
  filters to the current function (matching the user-typed name,
  i.e. without the `n_` prefix), and renders one section per fn.

Tangential fix discovered while testing on dev profile:
`emit_tuple_set_ops` had a `base_pos + offsets[i]` u16 overflow
when `base_pos` was the `database.position` u16::MAX sentinel
during first-pass placeholder resolution.  Release silently
wrapped; dev profile (with overflow checks) panics.  Switched
both arithmetic sites to `saturating_add` — first-pass IR is
regenerated in pass 2, so a saturated placeholder is safe.

Tests: `introspect_show_types_trace_renders_per_expression` in
`tests/exit_codes.rs`.

### Native-codegen source map + introspection `--diff`


Two developer-velocity wins, both targeting the long tail of
debugging time:

- **`// loft:<file>:<line>` comments in generated Rust.** Every
  function header and every statement boundary in `output_native`
  output now carries a comment mapping back to the originating
  loft source.  Lets `rustc` errors on `/tmp/loft_native.rs` be
  traced to the .loft line in seconds rather than by manually
  reading the generated code.  Cost: ~10 LOC; comments are
  cheap (one per source line).
- **`--introspect --diff <baseline>`.** Captures the requested
  sections to a buffer and runs `diff -u baseline tmp`.  Exits 0
  identical, 1 differs.  Lets devs answer "did this parser tweak
  change anything?" with one command.

Tests: `native_emit_includes_loft_source_map`,
`introspect_diff_against_baseline` in `tests/exit_codes.rs`.

### P194 — tuple-typed struct field reassignment


`p.v = (1, 2)` (where `v` is a tuple-typed struct field) used to
fail with `Tuple destructuring requires plain variable names`.
Root cause: `get_val::Type::Tuple` returns `Value::Tuple([reads])`
for a tuple field read, and the parser's destructuring branch
matched any `Value::Tuple` LHS unconditionally.  Fix: detect
"tuple of OpGet*-style reads (not all `Value::Var`) on a
`Type::Tuple` LHS" in `parse_assign` and route through
`emit_tuple_set_ops` instead of the destructuring branch.

- New helper `leaf_tuple_lhs` walks the leftmost leaf of the
  tuple-of-reads to recover `(host_ref, base_position)`; nested
  tuple elements recurse cleanly.
- `emit_tuple_set_ops` lifted to `pub(crate)` so the new branch in
  `parse_assign` can call it.

Tests: `p194_tuple_field_reassign`,
`p194_tuple_field_reassign_twice` in `tests/issues.rs`.

### P197 — returning `text` from tuple struct field corrupts memory


Surfaced while regression-testing P194.  Returning a `text` element
extracted from a tuple struct field returned garbage characters
(`.0`) or hard-crashed (`.1`, `.2`) with `ptr::copy_nonoverlapping
requires that both pointer arguments are aligned and non-null`.
Construction + read-via-print worked; only the function-return
path failed.

Root cause was two-part — both fixed in the same commit:

1. **`Type::Tuple` had no dep field**, so calling
   `.depending(host)` on a struct field's tuple type fell into the
   `_ => self.clone()` arm at `data.rs:580` and lost the host
   dependency entirely.  Fix: `depending` now recurses into tuple
   elements (each text/reference inside the tuple gets the host
   dep), and `depend()` returns the union of element deps.
2. **Native codegen materialised the tuple into a `(String,
   String)` work-var temp**, then borrowed `&temp.0` past its
   drop — `rustc` rejected with "borrowed value does not live long
   enough".  Fix: when `code` is already a literal `Value::Tuple`,
   `parse_part`'s tuple-index branch returns the indexed read
   directly instead of allocating the work-var temp.

Tests: `p197_text_returned_from_tuple_field`,
`p197_text_returned_from_tuple_field_index_one`,
`p197_text_returned_from_mixed_tuple_field` in `tests/issues.rs`.

### Plan-06 phase 4d.C step 2 — `Parts::DbRef` storage shape + new opcodes


Foundation pieces for closure storage in fn-ref struct fields and
tuple elements.  No user-visible behaviour change yet — the
parser still emits the truncated 4-byte `OpSetInt4` path, which
phase 4d.C step 4 will replace with the new opcodes.

**Database:**

- New `Parts::DbRef` variant in `src/database/mod.rs` — 12-byte
  raw `DbRef` storage cell (`u32` store_nr + `u32` rec + `u32`
  pos).  Match arms wired through `database/io.rs`,
  `database/structures.rs`, `database/format.rs`, and
  `database/search.rs`.  Non-collection operations panic;
  debug-format renders as `DbRef(s,r,p)` or `null` (rec == 0).
- `Stores::dbref()` registers a primitive type named `"dbref"`
  with `Parts::DbRef` and size 12 (idempotent).

**Opcodes (`default/01_code.loft`):**

- `OpSetDbRef(v1: reference, fld: const u16, val: reference)` —
  writes 3 × `set_u32_raw` words at `v1.pos + fld`.
- `OpGetDbRef(v1: reference, fld: const u16) -> reference` —
  reads 3 × `get_u32_raw` words and assembles a `DbRef`.
- OPERATORS array in `src/fill.rs` grown 243 → 245.  Interpreter
  dispatch fns `set_db_ref` / `get_db_ref` regenerated via
  `cargo test regen_fill_rs -- --ignored`.

### Slot allocator & frame layout (plans 04 + 05)


Two companion plans closed together; user-visible only as the
absence of a recurring heap-corruption class (P178 / P185).

**Runtime / codegen changes:**

- Single function-entry `OpReserveFrame(frame_hwm)` replaces the
  per-block `OpReserveFrame(block.var_size)` + `OpFreeStack`
  bookkeeping.  The whole frame is owned by the function and
  released on return.
- Positional init opcodes: `OpInitText(pos)`, `OpInitRef(pos)`,
  `OpInitRefSentinel(pos)`, `OpInitCreateStack(pos, dep_pos)`.
  Every first-assignment writes directly to the allocator-chosen
  slot; slot-move + gap-fill in `gen_set_first_at_tos` is gone.
- `OpText` deleted (−1 opcode).  The three compound ops
  `OpConvRefFromNull` / `OpNullRefSentinel` / `OpCreateStack`
  remain as dictionary-only entries for parser back-compat; their
  runtime bodies are dead code.
- `place_orphaned_vars` deleted (~150 LOC retired).  `process_scope`
  + `place_large_and_recurse` now reach every local: Insert-rooted
  function bodies, cross-scope `Set` in child operator lists, and
  the `BreakWith / Iter / Tuple / TuplePut / Yield / Parallel`
  IR shapes are all handled in the main walk.
- P185 fixed (`p185_slot_alias_on_late_local_in_nested_for` +
  `p185_late_local_after_inner_loop` un-ignored).

**Diagnostics:**

- Invariant **I7 — scope-frame consistency** in
  `src/variables/validate.rs`: each variable's `stack_pos` lies
  within its declared scope's frame region.  Converts the
  `Incorrect var X[slot] versus TOS` runtime panic into a
  compile-time `[I7]` diagnostic.
- V2 allocator (`src/variables/slots_v2.rs`) remains as a shadow
  validator invoked via `LOFT_SLOT_V2=validate`; I1–I6 green on
  the corpus as a correctness gate for future V1 edits.

**Retracted from the original @PLAN04 scope:**

- Single-pass V2 allocator driving codegen (both the
  codegen-is-allocator pivot and the direct V2-drive attempt hit
  the same failure mode on variables declared at outer scope but
  first-Set in inner scope).  V1 continues to drive codegen.

### Integer → i64 migration (Phase 2c)


`integer` is now 8 bytes end-to-end — on the stack, in struct
fields, in runtime arithmetic — across the interpreter, native
codegen, and WASM backends.  Arithmetic that used to silently
wrap at `i32::MIN / MAX` now traps (Phase 1 `?` / `??` dispatch
from `925ee36`) or round-trips correctly on i64.

**What users see:**

- `integer` literals beyond `i32::MAX` (e.g. `9_876_543_210`)
  type-check without any suffix.
- The `long` type keyword and the `l` literal suffix (`33l`,
  `0xFFl`) are **gone**.  Writing `long` in a type position now
  fails with `"Undefined type long"`; writing `33l` fails at the
  lexer.  Use `integer` and plain `33` instead.  Both were
  deprecation-warned in 0.9.0-early and fully removed in
  0.9.0-final (commits `3e976b3`..`0c46abb`).
- Narrow integer aliases — `u8`, `u16`, `i8`, `i16`, and `i32`
  — keep their compact field storage (`Parts::{Byte, Short,
  Int}`), with narrow↔wide conversion at read/write.  Pack
  density is preserved for image buffers, pixel arrays, and
  other bit-bounded data.
- File I/O for binary formats now **requires an explicit
  width cast** on scalar integer writes, e.g.
  `f += 2 as i32;` (4-byte GLB version), `f += 0 as u8;`
  (1-byte pixel).  Pre-2c `f += 2` wrote 4 bytes; post-2c
  writes 8 — silent regressions in existing binary writers
  are the most common footgun of this migration.

**Migration aid:** no external users of pre-0.9.0 loft exist,
so no migration path is needed in practice.  The internal
`loft --migrate-long <path>` CLI is retained as a utility
that rewrites `long` → `integer` and strips `l` suffixes, in
case an external user surfaces later.

**Downsides recorded** (`doc/claude/CAVEATS.md`): memory
footprint of integer-heavy data structures roughly doubles;
cross-crate cdylib packages keep 4-byte `vector<integer>`
element storage (narrow→wide conversion at the FFI boundary).
The bytecode opcode table was reduced from 268 to 234 after
the `Op*Long` family dedup (34 opcodes reclaimed across rounds
10b.1–10b.4 and 10d).

### JSON support


Loft now has built-in JSON parsing and generation.

**Parsing** — `json_parse(text)` turns a JSON string into a typed
`JsonValue` that you can inspect and navigate:

```loft
v = json_parse("{{\"name\":\"Alice\",\"age\":30}}");
println(v.field("name").as_text());   // Alice
println(v.kind());                     // JObject
println(v.to_json_pretty());           // formatted output
```

Bad input returns `JNull` instead of crashing; call `json_errors()`
to see what went wrong (with line numbers and context).

**Reading values** — `field("key")`, `item(index)`, `len()`,
`has_field("key")`, `keys()`, `fields()`, `kind()`.
Type extractors: `as_text()`, `as_number()`, `as_long()`, `as_bool()`.

**Writing JSON** — `to_json()` for compact output,
`to_json_pretty()` for readable indented output.

**Building values from code** — `json_null()`, `json_bool(v)`,
`json_number(v)`, `json_string(v)`, `json_array(items)`,
`json_object(fields)`.

**Struct integration** — `MyStruct.parse(json_value)` populates
a struct from a JsonValue. Type mismatches are reported via
`json_errors()`.

### Plan-06 phases 4c + 4d.A — typed parallel-for dispatch


Two coupled phases of @PLAN06 ("simple typed `par`: everything is a
store") landed.  User-visible only as one extra par canary closing
(`tests/threading_chars.rs::par_tuple_input_int_int`); structurally
this lays the foundation for the remaining phase 4 work (4d.B
keyed-collection input materialisation, 4e caller-supplied
destinations).

**Phase 4c (DESIGN.md D1b):** `Stitch::ConcatLegacy { elem_size,
ret_size }` retired in favour of payload-free `Stitch::Concat`.
`parallel_execute_and_collect` now takes `dispatch_mode:
DispatchMode` and routes via the `Text / Ref / Primitive` arms
keyed on the caller-supplied dispatch mode.  `grep ConcatLegacy
src/` returns zero (spec acceptance).  Opcode payload shrinks 2
bytes per call.

**Phase 4d.A:** typed worker-input dispatch via `InputKind` enum
(`Ref / Text / Primitive { size: u8 }`) with a 64-byte cap on the
`Primitive` slot.  New `read_primitive_at_wide` (stack-allocated
`[u8; 64]` reader) and `execute_at_raw_primitive_input_wide`
(byte-chunk push) handle 9..=64 byte first-arg slots — tuples,
fn-refs, and any inline-typed first arg whose stack representation
exceeds 8 bytes.  Both `run_parallel_direct` and
`run_parallel_light` got matching `prim_in > 8` arms.  Retires
the sentinel-encoded `primitive_first_arg_slot_size` channel.

### Local-var keyed collection iteration (P190)


`for x in <local sorted/hash/index>` used to panic at
`src/state/codegen.rs:1689` with "Too few parameters on
OpIterate (got 2, need 6)".  P188 enabled local-var keyed
collections but the iteration codegen path's
`src/parser/vectors.rs::get_type` only resolved the database
type-name for fields registered via `fill_database` — local-var
keyed collections never reached that registration path, so the
lookup returned `u16::MAX`, `fill_iter` exited early, and
`OpIterate` got 2 args instead of the 6 it needed.

Fix: register the type on demand in `get_type` when the name
lookup misses, mirroring `fill_database`'s `database.sorted` /
`database.hash` / `database.index` calls.  Idempotent — same
content+keys → same type id.  Regression test
`tests/issues.rs::p190_local_var_sorted_iteration`.

Note: this unblocked the iteration codepath; @PLAN06 phase
4d.B for sorted then closed by the parser-side desugar (see
the next entry).

### Plan-06 phase 4d.B sorted — par-over-keyed-collection materialise


`for s in sorted_items par(...)` now compiles end-to-end and
closes the `par_sorted_input_t4` canary (1 more canary
green; 11 ignored, was 12).

When parse_for sees a par() clause with a sorted/hash/index/
spacial input, the new `materialise_keyed_for_par` helper
allocates a temporary `vector<reference<T>>`, walks the
source via the existing `OpIterate`/`OpStep` machinery (the
same helpers `for x in sorted_items` uses), and appends each
element-DbRef.  The par dispatch then runs over the
materialised vector — workers receive the same 12-byte
DbRef as the closed `par_vec_of_refs_input_t4` canary.

The IR shape mirrors the parser's emission for the manual
workaround `refs += [s]`: `OpPreAllocVector` +
`OpNewRecord` + `OpCopyRecord` + `OpFinishRecord` per loop
iteration.  An earlier attempt missed `OpPreAllocVector`
and produced uninitialised slots; this commit lands the full
sequence.

Cost contract: O(N) materialisation + 12-byte temporary
vector + the par work itself.  Documented as known cost;
users can opt out by materialising explicitly into
`vector<reference<T>>` first.

`par_hash_input_t4` and `par_index_input_t4` stay
`#[ignore]`d on **P191** (filed in PROBLEMS.md) —
sequential local-var iteration over hash/index produces
wrong elements (0 for index, 195 instead of 30 for hash).
After P191 closes, both canaries should pass via the same
4d.B desugar.

Regression test:
`tests/issues.rs::p4d_b_par_over_sorted_via_materialise`.

### P191 — `index<T[key]>` bookkeeping field size mismatch


`database.index` appended `#left_N` / `#right_N` bookkeeping
fields declared as 8-byte `integer`, but `tree::add` writes
those tree pointers via `set_i32_raw` at hardcoded offsets
`[pos, pos+4, pos+8]` (RB_LEFT=0 / RB_RIGHT=4 / RB_FLAG=8).
Alignment-aware packing placed the 8-byte fields 8 bytes
apart, so tree pointers landed in the wrong record bytes.
Iteration only returned the root element (e.g., a struct-
field index iteration that should sum 60 returned 10).

Fix: switch bookkeeping to 4-byte `int<0,false>` so the
layout matches `tree::add`'s offsets.  Side benefit: indexed
records shrink by 8 bytes each.  `tree.rs` already operates
exclusively on i32 via `set_i32_raw` / `get_i32_raw`; no
other code changes.

Same commit also adds new `validate_layout` /
`validate_all_layouts` / `debug_layout` / `layout_summary`
helpers in `src/database/types.rs`, wired into the parser
flow after `database.finish()` so future regressions surface
as build-time errors.  16 unit tests cover overlap detection,
beyond-size, bookkeeping-offset mismatch, enum-variant
overlap-within-variant, and the layout-summary format.

Regression test:
`tests/issues.rs::p191_struct_field_index_iteration_after_layout_fix`.

### P192 — `len()` for `hash<T[key]>` and `index<T[key]>`


Only `vector` and `sorted` had `len()` overloads.  Added
two new runtime helpers — `hash::count` (walks the bucket
array, O(room)) and `tree::count` (walks via `first` +
`next`, O(n)) — exposed via `OpLengthHash` (normal stdlib
overload) and `OpLengthIndex` (parser hook in `call()` to
inject the bookkeeping-offset const).

Regression tests:
`p192_len_hash_struct_field`,
`p192_len_index_struct_field`.

### P188 follow-up — `field += elem` for keyed-collection fields


Two distinct bugs broke `db.x += Foo{...}` for hash / sorted /
index / spacial fields and local-vars; vector-literal init
(`db = Db { x: [...] }`) worked because its codegen built
records directly.  Both surfaced once P192's `len()` made
the broken state observable.

**Bug 1 — struct-literal RHS retarget.**  `Score{name: "a",
value: 10}` parses with the LHS field as its target, so the
field-init steps wrote into the field's storage —
overwriting the hash/index root pointer with stray bytes of
the score record.  Struct-field hash with `+=` reported
`len = 11` after one add then SIGSEGV on the next.

Fix: extend the `field += elem` branch in `expressions.rs` to
also match keyed-collection fields with struct-literal RHS,
allocate a fresh element via `new_record_field_op`, and walk
the parsed steps with a new `substitute_value` helper that
replaces the LHS field expression with `Var(elm)` so each
field write lands in the new record.  Gated on
`elm_tp.is_equal(&s_type)` so vector field `+= [1, 2, 3]`
(multi-element append) keeps its existing OpAppendVector path.

**Bug 2 — local-var dispatch via wrong db type.**  `new_record`
local-var branch looked up the keyed-collection's known_type
via `data.def(type_def_nr(lhs_tp)).known_type`, but
`type_def_nr` returns the GENERIC alias (`hash` / `index`),
not the specific `hash<Score[name]>` instantiation.  The
alias's known_type pointed at a Vector type, so
`record_finish` dispatched through `Parts::Vector` and
appended raw bytes — `hash::add` / `tree::add` never fired.
Local-var hash with 3 adds showed 6 records (vector_finish
appends without dedup); local-var index with 2 adds showed 1
(tree::add was bypassed entirely).

Fix: register the specific keyed-collection db type directly
(`database.hash(c, key)` / `index(c, key)` / etc.) —
idempotent with the gen_set_first_keyed_null and typedef-
walker registrations.

4 new P188 regression tests cover struct-field and local-var
hash and index `+=` (each asserts both `len` and the
iteration sum).

### Plan-06 phase 4d.A.2 — partial fix: parser hang eliminated, clean diagnostic emitted


A 2026-04-27 spike landed two contained changes that flip the
canary's failure mode from "infinite-loop in parser, requires
`pkill`" to "fast clean diagnostic, 0.02 s test failure".

**Root cause (parser hang)**: `src/parser/definitions.rs::sub_type`
had no `fn` keyword arm.  When the parser saw
`vector<fn(integer) -> integer>`, sub_type's identifier-only check
rejected `fn`, the lexer reverted past `<`, and the caller's
annotation parser (`expressions.rs::parse_assign:1009`) entered a
tight retry loop on the unconsumed `<`.  The loft binary's `--dump`
flag and `cargo test` both hung at 100% CPU during pass 1 of the
2-pass parser.

**Fix #1 — parser sub_type**: new `fn` arm in `sub_type` that
consumes the `fn(...) -> ...` declaration via `parse_fn_type`,
then emits a clean diagnostic and returns `Type::Unknown(0)` until
full storage support lands.  The parser advances cleanly instead
of looping.

**Fix #2 — vector literal new_record**: `parser/vectors.rs::new_record`
checks for `Type::Function` element type at entry and emits a
clean diagnostic with a workaround suggestion ("wrap the fn-ref in
a struct") instead of hitting the cryptic
`assert_ne!(ed_nr, u32::MAX)` assertion downstream.

**Tests pass**: `threading_chars` 31/0/8, `threading` 16/0/0,
`issues` 522/0/4 (the +3 ignored are diagnostic regression guards
documenting V1/V2/V3 reduced cases).  `cargo clippy` clean.

**Canary remains `#[ignore]`d** — full closure of the canary needs
real storage support for `vector<fn-ref>`, which is its own
@PLAN06 phase 4d.A.2 work (M effort, 2-3 days).  See
`/home/jurjen/.claude/plans/serialized-churning-journal.md` for
the full design (Steps A–E).

### Plan-06 phase 4d.A.2 — partial fix: parser hang eliminated, runtime cascade exposed


A 2026-04-27 spike landed three contained changes that flip the
canary's failure mode from "infinite-loop in parser, requires
`pkill`" to "fast SIGSEGV in runtime, 0.02 s test failure".

**Root cause (parser hang)**: `src/parser/definitions.rs::sub_type`
had no `fn` keyword arm.  When the parser saw
`vector<fn(integer) -> integer>`, sub_type's identifier-only check
rejected `fn`, the lexer reverted past `<`, and the caller's
annotation parser (`expressions.rs::parse_assign:1009`) entered a
tight retry loop on the unconsumed `<`.  The loft binary's `--dump`
flag and `cargo test` both hung at 100% CPU during pass 1 of the
2-pass parser.

**Fix**: new `fn` arm in sub_type that calls `parse_fn_type` and
registers a synthetic `__fn_ref` global struct via the new
`Data::fn_ref_def` helper.  Mirrors the tuple_def pattern (P189):
one global struct shared across all fn-ref shapes, since all
fn-refs have the same vector-storage shape (4-byte i32 d_nr).
`type_def_nr` and `type_elm` get matching `Type::Function` arms
returning the `__fn_ref` def's number.

**Generated-code diagnosis**: with parsing fixed, the test
framework wrote `tests/generated/threading_chars_par_vec_of_fns_input_t4.rs`
for the first time.  Reading it reveals 3 remaining bugs:

1. **`n_apply` empty match** — native codegen specialises
   `OpCallRef` to `match var_f.0 { ... }` over statically-known
   d_nrs.  For `apply(f)` where `f` flows from a generic vector,
   no analysis populates the arms — only `_ => unreachable!()`
   remains.
2. **Vector literal as struct-records** — my `__fn_ref` synthetic
   struct routed `[dbl, triple, quad]` through the
   `OpNewRecord/OpCopyRecord/OpFinishRecord` STRUCT-element
   vector path.  Each fn-ref becomes a heap record with a `d_nr`
   field; vector stride is the record size, not 4.  The
   interpreter SIGSEGVs reading back struct-DbRefs into a worker
   slot expecting flat 4-byte d_nr bytes.
3. **Par dispatch closure type-mismatch** —
   `|stores, elm: DbRef| { n_apply(stores, elm) as i64 }` but
   `n_apply` takes `(u32, DbRef)`.  Dispatcher needs a
   `Type::Function` worker-input arm that reads the 4-byte d_nr
   and constructs the tuple.

**Remaining work to fully close 4d.A.2** (effort: M, 2-3 days):

- Re-design `__fn_ref` as a primitive 4-byte alias (drop struct).
- Vector element-write flat-byte arm in `parse_append_vector`.
- Vector read-back unbox in `parser/fields.rs` (P189b-style).
- Par dispatcher worker-closure `(u32, DbRef)` wrap in
  `src/generation/dispatch.rs:792-870`.
- Native codegen — populate match arms or fallback to interpreter.

Tracked in `/home/jurjen/.claude/plans/serialized-churning-journal.md`.

The canary remains `#[ignore]`d but with an updated message naming
the new failure mode (SIGSEGV instead of hang).

### Plan-06 phase 4d.A.2 — investigation: vec-of-fn-refs is bigger than estimated


A 2026-04-27 spike attempted to close `par_vec_of_fns_input_t4`
by un-ignoring the canary and observing the failure.  Result:
**the worker infinite-loops** rather than failing cleanly.

The README's planned fix ("per-row synthesis of the 12-byte
null closure DbRef") turns out to only address half the gap:

- In-vector storage: 4 bytes per row (just the d_nr stored as i32 —
  `data::element_size(Type::Function) = 4`).
- Worker arg slot: 20 bytes — 8B i64 d_nr + 12B closure DbRef
  (`variables::size(.., Context::Argument) = 20`).

The current wide-input dispatcher (`read_primitive_at_wide`)
reads `element_size = 4` bytes into a 64-byte zero-initialised
buffer, then `execute_at_raw_primitive_input_wide` slices to
`prim_in = 20` bytes.  Slot bytes 4-7 are zero (high 32 bits of
i64 d_nr — fine for any practical d_nr) and bytes 8-19 are zero
(null closure DbRef).

The resulting fn-ref **runs** but `apply(f) → f(10)` loops
indefinitely, suggesting the call-dispatch path (likely
`OpCallRef`) doesn't tolerate a null closure DbRef in this
context — possibly because it interprets `store_nr=0, rec=0`
as a back-pointer to itself, or because the worker's stack
state after `OpCallRef` is wrong without a real closure.

Closing 4d.A.2 needs:

1. A `read_fn_ref_at_wide` helper that explicitly handles the
   i32→i64 d_nr widening (rather than relying on flat memcpy
   into a zeroed buffer).
2. A runtime fix to the `OpCallRef`-on-null-closure path so
   workers don't loop when the closure DbRef is null.
3. A new wide-input plumbing channel similar to
   `tuple_input_types: Option<Vec<Type>>` from P189d — likely
   generalised to `WideInputLayout::{Tuple, FnRef, Plain}`.

Effort revised: **S–M** (was S).  Test-side guard added: the
canary's `#[ignore]` message now warns "DO NOT un-ignore
without fixing — the test infinite-loops and needs `pkill` to
terminate."

### Plan-06 phase 3b.1 — extract shared `merge_batches` helper


Five sites across `src/parallel.rs` and `src/codegen_runtime.rs`
inlined the same 5-line loop after every `parallel_workers`
call: pre-fill a `Vec<R>` with a default value, then walk each
`(start, batch)` pair and write each element into
`results[start + offset]`.

Extracted to `parallel::merge_batches<R: Clone>(batches, n_rows,
default) -> Vec<R>` and applied at:

- `parallel::run_parallel_raw` (Vec<u64>, default `0u64`)
- `parallel::run_parallel_text` (Vec<String>, default `String::new()`)
- `parallel::run_parallel_int` (Vec<i64>, default `i64::MIN` — null sentinel)
- `codegen_runtime::run_native_workers_primitive` (Vec<i64>, `0i64`)
- `codegen_runtime::run_native_workers_text` (Vec<String>, `String::new()`)

Net retire ~25 lines.  The helper accepts the default as a
parameter rather than `R: Default` so the int variant can keep
its `i64::MIN` null sentinel and the text variant can document
the empty-String seed explicitly.

### Plan-06 phase 3b.1 — extract shared par result store helpers


Three native par fns (`n_parallel_for_native`,
`n_parallel_for_text_native`, `n_parallel_for_ref_native`)
shared two identical 7- and 10-line boilerplate blocks for
allocating + finalising the result store.

Extracted to two helpers in `src/codegen_runtime.rs`:

- `alloc_par_result(stores, n, elem_size) -> (DbRef, u32, u32)`
  — allocates the result store, claims the vector body
  (`n * elem_size` bytes) and the 1-word header record, returns
  (result_db, vec_rec, header_rec).
- `finalize_par_result(stores, result_db, n, vec_rec, header_rec) -> DbRef`
  — writes the vector length into `vec_rec[4]`, points the
  header record at the vector, returns the canonical
  `DbRef { …, pos: 4 }` every par caller expects.

Each native par variant now opens with one helper call and
closes with another instead of inlining the boilerplate.  Net
removal: ~30 lines.  No API change — all 30 generated test
fixtures in `tests/generated/threading_chars_par_*.rs` still
match.  Sets up phase 3b.2 (true unification with a `Stitch`
trait).

### Plan-06 phase 1 — clippy gate restored on threading build


`cargo clippy --release --all-targets` was failing on the
default (threading) build with two `not_unsafe_ptr_arg_deref`
errors:

- `state::execute_at_raw_to(dst: *mut u8)` (added by
  @PLAN06 phase 1 G4 / 4d.A in commit 6973b182) was a public
  function that called `ptr::copy_nonoverlapping` without an
  `unsafe` signature.  Now `pub unsafe fn` with a `# Safety`
  doc-comment block; the single caller in
  `parallel.rs::run_parallel_direct` wraps the call in
  `unsafe { … }` with a SAFETY comment naming the slot
  pre-allocation invariant.
- `parallel::run_parallel_direct(out_ptr: *mut u8)` (added by
  4b90d89a) had a `cfg_attr(not(feature = "threading"), allow(
  not_unsafe_ptr_arg_deref))` that suppressed the lint only on
  the WASM-style build.  The attached comment explained the
  reasoning ("making the public function `unsafe` would cascade
  across every par(...) call site and the QUALITY 6a native-
  codegen path") — applied to both builds, so the allow now
  hoists out of the `cfg_attr` and the `cfg_attr` keeps only the
  feature-specific `needless_pass_by_value` + `dead_code`.

`make ci`'s clippy step is green again on the default build.

### P189b / P189d — `vector<(T1, T2, …)>` access closed end-to-end


Two follow-ups to P189 / P189c that closed the remaining
read-side gaps for tuple-element vectors.

**P189b — index-access + for-loop iteration unbox.**

`pairs[0]` returns a `DbRef` into vector storage; the existing
`OpTupleGet(slot, byte_offset)` reads from a *local slot*, so it
decoded the DbRef bytes (`store_nr | (rec << 32)`) as if they
were the tuple's first element.  For-loop iteration hit a
matching shape mismatch and reported "Field access not supported
on type tuple([…])".

Fix: when the tuple value lives in vector storage, the parser
unboxes via the synthetic `__tuple<…>` struct.

- `parser/fields.rs::unbox_tuple_from_dbref` — for `p = pairs[i]`,
  emits per-element loads (`OpGetInt`, `OpGetText`, …) against
  the DbRef and packs the results into a `Value::Tuple` so the
  assignment target receives the inline-on-stack representation.
- `parser/control.rs` for-loop iteration — re-types the loop
  variable as `Reference(__tuple<…>)`, so `p.0` / `p.1` route
  through `parse_part`'s new `__tuple<` arm, which calls
  `get_val(elem, …, offset, …)` (struct-field-style access)
  instead of the stack-tuple `OpTupleGet`.
- `parser/collections.rs::for_iter` — keeps the iterator's
  block-result type aligned with the loop variable's `RefVar(Tuple)`,
  so the next-expression yields the 12-byte DbRef the body expects.

**P189d — text-element worker arg inflation.**

After P189c made `(int, int)` tuple-input workers wide-input
correct, `(int, text)` workers still saw `len(p.1) == 0`.  The
in-vector tuple stores text as a 4-byte interned-pointer; the
worker's argument slot expects the full 16-byte `Str` (8B ptr +
8B len).  `read_primitive_at_wide`'s flat memcpy left the upper
12 bytes of the `Str` slot zero.

Fix: per-element reader.

- `parallel.rs::read_tuple_at_wide(stores, row_ref, elem_types)`
  — walks the tuple element types, copies primitives by memcpy
  and inflates `Text` fields by reading the heap pointer and
  reconstructing a `Str` via `store.get_str(...)`.
- `native.rs::tuple_first_arg_types(def)` — extracts
  `Some(elems)` when the worker's first argument is a tuple,
  else `None`.  Threaded through both `n_parallel_for` (heavy
  path) and `n_parallel_for_light`, then through
  `parallel_execute_and_collect` /
  `parallel_light_execute_and_collect` to the underlying
  `run_parallel_direct` / `run_parallel_light` calls.
- `parallel.rs::run_parallel_direct` and `run_parallel_light` —
  new `tuple_input_types: Option<Vec<Type>>` parameter.  When
  `Some`, the wide-input branch routes through
  `read_tuple_at_wide` instead of `read_primitive_at_wide`; both
  the threaded and sequential branches.  The parameter is
  Arc-wrapped per-call so worker threads share a cheap clone.

**Native codegen header.**

`generation/mod.rs` now emits `use loft::hash;` and
`use loft::tree;` alongside the existing `loft::ops` /
`loft::vector` imports.  P192's `OpLengthHash` (`hash::count`)
and `OpLengthIndex` (`tree::count`) `#rust` templates referenced
the bare module names — without the imports, any program that
reaches `len(h)` / `len(ix)` failed native compilation with
`error[E0433]: cannot find module or crate "hash"`.

**Tests:** `par_tuple_input_int_text` un-`#[ignore]`d;
`p189b_vector_tuple_for_loop_int_int` and
`p189b_vector_tuple_for_loop_int_text` added to `tests/issues.rs`
(the existing index-access tests already cover P189b's first
half).

### P193 — eager init for `local: keyed_collection<T> = []`


`gen_set_first_keyed_null` (P188's local-var alloc) fired
lazily on first WRITE.  When that first write was inside a
`for` loop body, the OpInitRef + OpDatabase init bytecode
landed inside the loop body — every iteration zeroed the
collection's root pointer.  Symptom:
`for i in 0..N { ix += ... }` over a local-var keyed
collection left `len(ix) == 1` (only the last add) and leaked
N stores.  Reading the collection BEFORE any write also
panicked with `Incorrect var ix[65535] versus N`.

Two fixes in concert:

1. **Eager init via parser rewrite** (`parser/operators.rs::create_keyed`).
   When the parser sees `Set(v, Insert(empty))` for a
   keyed-collection local, rewrite to `Set(v, Null)` so
   codegen's existing `gen_set_first_keyed_null` arm fires at
   the declaration's statement position (outside any
   enclosing loop body).

2. **Scope-exit free** (`data.rs::heap_dep` and
   `scopes.rs::get_free_vars`).  Recognise Sorted / Hash /
   Index / Spacial as heap-owned (they each get a fresh
   OpDatabase store on init), so the scope-exit OpFreeRef
   pass emits cleanup for them.  Without this the store
   leaked on program exit ("Stores not freed at program exit:
   N(bc:M)").

3 new P193 regression tests cover loop-form add (index +
hash) and read-before-write.

### Plan-06 phase 4d.B hash + index — closed by P191/P192/P188


`par_hash_input_t4` and `par_index_input_t4` un-`#[ignore]`d
and pass once the underlying P191/P188 fixes landed: the same
4d.B materialise-then-route desugar that closed
`par_sorted_input_t4` extends to hash and index automatically
once the local-var keyed-collection iteration and `+=` paths
are correct.  Phase 4 partial → 4d.B fully done; remaining
phase 4 work: 4a (typed-arity declaration), 4b (5-arg form
retirement), 4e (caller-supplied destination via ref_return).

### Vector-of-tuple support (P189 / P189c)


`vector<(T1, T2, …)>` now parses, constructs, and serves its
elements correctly via the par worker path.  Previously the type
was rejected at parse, then panicked at construction, then read
garbage — three layers fixed jointly:

- `src/parser/definitions.rs::sub_type` accepts `(...)` as the
  inner type of `vector<T>` / `iterator<T>`.
- `src/data.rs::tuple_def(lexer, types) -> u32` registers a
  synthetic struct (`__tuple<T1,T2,…>`) with attributes `_0, _1,
  …` typed per the tuple element.  Idempotent — same shape →
  same def_nr.  `Type::Tuple` arms in `type_def_nr` and
  `type_elm` look up the registered struct.
- `src/parser/vectors.rs::new_record` got a `Value::Tuple(values)`
  arm that emits per-attribute `set_field(tuple_struct_d_nr, i, 0,
  elm, values[i])` calls, mirroring the struct-literal path's
  per-field writes (which are pre-emitted via Value::Insert).

**Open follow-ups documented in PROBLEMS.md:**
- P189b: `pairs[0].0` (DbRef-aware tuple field access) reads the
  DbRef bytes as inline tuple — needs heap-tuple unboxing opcodes.
- P189d: `vector<(integer, text)>` text element reads as
  zero-length — text has different in-vector (4-byte pointer) vs
  on-stack (16-byte Str) representation; read path needs to
  inflate.

### Local-var keyed collections (P188)


`sorted<T[key]>`, `hash<T[key]>`, `index<T[key]>`, and
`spacial<T[key]>` now work as locals; previously they were only
usable as struct fields.  Patterns like

```loft
fn build() -> sorted<Tag[id]> {
    out: sorted<Tag[id]> = [];
    out += Tag { id: 1, label: "v1" };
    out
}
```

used to crash at runtime with an out-of-bounds `mut_store`
because the slot allocator gave `out` a position but neither the
bytecode codegen nor the native generator emitted the
`OpDatabase` init that allocates the backing store and zeroes
the root pointer.  Both paths now allocate the backing record on
first assignment, and subsequent `+= T {...}` operations grow the
collection in place via `record_new`'s
`Parts::Sorted/Hash/Index/Spacial` dispatch.

### Crash fixes


Three crashes that affected programs using `match` on complex types
are now fixed:

- **Character interpolation** — returning `"{c}"` from a function
  no longer crashes. The generated code now correctly handles
  writing to the caller's text buffer.
- **Recursive match on struct-enums** — `match` arms with different
  amounts of local variables (e.g. a simple `Leaf` arm vs. a
  complex `Node` arm with a for-loop) no longer corrupt the return
  address. Both arms now exit at the same stack level.
- **Memory leaks on chained calls** — `json_parse(t).field("x")`
  and similar chains no longer leak memory. The compiler now tracks
  which native functions create new values vs. which ones borrow
  from their input.

### New CLI flag: `--dump`


`loft --dump file.loft` compiles your program and prints the
internal bytecode to stderr — without running it. Useful for
debugging compiler issues. Combine with `LOFT_LOG` for extra
detail:

```bash
LOFT_LOG=variables loft --dump file.loft   # include variable table
```

### WASM / browser improvements


- The `--html` export now correctly compiles programs that call
  text-returning methods (like `kind()`, `to_json()`, `as_text()`).
  Previously this produced a type error during WASM compilation.
- The WASM build is now a release-blocking gate — if the browser
  path breaks, the release is held.

### Brick Buster game


The built-in arcade game got a polish pass: heart-shaped lives,
hand-designed levels 1-5, three original chiptune music tracks,
balloon powerups, screen shake effects, fire-ball trails, high
score persistence, and faster ball/paddle speed.

### Other improvements


- **Crash reporter** — when the interpreter hits a fatal error, it
  now prints which function and instruction caused the crash before
  exiting. Makes bug reports much more useful.
- **Parallel blocks** — `parallel { }` now uses real OS threads.
- **WebGL gallery** — 24 graphics demos running in the browser.
- **HTTP server/client** — blocking HTTP in the `web` package.
- **Playground** — better syntax highlighting, categorized examples,
  assert results shown with checkmarks.
- **Test runner** — `scripts/find_problems.sh --bg` runs the full
  test suite in the background; `--peek` to check progress,
  `--wait` for the summary. Stale caches are cleaned automatically.

### Native Moros editor


A native OpenGL editor for the Moros hex RPG now ships as a standalone
application, independent of the browser shell:

- **Entry point:** `lib/graphics/examples/moros_editor.loft` — run with
  `loft --native --path . lib/graphics/examples/moros_editor.loft`.
- **Fullscreen support:** new `gl_create_fullscreen_window` API; F11
  toggles fullscreen at runtime.
- **Input:** scroll-wheel events + expanded key codes (Home, End,
  PageUp/Down, F1–F12, arrow modifiers) now reach loft programs.
- **Panel UI overlay:** 2D panel drawn after the 3D scene pass;
  `editor_click` routes mouse events to the correct panel or 3D pick.
- **Standalone packaging:** `make editor-dist` produces a self-contained
  `dist/moros-editor/` directory; the binary runs on a machine without
  `loft` installed.
- **Native codegen fix:** functions that reconstruct constants
  (const_refs) now compile correctly under `loft --native`.  This was
  the sole native-codegen regression surfaced during Phase 3b.

All seven phases of the initiative landed on 2026-04-22.  Deferred polish
items (FPS counter, resize handling, avatar, hex-pick highlight) roll into
follow-up work and are not blockers.

### Brick Buster 0.8.4 polish pass


Gameplay feel:

- **Cel-shaded sprites** — every icon and the ball have dark outlines
  over flat-shaded bodies; the ball is a real round sprite with a
  four-frame squash animation that stretches along its velocity
  direction, so diagonal bounces look like bounces instead of flat
  horizontal/vertical squishes.
- **Paddle break** split from 3 rigid pieces to a **12-slot system**.
  On ball-lost the pieces fly out as three 4-piece planks held together
  by 1-pixel overlaps; on `SP_EXPLODE` powerup only 7 of 12 slots are
  active (pairs hidden pseudo-randomly) so some sections look like they
  held together.
- **Balloon powerup** is a rising on-screen projectile with a two-part
  hitbox.  Top half bounces the ball up and shoves the balloon down;
  bottom half mirrors.  The ball's horizontal velocity nudges the
  balloon sideways so the player can herd a loose balloon, pops on
  brick contact and triggers screen shake.
- **Screen shake** implemented as projection-matrix translation so one
  offset shakes the whole world — HUD stays fixed.  Used by balloon
  pops and the `SP_EXPLODE` paddle break.
- **Fire-ball after-images** — ring buffer of past ball positions
  renders a desaturating orange→grey trail that shrinks and fades as
  each entry ages.

Content:

- **Hand-designed levels 1–5** via a `level_brick(lv, r, c)` dispatcher:
  solid 3-row intro → first powerups in row 1 → shoulder-gap pyramid →
  downward-arrow shaft with an explode tip → smile-face pattern of
  specials.  Levels 6+ fall back to the procedural generator with
  progressively denser specials (8/50 at level 5 → +1/50 per level,
  capped at 20/50).
- **Start-row count reduced** from 5 to 3 so early sparse-powerup
  boards aren't a wall of single-colour bricks.
- **Ball and paddle both ~40 % faster** (`BALL_SPEED_BASE` 300→420 px/s,
  `PADDLE_SPEED` 500→620 px/s) — the earlier pace felt sluggish.

HUD & UX:

- **Heart-shaped lives** replace the red squares, rendered from a new
  `S_HEART` atlas cell (point-down after the canvas Y-flip).
- **Roman-numeral level caption** in the top middle (compact 28-pt
  texture per level).
- **High-score persistence** — `.loft/brickbuster_score.txt` loaded at
  boot, written on game-over when beaten, shown below the live score
  as a grey "HI <n>" line.
- **+1 heart on level clear** (soft-capped at 7).
- **Atlas diagnostic overlay** — press **I** during play to toggle a
  labelled 4×5 grid of every sprite index, useful for debugging any
  future atlas remapping.

Audio:

- **Three original chiptune tracks** (C-major "Heroic", A-minor
  "Determined", F-major "Calm Bridge") rotate through each level in
  a random order with 3–8 s silences between.  Queue resets on level
  change; once the three songs have played the sequencer is silent
  until the next level.

Infrastructure:

- `make play` target — prerequisite-checking launcher for the native
  OpenGL build with auto-recovery from stale incremental `rand_core`
  mismatches.
- `loft --html` switched to `wasm-opt -O1` — `-Oz --asyncify` was
  stripping all host imports.  Brick Buster now actually runs on
  Pages.
- Sibling-package `loft.toml` registration and `pub use audio::*` so
  `--native` resolves every `#native` symbol without stubs.
- `tests/scripts/test_gl_snapshots.sh --update` documented in
  `doc/claude/GAME_TESTING.md` as the canonical way to regenerate
  golden PNGs after a visual change.

### WebGL graphics gallery


- **GL6.1** — Graphics library .loft files embedded in WASM binary; `use graphics;`
  resolves under WASM without a native cdylib.
- **GL6.2–GL6.3** — WebGL2 bridge (`wasm_gl.rs`): 43 native gl_* functions read
  interpreter stack arguments and forward to JavaScript via `host_call`.
  `State::replace_native()` swaps panic stubs with real implementations.
- **GL6.5** — Shader version patching: GLSL `#version 330 core` automatically
  converted to `#version 300 es` with precision qualifiers for WebGL2.
- **GAL.2** — Graphics gallery page (`doc/gallery.html`) with WebGL2 canvas,
  example selector, source viewer, and complete JavaScript GL implementation.

### Playground improvements


- Assert results rendered with checkmarks/crosses and pass/fail summary.
- Examples split into categorized groups (Getting Started, Basics, Collections,
  Types & Patterns, Advanced, System, Performance) with `<optgroup>`.
- FizzBuzz default example added; 4 performance benchmarks (Fibonacci, Sieve,
  Mandelbrot, Collatz).
- Syntax highlighting fix: parentheses and punctuation now visible.
- Success status shows "Ok" instead of "error []".
- Diagnostics Display outputs clean newline-separated text instead of debug format.

### Game protocol (Sprint 17)


- **SRV.P** — `game_protocol` package: `MsgType` enum, `WsMessage`,
  `GameEnvelope` structs, and message constructors (`msg_ping`, `msg_pong`,
  `msg_chat`, `msg_input`, `msg_state`, `msg_error`).

### Parallel threading


- **A15** — `parallel {}` now uses real OS threads via `std::thread::scope`.
  Each arm runs in its own thread with a cloned `WorkerStores` snapshot.
  Validated: loft HTTP server + client running concurrently in `parallel {}`.

### HTTP server (Sprint 16)


- **SRV.1** — Blocking HTTP server with polling model. Loft controls the
  request loop via I13 iterator protocol (`for req in srv`). Native cdylib
  handles TCP accept/parse/respond using `std::net` only — no tokio/hyper.
  Functions: `listen`, `next` (iterator), `respond`, `close`.

### Graphics native (Sprint 15)


- **GL5.1** — Window creation + event loop via `glutin` + `winit` with
  `pump_app_events` polling model. Thread-local state via `RefCell`.
- **GL5.2** — Shader compilation and linking (vertex + fragment GLSL).
- **GL5.3** — VBO/VAO upload from packed vertex data (position + normal + color).
- **GL5.4** — Draw calls + render loop with `gl_draw`, `gl_clear`, `gl_swap_buffers`.
- **GL5.5** — Texture upload, binding, and deletion via `glTexImage2D`.
- **GL3** — Font loading (`fontdue`), text width measurement, and alpha bitmap
  rasterization. All in the `lib/graphics/native/` cdylib — no font dependency
  in the interpreter.

### HTTP client (Sprints 13–14)


- **H4.1** — `HttpResponse` struct with `status: integer`, `body: text`, and
  `ok()` method in the `web` package (`lib/web/`).
- **H4.2** — `http_get`, `http_post`, `http_put`, `http_delete` via native
  cdylib using `ureq`.  The `ureq` crate is only in the cdylib — the
  interpreter has no HTTP dependency.
- **H4.3** — Header support: `http_get_h`, `http_post_h`, `http_put_h`,
  `http_delete_h` accept `vector<text>` of `"Key: Value"` headers.
- **loft_register_v1** — unified native extension registration protocol.
  Each cdylib exports one C-ABI function that registers all symbols via a
  callback.  Generic `HashMap<String, FnPtr>` replaces per-function statics.
  All native cdylibs (imaging, random, web) use the new protocol.

### Native codegen for packages (Sprint 11)


- **PKG.4** — Native codegen `--extern`: packages with `[native.functions]` in
  `loft.toml` now emit direct Rust calls in `--native` mode.  The build pipeline
  passes `--extern` flags for pre-compiled native rlibs.
- **PKG.5** — WASM codegen linking: `--native-wasm` resolves package WASM rlibs
  from `prebuilt/wasm32-wasip2/` or `native/target/wasm32-wasip2/release/`.

### Language ergonomics (Sprint 10)


- **C55** — Type aliases: `type Handler = fn(Request) -> Response` — compile-time
  substitution for function and tuple types in `type` declarations.
- **C56** — Null-coalesce with early return: `x ?? return err` desugars to a
  null-check with immediate function return, collapsing two-line null guards
  into one expression.
- **A15** — `parallel { }` structured concurrency block: runs each arm
  sequentially (threading deferred). Three new opcodes replace six dead
  superinstruction slots, freeing three net opcode slots.
- **I13** — Iterator protocol: any type with `fn next(self: T) -> Item?` can be
  used in a `for x in val` loop. Null return from `next` terminates the loop.

### Graphics library (pure-loft package)


- **GL0** — Package scaffolding: `lib/graphics/` with `loft.toml` manifest.
- **GL1** — `Canvas` struct with `canvas()`, `get_pixel()`, `set_pixel()`, `clear()`,
  `blend()`, `blend_pixel()`.
- **GL2.1** — Drawing primitives: `fill_rect()`, `hline()`, `vline()`, `draw_rect()`.
- **GL2.2** — `draw_line()`: Bresenham algorithm for all octants.
- **GL2.3** — `draw_circle()`, `fill_circle()`, `draw_ellipse()`: midpoint algorithms
  with octant/quadrant symmetry.
- **GL2.4** — `draw_bezier()`: cubic Bezier with adaptive de Casteljau subdivision.
- **GL2.5** — `fill_triangle()`: scanline fill with vertex sorting.
- **GL2.6** — `draw_aa_line()`: Xiaolin Wu anti-aliased line with alpha blending.
- `fill_ellipse()`: solid filled ellipse via midpoint algorithm.
- **GL4.1** — `math.loft`: `Vec2`, `Vec3`, `Vec4`, `Mat4` types with vector ops
  (`add3`, `sub3`, `scale3`, `dot3`, `cross`, `normalize3`, `length3`) and matrix
  ops (`mat4_identity`, `mat4_translate`, `mat4_scale`, `mat4_mul`, `mat4_transform`).
- **GL4.2** — `mesh.loft`: `Vertex`, `Triangle`, `Mesh` types with builders
  (`add_vertex`, `add_triangle`, `add_quad`, `cube()`).
- **GL4.3** — `scene.loft`: `Material`, `Node`, `Camera`, `Scene` types with
  PBR material support and scene graph builder.
- **GL5** — `glb.loft`: `save_glb(mesh, path)` exports a single `Mesh` as a
  GLB 2.0 file (POSITION, NORMAL, TEXCOORD_0, u32 indices).  5 binary tests.
- **GL6** — `glb.loft`: `save_scene_glb(scene, path)` exports a full `Scene`
  with multiple meshes, PBR materials, and nodes into one GLB BIN chunk.
  9 tests including JSON content verification and multi-mesh BIN size.
- **GL7** — `scene.loft`: `node_at(name, mesh, mat, transform)` constructor.
  glTF 2.0 compliance: material reference moved to mesh primitive; node
  transform outputs `"matrix"` field only when non-identity.
- RGBA color packing via `rgba()`/`rgb()` using long arithmetic to avoid i32::MIN
  sentinel collision.
- 30 canvas tests covering all primitives.

### Bug fixes


- **C54** — `**` exponentiation operator now works, mapped to `pow()`.
- **P104** — Test runner no longer picks up library functions as tests;
  only functions defined in the test file are executed.
- **P107** — `++` (not a valid operator) now produces a clear error instead
  of crashing in codegen with a confusing type mismatch.

### Package registry (Sprint 9)


- **REG.1** — `src/registry.rs`: registry file parser with version resolution,
  package classification (yanked/deprecated/outdated/current/unknown), and
  installed package scanner.
- **REG.2** — `loft install <name>[@version]`: download and install packages
  from the registry.  Detects already-installed versions, warns on yanked packages.
- **REG.3** — `loft registry sync`: download the latest registry from the
  source URL (`# source:` header, `LOFT_REGISTRY_URL` env, or compiled-in default).
- **REG.4** — `loft registry check`: compare installed packages against the
  registry, report yanked/deprecated/outdated status, exit 1 on security issues.
- `loft registry list [--installed]`: browse all registry packages with
  installed status.

### Package infrastructure


- **PKG.1** — Native stub registration: `#native` annotations generate stubs replaced
  at load time by real shared-library implementations.
- **PKG.2** — `loft install` command for local package installation to `~/.loft/lib/`.
- **PKG.3** — Transitive dependency resolution: packages with `[dependencies]`
  in `loft.toml` automatically discover sibling packages.
- **`loft doc`** — New subcommand generates HTML documentation for packages:
  API reference from `src/*.loft` and guide pages from `docs/*.loft`.
- **PKG.6** — `loft test` subcommand discovers and runs `tests/*.loft` in packages.
- **PKG.3** — `[dependencies]` section in `loft.toml` manifest parsing.
- Manifest parser: `name`, `version`, `loft` version constraint, `native` stem fields.

---

### A package's manifest chose where the package landed on disk (2026-08-20)


`loft install <dir>` files a package under `~/.loft/lib/<name>`, and since the name became
the MANIFEST's rather than the checkout directory's, nothing checked it before the join. A
manifest is data — on a fetched package, data somebody else wrote — so `name = "../../escaped"`
wrote the whole package tree to `$HOME/escaped/`, outside `~/.loft/lib` entirely. Verified
against the pre-fix binary, which is what makes the regression guard non-vacuous.

The rule already existed and was never asked here: `loft new` has enforced "lowercase ascii +
digits + underscore" since it was written, stated inline at the site where a package is
CREATED. It now lives in `libscan::is_valid_package_name` and three sites read it — `loft new`,
the install path, and the prebuilt-cdylib path, which takes `[library] native` from a manifest
that came off the network and uses it as a filename. Measured first: every package name and
every `native` stem in this tree passes, so no existing package is refused.

### An `i32` local kept a value an `i32` cannot hold (2026-08-20)


`guard_narrow_alias_local` clamps a compound assignment to a narrow-alias local's own range and
names five aliases it covers. It reached four. `is_signed32_template()` reads like a test for
the plain `integer` type, but `integer` carries no `forced_size` and has already left by that
line — so the only spec whose range IS the signed-32 range is the `i32` alias, and the clause
could only ever exclude the one thing the comment above it promised to include. `l: i32 =
2147483647; l += 1` kept 2147483648 on both backends where the same write to a `u8` clamped.
Now clamps to `-2147483647` (an `i32`'s minimum; `i32::MIN` is the null sentinel).

An `i32` FIELD still answers `null` where the narrower four answer their minimum — both say the
write did not fit, and making them identical means reclaiming `i32::MIN`, which the roadmap
already carries as deferred.

### Generics: five collapses in the deferred-marker and tuple paths (2026-08-20)


A template stamps a marker where a decision needs `T` and cannot have it yet, and
`rewrite_generic_type_defaults` answers those markers once `T` is concrete. That walk must be
TOTAL; it enumerated its own carriers and listed ten of the seventeen `Value` variants that
hold children. `Tuple` was among the seven missing, so `t = (a?, 1)` read the placeholder's
bytes as data — silently at `T = integer`, as a SIGSEGV in `OpFreeText` at `T = text`, and as an
E0308 that would not compile on `--native`. Recursion now delegates to
`Value::for_each_child_mut`.

Four more of the same shape landed with it: `type_mentions_tv` folded onto
`Type::contains_def` (which already answers it through the `Type::for_each_child` keystone, two
hundred lines from a call to it); the synthetic `__nullable<S>` is no longer minted for a
template's `T`, an attribute-less placeholder struct that satisfied every eligibility condition
and refused `-> (T?, integer)` on both backends; the tuple emitter's owned-text decision is
split from the literal that merely passes through it; and `tuple_has_text_leaf` peels
`Optional`, with the return path's inline copy of it now reading the shared predicate. That last
one was never a generic problem — a PLAIN `fn ret() -> (text?, integer)` would not compile on
`--native` before it.

Each proven under both gates: byte-identical IR and native Rust for the paths not being changed
(six reference corpora), and twin-compared matrices for the ones that were.

## [0.8.3] — 2026-04-03

### Bug fixes

- **P58** — Variables with unknown type (typos like `y = unknown_thing`) now
  produce a compile-time error instead of silently creating garbage values.
- **P99** — Empty struct comprehension (`[for x in 0..0 { Struct{} }]`) with
  multiple hash types no longer crashes the compiler.
- **P100** — Format left-align (`:<`) and center-align (`:^`) now work for
  integers, longs, and floats.
- **P101** — Float format `:.0` (zero precision) now correctly rounds to zero
  decimal places.
- **P102** — `rev(vector)` now works — plain vectors can be iterated in reverse.
- **P98** — Index range queries with descending primary key now return correct
  results, in both interpreter and native codegen.
- **P91** — Circular `= expr` field defaults (e.g. `a: integer = $.b, b: integer = $.a`)
  are now detected at compile time.
- **C54** — `file.lines()` now returns content after the last newline (or content
  with no newlines at all).
- **P103** (mitigated) — Compile-time warning when vector concatenation appears
  inline in an expression that could corrupt the stack.
- **Windows native codegen** — Backslashes in file paths are now escaped in
  generated Rust string literals.

### Test infrastructure

- `tests/wrap.rs` now discovers and runs all `fn test_*()` entry points, not
  just `main()`.  Supports `@EXPECT_FAIL`, `@EXPECT_ERROR`, `@EXPECT_WARNING`
  annotations per function with `catch_unwind` isolation.
- 12 new test scripts (61–74) covering vector sort/reverse, index range queries,
  format edge cases, hash edge cases, known-issue reproducers (caveats/problems),
  and constant vector initialisation.
- `SUITE_SKIP` emptied — `15-lexer.loft` and `16-parser.loft` now pass.
- Branch protection enabled on `main` — PRs required with all 5 CI checks.

### Optimisations

- **`const_eval`** module — compile-time constant folder for arithmetic, casts,
  comparisons, and boolean ops across all numeric types.
- **`OpPreAllocVector`** — pre-allocates vector capacity for known-size literals,
  eliminating all `store.resize()` calls.
- **Constant comprehension unrolling** — `[for i in 0..N { expr(i) }]` is unrolled
  at compile time when bounds and body are const-evaluable (10,000-element limit).

### Documentation

- New **PACKAGES.md** — unified package format design (native Rust + WASM,
  dependencies, test suites, OpenGL case study).
- New **CONST_DATA.md** — constant data initialisation design with safety analysis.
- New doc pages: **29-match.loft** (pattern matching) and **30-formatting.loft**
  (format string reference).
- Expanded: 14-image (pixel scanning), 19-threading (worker rules), 25-generics
  (bounded generics with interfaces).
- Regenerated 137-page PDF reference.
- ROADMAP split: all H/MH items into S/M testable sub-steps with sprint ordering.
- PLANNING pruned: 473 lines of completed items removed.

### Closures

- **Cross-scope text-capturing closures** (A5.6-text) — Functions that return
  closures (`fn make_greeter(prefix: text) -> fn(text) -> text`) now work correctly.
  Four interrelated bugs fixed: premature closure free at function return, missing
  work buffer for text-returning fn-ref calls, 12-byte fn-ref pre-init (should be
  16 bytes), and closure record leak at caller scope exit.  Test:
  `closure_capture_text`.

### Native codegen

- **Fn-ref `(u32, DbRef)` tuple type** (C39) — Fn-ref variables in native-compiled
  code are now `(u32, DbRef)` tuples instead of plain `u32`.  Closure records are
  correctly freed via `.1` destructuring when fn-ref variables go out of scope.
  Non-capturing lambdas use the null sentinel and are safely skipped.

### Closures — native parity

- **Native cross-scope closures** (C47) — Functions that return closures now
  work in `--native` mode.  Five fixes: FnRef emits closure DbRef, CallRef
  passes `.1` as `__closure`, scope analysis skips cross-function deps,
  `last_closure_work_var` reset after function body, FnRef added to reachable
  set.  Doc test `26-closures.loft` now includes cross-scope `make_adder`.

- **Capturing closures with map/filter** (C48) — `map(v, fn(x) { x * factor })`
  and `filter(v, fn(x) { x > threshold })` now work with capturing lambdas.
  The collections parser accepts fn-ref variables and emits CallRef in the
  desugared loop body.

### Slot assignment

- **Text slot reuse** (C43) — Sequential text variables with non-overlapping lifetimes
  now share the same 24-byte zone-2 slot.  Uses a full conflict scan
  (`find_reusable_zone2_slot`) restricted to Text-only reuse at the top-of-stack
  position.  Tests: `assign_slots_sequential_text_reuse`, `text_slot_reuse_sequential`.

### Bug fixes (continued)

- **C41** — Struct-enum local variable leak (Problem #85) confirmed fixed; regression
  test `struct_enum_local_freed` added.
- **C42** — Undefined variable diagnostic confirmed working; test
  `unknown_variable_error` added.
- **C40** — Debug logger fn-ref opcode guard documented with WARNING comments in
  `02_images.loft` to prevent accidental removal.

### Parallel execution

- **`par_light` runtime foundation** (A14.1–A14.4):
  - A14.1: `Store::borrow_locked_for_light_worker` — O(1) read-only view sharing the
    original's buffer pointer. `borrowed` field prevents double-free on Drop.
  - A14.2: `WorkerPool` — pre-allocates `n_workers × M` stores, reused across invocations.
  - A14.3: `Stores::clone_for_light_worker` — assembles worker view with shallow borrows
    of main stores + fresh pool stores. Zero large buffer copies.
  - A14.4: `run_parallel_light` — drop-in for `run_parallel_direct` using the pool.
  - A14.5: `check_light_eligible` — DFS call-graph analysis validates no recursive store
    allocation. Returns `M` (pool stores per worker) for eligible workers.
  - A14.6: `build_parallel_for_ir` automatically selects `n_parallel_for_light` when
    the worker qualifies (primitive return, no recursive allocation). No new syntax —
    `par(...)` is transparently optimized.
  - A14.7: `n_parallel_for_light` native function registered. Allocates result vector,
    creates `WorkerPool`, dispatches via `run_parallel_light`.
  Auto-selection is fully enabled: eligible `par()` workers (primitive return,
  no recursive store allocation) transparently use the light path.  Three bugs
  fixed in the enablement: stack pop order in the native function, result DbRef
  `pos` field (4 not 8), and store borrow range (all stores, not just `[..max]`).

### Sorted collection slicing (A8)

- **Partial-key match iterator** (A8.3): `idx[k1]` on a multi-key index now iterates
  all elements matching the first key. Parser detects `nr < key_types.len()` and emits
  an inclusive range with `from = till = [k1]`. The existing `key_compare` zip-based
  comparison treats partial prefixes as unconstrained on remaining fields.

### WASM parallel infrastructure (W1.18)

- **WASM Worker Thread infrastructure** (W1.18-1 through W1.18-5):
  - W1.18-1: `#[cfg(all(feature = "wasm", feature = "threading"))]` branch in
    `run_parallel_direct` dispatches to JS host via `parallel_run()`.
  - W1.18-2: `worker_entry(fn_index, start, end)` exported via `#[wasm_bindgen]`.
  - W1.18-3: `tests/wasm/worker.mjs` — Worker Thread park/wake loop.
  - W1.18-4: `tests/wasm/parallel.mjs` — `LoftThreadPool` class.
  - W1.18-5: `tests/wasm/harness.mjs` — `initThreaded()` for shared-memory WASM.
  W1.18-6 (test enablement) deferred until wasm-threads build is available.

### Debugging infrastructure

- **Debug boundary checks for DbRef, record fields, and stack pops** —
  Three `debug_assert!` additions (zero cost in release builds):
  - `keys::store()` / `keys::mut_store()`: assert `store_nr < allocations.len()` with
    clear message showing both values.
  - `Store::addr()` / `Store::addr_mut()`: validate field offset against the record's
    claimed size (first word of record header). Fires for `rec > 1, fld > 0`.
  - `Stores::get<T>()`: assert `stack.pos >= size_of::<T>()` before decrement, catching
    stack underflow from wrong native-function pop order.

### Safety fixes

- **Coroutine store-mutation guard promoted to always-on** (CO1.9) — The generation
  counter in `Store` and the `saved_store_generations` snapshot in `CoroutineFrame`
  were previously compiled only under `#[cfg(debug_assertions)]`.  All `#[cfg]` gates
  have been removed so the guard fires in release builds too.  `debug_assert!` in
  `coroutine_next` is replaced with `assert!`, meaning a mutated-store violation now
  panics with a clear diagnostic in any build profile:
  `"stale DbRef: store N was mutated between coroutine yields (generation at yield: X,
  now: Y) — DbRef locals held by the generator may point to freed or reallocated records"`.
  The affected sites in `store.rs` are `claim`, `resize`, `delete`, and the two
  `clone_locked*` constructors.  New test `coroutine_stale_store_guard_all_builds`
  (no `#[cfg(debug_assertions)]` gate) confirms the panic fires unconditionally.

### Language features

- **`interface` keyword and first-pass parser** (I1, I2, I3) — The first three steps
  of the interface subsystem are implemented:
  - I1 (`src/lexer.rs`): `"interface"` is now a reserved keyword.
  - I2 (`src/data.rs`): `DefType::Interface` added to the definition-type enum;
    `Definition.bounds: Vec<u32>` added to hold interface constraints for bounded
    generic functions (`<T: A + B>`); initialised to `vec![]` in `add_def`.
  - I3 (`src/parser/definitions.rs`, `src/parser/mod.rs`): new `parse_interface()`
    method parses `interface Name { fn method(params) -> type }` declarations.
    `Self` is temporarily registered as a type placeholder so `parse_type_full`
    resolves it during method signature parsing.  Duplicate interface names emit
    "Redefined interface Name".  `parse_interface` is added to the `parse_file`
    top-level dispatch chain alongside `parse_struct`, `parse_enum`, etc.
  Tests: `interface_empty_parses`, `interface_with_method_parses`,
  `interface_duplicate_name_rejected`.

- **Interface subsystem — op-sugar, bound syntax, factory-method guard, gendoc skip** (I3.1, I4, I5, I11):
  - I3.1 (`src/parser/definitions.rs`): `op <token> (params) -> type` in interface bodies
    is syntactic sugar for an `OpCamelCase` method stub. E.g. `op < (self: Self, other: Self) -> boolean`
    registers a method named `OpLt`. The `rename()` helper in `mod.rs` is now `pub(crate)` and
    covers `>` and `>=` in addition to its previous set.
    Tests: `interface_op_sugar_lt_parses`, `interface_op_sugar_multi_parses`.
  - I4 (`src/parser/definitions.rs`): `<T: A + B>` bound syntax in generic function declarations.
    Bound names are collected during parsing and resolved in the second pass to `DefType::Interface`
    def_nrs stored in `Definition.bounds` (introduced in I2). Unknown names emit
    `"'Name' is not a known interface"`; non-interface names emit
    `"'Name' is not an interface — bounds must be interface names"`.
    Tests: `generic_fn_with_bound_parses`, `generic_fn_unknown_bound_errors`,
    `generic_fn_struct_as_bound_errors`.
  - I5 (`src/parser/definitions.rs`): phase-1 factory-method restriction in interface bodies.
    A method that returns `Self` without a leading `self: Self` parameter emits
    `"factory methods not yet supported: 'name' returns Self without a 'self: Self' parameter"`.
    Test: `interface_factory_method_rejected`.
  - I11 (`src/gendoc.rs`): `sig_kind` now returns `"interface"` for `pub interface` / `interface`
    declarations (previously `"const"`). `generate_stdlib_section` skips interface items gracefully.
    Unit test: `sig_kind_interface_returns_interface`.

- **Interface subsystem — satisfaction checking, bounded method/operator calls** (I6, I7, I8.1, I10):
  - I6 (`src/parser/mod.rs`): `check_satisfaction` verifies that a concrete type implements
    every method declared in a bounded generic's interface constraints. Called from
    `try_generic_instantiation` — emits `"'Type' does not satisfy interface 'Name': missing Method"`.
    Tests: `satisfaction_check_passes_with_implementing_type`,
    `satisfaction_check_fails_missing_method`.
  - I7 (`src/parser/fields.rs`, `src/parser/definitions.rs`): T-parameterized method stubs
    (e.g. `t_1T_label`) are created during second-pass bounds resolution. `field()` looks up
    the T-stub via `find_fn` before reporting "field access requires a concrete type", enabling
    `v.method()` inside generic bodies. `re_resolve_call` substitutes the concrete implementation
    at specialization time.
    Test: `bounded_method_call_in_generic_body`.
  - I8.1 (`src/parser/mod.rs`): `call_op` looks up T-stubs for operators (e.g. `t_1T_OpLt`)
    before erroring, enabling `a < b` inside bounded generic bodies. First-pass operator calls
    on T now return `Type::Void` instead of erroring, allowing the second pass to proceed.
    Test: `bounded_operator_in_generic_body`.
  - I10: satisfaction diagnostics share the I6 implementation above.
  - Supporting changes: `Data::children_of` iterates definitions by parent;
    `field()` returns `Type::Unknown(0)` in the first pass for unknown-type field access
    (previously errored); user-defined operator functions (e.g. `fn OpLt(self: Score, ...)`)
    are now allowed in user code without a lowercase name error.

- **Interface operator variants and stdlib `Ordered`** (I8.2, I8.3, I8.4, I9):
  - I8.2: Return-type propagation from interface signature — verified: T-stubs correctly
    substitute `Self` → `T` in both parameter types and the return type.
    Test: `bounded_operator_self_return_type`.
  - I8.3: Mixed-type binary operators (`T op concrete`, e.g. `T * integer`) — verified:
    `call_op`'s T-stub lookup and `call_nr`'s argument matching handle mixed-type parameters.
    Test: `bounded_mixed_type_operator`.
  - I8.4: Unary operators on `T` (e.g. `op -`) — verified: single-operand dispatch uses the
    same `call_op` → T-stub path as binary operators.
    Test: `bounded_unary_operator`.
  - I9: `pub interface Ordered { op < }` added to `default/01_code.loft`. User types satisfy
    `Ordered` by defining `fn OpLt(self: T, other: T) -> boolean`. Existing tests updated to
    use the stdlib interface instead of local redefinitions.
    Test: `stdlib_ordered_interface`.

- **Built-in type satisfaction and stdlib Equatable/Addable** (I9-prim, I9-Eq, I9-Add, I9.1):
  - I9-prim: `find_fn` now falls back to the `possible` operator map when the method-style
    name (`t_7integer_OpLt`) is not found. This lets built-in types (integer, float, etc.)
    satisfy interfaces since their operators use the `add_op` convention (`OpLtInt`).
    `call_op` skips the main operator loop when an operand is a generic type variable,
    preventing false matches via `OpEqRef` / `OpEqBool` implicit conversions.
    `check_satisfaction` delegates to `find_fn` for both naming conventions.
    Tests: `builtin_integer_satisfies_ordered`, `builtin_float_satisfies_ordered`.
  - I9-Eq: `pub interface Equatable { op == }` added to `default/01_code.loft`.
    Test: `stdlib_equatable_interface`.
  - I9-Add: `pub interface Addable { op + }` added to `default/01_code.loft`.
    Test: `stdlib_addable_interface`.
  - I9.1: bounded generics with Addable work on integer and float types.
    Tests: `generic_sum_pair_on_integers`, `generic_sum_pair_on_floats`.

- **Vector<T> element access fix and Numeric interface** (I9-vec, I9.1, I9.2, I9+):
  - I9-vec: fix vector element access in generic specialization. `substitute_type_in_value`
    detects `OpGetVector` calls with baked-in `elm_size=0` (from type variable elements),
    recomputes the correct size from the concrete type, and adds the value-extraction wrapper
    (`OpGetInt`/`OpGetFloat`/etc.).  First-pass `call_op` for generic types now returns the
    type variable type (not `Type::Void`) to prevent "cannot change type" errors.
    Test: `generic_vector_element_access`.
  - I9.1: bounded-generic comparison on vector elements using `Ordered` bound.
    Test: `generic_min_of_vector_elements`.
  - I9.2: bounded-generic sum of vector elements using `Addable` bound.
    Test: `generic_sum_on_integer_vector`.
  - I9+: `pub interface Numeric { op * ; op - }` added to `default/01_code.loft`.
    Test: `stdlib_numeric_interface`.

- **Generic accumulator fix, Scalable interface** (I9-var, I9.1, I9.2, I9-Sc):
  - I9-var: skip `ref_return`/`text_return` for generic templates (`DefType::Generic`).
    The return type `T = Reference(tv_nr)` triggered `ref_return` which promoted local
    variables to hidden parameters.  After specialization to Integer/Float, the hidden
    params caused a codegen crash.  This enables for-loop accumulator patterns inside
    generic bodies.
    Tests: `generic_intermediate_variable`, `generic_for_loop_accumulator`.
  - I9.1: generic `find_max` on integer vectors using `Ordered` for-loop accumulator.
    Test: `generic_max_on_integer_vector`.
  - I9.2: generic `vec_sum` with caller-supplied identity using `Addable` for-loop.
    Test: `generic_sum_with_identity`.
  - I9-Sc: `pub interface Scalable { fn scale(self, factor: integer) -> integer }` in
    `default/01_code.loft`.  Uses a method (not `op *`) to avoid stub-name collision
    with `Numeric`.
    Test: `stdlib_scalable_interface`.

- **Interface stub collision fix, generic min_of/max_of/sum** (I9-stub, I9.1, I9.2):
  - I9-stub: interface method stubs now use `__iface_{d_nr}_{method}` naming instead of
    `t_4Self_{method}`. Multiple interfaces can now declare the same operator without
    collision. `has_bound_for_method` prevents T-stubs from leaking into unbound generics.
  - I9.1: `min_of` and `max_of` replaced with bounded-generic versions using `Ordered`.
    Now work on integer, float, and any user type satisfying `Ordered`. Unused helper
    functions (`__min_int`, `__min_float`, `__max_int`, `__max_float`) removed.
    Tests: `stdlib_min_of_generic`, `stdlib_max_of_generic`, `stdlib_min_of_float`,
    `stdlib_max_of_float`.
  - I9.2: `pub fn sum<T: Addable>(v: vector<T>, init: T) -> T` added. The caller supplies
    the identity element. Integer-specific `sum_of(v)` kept for backward compatibility.
    Test: `stdlib_sum_generic`.

- **Text-returning interface methods, Printable, coroutine yield-from-loop** (I9-text, I9-Pr, CO1.7):
  - I9-text: T-stub creation adds hidden `__work_1: RefVar(Text)` parameter for
    text-returning interface methods. Matches the hidden param from `text_return` so
    `re_resolve_call` finds the correct argument count.
    Test: `generic_text_returning_method`.
  - I9-Pr: `pub interface Printable { fn to_text(self: Self) -> text }` added to stdlib.
    Test: `stdlib_printable_interface`.
  - CO1.7 (partial): coroutine yield from range-based and vector for-loops verified.
    Tests: `coroutine_yield_from_range_loop`, `coroutine_yield_from_vector_loop`.

- **CO1.7 complete: coroutine yield from all for-loop types** —
  Fixed character null sentinel bug: `push_null_value(4)` uses `i32::MIN` as the
  sentinel for all 4-byte values, but `op_conv_bool_from_character` only checked for
  `char::from(0)`. The `i32::MIN` sentinel (0x80000000) looked like a valid character,
  causing for-loops over character iterators to infinite-loop. Also fixed UB in
  `var_character` (fill.rs): reading `i32::MIN` directly as `char` is not a valid
  Unicode scalar — now reads as `u32` and converts via `char::from_u32`.
  Tests: `coroutine_yield_from_text_loop`, `coroutine_character_iterator_exhausts`,
  `coroutine_yield_from_struct_vector_loop`, `coroutine_yield_from_field_text_loop`.

- **CO1.8 complete: multi-text coroutine safety** — Verified all three CO1.8 sub-items
  pass without code changes: (a) multiple text parameters serialised correctly,
  (b) text locals after first yield survive resume, (c) text locals in nested blocks
  freed correctly. Tests: `coroutine_multi_text_params`, `coroutine_text_local_after_yield`,
  `coroutine_text_local_nested_block`.

- **fix-tvscope: clear diagnostic for type variable name clash** — Defining `struct T`
  when `T` is a generic type variable (from stdlib generics) now produces
  `"'T' is reserved as a generic type variable"` instead of a confusing
  "Redefined struct" message or a runtime crash.

### Sorted collection slicing (A8) (continued)

- **Open-ended bounds, range iteration, comprehensions** (A8.1, A8.2, A8.4, A8.6):
  - A8.1: `col[lo..]`, `col[..hi]`, and `col[..]` now work on sorted collections.
    Parser detects `..` before the first expression (open-start) and missing expression
    after `..` (open-end). Runtime handles empty from/till arrays in OpIterate.
    Tests: `sorted_open_end_range`, `sorted_open_start_range`.
  - A8.2: `sorted[lo..hi]` range iteration verified working. Test: `sorted_range_iteration`.
  - A8.4: `[for e in sorted[lo..hi] { expr }]` comprehensions verified.
    Test: `sorted_range_comprehension`.
  - A8.6: nullable lookup `if !col[k]` verified. Test: `sorted_nullable_lookup`.
  - A8.1-idx: open-ended bounds also work on index collections. Test: `index_open_end_range`.
  - A8.5: `rev(col[lo..hi])` reverse range iteration on sorted collections. Parser sets
    `reverse_iterator` flag before the inner subscript expression so `fill_iter` picks it up.
    Test: `sorted_reverse_range`.

### Coroutine safety documentation

- **Coroutine text arg `Str` serialised at create; pointer-patched on resume** (S25.1, S25.2) —
  `State::coroutine_create` now calls `serialise_text_args` after copying the raw
  argument bytes.  For each text (`Str`) argument that points into a dynamic heap
  allocation (not a static literal in `text_code`), the function clones the string
  data into an owned `String` stored in `frame.text_owned`, then overwrites the
  `Str` bytes in `stack_bytes` to point to the owned buffer.  The owned `String`
  outlives any `OpFreeText` the caller may emit after the create; the `Str` pointer
  is therefore never dangling on the first or any subsequent resume (P2-R1, critical
  use-after-free).
  At `coroutine_next`, each owned String's current buffer address is patched back
  into the cloned `stack_bytes` before the bytes are copied to the live stack
  (M6-b pointer-patch step).
  At `coroutine_return`, the existing `frame.text_owned.clear()` now properly drains
  the owned Strings that were populated by S25.1, freeing their heap allocations via
  Rust RAII instead of leaking them (P2-R2, high memory leak).
  Two new tests `coroutine_text_arg_dynamic_serialised` and
  `coroutine_text_arg_freed_at_return` in `tests/expressions.rs` exercise the create
  → resume → exhaust cycle with a dynamically formatted text argument.

- **`const` parameter writes now panic in release builds** (S22) — The
  `#[cfg(debug_assertions)]` guard on auto-lock insertion has been removed from
  `src/parser/expressions.rs`.  `store.claim()` and `store.delete()` now use
  `assert!` instead of `debug_assert!`, so writes to `const` Reference or Vector
  parameters produce a panic in both debug and release builds.  Previously, release
  builds silently discarded the write into a dummy buffer, causing `par()` workers
  to continue with stale data.  Tests `claim_on_locked_store_panics` and
  `delete_on_locked_store_panics` in `tests/expressions.rs` verify the runtime
  enforcement.

- **`e#remove` on a generator iterator: defense-in-depth runtime guard** (S24) —
  Calling `e#remove` inside a generator `for` loop was already rejected at compile
  time (CO1.5c).  A matching runtime guard has been added to `state/io.rs::remove()`
  and `codegen_runtime.rs::OpRemove()`: if `store_nr == u16::MAX` (the coroutine
  sentinel), a `debug_assert!` fires and the call returns early, preventing
  release-build store corruption even if the compiler check is somehow bypassed.

- **Generator functions rejected as `par()` workers at compile time** (S23) — The
  parser now detects when a `par()` worker function has return type `iterator<T>` and
  emits a clear diagnostic instead of allowing the call to proceed.  At runtime,
  worker threads have their own (empty) coroutine table; passing a generator DbRef
  across thread boundaries would either panic with an out-of-bounds index or silently
  advance the wrong generator.  A runtime bounds guard in `coroutine_next` provides
  defence-in-depth.  Test `par_worker_returns_generator` in `tests/parse_errors.rs`
  covers the compile-time path.

- **Abandoned coroutine frame freed on early `for` loop exit** (S37) — When a `for`
  loop breaks before a generator exhausts, `OpFreeRef` calls `free_ref` on the
  coroutine DbRef.  `database.free()` is a no-op for `COROUTINE_STORE`
  (store_nr == u16::MAX), so `text_owned` buffers, `stack_bytes`, and `call_frames`
  in the `CoroutineFrame` were silently leaked on every early-break path.
  Fix: `free_ref` now checks `db.store_nr == COROUTINE_STORE` and calls
  `free_coroutine(db.rec)` explicitly before returning.  Test
  `coroutine_early_break_frame_freed` in `tests/expressions.rs` exercises the
  early-break path and verifies the correct first-yield value is returned.

- **Exhausted coroutine slots freed immediately** (S26) — `coroutine_return` now sets
  the slot to `None` after marking it `Exhausted`, so the `State::coroutines` Vec does
  not grow without bound across repeated `for n in gen() { }` loops.  A guard in
  `coroutine_next` handles the `None` case (push null, return) so existing code that
  re-iterates is unaffected.  Test `coroutine_frame_freed_after_exhaustion` in
  `tests/expressions.rs` runs 1 000 loops to confirm no slot leak.

- **Coroutine `text_positions` save/restore across yield (debug builds)** (S27) —
  In debug builds, `coroutine_yield` now saves the suspended frame's
  `text_positions` entries and removes them from the live set; `coroutine_next`
  restores them on resume.  This prevents false double-free warnings and
  mask-missing-free bugs in `TextStore` ownership tracking when a generator is
  interleaved with text operations in the caller.  Test
  `coroutine_text_positions_save_restore` in `tests/expressions.rs`.

- **`WorkerStores` newtype for compile-time worker-store isolation** (S30) —
  `clone_for_worker` now returns `WorkerStores` instead of plain `Stores`.
  `WorkerStores` is `Send` but not `Sync` (via `PhantomData<*mut ()>`), giving a
  compile-time guarantee that worker-thread store snapshots are passed exclusively to
  `State::new_worker` and cannot be aliased across threads.  A `Deref<Target = Stores>`
  impl allows existing test code to inspect fields without change.

- **Debug generation counter for stale-DbRef detection in coroutines** (S28) —
  `Store` now carries a `generation: u32` field (debug builds only), incremented on
  every `claim`, `delete`, and `resize` call.  `coroutine_yield` snapshots the
  generation of every live, unlocked store; `coroutine_next` asserts that no snapshot
  store changed between yield and resume.  This catches the stale-DbRef hazard — where
  a struct record held by a suspended generator is freed or reallocated by the caller —
  as an early `debug_assert!` panic rather than silent corruption.  Test
  `coroutine_stale_store_guard` in `tests/expressions.rs`.

- **Parallel worker stores use `thread::scope` and skip `claims` clone** (S29) —
  `run_parallel_direct` in `src/parallel.rs` now uses `thread::scope` instead of
  `thread::spawn` + manual join loop, giving lifetime-bounded joining with no `Vec`
  of handles.  `Store::clone_locked_for_worker` skips cloning the `claims` `HashSet`
  (workers never call `validate()`) and `store.valid()` skips the claims check for
  locked stores, removing a spurious "Unknown record" panic that appeared in debug
  builds when workers accessed struct fields.

- **Store allocator uses free-bitmap; non-LIFO slot reuse now correct** (S29 P1-R4) —
  `database_named` previously always allocated from `self.max` and only reclaimed the
  top slot on `free_named`.  Native `OpFreeRef` legitimately frees slots in non-LIFO
  order, leaving freed slots permanently wasted and `max` growing without bound.  A
  `free_bits: Vec<u64>` bitmap was added to `Stores`; `set_free_bit`/`clear_free_bit`
  helpers update it on every free/alloc, and `find_free_slot` scans for the lowest set
  bit below `max`.  `clone_for_worker` propagates the bitmap to worker stores.
  Test `store_non_lifo_free_reclaims_slot` in `tests/threading.rs` verifies that a
  freed non-top slot is reused by the next `database()` call and `max` does not grow.

### Language features (continued)

- **Tuple destructuring in `match`** (T1.9) — `match` now dispatches on `Type::Tuple`
  subjects.  New `parse_tuple_match` in `src/parser/control.rs` parses comma- or
  semicolon-separated arms with wildcard (`_`), binding-variable, and literal patterns.
  Logical AND for multi-element conditions is built as `v_if(a, b, false)` (there is no
  `OpAnd`).  Tests: `tuple_match_wildcard`, `tuple_match_literal`, `tuple_match_binding`.

- **Homogeneous-type tuple coverage** (T1.10) — Three new tests confirm that same-element-type
  tuples work across common data sources: `tuple_homogeneous_text` (`(text, text)` pair
  from function parameters), `tuple_store_text_fields` (text fields extracted from two
  struct records), and `tuple_from_vector_elements` (`(integer, integer)` from indexed
  vector reads).  `tuple_struct_refs` (two `(Point, Point)` DbRefs) remains ignored
  pending T1.8 lifetime tracking for DbRef tuple slots.

- **Tuple type constraint diagnostics** (T1.11) — Two new compile-time guards:
  (a) `struct Foo { pair: (integer, integer) }` now emits "struct field cannot have a
  tuple type — tuples are stack-only values" at parse time (`parse_field` in
  `definitions.rs` detects `(` via `parse_type_full` before `fill_all` is reached);
  (b) `(a, b) += expr` now emits "compound assignment is not supported for tuple
  destructuring — use (a, b) = expr instead" (`parse_assign` in `expressions.rs` returns
  early in both passes, consuming the operator and RHS to keep the parser state clean).

### Coroutine safety documentation (continued)

- **Store-backed `Str` debug guard in `coroutine_yield`** (P2-R5 M10-a) — In
  `#[cfg(debug_assertions)]` builds on 64-bit targets, `coroutine_yield` now
  scans every tracked text local in the generator's `locals_bytes` and warns
  (`eprintln!("[P2-R5] ...")`) if the first 8 bytes (the `Str.ptr` field) fall
  within any live non-stack store allocation.  A store-backed Str in a suspended
  generator dangles if the consumer frees or reuses the backing record before
  the next resume.  The check is a heuristic (cannot cover full pointer
  provenance) but catches the common case of a recently-read text field local.
  No change to correct-program behaviour; the warning is diagnostic only.
  See `COROUTINE.md` CL-2b and `SAFE.md` § P2-R5.

- **Yielded `Str` ownership rule documented** (P2-R10) — `COROUTINE.md` CL-7 records
  the ownership invariant for `text` values produced by `yield`: the value is a
  zero-copy reference into the generator's frame (or `text_owned` buffer once CO1.3d
  lands) and is valid only for the current loop-body iteration.  Consumers that need
  to keep the text beyond one iteration must copy it (`stored = "{value}"`) or pass
  it to a function that calls `set_str`.  No runtime change; documentation only.

- **Text locals survive yield/resume in coroutines** (P2-R3 CO1.3d) — Text
  variables in generator functions are `String` objects (24 B) on the live stack.
  The bitwise copy of the locals region at yield is safe: `String` owns its heap
  buffer and no external code can free that buffer while the generator is suspended.
  The M8-b `debug_assert!` that fired for any text local at yield time has been
  removed; the S27 `text_positions` save/restore is preserved for correctness.
  Additionally, `coroutine_return` and `push_null_value` now push
  `Str::new(STRING_NULL)` (not 16 zero bytes) when an exhausted `iterator<text>`
  generator returns its null sentinel — the zero-pointer `Str` caused a panic in
  `append_text` via `slice::from_raw_parts(0, 0)`.  Test
  `coroutine_text_local_survives_yield` in `tests/expressions.rs` is now active and
  passing.

### Native store safety

- **Locked store cleared on free; `40-par-ref-return.loft` fixed** (S36) —
  `free_named` in `src/database/allocation.rs` now calls `unlock()` on the store
  before marking it free in the bitmap.  The parser auto-inserts
  `n_set_store_lock(stores, param, true)` at the start of functions with `const`
  reference parameters but does not emit the matching unlock before return.  When
  the store was freed while still locked, `find_free_slot` selected the freed slot
  for reuse and `database_named` called `init()` on a locked store, triggering:
  "Write to locked store at rec=1 fld=0".  The bug was invisible in the interpreter
  because `test_runner.rs` creates a fresh `Stores` per test function; in native
  mode all `test_*` functions share one `Stores`, so the leaked lock carried over
  from `test_par_struct_simple` into `test_par_struct_return_single_thread`.
  `40-par-ref-return.loft` now passes in `native_scripts` with 45/45.

### Interpreter fixes

- **`20-binary.loft` double-free fixed** (S34) — When `adjust_first_assignment_slot`
  cannot move a work variable downward (same-scope siblings block the move) and
  Option A fires — forcing the variable to the current TOS, aliasing it with the
  outer `rv` — the variable is now marked `skip_free` at that point.
  `generate_call` suppresses the `OpFreeRef` bytecode for any `skip_free` variable,
  preventing the "Double free store" panic caused by both `rv` and `_read_34` each
  trying to free the same database record at slot 820.  `skip_free` flags set during
  codegen are propagated back to `data.definitions[def_nr].variables` before
  `validate_slots` runs, which now skips slot-overlap pairs where either variable is
  `skip_free`.  The `binary` test (`tests/scripts/20-binary.loft`) no longer has
  `#[ignore]`; `"20-binary.loft"` removed from `ignored_scripts()` in `tests/wrap.rs`.

### WASM / native codegen fixes

- **Native codegen: Insert-return pattern fixed** (S35) — `output_set` in
  `src/generation/dispatch.rs` now detects `Value::Insert` as the RHS of an
  assignment and hoists all-but-last ops as standalone statements before the
  declaration line, emitting only the final expression as the assignment value.
  Previously the inner `Set` ops were emitted inline inside an expression context,
  producing malformed Rust (`let mut var_rv: DbRef = let mut var__read_34: DbRef = …`).
  The same function now also suppresses `OpFreeRef` for variables marked `skip_free`,
  matching the bytecode interpreter fix (S34) and preventing a double-free in the
  native binary.  `"20-binary.loft"` removed from `SCRIPTS_NATIVE_SKIP` in
  `tests/native.rs`; `native_binary_script` test passes without `#[ignore]`.

- **WASM random bridge wired; `rand_indices` shuffles via host bridge** (W1.19) —
  `codegen_runtime::n_rand` previously returned `i32::MIN` (null) when compiled
  without `feature = "random"`, making all `rand(lo, hi)` calls return null in WASM.
  It now delegates to `ops::rand_int`, which already had a WASM fallback calling
  `host_random_int` from `src/wasm.rs`.  A matching WASM `shuffle_ints` fallback
  (feature="wasm", not feature="random") was added to `src/ops.rs`, performing a
  Fisher-Yates shuffle via repeated `host_random_int(0, i)` calls; `n_rand_indices`
  in `codegen_runtime.rs` now enables the shuffle for both the PCG and WASM code
  paths.  `"21-random.loft"` removed from `WASM_SKIP` in `tests/wrap.rs`; the WASM
  compilation test now exercises `rand()`, `rand_seed()`, and `rand_indices()`.

- **WASM time bridge wired to `std::time::SystemTime`** (W1.20) — `host_time_now()`
  and `host_time_ticks()` in `src/wasm.rs` previously returned hard-coded `0`.
  They now call `std::time::SystemTime::now()` via the WASI clock interface (available
  in `wasm32-wasip2` through Rust's std).  `host_time_ticks()` delegates to
  `host_time_now()` (millisecond wall-clock); `n_ticks` computes elapsed microseconds
  as `(host_time_ticks() - start_time_ms) * 1000`, which is sufficient for benchmark
  timing.  `"22-time.loft"` removed from `WASM_SKIP` in `tests/wrap.rs`; the WASM
  compilation test now exercises `now()` and `ticks()` end-to-end.


- **WASM suite subprocess isolation; run-one.mjs helper** (W1.13) — Each test in
  `tests/wasm/suite.mjs` now runs in its own Node.js subprocess via `spawnSync` +
  `tests/wasm/run-one.mjs`.  Previously, a WASM crash (`RuntimeError: unreachable`
  or `memory access out of bounds`) in one test corrupted the shared module's linear
  memory, causing all subsequent tests in the same process to also fail.  `run-one.mjs`
  loads a fresh `pkg/loft.js` module and VirtFS default tree per invocation and writes
  the JSON result to stdout.  `suite.mjs` no longer imports `createHost` /
  `buildDefaultTree` / `withFiles`; the subprocess helper owns that setup.

- **`wasm_compile_and_run_smoke` converted to real integration test** (W1.9) — The
  hollow `#[ignore]` placeholder in `tests/wasm_entry.rs` has been replaced by an
  integration test that runs `node tests/wasm/bridge.test.mjs` as a subprocess.
  The test skips gracefully when the WASM package is not built or Node.js is absent,
  and fails with a clear message when the bridge tests report a non-zero exit code.

- **`13-file.loft` removed from `WASM_SKIP`** — File I/O operations (`OpDelete`,
  `OpMoveFile`, `OpMkdir`, `OpMkdirAll`) now route through `codegen_runtime::fs_*`
  functions that compile cleanly for the `wasm32-wasip2` target.  The wasm32-wasip2
  compilation test (`wasm_dir`) no longer skips `tests/docs/13-file.loft`; `#74`
  is fully resolved.


- **WASM file I/O wired to VirtFS host bridge** (W1.16) — All file operations
  (`read_text`, `write_text`, `read_bytes`, `write_bytes`, `seek`, `file_size`,
  `truncate`, `is_file`, `is_dir`, `list_dir`, `delete`, `move`, `mkdir`,
  `mkdir_all`) now call `globalThis.loftHost.*` via `js_sys::Reflect` under the
  `wasm` feature.  Helpers `assemble_write_data` and `dispatch_read_data` extracted
  from `state/io.rs` to share assembly logic between WASM and native paths and
  satisfy clippy `too_many_lines`.  `tests/wasm/bridge.test.mjs` gains three binary
  I/O tests (BigEndian write/read, seek + partial read, truncate); `doc/claude/ROADMAP.md`
  updated to mark W1.16 as done.

- **WASM skip for lock functions removed** (W1.17) — `n_get_store_lock` and
  `n_set_store_lock` are resolved from `loft::codegen_runtime` (listed in
  `CODEGEN_RUNTIME_FNS` in `generation/mod.rs`), so no `todo!()` stub is emitted.
  `18-locks.loft` removed from `WASM_SKIP`; the WASM compilation test now exercises
  `#lock` attribute syntax and `get_store_lock()` / `set_store_lock()`.

- **WASM skip for function references removed** (W1.15) — `output_call_ref` in
  `emit.rs` generates a `match` dispatch over all reachable definitions with a
  matching signature, implementing fn-ref calls (`f(args)` where `f: fn(T) -> R`)
  in native/WASM output.  `06-function.loft` removed from `WASM_SKIP`; the WASM
  compilation test now exercises function references, lambdas, and higher-order
  functions (`map`, `filter`, `reduce`).

### Native test harness fixes

- **`any`, `all`, `count_if` now work in native code generation; `47-predicates.loft` and `46-caveats.loft` unskipped** (N8a.4) —
  `predicate_loop_scaffold` in `src/parser/collections.rs` previously wrapped
  `[for_next, break_if_done]` in a `v_block`, which in native codegen became a
  Rust `{ ... }` block.  The loop variable (`any_elm`, `all_elm`, `cntif_elm`) was
  declared inside that block, making it invisible to the `short_circuit` or
  `count_step` expression that followed outside the block.  The fix inlines
  `for_next` and `break_if_done` directly in the loop body (the scaffold now returns
  a 4-tuple instead of 3), eliminating the nested block.  Both `47-predicates.loft`
  and `46-caveats.loft` (which uses `any`/`all` internally) removed from
  `SCRIPTS_NATIVE_SKIP`.

- **Native coroutine `yield from` delegation** (N8b.3) — `yield from sub_gen()`
  now works in native-compiled generators.  The sub-generator is stored as
  `Option<Box<dyn LoftCoroutine>>` directly in the outer struct, avoiding the
  `NATIVE_COROUTINES` `RefCell` that would cause a "RefCell already borrowed" panic
  when the outer `next_i64` tries to advance the inner generator.  The outer
  `next_i64` body is wrapped in a `loop {}` when yield-from segments are present;
  exhausted sub-generators set the next state and `continue` immediately.  Factory
  functions for sub-generators are called directly (not via `alloc_coroutine`) so
  sub-generators are never registered in the shared table.  CO1.4 test in
  `51-coroutines.loft` (`outer_with_from` producing 1+10+20+2 = 33) now passes.

- **Native coroutine state-machine code generation** (N8b.1, N8b.2) — Generator
  functions (`fn foo() -> iterator<integer>`) are now supported by the `--native`
  Rust backend.  Each generator is translated into a hand-written Rust state-machine
  struct (e.g. `NCountGen { state: u32, … }`) implementing the new `LoftCoroutine`
  trait (`fn next_i64(&mut self, stores: &mut Stores) -> i64`).  The coroutine body
  is split at `yield` nodes into match arms; a catch-all `_ =>` arm returns
  `COROUTINE_EXHAUSTED` (= `i32::MIN as i64`).  Three new pieces land in
  `src/codegen_runtime.rs`: the `LoftCoroutine` trait, a thread-local
  `NATIVE_COROUTINES` table (avoiding changes to `Stores`), `alloc_coroutine`,
  `coroutine_next_i64`, and `coroutine_is_exhausted`.  Call sites emit
  `loft::codegen_runtime::alloc_coroutine(foo(stores, args))` via a new
  `src/generation/coroutine.rs` module.  `OpCoroutineNext` and `OpCoroutineExhausted`
  are dispatched in `src/generation/dispatch.rs`.  `collect_calls` in
  `src/generation/mod.rs` now walks `Value::Yield` nodes so helper functions called
  from yield expressions are included in the reachable set.  `51-coroutines.loft`
  removed from `SCRIPTS_NATIVE_SKIP`; `native_scripts` passes all 4 generator tests.

- **`45-field-iter.loft` stale skip removed from native test harness** (N8a.5) —
  The `// A10` skip entry for `45-field-iter.loft` in `SCRIPTS_NATIVE_SKIP` was
  stale: the field-iteration native backend already worked correctly after the A10
  implementation.  The entry has been removed; `45-field-iter.loft` now runs in the
  `native_scripts` test alongside all other unblocked scripts.

- **Tuple types now supported in native code generation; `50-tuples.loft` unskipped** (N8a) —
  Three complementary fixes enable tuple types in the `--native` backend:
  (N8a.1) `rust_type(Type::Tuple)` now emits the correct Rust type `(T0, T1, …)`
  instead of `()`, and `default_native_value` returns `String` so tuple zero-values
  `(0, 0)` are built dynamically.
  (N8a.2) `Value::TupleGet` in `emit.rs` now uses the variable's declared name instead
  of its internal index number; `Value::TuplePut` emits the actual element assignment
  `var_x.i = …` rather than a stub.  `TuplePut` added to `is_void_value` in
  `pre_eval.rs` so the block emitter treats it as a statement, not a return expression.
  (N8a.3) Tuple-returning functions `make_pair`/`swap_pair` added to
  `tests/scripts/50-tuples.loft` (with LHS destructuring); the script removed from
  `SCRIPTS_NATIVE_SKIP`.  Both interpreter and native backends pass all tuple assertions.

- **Slot conflict in `20-binary.loft` fixed; removed from native skip list** (S32) —
  `adjust_first_assignment_slot` in `src/state/codegen.rs` now checks for same-scope
  sibling overlap (`has_sibling_overlap`) before moving a variable down to TOS, mirroring
  the existing `has_child_overlap` guard for child-scope variables.  This prevented `rv`
  and `_read_34` in `n_main` from being assigned the same slot range `[820, 832)` despite
  overlapping live intervals.  `20-binary.loft` removed from `SCRIPTS_NATIVE_SKIP`.

- **Generic instantiation confirmed working in native backend; `48-generics.loft` unskipped** (N8c) —
  Audit (N8c.1) showed that monomorphised generic functions already emit correct native
  code.  `48-generics.loft` removed from `SCRIPTS_NATIVE_SKIP`.

- **Optional feature dependencies now passed to standalone `rustc`** (S31) — The
  native test harness now calls `collect_extra_externs()`, which scans all `.rlib`
  files in the current test binary's `deps/` directory and passes each as
  `--extern crate_name=path`.  This unblocks scripts that use `rand`, `rand_seed`,
  or `rand_indices`: `tests/scripts/15-random.loft` and `tests/docs/21-random.loft`
  have been removed from the native skip lists.

- **Native rlib lookup now uses the current test binary's profile** (S33) — The
  previous `find_loft_rlib()` compared modification times across `release/` and
  `debug/` deps directories and could select the wrong profile's rlib (e.g. a
  newer no-features rlib from a `--no-default-features` CI step).  The function
  now uses `current_exe().parent()` — always the current test binary's own `deps/`
  directory — so the selected rlib always matches the features the test was compiled
  with.  `tests/docs/14-image.loft` has been removed from `NATIVE_SKIP`.

### Test coverage

- **`single` (f32) type fully covered** — New `tests/scripts/52-single.loft` covers
  all previously zero-coverage `single` operations: arithmetic (sub, mul, div, rem),
  all six comparison operators, NaN null semantics and propagation, null coalescing,
  positive/negative infinity (non-null), conversions (`as single` from integer/float/text;
  `single as` float/integer/long/text), format specifiers, and NaN-producing casts.
  The test is registered in `tests/wrap.rs` as `single_type`.

### Closure improvements

- **Spurious closure diagnostics suppressed** (A5.6d) — The "closure record '…' created"
  diagnostic is now `Level::Debug` (invisible in normal output and tests).  Captured outer
  variables are now marked as read at the call site via `var_usages`, eliminating false-positive
  "Variable X is never read" and "Dead assignment" warnings for validly captured variables.
  Tests `closure_capture_integer`, `closure_capture_after_change`, `closure_capture_multiple`,
  `closure_capture_text_integer_return`, and `closure_capture_text_return` no longer assert
  spurious warnings.

- **Closure capture coverage tests added** (A5.6e) — Four new tests in `tests/expressions.rs`
  verify closures across data-source scenarios: `closure_capture_struct_ref` (12-byte DbRef
  capture), `closure_capture_vector_elem` (vector element capture), and the existing
  `closure_capture_text_return` / `closure_capture_text_integer_return` tests cover text captures.

- **Work buffer cleared before each closure call** (A5.6f) — The hidden work-buffer `String`
  is now cleared (`v_set(wv, "")`) before each `OpCreateStack` injection at call sites.  Without
  this fix, calling a text-returning lambda inside a loop accumulated text from previous iterations
  (e.g. `"hello, world!"` became `"hello, world!hello, world!"` on the second call).  New test
  `closure_capture_text_loop` in `tests/expressions.rs` verifies the fix.

- **`fn`-ref conditional assignment no longer SIGSEGVs** (A5.6h) —
  `f = if flag { inc } else { dec }` caused a SIGSEGV at the `CallRef` opcode.
  Root cause: a fn-ref slot is 16 bytes (`[d_nr 4B][closure DbRef 12B]`), but
  each branch of an if-else expression generated only 4 bytes (the d_nr via
  `OpConstInt`), because `generate_block` (called for each branch) was setting
  `stack.position = to + size(Function) = to + 16` without emitting any instruction
  to push the 12-byte sentinel.  This phantom advance caused the codegen stack
  tracker to skip `OpNullRefSentinel` and left `CallRef` reading from the wrong
  stack position (the frame header, containing d_nr=0, which dispatched to
  `i_parse_errors()` and then SIGSEGVed in `dump_stack` on a garbage text pointer).
  Fix: `generate_block` now emits `OpNullRefSentinel` when the block result type is
  `Type::Function` and the block's content pushed fewer than 16 bytes.  A defensive
  `gen_fn_ref_value` helper in `generate_set` handles non-Block fn-ref values.
  Additionally, three native-codegen regressions introduced in A5.6g were resolved:
  (1) `visible_attr_count` (not `def.attributes.len()`) is now used in the candidate
  filter for closure-capturing lambdas; (2) the closure work-variable is injected at
  call sites for closure-capturing dispatch; (3) `Value::FnRef(d_nr, …)` is added to
  `collect_int_fn_refs` and emits `{d_nr}_u32` in native output so closure lambda
  functions appear in the reachable set and are compiled.  Test: `fn_ref_conditional_call`
  in `tests/issues.rs`; all 8 closure interpreter tests and the full native suite pass.

- **Definition-time capture semantics and multi-call closure injection** (A5.6g) —
  Closures now capture variable values at definition time (when the lambda is written),
  not at call time (when it is first invoked).  `emit_lambda_code` allocates and
  populates the closure record inside the `fn_ref_with_closure` block — the block is
  the `*code` assigned to the fn-ref variable, so it runs exactly once at definition
  time.  A `closure_vars` fallback was restored in `src/parser/control.rs` (both
  `try_fn_ref_call` and `parse_call` paths): when `last_closure_alloc` has already
  been consumed by a first call site, subsequent call sites to the same fn-ref variable
  look up the closure work variable via `self.closure_vars.get(&v_nr)` and inject it
  as the hidden `__closure` arg.  This fixes `closure_capture_struct_ref` and
  `closure_capture_vector_elem`, which each call the lambda twice (condition + format
  string).  Native codegen was also fixed: `OpVarFnRef`/`OpStoreClosure` declarations
  were removed from `default/02_images.loft` (they would have overflowed the 254-entry
  OPERATORS array); the `output_call_ref` dispatch in `src/generation/emit.rs` now
  compares total attribute count (including `__closure`) against total args (since
  the closure is injected explicitly at the call site, not by `fn_call_ref`); the
  `OpGetClosure` injection was removed.  The block result type was changed to a
  full-range integer to prevent native codegen from emitting a truncating `as u8`
  cast that corrupted the d_nr dispatch value.  All 8 closure tests pass (1 ignored
  for cross-scope closures, a known limitation in CAVEATS.md C1);
  `tests/docs/26-closures.loft` updated to reflect definition-time semantics.

### New features

- **Mutable closure capture works** (A5.6a) — `count += x` inside a lambda now
  compiles and executes correctly.  The `+=` operator on a captured integer variable
  routes through `call_to_set_op` → `OpSetInt`, bypassing the `generate_set`
  self-reference guard that previously caused a codegen panic.  Test `capture_detected`
  in `tests/parse_errors.rs` passes without `#[ignore]`.  Text capture remains
  blocked by two runtime bugs (see CAVEATS.md C1).

- **Lambda function type no longer includes text work variables** (A5.6a fix) —
  `parse_lambda` previously built the `Function(params, ret)` type from
  `data.attributes(d_nr)`, which also includes internal text work variables
  registered by `text_return()`.  This caused spurious "expects N argument(s),
  got M" errors when calling text-returning lambdas via function references.  The
  type is now built directly from the declared `arguments` list, which is always
  correct regardless of how many work variables are registered.

- **Closure capture works in debug builds** (A5.6) — The debug-mode store leak
  where closure record variables (`___clos_N`) were never freed has been fixed.
  `scopes.rs` now pre-registers block-result Reference variables at the enclosing
  outer scope so `get_free_vars` emits `OpFreeRef` at function exit.  A compile-time
  checker (`check_arg_ref_allocs`) panics in debug builds if any `Set(ref, Null)`
  initialisation is still nested inside a call argument, catching this class of
  scope-registration bug early.  Tests `closure_capture_integer`,
  `closure_capture_multiple`, and `closure_capture_after_change` all pass without
  `#[ignore]` in both debug and release builds.  Text capture and mutable capture
  remain deferred (A5.6 in ROADMAP.md).

- **Mutable closure captures write back to outer scope after each call** (A5.6c)
  — Void-return lambda calls now emit a write-back sequence after the `CallRef`
  instruction: for each field of the closure record, `OpGetInt` (or the
  field-type equivalent) reads the updated value back and stores it to the
  corresponding outer-scope variable.  Two root-cause bugs were fixed along the
  way: (1) `closure_vars.insert` was executing before the RHS lambda was parsed
  (because the insert check ran before `parse_assign_op`, which is where the
  lambda tokens are consumed); (2) the write-back used `Value::Block` (which
  creates a new scope), causing `scopes.rs` to emit `OpFreeRef` for the closure
  variable at the inner scope exit — leaving a dangling DbRef for the second
  call.  The fix uses `Value::Insert` instead, keeping the closure record alive
  across all calls in the outer scope.
  Test `p1_1_lambda_void_body` in `tests/issues.rs` passes without `#[ignore]`.

- **Text capture via `CallRef` no longer produces garbage DbRef** (A5.6b.1) —
  In `generate_call_ref`, the `__closure` argument (a `DbRef`) was being pushed
  onto the wrong stack frame: it was placed at the stack position of `x`
  (the first explicit argument), not at the position expected by the lambda
  body.  Two separate code paths were fixed: (1) for zero-param fn-refs the
  fast path now injects the closure arg; (2) `text_return` no longer adds
  captured RefVar(Text) variables as spurious extra args to the lambda's
  parameter type, which previously caused arity-mismatch failures.

- **`generate_call_ref` pre-allocates text work buffers for closures** (A5.6b.2)
  — A spurious `debug_assert!(work_vars.is_empty())` in `generate_call_ref`
  fired when a capturing lambda returned text, because the closure record
  contains a RefVar work buffer.  The assert has been removed; the existing
  logic already handles non-empty `work_vars` correctly.  Test
  `closure_capture_text_integer_return` passes without `#[ignore]`.

- **`yield` inside `par(...)` body now produces a compile-time error** (P2-R6
  M11-a) — The parser sets an `in_par_body` flag while parsing the body block
  of a `for … par(…)` loop.  When `yield` is encountered with `in_par_body`
  true, an Error diagnostic is emitted: "yield is not allowed inside a
  par(...) parallel body".  The yield expression is still consumed (to keep the
  lexer in sync) but no coroutine IR is generated, so scope analysis does not
  see orphaned reference variables.  The `in_par_body` flag is saved and
  restored for nested par() bodies.  Test
  `p2_r6_yield_inside_par_body_rejected` in `tests/issues.rs` passes without
  `#[ignore]`.  The existing runtime out-of-bounds guard (S23 / M11-b) in
  `coroutine_next` remains as defence-in-depth.

- **`yield from` slot-assignment regression fixed** (CO1.4-fix) — `yield from
  inner()` inside a coroutine with local variables before the delegation now
  produces correct results.  The two-zone slot redesign (S17/S18) already
  eliminated the overlap between the `__yf_sub` handle and inner loop
  temporaries; no additional IR restructuring was required.  Test
  `coroutine_yield_from` passes without `#[ignore]`.

- **`stack_trace()` works in parallel workers** (S21, fix #92) — Calling
  `stack_trace()` inside a `par(...)` loop body or any `run_parallel_*` worker
  now returns the actual call frames instead of an empty vector.  Two changes
  enable this: (1) `WorkerProgram` now carries `stack_trace_lib_nr` so the
  resolved index of `n_stack_trace` travels from the main state into each
  worker state; (2) `static_call` takes the call-stack snapshot when
  `stack_trace_lib_nr` matches even when `data_ptr` is null, using a
  `"<worker>"` placeholder for frames that lack `Data` context.  Worker states
  created via both `n_parallel_for_int` (bytecode path) and the direct
  `run_parallel_*` Rust API now report correct frame counts.  Test
  `parallel_stack_trace_non_empty` passes without `#[ignore]`.

- **`init(expr)` circular dependency detection** (S20) — Struct fields that
  form a mutual initialisation cycle (`a: integer init($.b), b: integer init($.a)`)
  now produce a compile error naming the cycle (e.g.
  `circular init dependency: a -> b -> a`).  A DFS cycle check runs after all
  struct fields are parsed; `$.field` reads inside `init(...)` are tracked by
  the parser and checked for cycles per root field.

- **`stack_trace()` vector fields zeroed + call-site line numbers** (S19) —
  `stack_trace()` now returns correct call-site line numbers (`StackFrame.line`)
  for every frame.  Three fixes: `n_stack_trace` explicitly zeroes the
  `arguments` and `variables` fields of each `StackFrame` element so that
  reused store blocks don't leave garbage data; `execute_log_steps` now
  pushes the same synthetic entry `CallFrame` as `execute_argv` (Fix #88
  parity); `fn_call` now resolves call-site lines with a BTreeMap backward
  range search, recovering the correct source line even when `code_pos` has
  advanced past the `line_numbers` entry.
  Tests `stack_trace_returns_frames`, `stack_trace_function_names`, and
  `call_frame_has_line` all pass without `#[ignore]`.

- **Tuple text elements** (T1.8b) — Functions returning `(integer, text)` (or any
  tuple containing a `text` element) now compile and execute correctly.  Text elements
  are stored as `Str` (16B borrowed reference) in tuple slots via the new `OpPutText`
  opcode, consistent with loft's text-argument convention.  Four codegen sites were
  updated: null-init now emits `OpConvTextFromNull`; slot stores use `OpPutText` instead
  of `OpAppendText`; tuple element reads use `OpArgText` instead of `OpVarText`.

- **Tuple function return + destructuring** (T1.8a) — Functions declared `-> (T1, T2)`
  now work end-to-end: the return value is materialised on the caller's stack, element
  access (`pair(3,7).0`) compiles and executes correctly, and LHS tuple destructuring
  (`(a, b) = pair(5)`) is fully supported.  Two fixes enabled this: the two-zone slot
  allocator now emits a no-op for zone-1 Tuple null-inits (space pre-reserved by
  `OpReserveFrame`) and a per-element push for zone-2 Tuple null-inits; the parser
  now marks destructuring targets as defined and types them on both passes so
  `known_var_or_type` does not fire a false "Unknown variable" on the second pass.

- **`size(t)` character count** — `size("héllo")` returns 5 (Unicode code points),
  complementing `len()` which returns byte length. Backed by a new `OpSizeText` opcode.

- **`FileResult` enum** — Filesystem-mutating operations (`delete`, `move`, `mkdir`,
  `mkdir_all`, `set_file_size`) now return a `FileResult` enum (`Ok`, `NotFound`,
  `PermissionDenied`, `IsDirectory`, `NotDirectory`, `Other`) instead of `boolean`.
  Use `.ok()` for a simple success check.

- **Vector aggregates** — `sum_of`, `min_of`, `max_of` for `vector<integer>`, implemented
  as `reduce` wrappers with internal helper functions. Predicate aggregates `any(vec, pred)`,
  `all(vec, pred)`, `count_if(vec, pred)` with short-circuit evaluation and lambda support.

- **Nested match patterns** — Field positions in struct match arms support sub-patterns:
  `Order { status: Paid, amount } => charge(amount)`. Supports enum variants, scalar
  literals, wildcards, and or-patterns (`Paid | Refunded`).

- **Field iteration** — `for f in s#fields` iterates over a struct's primitive fields
  at compile time. Each iteration provides `f.name` (field name) and `f.value` (a
  `FieldValue` enum wrapping the typed value). Works for uniform and mixed-type structs.

- **Generic functions** — `fn name<T>(x: T) -> T { ... }` declares a generic function.
  T must appear in the first parameter (directly or as `vector<T>`). The compiler creates
  specialised copies per concrete type at each call site (P5.2). Disallowed operations on
  T (arithmetic, field access, methods) produce clear compile-time errors (P5.3).
  Documentation test and LOFT.md section added (P5.4).

- **Shadow call-frame vector** (TR1.1) — The interpreter now tracks a shadow call stack
  with function identity and argument layout on each call/return.  The OpCall bytecode
  format encodes the definition number and argument size.  Foundation for `stack_trace()`.

- **Stack trace types** (TR1.2) — `ArgValue`, `ArgInfo`, `VarInfo`, and `StackFrame` types
  declared in `default/04_stacktrace.loft`.  These will be materialised by `stack_trace()`
  in TR1.3.

- **Closure capture analysis** (A5.1) — Lambdas that reference variables from an enclosing
  scope now produce a clear error: "lambda captures variable 'name' — closure capture is
  not yet supported, pass it as a parameter".  Previously this silently created a broken
  local variable.

- **Closure record layout** (A5.2) — For each capturing lambda, the parser now synthesizes
  an anonymous struct type (`__closure_N`) whose fields match the captured variables'
  names and types.  The record def_nr is stored on the lambda's Definition.

- **`stack_trace()` function** (TR1.3) — Returns `vector<StackFrame>` with function name,
  file, and call-site line for each active call frame.  Arguments/variables vectors are
  left empty (full population is future work).  Implemented as a native function with
  call-stack snapshot bridging State to Stores.

- **Call-site line numbers** (TR1.4) — `CallFrame` now stores the source line directly,
  resolved from `line_numbers` at call time.  Eliminates the per-frame HashMap lookup
  during stack trace materialisation.

- **Coroutine types** (CO1.1) — `CoroutineStatus` enum (Created, Suspended, Running,
  Exhausted) declared in `default/05_coroutine.loft`.  `CoroutineFrame` struct and
  coroutine storage infrastructure added to State.

- **`init(expr)` field initialiser** (L7) — `init(expr)` field modifier evaluates once
  at record creation (with `$` access), stores the result, and allows mutation afterward.
  Complements `computed(expr)` (read-only, recomputed on every access).

- **Tuple type system** (T1.1) — `Type::Tuple(Vec<Type>)` variant added to the type
  enum.  Helper functions `element_size`, `element_offsets`, and `owned_elements`
  provide reusable layout calculations for tuples and closure records.

- **Tuple parser** (T1.2) — Tuple type notation `(T1, T2)` is recognized in all type
  positions.  Tuple literals `(expr, expr)`, element access `t.0`, and LHS
  destructuring `(a, b) = expr` are parsed.  `Value::Tuple` IR variant added.

- **Tuple scope analysis** (T1.3) — Scope analysis recognizes `Type::Tuple` variables
  and identifies owned elements for reverse-order cleanup on scope exit.

- **Closure capture diagnostic** (A5.3) — The closure capture error message now
  indicates that closure body reads (A5.4) are the remaining blocker.  The closure
  record struct from A5.2 is still synthesized.

- **Tuple bytecode codegen** (T1.4) — `Value::TupleGet(var, idx)` IR variant for
  element reads.  Codegen emits `OpVar*` at the element's stack offset.  Tuple
  literals, element access, type annotations, and parameters now work end-to-end.

- **Closure body reads** (A5.4) — Captured variable reads inside lambdas now redirect
  to field loads from a hidden `__closure` parameter backed by the A5.2 closure record
  struct.  Read-only captures work; mutable captures are pending.

- **Coroutine opcodes** (CO1.2) — `OpCoroutineCreate` and `OpCoroutineNext` opcodes
  implemented.  Create copies arguments into a `CoroutineFrame` without entering the
  body.  Next restores the frame's stack and resumes execution.

- **`OpCoroutineReturn`** (CO1.3a) — Opcode to exhaust a running coroutine: clears
  frame state, pushes null, returns to consumer.

- **`OpCoroutineYield`** (CO1.3b) — Opcode to suspend a generator: serialises the
  live stack to `stack_bytes`, saves call frames, slides the yielded value to the
  frame base, and returns to the consumer.  Integer-only path; text serialisation
  pending (CO1.3d).

- **`yield` keyword** (CO1.3c) — Parser recognises `yield expr` in generator
  functions (return type `iterator<T>`).  Codegen emits `OpCoroutineCreate` for
  generator calls, `OpCoroutineYield` for yield statements, and `OpCoroutineReturn`
  at generator body end.  `iterator<T>` single-parameter syntax now accepted.

- **Generator type fixes** (CO1.3c-fix) — Generator body return-type check
  suppressed.  `next(gen)` and `exhausted(gen)` wired as special dispatch calls.
  Coroutine iterators no longer materialised into vectors.  `Type::Iterator` sized
  as DbRef.  `coroutine_create_basic` and `coroutine_next_sequence` tests pass.

- **Closure lifetime** (A5.5) — Closure record work variable is already freed by
  existing `OpFreeRef` scope-exit logic.  No new code needed.

- **`exhausted()` stdlib** (CO1.6) — `OpCoroutineExhausted` opcode and `pub fn
  exhausted(gen) -> boolean` declared in `05_coroutine.loft`.

- **`next()` stack tracking fix** (CO1.6a) — `OpCoroutineNext` and
  `OpCoroutineExhausted` now bypass the operator codegen path.  Stack position
  manually adjusted for DbRef consumption and value push.

- **Null sentinel on exhaustion** (CO1.6c) — `coroutine_next` pushes `i32::MIN`
  (integer null) when the generator is exhausted, not uninitialized bytes.

- **For-loop over generators** (CO1.5a+b) — `for n in gen() { ... }` works.
  The iterator protocol detects generator calls, stores the DbRef in a `__gen`
  variable, and uses `OpCoroutineNext` as the advance step with null-check
  termination.  All 6 coroutine tests pass.

- **`e#remove` rejection** (CO1.5c) — `#remove` on a generator for-loop variable
  produces a compile error (existing guard; coroutine loops never call `set_loop`).

- **Nested yield verified** (CO1.3e) — Generator calling a helper function between
  yields correctly saves/restores call frames across yield/resume.

- **`yield from` parsing** (CO1.4) — `yield from sub_gen` desugars to a loop that
  advances the sub-generator and forwards each value via `yield`.  Test `#[ignore]`
  pending slot-assignment fix.

- **Closure call-site allocation** (A5.3) — Capturing lambdas now allocate the
  closure record on the heap, populate fields from captured variables, and inject
  the record as a hidden argument at call sites.  Multi-capture variable redirect
  fixed (pre-has_var check).  Blocked by slot-assignment issue at codegen time.

- **Tuple element assignment** (T1.4) — `t.0 = expr` now works via `Value::TuplePut`
  IR variant.  Parser detects `TupleGet` on the LHS of `=` and routes through
  element-write codegen.

- **Reference-tuple parameters** (T1.5) — A `RefVar(Tuple)` parameter can now have
  its elements read and written using `.0`, `.1` … notation.  Codegen emits
  `OpVarRef` plus element `OpGet*`/`OpSet*` at the correct byte offset.

- **Unused-mutation guard for tuple refs** (T1.6) — Passing a tuple by reference to
  a function that never writes its elements now produces a WARNING (not an error),
  consistent with the existing scalar-ref mutation guard.

- **`integer not null` annotation** (T1.7) — `Type::Integer` gains a third boolean
  field (`not_null`).  The parser accepts the `not null` suffix on integer type names.
  Assigning a nullable value to a `not null` element in a tuple literal is a
  compile-time error.

- **Text parameter survives coroutine yield** (CO1.3d) — Two root causes for SIGSEGV
  in generators that hold a `text` parameter across `yield`:
  (1) `coroutine_create` now appends the 4-byte return-address slot to `stack_bytes`
  so that `get_var` offsets match the codegen-time layout on every resume;
  (2) `Value::Yield` codegen now decrements `stack.position` by the yielded value's
  size after emitting `OpCoroutineYield`, so subsequent variable accesses in the same
  generator use correct offsets on the second and later resumes.

### Bug fixes (continued)

- **Fix #87** — `static_call` no longer snapshots the call stack on every native
  function call; the snapshot now only runs when `n_stack_trace` is dispatched.

- **Fix #88** — `stack_trace()` now includes the entry function (main/test) as the
  outermost frame.

- **Null-coalescing fix** — `f() ?? default` no longer calls `f()` twice; non-trivial
  LHS expressions are materialised into a temporary before the null check.

- **Format specifier warnings** — Compile-time warnings for format specifiers that
  have no effect: hex/binary/octal on text or boolean, zero-padding on text.

- **Slot bug S17: text below TOS in nested scopes** — The two-zone slot redesign
  (0.8.3) fixed the `[generate_set]` panic for text variables pre-assigned below
  the actual TOS in deeply nested scopes.  `text_below_tos_nested_loops` passes;
  `#[ignore]` removed.  CAVEATS.md C4 closed.

- **Slot bug S18: sequential file blocks conflict** — Same two-zone redesign fixed
  the `validate_slots` panic from ref-variable slot override in sequential file
  blocks.  `sequential_file_blocks_read_conflict` passes; `#[ignore]` removed.
  CAVEATS.md C5 closed.

- **`while` loop** (L10) — `while cond { body }` is now a first-class keyword.
  Desugars to a loop with an `if !cond { break }` guard at the top, identical to
  the `for + break` workaround but with familiar syntax.  C11 closed.

### Language changes

- **Format specifier mismatches are now errors** (L9) — Using a radix specifier
  (`:x`, `:b`, `:o`) on a `text` or `boolean` value, or zero-padding (`:05`) on a
  `text` value, is now a compile error rather than a silent no-op.  C14 closed.

### Bug fixes (continued)

- **S15: match arm binding type reuse** — When multiple struct-enum match arms bind the
  same field name with different types, each arm now gets its own variable. Previously
  the second arm reused the first arm's type, causing garbled values.

- **S14: stdlib struct-enum field positions** — Struct-enum types defined in the default
  library (`FieldValue`, etc.) no longer panic with "Fld N is outside of record". Fixed
  two issues in `typedef.rs`: loop range for `fill_all()` and lazy byte-type registration.

---

## [0.8.3] — 2026-03-27

### New features

- **WASM output capture** (W1.2) — `output_push` / `output_take` helpers buffer `println`
  output in a thread-local string.  Used by `compile_and_run()` to collect program output
  without touching the filesystem.

- **WASM `compile_and_run()` entry point** (W1.9) — A `compile_and_run(files_json) -> String`
  function accepts a JSON array of `{name, content}` objects, runs the loft pipeline entirely
  in memory, and returns `{output, diagnostics, success}` JSON.  Exported via `wasm_bindgen`
  when built with `--features wasm`.  Default standard library files are embedded with
  `include_str!()`.  A virtual filesystem (`VIRT_FS`) routes `use` imports to the supplied
  in-memory files.

- **`#native "symbol"` annotation** (A7.1) — Functions declared in loft can carry a
  `#native "symbol_name"` annotation.  When the compiler resolves such a function it emits
  an `OpStaticCall` pointing to `symbol_name` in the native registry instead of the loft
  function name.  This decouples the loft identifier from the Rust symbol.

- **Native extension loader** (A7.2) — The `native-extensions` Cargo feature enables
  loading cdylib shared libraries at runtime via `libloading`.  `extensions::load_all()`
  is called between byte-code generation and execution; each library must export a
  C-ABI `loft_register_v1(*mut LoftPluginCtx)` entry point.

- **`LoftPluginCtx` public ABI** (A7.3) — `LoftPluginCtx` is a stable `repr(C)` struct
  published from `loft::extensions` and mirrored in the standalone `loft-plugin-api` crate.
  Plugin crates call `ctx.register_fn(name, fn_ptr)` once per exported function.

- **Format-string buffer pre-allocation** (O7) — The native/WASM code generator now emits
  `String::with_capacity(N × 8)` instead of `"".to_string()` at the start of format strings
  with ≥ 2 segments.  This avoids repeated `String` reallocations during format-string
  assembly, reducing the wasm/native performance gap on string-heavy workloads.

- **VirtFS JavaScript class** (W1.10) — `tests/wasm/virt-fs.mjs` provides a full in-memory
  virtual filesystem for WASM Node.js tests.  Features: tree-based JSON representation
  (`$type`/`$content` conventions), base64 binary support, path normalisation (`.`/`..`/`//`),
  `snapshot()`/`restore()` for test isolation, binary cursors (`seek`/`readBytes`/`writeBytes`),
  `toJSON()`/`fromJSON()` serialisation, and a minimal test harness (`harness.mjs`).
  13 unit tests in `virt-fs.test.mjs` cover all operations.  Runs via
  `node tests/wasm/virt-fs.test.mjs` when Node.js is available.

- **WASM test suite runner** (W1.13) — `tests/wasm/suite.mjs` discovers all loft programs
  in `tests/scripts/` and `tests/docs/`, runs each through the WASM module with a
  pre-populated VirtFS, and compares output against the native `cargo run` interpreter.
  Skips non-deterministic tests (time, unseeded random, images); verifies WASM success only
  for those.  Run via `node tests/wasm/suite.mjs` after building with `wasm-pack`.
  This is the main confidence gate for the WASM port.

- **LayeredFS class** (W1.12) — `tests/wasm/layered-fs.mjs` implements a two-layer virtual
  filesystem: an immutable base tree (bundled examples/docs/stdlib) plus a mutable delta
  overlay (user edits, persisted to localStorage).  Reads check delta first then fall through
  to base; writes always go to delta, leaving the base untouched.  Supports
  `getDelta()`/`setDelta()`/`saveDelta()`/`resetToBase()`/`isModified()`/`isDeleted()`.
  `ide/scripts/build-base-fs.js` reads `tests/docs/*.loft`, `doc/*.html`, and
  `default/*.loft` to emit `ide/assets/base-fs.json`.  20 unit tests in
  `layered-fs.test.mjs` cover all operations including delta serialisation and snapshot
  isolation.

- **loftHost factory** (W1.11) — `tests/wasm/host.mjs` exports `createHost(tree, options)`
  which wires a `VirtFS` instance to the full `loftHost` bridge API.  Uses a deterministic
  xoshiro128** PRNG for reproducible `rand()` / `rand_seed()` behaviour in tests.  Supports
  configurable `fakeTime`, `fakeTicks`, `env`, and `args` overrides.  Comes with:
  `bridge.test.mjs` (7 WASM integration tests; skips gracefully when `pkg/` not built),
  `file-io.test.mjs` (14 host-level edge-case tests, no WASM required),
  `random.test.mjs` (host PRNG tests + optional WASM-level determinism tests),
  and three fixtures in `tests/wasm/fixtures/`.

---

## [0.8.2] — 2026-03-24

### New features

- **Lambda expressions** — Write inline functions with `fn(x: integer) -> integer { x * 2 }`
  or the short form `|x| { x * 2 }`. Parameter and return types are inferred when the
  context makes them clear (e.g. inside `map`, `filter`, `reduce`). Lambdas cannot capture
  variables from the surrounding scope yet — pass needed values as arguments.

- **Named arguments and defaults** — Functions can declare default values
  (`fn connect(host: text, port: integer = 80, tls: boolean = true)`). Callers can skip
  middle parameters by name: `connect("localhost", tls: false)`.

- **Native compilation** — `loft --native file.loft` compiles your program to a native
  binary via `rustc` and runs it. `loft --native-emit out.rs` saves the generated Rust
  source. `loft --native-wasm out.wasm` compiles to WebAssembly.

- **JSON support** — Serialise any struct to JSON with `"{value:j}"`. Parse JSON into a
  struct with `Type.parse(json_text)` or into an array with `vector<T>.parse(json_text)`.
  Check for parse errors with `value#errors`.

- **Computed fields** — Struct fields marked `computed(expr)` are recalculated on every
  read and take no storage: `area: float computed(PI * $.r * $.r)`.

- **Field constraints** — Struct fields can declare runtime validation:
  `lo: integer assert($.lo <= $.hi)`. Constraints fire on every field write.

- **Parallel workers now support text and enum returns** — `par(...)` workers can return
  `text` and inline enum values in addition to the existing `integer`, `long`, `float`,
  and `boolean`. Workers can also receive extra context arguments beyond the loop element.

### Language changes

- **Function references drop the `fn` prefix** — Write `apply(double, 7)` instead of
  `apply(fn double, 7)`. Using `fn name` as a value is now a compile error.

- **Short-form lambdas infer types** — `|x| { x * 2 }` infers parameter and return
  types from the call site. Use the long form `fn(x: integer) -> integer { ... }` when
  you need explicit types.

- **Private by default** — Definitions without `pub` are no longer visible to `use`
  imports from other files.

### Better error messages

- Using `string` as a type now suggests `text` instead of a generic error.
- Match exhaustiveness errors now point at the `match` keyword, not the closing brace.
- Six common errors now include fix suggestions (e.g. "use a new variable name or
  cast with 'as'" for type-change errors).
- Three errors that previously stopped all parsing now let the compiler continue and
  report additional issues.
- Several places that crashed the compiler on unusual input now produce a proper error.

### Bug fixes

- `c + d` where both are characters no longer crashes. The result is text concatenation.
- PNG image loading now reports correct `width` and `height` values.
- Passing an empty vector `[]` directly as a function argument no longer crashes.
- `v += other_vec` on vectors containing text fields no longer corrupts the original.
- `&vector` parameters correctly propagate appends back to the caller.
- Vector slices assigned to a variable (`s = v[1..3]`) are now independent copies.
- `map`, `filter`, and `reduce` no longer cause internal slot conflicts.

---

## [0.8.0] — 2026-03-17

### New features

- **Match expressions** — Pattern match on enums, structs, and scalar values:
  ```loft
  match shape {
      Circle { r } => PI * pow(r, 2.0),
      Rect { w, h } => w * h,
  }
  ```
  The compiler checks that all variants are handled. Supports or-patterns
  (`North | South =>`), guard clauses (`if r > 0.0`), range patterns (`1..=9`),
  null patterns, character patterns, and block bodies.

- **Code formatter** — `loft --format file.loft` formats a file in-place.
  `loft --format-check file.loft` exits with an error if the file is not formatted.

- **Wildcard and selective imports** — `use mylib::*` imports everything;
  `use mylib::Point, add` imports only specific names. Local definitions take priority
  over imports.

- **Callable function references** — Store a function in a variable and call it:
  `f = fn double; f(5)`. Function-typed parameters also work.

- **`map`, `filter`, `reduce`** — Higher-order collection functions that accept
  function references: `map(numbers, fn double)`.

- **Test runner improvements** — `loft --tests file.loft::test_name` runs a single test.
  `loft --tests 'file.loft::{a,b}'` runs multiple. `loft --tests --native` compiles
  tests to native code first.

- **`now()` and `ticks()`** — `now()` returns milliseconds since the Unix epoch.
  `ticks()` returns microseconds since program start (monotonic timer).

- **`mkdir(path)` and `mkdir_all(path)`** — Create directories from loft code.

- **`vector.clear()`** — Remove all elements from a vector.

- **External library packages** — `use mylib;` can now resolve packaged library
  directories with a `loft.toml` manifest file.

### Diagnostics

- Warning for division or modulo by constant zero.
- Warning for unused loop variables (suppress with `_` prefix: `for _i in ...`).
- Warning for unreachable code after `return`, `break`, or `continue`.
- Warning for redundant null checks on `not null` fields.
- Warning when not all code paths return a value in a `not null` function.

### Bug fixes

- `x << 0` and `x >> 0` now correctly return `x` instead of null.
- `NaN != x` now returns `true` (was incorrectly `false`).
- `??` (null coalescing) on float values works correctly.
- Using `if` as a value expression without `else` is now a compile error instead of
  silently producing null.
- Assigning `null` to a struct field no longer causes a runtime crash.
- Functions with multiple owned struct variables no longer crash on cleanup.
- `sorted[key] = null` and `hash[key] = null` removal works again (was broken by a
  null-handling fix).
- `v += other_vec` on vectors with text fields no longer corrupts data.
- `index<T>` fields inside structs can now be copied and reassigned.
- Sorted filtered loop-remove, index key-null removal, and index loop-remove all fixed.
- `??` null coalescing, non-zero exit on errors, reverse iteration on `sorted<T>`,
  CLI args in `fn main`, format specifier sign order, XOR/OR/AND with null values,
  and `for c in enum_vector` infinite loop — all fixed.

---

## [0.1.0] — 2026-03-15

First release.

### Language

- **Static types with inference** — Types are checked at compile time. No annotations
  needed; the type is inferred from the first assignment.
- **Null safety** — Every value is nullable unless declared `not null`. Null propagates
  through arithmetic. Use `?? default` to provide a fallback value.
- **Primitive types** — `boolean`, `integer`, `long`, `float`, `single`, `character`, `text`.
- **Structs** — Named records with fields: `Point { x: 1.0, y: 2.0 }`.
- **Enums** — Plain enums (named values) and struct-enums (variants with different fields
  and per-variant method dispatch).
- **Control flow** — `if`/`else`, `for`/`in`, `break`, `continue`, `return`.
- **For-loop extras** — Inline filter (`for x in v if x > 0`), loop attributes
  (`x#first`, `x#count`, `x#index`), in-loop removal (`v#remove`).
- **Vector comprehensions** — `[for x in v { expr }]`.
- **String interpolation** — `"Hello {name}, score: {score:.2}"` with format specifiers.
- **Parallel execution** — `for a in items par(b=worker(a), 4) { ... }` runs work across
  CPU cores.
- **Collections** — `vector<T>` (dynamic array), `sorted<T>` (ordered tree),
  `index<T>` (multi-key tree), `hash<T>` (hash table).
- **File I/O** — Read, write, seek, directory listing, PNG image support.
- **Logging** — `log_info`, `log_warn`, `log_error` with source location and rate limiting.
- **Libraries** — `use mylib;` imports from `.loft` files.

---

[0.8.3]: https://github.com/loft-lang/loft/compare/v0.8.2...v0.8.3
[0.8.2]: https://github.com/loft-lang/loft/compare/v0.8.0...v0.8.2
[0.8.0]: https://github.com/loft-lang/loft/compare/v0.1.0...v0.8.0
[0.1.0]: https://github.com/loft-lang/loft/releases/tag/v0.1.0
