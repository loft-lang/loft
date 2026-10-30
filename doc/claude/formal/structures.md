<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/structures.md — the stored structures and the image format (strict)

**Catalogue:** @F3 (heap/store), @PLN135 (the keyed placement), @PLN97 (layout contract).

> **Rules then deviations** (see [README](README.md)). [layout.md](layout.md) defines the bytes
> of a RECORD — a field's offset, a scalar's width, a reference's encoding.  This chapter defines
> the bytes of everything a store holds BESIDE those records, which a reader has to reproduce to
> find a value in an image it did not write:
>
> - the **frame and the image** — what `store_persist_copy` writes, and what is free to differ;
> - the **live buffer** — the loft file layout: every block's bytes as a store holds them in
>   memory, in a mapped file and under either runtime, and what lives beside the buffer;
> - the **sidecar and the format identity** — how a reader knows which rules an image was
>   written under, and what it does when they are not its own;
> - the **keyed structures** — the hash table and its arena, the index's B+tree, the spatial
>   and trie trees, and the vector forms;
> - the **paged reader** — what a reader that fetches byte ranges (a local file, or HTTP with no
>   code on the server) may rely on, and what it must answer.
>
> Four readers depend on these bytes: the resident store (interpreter and `--native` share the
> Rust modules), the whole-image loaders, the paged reader (`src/paged_reader.rs`) and the C
> backend (`src/c/loft_hash.h`, `loft_btree.h`, `loft_radix.h`, `loft_image.h`).  So does every
> image a program has already published, and that one cannot be upgraded with the binary —
> which is why every byte below is a contract, and why the rules say what may NOT be relied on
> as carefully as what may.
>
> [collections.md](collections.md)'s `Col-Bucket`, `Col-Tag`, `Col-Inline` and `Col-Layouts`
> state the hash's layouts as BEHAVIOUR (a probe compares the tag first; layouts live side by
> side); the rules here give their bytes.  [allocator.md](allocator.md)'s rules decide where
> a block LANDS, which no reader relies on and an image does not carry; the bytes each kind of
> block holds are here (`S-Live`), because a live buffer and a mapped file carry them.

## Notation

- A store is an array of 8-byte **words**; `rec` is a word index, so record `rec`'s byte `f` is
  at file offset `rec·8 + f`.  Every multi-byte number is in the WRITER's byte order, which the
  sidecar records (`@endian`, layout.md); every current target is little-endian.
- `u32@f`, `i64@f` — a number at byte `f` of the record or node under discussion.
- **record payload** — a record's fields start at byte 8 (layout.md's frame); bytes 0..4 are
  its `i32` size in words.
- `count` — a hash table's bucket count; `stride` — bytes per entry; `sb` — bytes per slot.

---

## Rules

### The frame and the image

```
  (S-Frame)   an image is a store's bytes: word 0 holds the SIGNATURE 0x53746f31 at bytes 0..4
              (a reader refuses any file without it); bytes 4..8 hold the image's LENGTH in
              words, as a u32, written by every image writer — a reader refuses a file of any
              other length, so a cut on a block boundary is told from a whole image.  0 or 1
              there records no length (what a writer before it wrote, and what a store in
              memory or bound to a file holds, since those grow).  From word 1 the file is a
              CHAIN of blocks, each opening with an i32 size in words — positive for a claimed
              record, negative for a free block — that tiles the file exactly: the last block
              ends at the file's last word, so a reader refuses a file that is not whole words
              or whose chain does not tile it.  Word 1 is the
              ROOT record (PRIMARY); the stored value is the root's payload, at (1, 8), laid out
              as its type (layout.md), and every collection, text and child record it owns is a
              claimed block of the same file, named by record number.
  (S-Image)   store_persist_copy writes a REBUILT copy, never the live store's bytes: the root is
              claimed first, so it lands on PRIMARY, and every block it owns is claimed again in
              the copy's walk order.  The file ends at the end of the last claimed block, padded
              to at least 1024 words by ONE free block; the padding is zeros past that block's
              header.  The writer may lay out a tree's nodes in any order a reader cannot see
              (S-Tree), and the copy's block placement is the allocator's.
  (S-Derived) a reader relies on the chain and on the bytes this chapter and layout.md define,
              and on nothing else.  The free-block tree, the free footers, the parked lists and
              the claim and removal bitsets (allocator.md, heap.md) are NOT in an image: a
              reader that needs them derives them from the chain.  The order of the blocks, the
              size of a free block, the van Emde Boas order of a tree's nodes and the padding
              past the floor are the writer's choice, and two images of one value may differ in
              any of them.
```

**In words.** An image is not a dump of memory but a clean copy of one value: a fixed first
word, the value at word 1, and a chain of blocks a reader can walk without an index.  The
allocator's bookkeeping stays home; a reader rebuilds what it needs.  Because the writer
chooses where blocks go, two images of the same value can differ byte for byte and still be
equally correct — what must agree is what a lookup finds, not where it was put.

### The live buffer — the loft file layout

```
  (S-Live)    a store's BUFFER is the loft file layout: the S-Frame chain together with the
              bytes of every block's STATE — claimed (S-Block), free (S-Free), parked (S-Park),
              a tombstone (S-Tomb).  It is the same bytes in memory, in a file a store is bound
              to (S-Mapped), and under either runtime: the interpreter's store (src/store.rs)
              and the C runtime's (src/c/loft_store.h) write the same bytes for the same steps,
              so either may read or continue a buffer the other wrote.  Where a block LANDS is
              allocator.md's choice and may change; the bytes a block of each state holds may
              not, short of a format change (S-Format).  An image is a rebuilt copy (S-Image)
              and relies on S-Block and S-Free only.
  (S-Block)   a CLAIMED block of n words: i32 +n at @0.  Its payload is the record's content,
              laid out as its type (layout.md) from @8; the half-word @4 belongs to its kind —
              a vector's, a sorted's, an ordered's and an array's length (S-Vector), a text's
              byte length, a keyed collection's entry record the number of its collection's
              record — and is content where the kind gives it none.  n is exactly the words
              the record's claim asked for (allocator.md H-Exact); a vector's or a text's
              spare room is room its own growth asked for (collections.md Col-Capacity).  A
              TEXT record holds its length at @4 (u32) and its UTF-8 bytes from @8; the
              bytes past len are not content.  A claim promises no value in
              its payload (heap.md H-Claim).
  (S-Free)    a FREE block of n words: i32 −n at @0, and −n again in the HIGH half of its last
              word (allocator.md H-FreeFooter; a one-word free block's footer shares its
              header's word, at @4).  A free block of at least two words that the free tree
              holds (MIN_FREE_TREE = 2) keeps the tree's links in its first payload word: the
              LEFT child's block number at @4 (u32) and the RIGHT child's at @8 (u31), with the
              node's color — red — in bit 31 of @8.  A link of 0 is no child.  The block that
              ends the store (the wilderness, H-Wilderness) and a block freed during the lazy
              phase (H-LazyFree) hold the header and footer and no links a reader may follow.
  (S-Park)    a PARKED block (allocator.md H-Park, 2 to 16 words): a free block (S-Free's
              header and footer) whose @4 holds the PREVIOUS block of its size's list and @8 the
              NEXT, as u32 block numbers, 0 ending the list — the half-words a tree node keeps
              its links in.  A reader that does not keep the lists reads it as a free block.
  (S-Tomb)    a TOMBSTONE (heap.md H-Removed): a removed record a stored reference named
              becomes a claimed block of ONE word — i32 +1 at @0, 0 at @4 — and the rest of its
              words a free block behind it.  It is claimed by its header and named by no
              collection; what marks it removed is the runtime's removal set (S-Side).
  (S-Side)    what the buffer does NOT hold is the runtime's, kept beside it and re-derived
              from the chain when a buffer is opened or adopted: the claim set (every block
              with a positive header), the free tree's root and the wilderness (rebuilt from
              the free blocks), the parked lists' heads and the parked set (empty — a parked
              block reads as free), the lazy phase's record (restarted), the referenced and
              removed sets, which of its bytes were written, and its lock.  A runtime handing a
              live store to the other (@PLN206's hosted unit) hands these with it; a file
              carries none of them.
```

**In words.** A store is one buffer, and the buffer is the file: what a program builds in memory,
what a mapped file holds and what the other runtime reads are the same bytes. Each block says
what it is in its first word: positive is a record, negative is free, and a free block's first
payload word holds the free tree's links. Everything the allocator knows beyond that — which
blocks are claimed, where the tree's root is, which blocks wait on a parked list — is kept beside
the buffer and can be rebuilt by walking it. That is the line a second runtime has to respect:
it may place blocks differently, but it must write each kind of block the same way.

### The sidecar and the format identity

```
  (S-Sidecar) beside an image at <path> lies its sidecar <path>.dschema, UTF-8 lines:
                LOFT-DSCHEMA v2
                hash=<u64, decimal>
                [root=<the root type's name>]
                [slot=<u16: the store number the image's own references name>]
                [version=<the writing program's release>]
                --
                <the layout dump>
                --facts--
                <facts>
              hash is FNV-1a 64 over the dump's bytes.  The dump opens with an @endian line and
              lists every type the root reaches, one line each, sorted by name (layout.md's
              identity).  The facts (a type's `linked`, a field's `nullable`, `target=`,
              `group=`) are carried for the reader and are not part of the identity.  A sidecar
              with none of root, slot, version and facts is written in the v1 form: the same
              text without those lines and without the facts fence.  A project baseline adds a
              --sources-- section after the facts (layout.md L-Upgrade-Source).
  (S-Format)  every byte this chapter defines for a keyed kind belongs to that kind's PLACEMENT
              TOKEN (src/placement.rs: hash, index, spatial, trie), rendered into the kind's
              line of the layout dump, so it is part of the identity.  A change to anything a
              reader must reproduce to find an entry — the table or node layout, the digest,
              how a digest becomes a slot, the probe or descent order, how the slot count
              follows from the claimed size — changes that kind's token.  A build lists the
              older tokens it reads as written (HASH_READS), and reads such an image as its own.
              A token equal to the baseline renders as nothing.  sorted and ordered carry no
              token: a reader finds their entries by comparing keys (keys::compare), whose
              order is every kind's.
  (S-Gate)    a reader compares an image's sidecar with its own identity before it reads one
              byte under it:
                - equal hash and dump, or equal after reading its older tokens as its own
                  (S-Format) — the bytes are read as written;
                - a release below the reader's floor — refused (layout.md L-Release);
                - a different identity — converted by a loader that reads the whole image
                  (the upgrade, layout.md) and refused by a paged one; never read raw;
                - a sidecar that does not parse, or cannot be read — refused;
                - NO sidecar — refused: the image records no layout, so a raw read is a guess
                  at every stride.  Over HTTP a sidecar that cannot be fetched (a 404, an
                  unreachable server) is no sidecar.
              The refusal is the loader's failure answer (false, or a count of 0), and nothing
              is loaded.
  (S-Pair)    an image and its sidecar are ONE write.  The writer stages both beside the target
              and replaces what is there only when both are written; the call answers true only
              then.  A write that fails before the replace leaves the previous image and its
              sidecar as they were.
```

**In words.** The sidecar is the image's passport: which layout, which placement, which release.
A reader checks it first and reads raw only on a match, which is what lets the format change
between releases without one image ever being read under the wrong rules.  The placement token
is the part that covers this chapter: the layout hash alone says where a record's FIELDS are,
the token says how the hash or tree finds the RECORD.  Writing the two files as one is what
keeps the passport honest: an image beside no sidecar is refused, and one beside an older
writer's sidecar would be read under rules it was not written by.

### The hash table

```
  (S-Hash)    a hash collection's slot names its TABLE record.  Its fields, by byte:
                @4  u32 the entry count (an inline table: its owner record)
                @8  u64 the SEED (S-Digest), low half first
                @16 u32 the arena's chunk directory record (an inline table: the entry count)
                @20 u32 the arena's next unused index, 1-based (an inline table: the next
                        insertion sequence number, from 1)
                @24 u32 the arena's free-list head (an inline table: the tombstone count)
                @28 u32 the FLAGS: bits 0..16 the entry stride in bytes (0: the table borrows
                        its records and owns no arena); bit 31 TAGGED; bit 30 INLINE; for an
                        inline table bits 16..21 hold log2(count)
                @32 the slots, count of them, sb bytes each
              An untagged slot is 4 bytes, a tagged one 8: the u32 entry at +0 and the low 32
              bits of the key's digest (the TAG) at +4.  An empty slot's entry is 0.  In a table
              whose stride is 0, and in any slot whose entry has bit 31 set, the entry with bit
              31 cleared is a RECORD number (its payload at 8) — a record the table borrows;
              every other entry is a 1-based index into the arena (S-Arena).  For a record of `words`
              words, a tagged table's count is the largest power of two not above words − 4,
              an untagged table's is (words − 4) · 2.
  (S-Digest)  a key's DIGEST is SipHash-1-3 with both keys 0, fed the table's seed (8 bytes)
              and then each key field in key order: an integer-class field as the 8-byte value
              its declared type decodes (layout.md L-Narrow-Decode), a text as its UTF-8 bytes
              followed by one 0xFF.  A float or single field adds nothing, so a key of floats
              alone has one digest per table.  A table's seed is drawn when it is created
              (LOFT_HASH_SEED fixes it), is never 0, and is stored in the table: a reader takes
              it from the bytes and never draws its own.
  (S-Probe)   a key's HOME slot is digest mod count; a lookup reads slots from home onward,
              wrapping, and stops at the first empty slot, having read at most count slots.  A
              tagged table compares the tag before it reads an entry (Col-Tag).  A removal from
              a table that is not inline leaves no tombstone: each later entry of the run moves
              back to the freed slot when that slot lies between it and its home.
  (S-Arena)   an arena table's entries live in CHUNKS named by the directory record (u32
              record numbers from byte 8; its capacity (words − 1) · 2).  Chunk k holds 64·2^k
              entries for k ≤ 7 and 8192 entries from k = 8 on; each chunk record names the
              table's owner at byte 4 and holds its entries from byte 8, stride bytes each.
              Index i (1-based) lies in the chunk and at the offset this tiling gives i − 1.
              A freed entry is threaded through its first 4 bytes.
  (S-Inline)  an INLINE table (Col-Inline) holds its entries in its own record: slot i is 8
              bytes, an insertion sequence number at +0 (0 empty, 0xFFFFFFFF removed) and the
              tag at +4, and its entry lies at 32 + count·8 + i·stride.  Its count is the largest
              power of two the record holds at 8 + stride bytes per slot.  A walk yields entries
              in sequence order; a removal leaves its tombstone until the table is rebuilt.
```

**In words.** Five numbers in a fixed header, then a run of slots; a slot names an entry by
index into chunked storage, or directly by record for a borrowed one.  The digest is SipHash
over the seed and the key's decoded values, so a reader with the seed from the table and the key
from the caller lands on the same slot the writer used.  Everything a reader needs is in the
table's own bytes; nothing depends on the build that wrote it except through the token.

### The index

```
  (S-Index)   an index collection's slot names its ARENA record: node n at byte 8 + 296·n.
              Node 0 is the header, its u32 fields at 8 +: 0 the root node, 4 the height (0: the
              root is a leaf), 8 the record count, 12 the nodes in use (the header included),
              16 the free-node head, 20 the capacity in nodes, 24 the first leaf, 28 the last.
              Every other node, by byte from its start:
                @0   u32 the slot count            @8   u32 the next leaf (a free node: the
                @4   u16 1 leaf, 0 internal, 2 free     next free node)
                @6   u16 the shared text prefix    @12  u32 the previous leaf
                         length (≤ 24)             @16  i64 parts[16]
                @144 u32 records[16]               @208 u32 children[16]
                @272 u8  the shared text prefix[24]
              A node holds 4 to 16 slots (the root fewer).  A leaf slot is (part, record); an
              internal node's slot 0 is its first child only, and slot i ≥ 1 is (part, the first
              record under child i, child i).  A PART is the first key field as an
              order-preserving i64: an integer itself, a float its total-order bits with −0 as
              0, a text the 8 bytes after the node's shared prefix, big-endian, zero-padded,
              with the top bit flipped; a descending field is bitwise negated.  Two parts that
              tie are ordered by the records' keys (keys::compare).  A position in a walk is
              leaf·16 + slot.
```

### The spatial and trie trees

```
  (S-Tree)    a spatial or trie collection's slot names its TREE record: @4 i32 the TOP, @8 u32
              the record count, @12 u32 the highest node id in use, @16 u32 the KIND's word,
              @20 u32 the free-node head; node n (1-based) at byte 24 + 16·(n − 1):
              @0 u32 the bit index it tests, @4 u32 its parent, @8 i32 the child for a 0 bit,
              @12 i32 the child for a 1 bit.  A child (and the top) is 0 for none, a record
              number when positive, −id of a node when negative.  The tree is a PATRICIA tree:
              the bit index strictly grows going down.  Node ids are a numbering, not an order —
              a writer may renumber them (S-Image) and a reader follows ids, so a reader relies
              on every id it meets lying in 1 ..= the highest id, and on nothing else about them.
              The node capacity is the claim's: (record bytes − 24) / 16.  The kind's word is 0
              in a spatial; in a trie it names the record of the absent key with bit 31 set, or
              is 0 when there is none, and that record is not in the tree.  A trie under
              placement 1 holds the node capacity there, bit 31 clear — read as no such record.
  (S-TreeKey) a record's KEY STRING is its user bits, then 8 zero bits, then its record number
              as 32 bits, most significant first; a lookup that has no record uses 0.  The user
              bits of a TRIE are its text's bytes in order, a 0x00 byte written as 0x01 0x01 and
              a 0x01 byte as 0x01 0x02, so no key byte is 0 and byte order is kept; under
              placement 1 the bytes are written as they are.  The user bits of
              a SPATIAL record interleave its axes — at most 3 — bit by bit, most significant
              first, bit b from axis b mod n: each axis is the 64-bit value its declared type
              decodes (L-Narrow-Decode) with its top bit flipped, so the order of the codes is
              the order of the values (Col-Axis).
```

**In words.** The tree is a PATRICIA trie over a bit string that ends in the record's own
number, which is what lets two records share a key and still have distinct leaves.  The nodes
carry no data of their own beyond which bit to test; their numbering is free, which is why the
image writer may lay them out again for locality without any reader noticing.

### The vector forms

```
  (S-Vector)  a vector's, a sorted's, an ordered's and an array's slot names a record holding
              the length at @4 and its elements from @8.  A vector's and a sorted's elements are
              inline, size(T) bytes each (layout.md H-Stride); an ordered's and an array's are
              u32 record numbers.  A sorted and an ordered keep their elements in key order
              (keys::compare); a reader finds a key by binary search.
```

### The paged reader

```
  (S-Paged)      a paged reader reads an image by byte RANGE — a local file or an HTTP range
                 request, with no code on the server — and keeps no index of its own: every
                 address it reads is computed from the caller's key and from bytes these rules
                 define.  It applies S-Gate to the image's sidecar before it reads anything
                 else, and refuses a file without the S-Frame signature.  It finds the root
                 collection at the position the caller's own collection has in its root record
                 (the two share the root's type, so they share its layout).  It serves a hash
                 by S-Digest and S-Probe, a trie and a spatial by S-Tree, and a range of an
                 integer key of a sorted or an ordered by binary search (S-Vector); it serves no
                 index.  A byte past the end of the image reads as 0, and every walk is bounded
                 by the counts in the bytes it read.
  (S-Paged-Same) a paged lookup answers, for every key, the record a whole-image load of the
                 same image answers for that key, with the same field values — the storage-
                 independence half of Col-Axis.  Each key field is decoded by the reader of its
                 declared type (L-Narrow-Decode), on both sides of the comparison.
```

**In words.** The paged reader is the reason this chapter exists: it reads a published image
with nothing but range requests, so the image's bytes are its only interface, and anything it
computes differently from the resident store becomes a lookup that silently answers "absent".

### The mapped store

```
  (S-Mapped)  a store bound to a file holds the S-Frame chain over the file's whole length,
              with the allocator's free-block links and footers in its free blocks
              (allocator.md); it is not an image — no rebuild, no padding to the live end, no
              sidecar — and the runtime binds one only to its own files (the IR cache, the call
              arena).  Opened only to be read, it writes nothing (H-ReadSurface).
```

---

## Deviations

**OPEN: 0.**  The full register, with each entry's measurement, is in
[structures-history.md](structures-history.md).

## Conformance

- **`S-Frame` / `S-Image` / `S-Derived`** — `tests/store_persist_loft.rs`:
  `a_written_image_reports_a_trustworthy_mark` (the chain tiles the file), the slack and
  compaction cells, `store_load_rejects_garbage_file`, `a_store_image_cut_short_is_refused_on_every_backend`
  (cuts inside a word, inside a block and on a block boundary, three backends);
  `tests/lazy_source_not_an_image.rs` (the
  signature, a truncated file); `src/store.rs` `from_bytes` cases.
- **`S-Live` / `S-Block` / `S-Free` / `S-Park` / `S-Tomb`** — `tests/c_emit.rs`
  `the_c_store_allocator_places_every_record_where_store_does`: one seeded run of claims,
  deletes, resizes and referenced-record marks through `Store` and through the C allocator, the
  WHOLE buffer compared after it (headers, footers, tree links, parked links, a tombstone);
  `src/store.rs`'s `tree_in_order`, `chain`, `check_wilderness` and `check_parked` re-derive
  the side state from the bytes after every step (allocator.md § Conformance).  `S-Side` is
  `Store::resync_allocator` — the walk `open` does, and the one a side runs when the other
  hands it a mapped store — rebuilding the free tree and the claim set from the chain.
- **`S-Sidecar` / `S-Gate`** — `src/schema_sidecar.rs` unit tests (identical, changed, an endian
  flip, a garbage sidecar refused, the v2 round trip); `tests/layout_golden.rs`
  `schema_sidecar_identity_end_to_end`; `tests/scripts/store_load_layout_gate*.loft`.
- **`S-Format`** — `tests/layout_golden.rs` `placement_contract_is_pinned` (the hash and arena
  constants and digests, the tokens), `a_changed_placement_token_refuses_the_store`,
  `an_older_hash_token_this_build_reads_hands_the_store_over_raw`.
- **`S-Hash` / `S-Digest` / `S-Probe` / `S-Arena` / `S-Inline`** — the constants and digests by
  `placement_contract_is_pinned`; the layouts side by side by
  `every_hash_layout_lives_beside_the_others_in_one_store`.
- **`S-Index`, `S-Tree`, `S-TreeKey`** — ⚠ pinned by NO frozen value: `src/btree.rs` and
  `src/radix_tree.rs` keep their constants private and their unit tests check behaviour, so a
  changed node offset passes every test that does not read an old image.  The C backend's
  replays (`tests/c_emit.rs`, the index and tree cells) compare the C's bytes with the Rust
  modules' at every step, which pins the two implementations together, not either to a value.
  `r11_relayout_preserves_every_answer` (`src/radix_tree.rs`) covers the renumbering S-Tree
  allows.
- **`S-Paged` / `S-Paged-Same`** — `trie_db.rs` `the_paged_walk_answers_what_the_resident_walk_answers`,
  `radix_db/paged.rs` `a_paged_box_agrees_with_the_resident_one` and
  `every_axis_width_decodes_to_what_was_written`; the hash on every narrow key width,
  `tests/scripts/2172-a-paged-lookup-decodes-a-narrow-key-as-the-store-does.loft`; a struct-enum
  variant's payload under every paged loader,
  `tests/scripts/2178-a-paged-load-relocates-an-enum-variants-payload.loft`;
  `tests/paged_read_alloc.rs`, `tests/paged_browser.rs`, `tests/scripts/store_load_*.loft`;
  the bound on a malformed image — every claimed record's length and size word made huge, one
  at a time, under every paged loader —
  `tests/scripts/2177-a-malformed-image-answers-every-paged-load.loft`; the C's reader beside
  the interpreter's and native's, `tests/c_emit.rs` `the_paged_loaders_read_every_backends_image`.
- **`S-Pair`** — `tests/scripts/2173-an-image-and-its-sidecar-are-written-together.loft`.
- **`S-Gate`, an image with no sidecar** —
  `tests/scripts/2174-an-image-without-its-sidecar-is-refused.loft` (whole, untrusted and paged
  loaders refused; the pair loads), and `dschema_lifecycle_beside_a_store` (`Bare`).
- **Every rule, across releases** — the GOLDEN IMAGES: `tests/golden/images/<set>/`, one image
  and sidecar per structure and per hash layout, written once by `scripts/golden_images.sh` (it
  refuses a set that exists, so their bytes always come from an earlier build) and read by every
  later one — `tests/scripts/golden-images-whole.loft` through `store_load` on the interpreter,
  `--native` and the C backend, `tests/scripts/golden-images-paged.loft` through the paged
  loaders.  Every other guard writes and reads with one build, so a byte that moves in writer
  and reader alike passes it; these are the guard that does not.  A set is named after the
  placement tokens that wrote it; a token bump adds a new set, and the old set's reader says
  what the new build does with it.
