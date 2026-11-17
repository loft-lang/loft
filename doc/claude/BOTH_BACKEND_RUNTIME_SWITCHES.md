<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Both-backend runtime switches

Which switch turns off a RUNTIME policy that the interpreter and the compiled backends share —
inside the store, the keyed collections and the free tree — and what checks it.  The lowering
rewrites, made at parse time or in the scope pass, are in
[BOTH_BACKEND_SWITCHES.md](BOTH_BACKEND_SWITCHES.md), which also lists the falsifiers every
entry here uses; a wrong answer on `--native` only belongs to
[NATIVE_SWITCHES.md](NATIVE_SWITCHES.md).  Because both backends share the policy, agreement
between them proves nothing: hand-computed cell values and the switch A/B are the evidence.

## Runtime: repeat fills and self-appends

**`LOFT_NO_BLOCK_REPEAT=1`** (runtime, BOTH backends) makes `[x; n]` fill one element at a
time again — a `copy_block` and a `copy_claims` call each — where with it off
`Stores::fill_from_template` doubles block copies and walks claims only for a heap-owning
template (`lock_curved` −14 %); first bisect step for a wrong element out of a repeat literal or
a constant comprehension.  **`LOFT_NO_FIELD_FILL=1`** (parse time, BOTH backends) keeps a
constant comprehension in a struct FIELD (`Lay { best: [for _ in 0..n { 2.0 }] }`) on its
per-element push loop — with it off it takes the repeat-literal lowering a local's already has,
when the field is a plain vector (`is_plain_vector`; a keyed collection stays on the loop, since
n appends are not n inserts).  **`LOFT_NO_SELF_APPEND_BLOCK=1`** (runtime, BOTH backends) makes `v += v` copy through a
byte snapshot taken before the growth again — with it off, a self-append is one block copy
inside the grown record, its source re-read from the field slot after the growth (which is
also what fixed the heap-owning case: the claims walk read the source through a number
captured before the growth relocated it — a freed block).  The doubling fill a canvas is
built with is this shape run to a ladder; `render_marks` −8 %.  First bisect step for a
wrong element out of a self-append.
**`LOFT_NO_EXACT_COPY_CLAIM=1`** (runtime, BOTH backends) gives a whole vector appended
into an EMPTY destination (`Rec { xs: v }`, `r.ys += r.xs`) `vector_append`'s eleven-element
first claim again — with it off, `Stores::vector_add` claims the source's length, as a deep
copy does, so a short vector in a fresh field is not a flat block (a rebuilt store reaches
the from-scratch size, @PLN123; a builder written in its element keeps the size the copy
had).  First bisect step for a wrong element or a size change out of such an append;
`store_rebuild_b1` / `store_compact_b3` are the falsifiers.
**`LOFT_NO_VADD_FITS=1`** (runtime, BOTH backends) sends every whole-vector append
(`v += w`, `Rec { xs: v }`, a field copy) through `Stores::vector_add`'s general path again —
with it off, an append of no-heap elements into a destination record that already has room,
distinct from the source, is one block copy and one length write.  First bisect step for a
wrong element or length out of such an append.
**`LOFT_NO_SLICE_BLOCK=1`** (runtime, BOTH backends) makes `Stores::vector_slice` — the
`OpSliceVector` a slice append, a slice bind and `(R-ByteCopy)` lower to — snapshot the span
into a temporary and append that again.  With it off, the span is copied store to store in
one block: into the destination's spare room when it has some, otherwise after growing it,
re-reading the source's record after the growth because a slice of the destination itself
(`b += b[1..10]`, or `v += v[1..10]` through a `&` parameter) moves with it.  First bisect step for a wrong element
or length out of a slice append; guard
`tests/scripts/a-byte-range-of-a-vector-copied-one-at-a-time-is-one-append.loft` (v25, and v28
for the move).

**`LOFT_NO_PLAIN_FIELD_APPEND=1`** (runtime, BOTH backends, default-ON)
routes a record appended to a plain vector FIELD of a plain struct — `self.items += [x]`, the
library idiom — through the general record dispatch again.  With it off, `record_new` and
`record_finish` serve the shape the way they already served a bare vector's: the field's
vector resolved by its position, the element the inline slot `vector_append` claims, the
finish a length bump — where the general path asked the nullable-parent redirect and the
`__nullable<` name test twice, dispatched `insert_record`, and walked an empty sibling list
and an empty displaced list, 27 % of mesh3d's `sphere` on both backends.  The shape is
`Stores::plain_vector_field`'s (`@FR-R-Mint`'s field clause): a `Parts::Struct` parent, a
`Parts::Vector` field, no sibling sharing its records; a struct-enum variant, a `__nullable<S>`
element's field, a keyed field and a linked group keep the general path.  First bisect step for
a wrong or missing element out of an append to a struct's vector field; guard
`loft-suite/attic/scripts/a-record-appended-to-a-struct-field-vector-takes-the-plain-path.loft`.

## Runtime: keyed collections

**`LOFT_NO_FAST_ORDER=1`** (@PLN158 keyed class, runtime, BOTH backends) makes every search
that ORDERS — an `index` descent, a `sorted` / `ordered` binary search — compare through the
general `key_compare` / `compare` again, an exact `index` lookup take the boundary descent,
and an `index` insert look its duplicate up before it descends — with it off, the key is
resolved once per search (`keys::FastOrder`), a full-key lookup stops at the equal node
(`btree::find_exact`), and a refused `btree::add` names the duplicate its own descent met
(`index` fill-and-find −33 %) — and is the first bisect step for a wrong element, order or
lookup out of a `sorted`, `ordered` or `index`.  It also sorts a `for e in hash` walk through
`keys::compare` again, where with it off a single key, or a compound key of two to four
integer parts, is read once per record and the reads sorted (`keys::sort_records`) — the
first bisect step for a hash walk visiting in the wrong order.  **`LOFT_NO_ONE_PROBE_INSERT=1`** makes a
`hash` insert look its duplicate up and then file the entry as two hash-and-probe walks again
— with it off, one walk answers both (`hash::probe_for_insert`; a 5,000-key fill −37 %) — and
is the first bisect step for a lost, duplicated or unfindable `hash` entry.
**`LOFT_NO_KEYED_REPLACE=1`** makes a `hash` insert of a key already present unlink the
displaced entry (with the back-shift of the entries probed past it) and file the new one with
a second walk again — with it off, the new record takes the displaced entry's bucket in one
write (`hash::replace_at`; the key's probe chain is the same, the count does not move) — and is
the first bisect step for a lost or doubled entry, or a leak, after a repeated key.
**`LOFT_NO_SLICE_WRITE=1`** writes every vector to a file element by element through
`Stores::read_data` again — with it off, a vector whose elements are stored as they are written
(`integer`, `long`, `float`, `single`, `character`, a 4-byte sized integer, in the host's byte
order) goes out as one copy of its payload (`Stores::write_vector_payload`, the one walk both
backends and a vector field inside a record share) — and is the first bisect step for wrong bytes
in a binary file written from a vector.
**`LOFT_NO_KEEP_RANGE=1`** makes `v = v[lo..hi]` over a scalar element kind (lowered to
`OpKeepRange`) copy the kept span out, clear the vector and append the span again — with it
off, the span moves to the front of the vector's own record and the length is set
(`Stores::vector_keep_range`; a foreign view or a locked store always takes the copy form,
since neither may be written) — and is the first bisect step for a wrong element or length
after a vector is assigned its own slice.  `LOFT_NO_SLICE_COPY=1` keeps the parse-time
per-element form for every slice, this one included.
**`LOFT_KEYED_VERIFY=1`** is the falsifier for both: every pre-resolved comparison, every
exact lookup and every one-probe insert is checked against the general form as it is made,
and a disagreement panics naming both answers — run the keyed cells or the script corpus
under it after touching `keys.rs`, `hash.rs`, `btree.rs` or the `sorted` searches.  The C
backend honours it too, for the fast paths it has (`-DLOFT_KEYED_VERIFY`, C_BACKEND.md § What
switching to C loses; `scripts/c_sweep.py --keyed-verify` runs the corpus both ways).  Two more
levers of the same pass carry no switch because they have no second answer to bisect to: a
miss on a collection with no lazy binding answers at once (`Stores::lazy_bound`, a lookup
loop that misses half the time −20 %), and a whole word fed to the hasher on a word boundary
is one inline round (`SipHasher13::write_u64`, digest pinned by `siphash_std_parity`).
**`LOFT_NO_HALF_LOAD=1`** (runtime, BOTH backends) rebuilds a `hash` table at three quarters
full again instead of at half — a writer's policy no reader assumes, so stores written under
either read under both; with it off a miss walks ~1 bucket where it walked 5 at the old
threshold (removal −25 %, the vector + hash group −22 %, fill −21 %), for ~3.6 bytes an entry
more bucket array — and is the bisect step for a `hash` whose table size matters.
**`LOFT_NO_POW2_BUCKETS=1`** (`@FR-Col-Bucket`, @C146; runtime, BOTH backends) grows and
reserves a `hash` table to the sizes it had before — a count that is not a power of two — and
places every key by division.  The placement formula is `digest mod count` either way, so
stores written under either read under both; it is the A/B for the rule and the way to write a
table whose count the mask does not serve.
**`LOFT_NO_HASH_TAG=1`** (`@FR-Col-Tag`; runtime, BOTH backends) makes and rebuilds every
`hash` table with untagged 4-byte slots, the layout a store written before the tag carries;
it implies **`LOFT_NO_HASH_INLINE=1`** (`@FR-Col-Inline`), which keeps the entries of a table
that owns them in the arena instead of in the table.  Both are a WRITER's policy (@C152): each table
records its layout in its header, so one store holds tables of every layout side by side and
every build reads and writes all of them (`tests/store_persist_loft.rs`
`every_hash_layout_lives_beside_the_others_in_one_store`).  **`LOFT_TRACE_HASH_LAYOUT=1`**
names each table made, grown or demoted, its layout before and after and the field that holds
it — what a store holds is otherwise visible only in its bytes.
**`LOFT_NO_TYPED_KEYED=1`** (`@FR-R-TypedKeyed`, generation time, `--native` only) emits a
lookup in a `hash` with ONE integer or `text` key, or two to four integer parts, as the
general `OpGetRecord` again — with it off it is `OpGetHashLong` (or `OpGetHashText`, the key
handed over as the text; `OpGetHashInts`, the parts as values), the
integer key handed over as the integer it is, with no `Content` built, no
type-row dispatch and the walk compiled for the key's kind (613 → 444 instructions a lookup,
`hash_find` −13 %) — and is the first bisect step for a wrong or missing record out of such
a lookup on native; `LOFT_KEYED_VERIFY=1` answers every typed lookup through the general
entry too, and checks a removal's recognised slot against the entry's own index.
`bench/portal/analysis/keyed.md` has the ledger and what is left.

## Runtime: store claims, clears and prefill

**`LOFT_NO_POOL_RESET=1`** (`@FR-R-Reuse`'s reset clause, default-ON, both backends) makes a
reused call buffer's release walk its tree again — with it off, the pool's release before every
call after the first resets the buffer's store to a fresh root (`Stores::reset_pool_root`), the
way `OpDatabase` re-mints a reused store.  The first bisect step for a value lost, or a store
read stale, after a reused call buffer's release.

**A claim writes nothing** (`@FR-H-Claim`, `@C137`): a claimed payload, and the tail an
in-place grow exposes, hold whatever the block held last; every value is written by the code
that builds it.  **`LOFT_POISON_CLAIM=1`** (`Store::poison_fill` / `poison_range`) fills both
with `0xDEADBEEF` instead — the claim-side twin of `LOFT_POISON`'s poison-on-free, and the
falsifier for *"does this reader rely on bytes nothing wrote?"*: a handle or length read out of
unwritten space becomes a loud out-of-range value the store's guards refuse, where the default's
stale bytes often look enough like valid data to pass.  The nightly `poison-claim` job runs the
suite under it on both backends, and `tests/poison_claim.rs` keeps the interpreter corpus at
zero dependents.  **`LOFT_ZERO_CLAIM=1`** zeroes every claim again — a debugging lever, never a
fix (DEBUG_STORES.md).

**`LOFT_NO_CLEAR_RELEASE=1`** (`@FR-H-ClearRelease`, runtime, BOTH backends) makes a
vector's entry clear a pure length reset again — with it off, clearing a REUSED
store-root vector whose elements own heap releases what they own first, closing an
unbounded leak in every shape-A return buffer (~357 KB per `fronds` call) — and is the
bisect step for a double free or a wrong value at a cleared vector.  `LOFT_TRACE_CLEAR=1`
names each clear's shape and element verdict.

**`LOFT_NO_STORE_RESET_CLEAR=1`** (@PLN157 § V-ag, runtime, BOTH backends) makes that
release a WALK again — with it off, clearing a store-ROOT vector resets the store in one
step and re-establishes its two records, because the shape `@FR-H-ClearRelease` already
tests says the vector owns the store's whole extent (`fronds` −36 %, the free tree was
28 % of that row) — and is the first bisect step for a wrong value, a leak or a
use-after-free at a recycled vector-returning call.
**`LOFT_NO_RESET_CAPACITY=1`** (@PLN157 § V-ai, runtime, BOTH backends) makes that reset
re-establish the vector at the fresh minimum again — with it off, the vector comes back at
the CAPACITY the previous fill reached (the buffer is reused across calls and the store
already holds the extent), so the growth ladder runs once per buffer instead of once per
call, and the freed rungs that took every later claim off `bump_tail` and into the free
tree are never made (`fronds` −22.5 %, 4.28× → 3.42× on x86-64) — and is the bisect step
for a wrong element or length out of a reused vector-returning call.  `LOFT_TRACE_CLEAR=1`
prints the capacity each reset re-establishes.

**`LOFT_NO_EMPTY_CLEAR_SKIP=1`** (STORE_MODEL D19f, runtime, BOTH backends) makes clearing a
store-root vector that holds nothing reset its store again — with it off, a clear of a vector
with no block, or a block of length zero, does nothing, as the C runtime's
`loft_vec_clear_release` does nothing (STORE_MODEL's rule: the C model is the standard) —
and is the A/B for the store events a reused, empty buffer makes.  `LOFT_TRACE_CLEAR=1`
prints each reset made.

**`LOFT_NO_PREFILL_IMAGE=1`** (@PLN164 C4, `@FR-R-Prefill`, runtime, BOTH backends) makes
every record mint that is not proven complete-write prefill its defaults field by field
again — with it off, the prefill is ONE block write of a per-type image captured from the
walk's first run over a zeroed span (the `parse` row −11 %) — and is the first bisect step
for a wrong default, sentinel or variant tag in a minted record.  `LOFT_PREFILL_VERIFY=1`
re-runs the walk after every image write and panics naming the type where the bytes
disagree; `LOFT_TRACE_PREFILL=1` names each capture and each use — and a cell can pass
without ever reaching the image, because on `--native` a literal's mint is a complete
write that never prefills and a callee's buffer is minted once per caller activation.

## Runtime: the free tree

**Free-block footers (@PLN157 § V-v, default-ON, both backends, `@FR-H-FreeFooter`):** a
store's FREE block carries its size at both ends, so a record delete coalesces BACKWARD in
O(1) off the footer (tree-confirmed — claimed data can spell a false footer) instead of
leaving adjacent frees to `claim`'s lazy O(blocks) sweep; the sweep stays armed only for
the untracked one-word case.  **`LOFT_NO_FREE_FOOTER=1`** restores the sweep-only form
whole (every delete arms it) and is the first bisect step for a store-layout fault in a
delete-heavy run.  The class it retires is the `coalesce_free` cliff (PERFORMANCE.md § V-j P2).

**The tail free block is a wilderness (@PLN164, default-ON, both backends,
`@FR-H-Wilderness`):** the free block that ends a store is held beside the free tree, and the
tree's insert, remove and best-fit take treat it as the node it would have been — so every
claim takes the block it always took (a seeded side-by-side unit test pins the layout), and a
claim from the tail or a delete into it costs no tree delete, insert or rebalance (64 % of the
parse row's tree claims took the tail; the row −6 % in cycles).  **`LOFT_NO_WILDERNESS=1`**
keeps the tail in the tree again (read per store at construction; on C the
`-DLOFT_NO_WILDERNESS` define the driver adds from it) and is the first bisect step
for a store-layout fault or a claim that hands out a live block.

**A record at the store's tail grows the store, not a new block (`@FR-H-TailGrow`):** a resize
that cannot grow in place, of a record nothing claimed follows, grows the store and then the
record where it is — moving it left its old block as a hole beside the new one at every
doubling of a tail buffer.  **`LOFT_NO_TAIL_GROW=1`** (interpreter; read once) moves it again:
the A/B for a footprint, and a bisect step for a store-layout fault.  The C runtime has no
switch, as for the wilderness.

**A text loop variable borrows its element on both backends (`@FR-R-TextBorrow`,
@PLN198):** the interpreter binds `for p in words` over a `vector<text>` as a `Str` view under
the native generator's own admission, where it copied each element into a `String`;
**`LOFT_NO_TEXT_BORROW=1`** restores the copy on both.  The switch and the admission are
written up in [NATIVE_SWITCHES.md](NATIVE_SWITCHES.md).

**A claim on an empty tree bumps the wilderness (default-ON, both backends,
`@FR-H-Wilderness`):** while the free tree holds nothing the wilderness is the only block a
best fit could choose, so the claim is carved off its front directly — `bump_tail`'s twin for a
store that keeps a wilderness, which `bump_tail` skips.  Same block, split and remainder; what
goes is the tree walk and two calls per claim.  A store reset per call (a returned
`vector<text>`'s root reset) claims every record this way: `split` −10 %, @PLN179 finding 021.
**`LOFT_NO_WILD_BUMP=1`** sends such a claim through `claim_best_fit` again.

**A best-fit claim carves its node in place (default-ON, both backends, `@FR-H-Carve`):** a
claim from a free-tree block at least twice the request leaves the remainder in the block's
place in the tree (links, color, parent pointer) instead of a delete and an insert — the block
is the smallest that fits, so the remainder keeps its order, and every claim takes the block it
always took (`hash_text_keys` −8 %).  **`LOFT_NO_CARVE_IN_PLACE=1`** deletes and inserts again
(read per store at construction) and is the bisect step for a store-layout fault or a corrupt
free tree after a claim.

**A store starts in a lazy-free phase (default-ON, both backends, `@FR-H-LazyFree`):** a
delete of a small block (at most 64 words) merges with its free neighbours as ever and leaves
the result out of the tree (a block ending the store becomes the wilderness); a larger block is
tracked at its delete as ever — and the phase ends at the first
claim once the untracked words reach 256 and a fifth of the extent written, with one sweep
that merges and tracks every free block, or at an explicit reclaim; a fresh or reset store
starts the phase again, a store bound to a file never enters it.  A store built once and freed whole
(a call's result buffer, a decoder's tree) never pays for its deletes, and its claims stay on
the tail.
**`LOFT_NO_LAZY_FREE=1`** tracks every delete at its site again (read per store at
construction) and is the first bisect step for a store-layout fault, a claim that hands out a
live block, or a store that grows instead of reusing a freed block.  The falsifiers are the free tree's
own — `LOFT_POISON=1`, `LOFT_STRICT_STORES=1` — and the seeded unit tests in `src/store.rs`
pin the untracked delete, the growth below the floor and the one sweep at the bound.

**`LOFT_NO_SCAN_INDEX=1`** (runtime, BOTH backends, `@FR-H-ScanIndex`) makes a claim the free
tree and the wilderness cannot serve walk the block chain from the store's start again — with
it off, the first fit is read off the lazily freed blocks' bitset and the tail block off its
confirmed footer, which is the same block the walk takes.  First bisect step for a claim that
lands on a live block or a store that grows where the walk would have reused a free block; the
seeded test `store::tests::the_indexed_scan_takes_the_block_the_walk_takes` compares the two
answers at every claim.

**`LOFT_NO_HEAP_FREE_VECTOR_COPY=1`** (runtime, BOTH backends) makes the deep copy of a vector
whose elements own no heap list and visit every element after its one block copy again — with
it off, that walk is skipped, as the record arm already skips a record that owns no heap.
First bisect step for a copied record whose vector reads the source's data; guard
`loft-suite/attic/scripts/157-a-heap-free-vector-copies-in-one-block.loft`.

**`LOFT_NO_PLAIN_VECTOR_RECORDS=1`** (runtime, BOTH backends) sends a record that owns plain
vectors and nothing else — every field that owns heap a `vector<E>` whose elements own none —
through the walk of its type again when it is copied or released.  With it off, such a record
is copied and released by the list of those fields (`Stores::plain_vectors_of`, at most eight;
one bit of the type's facts says whether a type has the list): each vector's block is claimed
and copied as one piece, or freed and its field cleared, with no descent through the fields'
types.  A field holding anything but an empty handle or a block that fits its store sends the
release back to the walk, which reports it.  First bisect step for a copied record whose vector
reads its source's data, or a store that grows across rebinds of such a record; guard
`loft-suite/attic/scripts/157-a-record-of-plain-vectors-copies-and-releases-by-its-fields.loft`.

**`LOFT_NO_RELEASE_PLAN=1`** (runtime, BOTH backends) sends a record whose heap is texts and
vectors of text-owning elements through the walk of its type again when it is released in a
store that lives on (a reused buffer, an element written over).  With it off, such a record is
released by its type's plan (`Stores::release_by_plan`, at most eight slots, an element of at
most four texts): each text deleted, each vector's element texts and then its block, every field
cleared, in the walk's order.  A pointer past the store or a length that does not fit sends the
release back to the walk, which reports it.  First bisect step for a store that grows across
rebinds of such a record, or a text that reads freed bytes after one; guard
`loft-suite/rules/heap/H-ClearRelease/a-record-of-texts-is-released-by-its-plan.loft`.

**A removed record a stored reference named leaves a tombstone (default-ON, both backends,
`@FR-H-Removed`):** `OpSetDbRef` marks the record a stored reference names; `Store::delete`
keeps such a record's header word as a one-word block that no claim reuses and frees the rest,
and `OpGetDbRef` reads null there.  A store minted again in place by `OpDatabase` keeps the
marks of the records it held.  It costs one word per such removal, and nothing for a program
without stored references.  **`LOFT_NO_TOMBSTONE=1`** frees the record like any other,
so the reference reads whatever is claimed there next.  It is the bisect step for a reference
that reads null where its record should still be there, or for a store that grows under
removals.

**A record a stored reference names lives as long as its holder's store (default-ON, both
backends, `@FR-H-RefHome`):** after pass 2, `scopes::stored_reference_homes` finds each
`reference<T>` written, copied or linked by a callee into a holder that outlives the variable
holding the record.  A named local held from an enclosing block has its store moved out to that
block; a literal, a call result or a refilled local's own record is built in the holder's store
(`OpPlaceRecord`); a record a parameter or the returned value names is copied into that holder's
store just before its home is freed (`OpEvacuate`).  What none of those keeps is refused.  A
reassignment of a local vector such a reference names into empties it in place.
**`LOFT_NO_REF_HOME=1`** withholds the move, the placement, the copy-out and the refusal (the
in-place reassignment stays).  It is the bisect step when a program that compiled before is
refused, or reads a stored reference differently, and the A/B that shows what such a shape
reads without it.
