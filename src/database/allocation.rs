// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)
//! Memory/store allocation helpers and claim management.

use crate::database::{Parts, Stores, WorkerStores};
use crate::hash;
use crate::keys::DbRef;
use crate::radix_db;
use crate::store::Store;
use crate::tree;
use crate::vector;

/// `OpClearKeyed`'s `tp` operand sets this bit when the field being cleared is a
/// SECONDARY VIEW of a linked collection group — a second route to records a
/// sibling field owns (loft#898).
///
/// A flag bit on the type number rather than a new operand: `OpSetKeyed` and
/// `OpReplaceKeyed` already carry `0x8000` on theirs, so the op's arity and both
/// backends' emitters stay unchanged. Type numbers are dense from 0 and the
/// schema is bounded well under 0x8000, which is what makes the top bit free.
pub const CLEAR_KEYED_VIEW: u16 = 0x8000;

/// Which keyed collection a paged working-set load is reading and filling.
///
/// The only thing that differs between the two once a record is located: a `hash`
/// probes a bucket table, a `trie` descends a PATRICIA tree (@PLN134). Everything
/// between — the flat copy, the pointer relocation, the copyability refusal — is
/// shared, so the kind is a parameter rather than a second loader.
#[cfg(paged_store)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PagedKind {
    Hash,
    Trie,
    Radix,
}

// @PLN103 P3 — the `LOFT_STORES=timeline` runtime store timeline. A pure diagnostic
// (gated on the env var), so its state lives thread-local rather than on `Stores` — no
// struct/constructor ripple. A per-logical-store id is `<store_nr>.<seq>`: `store_nr` is a
// REUSED slot index (P0.4), so `seq` (a monotonic alloc counter) disambiguates slot reuse.
// `live` maps each live slot to its current seq so a free prints the SAME id as its alloc.
#[derive(Default)]
struct TimelineState {
    seq: u64,
    live: std::collections::HashMap<u16, u64>,
    peak_live: usize,
    total_alloc: u64,
    total_free: u64,
}

thread_local! {
    static TIMELINE: std::cell::RefCell<TimelineState> = std::cell::RefCell::new(TimelineState::default());
}

/// True when `LOFT_STORES=timeline` (cached per thread — the env is stable for a run).
fn timeline_on() -> bool {
    thread_local! { static ON: bool = std::env::var("LOFT_STORES").as_deref() == Ok("timeline"); }
    ON.with(|b| *b)
}

/// @PLN103 P3.3 — the working-set-vs-leak summary (the disambiguation `LOFT_STORES=warn`
/// cannot make): **peak concurrency** (the working set) vs a real leak. `real_leaked` is the
/// AUTHORITATIVE count from `collect_store_leaks()` (which excludes the eval-stack / const /
/// locked runtime infrastructure the raw timeline `live` map would false-positive on — the
/// interp-vs-native divergence this reconciles). Called at exit; no-op unless timeline mode.
pub fn timeline_summary(real_leaked: usize) {
    if !timeline_on() {
        return;
    }
    TIMELINE.with(|t| {
        let t = t.borrow();
        let verdict = if real_leaked == 0 {
            "NO leak (every user store freed)".to_string()
        } else {
            format!("{real_leaked} user store(s) LEAKED — see the leak warning for which")
        };
        crate::loft_eprintln!(
            "[timeline] SUMMARY: {} allocs, {} frees, peak {} concurrently-live (working set) — {verdict}",
            t.total_alloc, t.total_free, t.peak_live
        );
    });
}

/// One owned nested-heap child of a container record, as enumerated by
/// [`Stores::for_each_owned_child`] — the single per-`Parts` heap-cascade walk
/// that `remove_claims` and `copy_claims_hash_body` read instead of each
/// re-encoding the container layout (loop bounds, element strides, slot drift).
/// The historical `@P290`/`@P306`/`@P318`/`@P309` bugs were all in this walk,
/// hand-copied divergently across the dispatchers; carrying it once is the fix by
/// construction.
///
/// Two other per-`Parts` walkers deliberately do NOT fold onto this keystone — the
/// boundary is load-bearing (H10 / Cluster C), so it is stated here rather than
/// re-derived:
/// - `validate_claims` is a separate DEFENSIVE family, NOT a thin visitor over this
///   walk.  Enumerating a collection's children HERE means dereferencing the
///   container pointer — `length_vector`, `record_words(cur)`, tree navigation — which
///   this keystone TRUSTS (the accessors `debug_assert!` on a freed/out-of-range
///   record).  `validate_claims` runs on *suspected-corrupt* heaps (the `#306`
///   `LOFT_TRACE_CR` pre-walk before `OpCopyRecord`, naming the first broken edge
///   instead of faulting on it), so it bounds-checks each pointer BEFORE following it
///   and does not recurse into the per-element-record kinds (`Array`/`Ordered`/`Hash`/
///   `Index`) at all.  Its `Struct`/`Vector`/`ChildRec`/`Enum` child arithmetic
///   matches this keystone's, but those arms need no bound check and save nothing by
///   folding; the arms that WOULD save code are exactly the ones whose
///   guard-before-deref is the whole point — folding them would turn "name the broken
///   edge" back into "fault on it".  Keep it separate.
/// - `copy_claims`' destination construction is genuinely per-kind (allocate-into-`to`,
///   header writes, re-insert-vs-slot-copy) and is not a walk over this enumeration.
///   Its SOURCE enumeration now reads this keystone in ALL FOUR kinds — `hash_body`,
///   `index_body`, `array_body`, and `seq_vector` (folded H10, 2026-07) — so the
///   per-`Parts` source layout lives in ONE place.  Each helper then pairs a keystone
///   source child with its own per-kind destination build (re-insert for hash/index, a
///   freshly-claimed slot for array, a same-offset position after the bulk copy for
///   vector).
///
/// `child` is the `DbRef` (in the SOURCE record's store) at which the child value
/// lives; `child_tp` is its type.  `owning_elem` is the separate element record
/// that HOLDS this child for the per-element container kinds (`Array`/`Ordered`,
/// `Hash`, `Index`) — `remove_claims` `delete`s it after recursing — and is
/// `None` for the kinds whose children sit inline in the parent / container block
/// (`Struct`, `Vector`/`Sorted` contiguous elements, `Enum` variant re-dispatch,
/// `ChildRec`).
#[derive(Clone, Copy)]
pub(super) struct OwnedChild {
    pub child: DbRef,
    pub child_tp: u16,
    pub owning_elem: Option<u32>,
    /// This child is a SECONDARY VIEW of records a sibling field owns, so the
    /// recursion must drop only its own spine (loft#898).  Set by the `Struct`
    /// arm from `Field.other_indexes` — the one place the schema says which
    /// member of a linked group is the view.  `false` everywhere else: a child
    /// reached through a collection is an element, never a view.
    pub borrowed: bool,
}

/// The container-level teardown a `remove_claims` walk performs AFTER recursing
/// into every owned child: which container/spine record (if any) to `delete`, and
/// whether the field pointer must be zeroed.  Read from the same keystone so the
/// per-kind spine layout lives in ONE place.
///
/// - `container_rec` is the block that backs the collection (the `Vector`/`Array`/
///   `Hash` payload record, or the `ChildRec` child record); `Index` has no
///   separate container block (its nodes ARE the records, freed via
///   `OwnedChild::owning_elem`), so it is `None` there.
/// - `zero_field` is true for the kinds whose value is reached through a heap
///   POINTER stored at `(rec.rec, rec.pos)` (`Vector`/`Sorted`, `Array`/`Ordered`,
///   `Hash`, `Index`, `ChildRec`) — `remove_claims` resets that pointer to 0 so a
///   later reassignment starts clean.  `Struct`/`EnumValue`/`Enum` hold their
///   children INLINE (no pointer to zero), so it is false.
pub(super) struct OwnedWalk {
    pub children: Vec<OwnedChild>,
    pub container_rec: Option<u32>,
    /// Further records the container's storage occupies, freed after the children and
    /// alongside `container_rec`.  A hash's entry arena is the one producer: its
    /// chunks and their directory hold the entries, so a chunk missed here leaks
    /// every entry in it — the class @PLN135 already fixed once for abandoned bucket
    /// tables, and the reason [`crate::hash::arena_records`] is read here rather than
    /// re-derived (@PLN135 arc H).
    pub extra_recs: Vec<u32>,
    pub zero_field: bool,
}

impl Stores {
    /// Is `cur` a record this store could actually contain?
    ///
    /// Every container arm below reaches its children through a raw record number
    /// read out of the record's OWN bytes. `cur != 0` says the field is not empty; it
    /// does not say the field is an edge. A record whose slot was recycled, or one
    /// over-freed while something still pointed at it, holds whatever the previous
    /// tenant left there — and the walk would then follow that: a wild read
    /// (loft#796's SIGSEGV), or, when a stale LENGTH comes with it, a `Vec` sized
    /// from garbage, which is how one run reached 59.6 GiB and had the kernel kill a
    /// bystander.
    ///
    /// So the walk asks whether the edge is inside the store before following it —
    /// the same question the `Parts::Base` text arm already asks of its string
    /// pointer, now asked of every container kind, which is where the fault landed.
    /// A record number indexes WORDS, so the store's word capacity is the bound.
    fn owned_edge_in_store(&self, rec: &DbRef, cur: u32) -> bool {
        cur < self.store(rec).capacity_words()
    }

    /// Is this stored container id the *absent* marker rather than a record to follow?
    ///
    /// loft#917 reserves `DbRef::ABSENT_REC` in a collection field to mean "no collection
    /// here". The walk must read that as an EDGE THAT IS NOT THERE — the same answer it
    /// gives id `0` — and not as a record. Left to the bound check below it fails
    /// `cur < capacity_words()` and reports `BUG (#796)`, so every free of a struct holding
    /// a null collection field would have announced corruption in a correct program.
    fn owned_edge_absent(cur: u32) -> bool {
        cur == DbRef::ABSENT_REC
    }

    /// Report an edge the walk refused to follow, once per site.
    ///
    /// Loud rather than silent: refusing the edge turns a crash into a leak, which is
    /// survivable, but the record is still corrupt and a run that says nothing about
    /// it would be a run that hides its own bug.
    fn refuse_owned_edge(&self, rec: &DbRef, tp: u16, cur: u32, kind: &str) {
        thread_local! {
            static SEEN: std::cell::RefCell<std::collections::HashSet<(u16, u32, u32)>> =
                std::cell::RefCell::new(std::collections::HashSet::new());
        }
        if SEEN.with(|s| s.borrow_mut().insert((rec.store_nr, rec.rec, rec.pos))) {
            let tn = self.types.get(tp as usize).map_or("?", |t| t.name.as_str());
            crate::loft_eprintln!(
                "loft: BUG (#796): refused to walk a {kind} edge of `{tn}` (kt={tp}) at \
                 store #{}, rec={}, pos={} — it points at record {cur}, outside the store \
                 (capacity {} words). The record holds a stale interior claim: its slot was \
                 recycled, or it was freed while something still pointed at it. The edge is \
                 skipped (its records leak) so the run can continue and name the site.",
                rec.store_nr,
                rec.rec,
                rec.pos,
                self.store(rec).capacity_words(),
            );
        }
    }

    /// How many elements of `elem_bytes` can start at `first_byte` in `rec`'s store.
    ///
    /// The companion bound to [`Self::owned_edge_in_store`]: a container pointer can be
    /// inside the store while the LENGTH written beside it is not, and it is the length
    /// that sizes the walk. Reading `n` elements the store cannot hold is the same
    /// fault one step later.
    fn max_elements(&self, rec: &DbRef, first_byte: u32, elem_bytes: u32) -> u32 {
        let capacity_bytes = u64::from(self.store(rec).capacity_words()) * 8;
        let room = capacity_bytes.saturating_sub(u64::from(first_byte));
        if elem_bytes == 0 {
            return 0;
        }
        u32::try_from(room / u64::from(elem_bytes)).unwrap_or(u32::MAX)
    }

    /// The Cluster-C keystone: enumerate the owned nested-heap children of the
    /// record `rec` of type `tp`, plus the container record backing them.  This
    /// is the SINGLE per-`Parts` heap-cascade walk (element type, stride,
    /// container traversal) that `remove_claims` consumes as a thin visitor, and
    /// that `copy_claims_hash_body` reads for its source-bucket enumeration —
    /// replacing the divergent per-dispatcher re-encodings that produced the
    /// `@P290`/`@P306`/`@P318`/`@P309` family.
    ///
    /// Returns the children to recurse on (each carrying its own `DbRef`, type,
    /// and — for per-element kinds — the element record that owns it) and the
    /// container record to free.  Leaf / empty / null shapes yield no children
    /// and no container.  `Radix` is unsupported (callers panic on it); the
    /// keystone returns an empty walk so a non-cascading caller stays safe.
    ///
    /// Collects into a `Vec` (rather than borrowing an iterator) so callers can
    /// take `&mut self` to recurse — the same shape `collect_index_nodes`
    /// already uses.
    pub(super) fn for_each_owned_child(&self, rec: &DbRef, tp: u16) -> OwnedWalk {
        self.owned_walk(rec, tp, false)
    }

    /// What a SECONDARY VIEW of a linked collection group actually owns: its own
    /// spine, and none of the element records (loft#898).
    ///
    /// The spine is per-kind, and it is exactly the `container_rec`/`extra_recs`
    /// the owning walk already names for that kind — so this reads them from the
    /// same layout knowledge rather than a second encoding of it:
    ///
    /// - `Hash` — the table record, plus its arena chunks.  A true secondary
    ///   allocates no arena (`owns_entries` is false, which is precisely what
    ///   makes its slots BORROWED), but the chunks are the table's own storage
    ///   either way, so they are released and the entries are never followed.
    /// - `Array`/`Ordered` — the slot-list block.  Its slots are 4-byte record
    ///   numbers naming records the primary holds; dropping the list drops this
    ///   route to them and nothing more.
    /// - `Index` — nothing to free.  A b-tree's nodes ARE the element records
    ///   (its links live in extra fields of each), so there is no spine block to
    ///   release; zeroing the root pointer is the whole teardown.  The stale
    ///   links left in the surviving records are unreachable once the root is
    ///   gone, and a later insert rewrites them.
    /// - `Vector`/`Sorted` — contiguous storage holds its elements INLINE, so it
    ///   can share them with nothing and is never marked a view.  Falling back to
    ///   the owning walk keeps a mismarked field a leak rather than a silent
    ///   no-op that would drop the field pointer over live records.
    fn borrowed_spine(&self, rec: &DbRef, tp: u16) -> OwnedWalk {
        let empty = |container_rec, extra_recs| OwnedWalk {
            children: Vec::new(),
            container_rec,
            extra_recs,
            zero_field: true,
        };
        match &self.types[tp as usize].parts {
            Parts::Hash(_, _) => {
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur == 0 || Self::owned_edge_absent(cur) || !self.owned_edge_in_store(rec, cur) {
                    return empty(None, Vec::new());
                }
                let extra = if hash::owns_entries(self.store(rec), cur) {
                    hash::arena_records(rec, &self.allocations)
                } else {
                    Vec::new()
                };
                empty(Some(cur), extra)
            }
            Parts::Array(_) | Parts::Ordered(_, _) => {
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur == 0 || Self::owned_edge_absent(cur) || !self.owned_edge_in_store(rec, cur) {
                    return empty(None, Vec::new());
                }
                empty(Some(cur), Vec::new())
            }
            // A trie / spatial view drops the TREE and keeps its leaves, because the
            // leaves are the primary's element records.  One block: the owning arms free
            // exactly `cur` beside the leaves they walk, so the tree's interior nodes
            // live inside it and this releases the whole spine (loft#927 — these became
            // reachable as views the moment a trie could join a group at all).
            Parts::Radix(_, _) | Parts::Trie(_, _) => {
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    return empty(None, Vec::new());
                }
                empty(Some(cur), Vec::new())
            }
            Parts::Index(_, _, _) => empty(None, Vec::new()),
            _ => self.owned_walk(rec, tp, false),
        }
    }

    /// The keystone above, plus the loft#898 `borrowed` mode: enumerate what a
    /// SECONDARY VIEW of a linked collection group owns.
    ///
    /// Two keyed fields over one element type in one struct are two routes to a
    /// single record set (`Field.other_indexes`, loft#843).  The records belong to
    /// exactly ONE of them — the primary — so releasing the view must drop its own
    /// spine (the hash table, the ordered slot list, the b-tree root) and nothing
    /// else.  Freeing them through the view is what left the primary reading
    /// freed memory.
    ///
    /// This rides the SAME per-`Parts` match rather than a walker beside it: the
    /// spine a view drops is the same `container_rec`/`extra_recs` the owning walk
    /// already names, so a layout change cannot move one and miss the other.  The
    /// only difference is that a view yields no children.
    fn owned_walk(&self, rec: &DbRef, tp: u16, borrowed: bool) -> OwnedWalk {
        let mut children = Vec::new();
        let mut container_rec = None;
        let mut extra_recs = Vec::new();
        // A view over records a sibling owns: keep the spine teardown, drop the
        // element walk.  Only the per-element-record kinds can be a view — a
        // contiguous `Vector`/`Sorted` stores its elements INLINE, so it cannot
        // share them with anything and never carries the marker.
        if borrowed {
            return self.borrowed_spine(rec, tp);
        }
        match &self.types[tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                let capacity_bytes = u64::from(self.store(rec).capacity_words()) * 8;
                for f in fields {
                    let pos = rec.pos + u32::from(f.position);
                    // A field offset comes from the TYPE, so a field landing outside the
                    // store means the record is being walked as the wrong type — the
                    // parent's own position is stale, or `tp` does not describe these
                    // bytes.  Either way the offsets below it are fiction, so stop here
                    // rather than hand a fabricated `DbRef` to the recursion.
                    if u64::from(pos) >= capacity_bytes {
                        self.refuse_owned_edge(rec, tp, pos, "struct field");
                        break;
                    }
                    children.push(OwnedChild {
                        child: DbRef {
                            store_nr: rec.store_nr,
                            rec: rec.rec,
                            pos,
                        },
                        child_tp: f.content,
                        owning_elem: None,
                        // loft#898 — a leading `u16::MAX` in `other_indexes` marks this
                        // field as a SECONDARY VIEW of records a sibling field owns
                        // (`types.rs` writes it; the JSON default-init already reads it).
                        // Tearing the struct down must free those records ONCE, through
                        // the primary; the view contributes only its spine.
                        borrowed: f.other_indexes.first() == Some(&u16::MAX),
                    });
                }
            }
            Parts::Vector(v) | Parts::Sorted(v, _) => {
                let v = *v;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 && !Self::owned_edge_absent(cur) && !self.owned_edge_in_store(rec, cur)
                {
                    self.refuse_owned_edge(rec, tp, cur, "vector");
                } else if cur != 0 {
                    let length = vector::length_vector(rec, &self.allocations);
                    let size = u32::from(self.size(v));
                    // The length lives beside the pointer and rots with it; a garbage
                    // count here is what sizes the walk, so bound it by what the store
                    // can physically hold.
                    let room = self.max_elements(rec, 8, size);
                    if length > room {
                        self.refuse_owned_edge(rec, tp, cur, "vector (length)");
                        return OwnedWalk {
                            children,
                            container_rec: Some(cur),
                            extra_recs: Vec::new(),
                            zero_field: true,
                        };
                    }
                    for i in 0..length {
                        children.push(OwnedChild {
                            child: DbRef {
                                store_nr: rec.store_nr,
                                rec: cur,
                                pos: 8 + size * i,
                            },
                            child_tp: v,
                            owning_elem: None,
                            borrowed: false,
                        });
                    }
                    container_rec = Some(cur);
                }
            }
            Parts::Array(v) | Parts::Ordered(v, _) => {
                let v = *v;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 && !Self::owned_edge_absent(cur) && !self.owned_edge_in_store(rec, cur)
                {
                    self.refuse_owned_edge(rec, tp, cur, "array");
                } else if cur != 0 {
                    let length = vector::length_vector(rec, &self.allocations);
                    // Element slots are 4-byte record numbers here, read from the
                    // container — bound the count by what the store can hold.
                    if length > self.max_elements(rec, 8, 4) {
                        self.refuse_owned_edge(rec, tp, cur, "array (length)");
                        return OwnedWalk {
                            children,
                            container_rec: Some(cur),
                            extra_recs: Vec::new(),
                            zero_field: true,
                        };
                    }
                    for i in 0..length {
                        let elm = self.store(rec).get_u32_raw(cur, 8 + i * 4);
                        children.push(OwnedChild {
                            child: DbRef {
                                store_nr: rec.store_nr,
                                rec: elm,
                                pos: 8,
                            },
                            child_tp: v,
                            owning_elem: Some(elm),
                            borrowed: false,
                        });
                    }
                    container_rec = Some(cur);
                }
            }
            Parts::Hash(v, _) => {
                let v = *v;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 && !Self::owned_edge_absent(cur) && !self.owned_edge_in_store(rec, cur)
                {
                    self.refuse_owned_edge(rec, tp, cur, "hash");
                } else if cur != 0 {
                    // Enumerate the live entries through `hash::records`, the single
                    // owner of the bucket-record layout (the seed word, the bucket
                    // offset and the arena decode live only in `src/hash.rs`).
                    // `cur` is the table record itself — a separate container to free.
                    //
                    // @PLN135 arc H — an entry is a SLOT in the arena, not a record of
                    // its own, so there is no per-element record to `delete`:
                    // `owning_elem` is `None` and the storage comes back with the
                    // chunks in `extra_recs`.  Creation and freeing had to change in
                    // one step for exactly this reason — redirecting creation while
                    // this arm still freed each entry as a record would hand interior
                    // arena bytes to `Store::delete` as if they were blocks, which is
                    // silent heap corruption rather than a leak.
                    //
                    // A hash that BORROWS its entries (a secondary index over another
                    // collection's records) is unchanged: its slots still name records
                    // the primary owns, so they come back as `owning_elem` exactly as
                    // before and the arena contributes nothing.
                    // Per ENTRY, because one table can hold both kinds. An entry the
                    // arena handed out is freed with the chunks below and must not be
                    // deleted as a record; a BORROWED one — a record a sibling
                    // collection allocated and still points at — is left completely
                    // alone, since walking into it would free that collection's text
                    // and vectors out from under it.
                    let owned = hash::owns_entries(self.store(rec), cur);
                    for (entry, ours) in hash::entries(rec, &self.allocations) {
                        if owned && !ours {
                            continue;
                        }
                        children.push(OwnedChild {
                            child: entry,
                            child_tp: v,
                            owning_elem: (!ours).then_some(entry.rec),
                            borrowed: false,
                        });
                    }
                    if owned {
                        extra_recs = hash::arena_records(rec, &self.allocations);
                    }
                    container_rec = Some(cur);
                }
            }
            Parts::Index(c, _, _) => {
                let c = *c;
                let left = self.fields(tp);
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 && !Self::owned_edge_absent(cur) && !self.owned_edge_in_store(rec, cur)
                {
                    self.refuse_owned_edge(rec, tp, cur, "index");
                } else if cur != 0 {
                    for node in self.collect_index_nodes(rec, left) {
                        children.push(OwnedChild {
                            child: DbRef {
                                store_nr: rec.store_nr,
                                rec: node,
                                pos: 8,
                            },
                            child_tp: c,
                            owning_elem: Some(node),
                            borrowed: false,
                        });
                    }
                    // No separate container block: index nodes ARE the records.
                }
            }
            Parts::ChildRec(ct) => {
                let ct = *ct;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 && !Self::owned_edge_absent(cur) && !self.owned_edge_in_store(rec, cur)
                {
                    self.refuse_owned_edge(rec, tp, cur, "child record");
                } else if cur != 0 {
                    children.push(OwnedChild {
                        child: DbRef {
                            store_nr: rec.store_nr,
                            rec: cur,
                            pos: 8,
                        },
                        child_tp: ct,
                        owning_elem: None,
                        borrowed: false,
                    });
                    container_rec = Some(cur);
                }
            }
            Parts::Enum(values) => {
                // Inline struct-enum: the live variant's payload sits at the SAME
                // pos (in-place re-dispatch).  `get_byte(.., -1)` shifts so stored
                // byte 1 = variant 0; a null/absent enum reads NEGATIVE and owns no
                // payload (skip rather than index `values` OOB).  A simple
                // (payload-less) variant is marked `u16::MAX`.
                let e_nr = self.store(rec).get_byte(rec.rec, rec.pos, -1);
                if e_nr >= 0 && (e_nr as usize) < values.len() {
                    let vtp = values[e_nr as usize].0;
                    if vtp != u16::MAX {
                        children.push(OwnedChild {
                            child: *rec,
                            child_tp: vtp,
                            owning_elem: None,
                            borrowed: false,
                        });
                    }
                }
            }
            Parts::Radix(v, _) => {
                let v = *v;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 {
                    // The tree's leaves ARE the element records; walk them key-free
                    // (`radix_db::records` → `rtree_first`/`next`).  `cur` is the tree
                    // container — a separate block to free (below).  Mirrors Hash.
                    for elm in radix_db::records(rec, &self.allocations) {
                        children.push(OwnedChild {
                            child: DbRef {
                                store_nr: rec.store_nr,
                                rec: elm,
                                pos: 8,
                            },
                            child_tp: v,
                            owning_elem: Some(elm),
                            borrowed: false,
                        });
                    }
                    container_rec = Some(cur);
                }
            }
            Parts::Trie(v, _) => {
                let v = *v;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 {
                    // Identical SHAPE to the Radix arm — the tree's leaves are the
                    // element records, walked key-free — but through `trie_db`, because
                    // the two kinds share the tree and nothing above it.  Without this
                    // arm a trie fell into the catch-all below and `remove_claims` freed
                    // NOTHING: every element and the tree itself leaked, silently.
                    for elm in crate::trie_db::records(rec, &self.allocations) {
                        children.push(OwnedChild {
                            child: DbRef {
                                store_nr: rec.store_nr,
                                rec: elm,
                                pos: 8,
                            },
                            child_tp: v,
                            owning_elem: Some(elm),
                            borrowed: false,
                        });
                    }
                    container_rec = Some(cur);
                }
            }
            // Base text leaf, scalars, DbRef: no cascade.
            _ => {}
        }
        // The value is reached through a heap pointer (zeroed on teardown) for the
        // collection / child-record kinds; `Struct`/`EnumValue`/`Enum` hold their
        // children inline and have no pointer field to reset.
        let zero_field = matches!(
            self.types[tp as usize].parts,
            Parts::Vector(_)
                | Parts::Sorted(_, _)
                | Parts::Array(_)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Radix(_, _)
                | Parts::Trie(_, _)
                | Parts::Index(_, _, _)
                | Parts::ChildRec(_)
        );
        OwnedWalk {
            children,
            container_rec,
            extra_recs,
            zero_field,
        }
    }

    /// True when `store_nr` is the interpreter's protected eval-stack store
    /// (slot 0 with `stack_store_at_zero` set by `State::new`).  The native
    /// runtime has no stack store — its slot 0 is an ordinary heap store that
    /// must stay freeable and leak-checkable, so every "skip the stack store"
    /// guard must use this predicate instead of a bare `store_nr == 0`
    /// (#490 hid the first leaked native store; #491 made it unfreeable).
    #[must_use]
    pub fn is_stack_store(&self, store_nr: u16) -> bool {
        store_nr == 0 && self.stack_store_at_zero
    }

    /**
    Try to allocate a new store.
    # Panics
    When a store already in use is allocated again.
    */
    pub fn database(&mut self, size: u32) -> DbRef {
        self.database_named(size, "")
    }

    /// Try to allocate a new named store.
    /// # Panics
    /// When a store already in use is allocated again.
    pub fn database_named(&mut self, size: u32, name: &str) -> DbRef {
        // S29: find the lowest free slot using the free_bits bitmap.
        // If a freed slot exists below max, reuse it; otherwise grow max.
        //
        // ARC.md A2: workers spawned by `run_parallel_queue_ref` set
        // `disable_slot_reuse = true` and carry a shared
        // `worker_slot_dispenser` (an `Arc<AtomicU16>`).  Every named
        // alloc consumes the next index from the dispenser, extends
        // the worker's clone's `allocations` to fit, and records the
        // index in `worker_allocated_indices`.  Cross-thread
        // collisions are impossible because the atomic dispenses
        // globally-unique indices; growth is unbounded (replaces
        // 8d.3's fixed 16-slot per-thread cap).
        // ARC.md A2.3 — invariant: if a dispenser is attached, every
        // named allocation MUST route through it.  The dispenser
        // becomes the single source of truth for the worker's
        // parent-namespace indices; bypassing it (e.g. by clearing
        // `disable_slot_reuse` mid-call) would let the worker push
        // into its own clone at an index that collides with another
        // worker's dispensed slot, silently corrupting the swap-back
        // at thread join.
        //
        // Always-on (not gated by `debug_assertions`) because the
        // loft library compiles with `debug-assertions = false` in
        // the test profile (per `[profile.dev.package.loft]`), so a
        // `debug_assert!` here would be silently a no-op in `cargo
        // test`.  This is a slow-path check (one-shot per fresh
        // store allocation), so the perf cost is negligible.
        assert!(
            self.worker_slot_dispenser.is_none() || self.disable_slot_reuse,
            "database_named: worker has a dispenser but disable_slot_reuse \
             was cleared — every dispenser-attached allocation must go \
             through the offset-aware path"
        );
        let slot = if let Some(dispenser) = self.worker_slot_dispenser.clone()
            && self.disable_slot_reuse
        {
            let idx = dispenser.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.worker_allocated_indices.push(idx);
            // Worker's clone may not have a slot at `idx` yet —
            // extend `allocations` (with empty Store::new(100)
            // placeholders for any skipped indices owned by other
            // threads).  Skipped indices stay `free=true` in this
            // worker's clone; only the just-allocated `idx` will be
            // reinitialised below.
            while self.allocations.len() <= idx as usize {
                self.allocations.push(Store::new(100));
            }
            idx
        } else if self.disable_slot_reuse {
            // 8d.2: push at the end of allocations so worker writes
            // never reuse a cloned slot.  Used when no dispenser is
            // attached (single-thread queue dispatch or legacy paths
            // that opt into disable_slot_reuse without queue_ref's
            // dispatcher).
            self.allocations.len() as u16
        } else {
            self.find_free_slot()
        };
        // #306 — slot space is u16 and store_nr 65535 is the null-DbRef
        // sentinel.  Without this cap, `max = slot + 1` wraps to 0 at slot
        // 65535 and the next allocation hands out slot 0 — the eval-stack
        // store — corrupting the whole runtime (SIGSEGV at the next big
        // copy).  Fail loudly instead: hitting this cap means ~65k stores
        // are live at once, which in practice is a store leak.
        assert!(
            slot != u16::MAX,
            "store table exhausted: 65535 stores live at once (store_nr is \
             u16; 65535 is the null sentinel).  This usually indicates a \
             store leak — run with LOFT_STORES=timeline, which lists the live \
             stores by type and says whether they are a LEAK or a large but \
             clean working set."
        );
        if slot >= self.allocations.len() as u16 {
            self.allocations.push(Store::new(100));
        } else {
            // A free slot may still carry a stale lock if set_store_lock was
            // called with a dangling DbRef after the store was freed.  Clear
            // the lock before reinitialising to prevent a spurious panic in
            // Store::init().
            self.allocations[slot as usize].unlock();
            self.reinit_reused_slot(slot as usize);
        }
        // Maintain the invariant `max == highest_allocated_index + 1`.
        // The dispenser path can yield indices > current max (each
        // dispense skips ahead in parent-namespace), so a bare
        // `slot == self.max` check would leave max stale and
        // subsequent free() trims to the wrong position.
        if slot >= self.max {
            self.max = slot + 1;
            // Monotonic watermark: the only site where `max` grows.  `peak`
            // never decrements, so it survives to end-of-run as the store
            // high-water (plan-57).
            if self.max > self.peak {
                self.peak = self.max;
            }
        }
        // Clear the bitmap bit for this slot (it is now active).
        self.clear_free_bit(slot);
        // @PLN101 Slice 0 — the true heap metric: a store slot just went live.
        self.stores_allocated += 1;
        // @PLN105 leak provenance — stamp the currently-executing op position so a
        // leaked store's allocation site is attributable for EVERY path (reuse/copy
        // included), not only `OpDatabase`. Read before the mutable slot borrow.
        let alloc_pc = self.alloc_pc;
        let serial = self.stores_allocated;
        let store = &mut self.allocations[slot as usize];
        // @P317 — enrich the tripwire: this fires when the free-bitmap and the
        // per-store `free` flag disagree (a double-free, an rc under-count, or a
        // store-pool overflow that wrapped `max` to 0 and re-selected slot 0).
        // The slot + rc + type + name pinpoint the victim far faster than the
        // bare message did during the @P311/@P313/@P317 store-ownership hunts.
        assert!(
            store.free,
            "Allocating a used store #{slot} (known_type={}, requested by={})",
            store.known_type,
            if name.is_empty() { "<anon>" } else { name },
        );
        store.free = false;
        store.pinned = false;
        // The monotonic stamp: `stores_allocated` was bumped for THIS allocation just above,
        // so the slot carries a value no other live slot has and no reuse of this one repeats.
        // `State::release_fnref_bufs` compares it against a per-call snapshot to tell a store
        // the callee MINTED from one that was already there (loft#1185).
        store.alloc_serial = serial;
        // @PLN154 phase 3 — the slot's own number, stamped where the slot goes live so a
        // record relocation can name the store it happened in.
        store.store_nr = slot;
        store.set_created_at(alloc_pc);
        store.last_op_at = 0;
        let rec = if size == u32::MAX {
            0
        } else {
            store.claim(size)
        };
        // @PLN133 S8 — while a lazy DRIVER is running, remember every store it
        // creates. A fault inside it abandons its frames, and a frame's locals
        // are freed by the scope-exit code the fault skipped, so this list is
        // what the containment frees instead.
        if let Some(watch) = &mut self.lazy_driver_allocs {
            watch.push(slot);
        }
        let result = DbRef {
            store_nr: slot,
            rec,
            pos: 8,
        };
        // LOFT_STORES=log  → full alloc/free trace
        // LOFT_STORES=warn → only warn when active stores > 30
        let active = self.allocations.iter().filter(|s| !s.free).count();
        match std::env::var("LOFT_STORES").as_deref() {
            Ok("log") => {
                let label = if name.is_empty() { "" } else { name };
                crate::loft_eprintln!(
                    "[store] + alloc #{} {label:>12} | active={active:<4} max={:<4} size={size}",
                    result.store_nr,
                    self.max
                );
            }
            Ok("warn") if active > 30 => {
                crate::loft_eprintln!(
                    "[store] WARNING: {active} active stores (max={}) — possible leak at alloc #{}",
                    self.max,
                    result.store_nr
                );
            }
            Ok("timeline") => {
                let seq = TIMELINE.with(|t| {
                    let mut t = t.borrow_mut();
                    let s = t.seq;
                    t.seq += 1;
                    t.total_alloc += 1;
                    t.live.insert(result.store_nr, s);
                    t.peak_live = t.peak_live.max(t.live.len());
                    s
                });
                let label = if name.is_empty() { "·" } else { name };
                crate::loft_eprintln!(
                    "[timeline] alloc #{}.{seq}  {label:<14} live={active} size={size}",
                    result.store_nr
                );
            }
            _ => {}
        }
        result
    }

    /**
    Free a reference to a store. Make it available again for later code.
    # Panics
    When the code doesn't free the last claimed store first.
    */
    /// Release the store `db` names, and everything in it.
    ///
    /// Enforces @FR-H-Free.  The guards below are the fault rules, in the order they are
    /// checked: @FR-H-FreeNull (freeing the null sentinel is a no-op), @FR-H-FreeStack
    /// (freeing store 0, the eval stack, is refused loudly — a `#306` bug IS a stack-record
    /// ref mistaken for an owned heap store), and @FR-H-FreeTwice.
    ///
    /// ⚠ @FR-H-FreeTwice says a double free is a FAULT, and this function answers it with a
    /// silent no-op.  That is the model, not a divergence: `heap.md` states the corrupting
    /// frees are "not re-checked at runtime — discharged statically" by ownership.md's
    /// checker, so reaching one means the STATIC system already failed.  `LOFT_POISON` is
    /// the empirical cross-check that turns a surviving one into a corrupted read.
    pub fn free(&mut self, db: &DbRef) {
        self.free_named(db, "");
    }

    /// Release the store a binding STOPPED pointing at — the ownership-transition free.
    ///
    /// `displaced` is the store the binding held before a write installed `witness`.  When the
    /// two name one store the write kept it, and there is nothing to release.
    ///
    /// The second guard is the `store(r) ≠ 0` side condition of the rule cited at it: the
    /// eval-stack store is never an owned heap store, so a binding that stopped pointing at
    /// a stack record released nothing.  It is a NO-OP here rather than the loud `#306`
    /// refusal [`Self::free_named`] raises, and the two are not in tension — they answer
    /// different questions.  `free_named` is asked *"release this store"*, and for store 0
    /// the FreeStack rule makes that a FAULT: the caller believed it owned a heap store and
    /// did not.  This function is asked *"release the previous value IF it was ours"*, which
    /// the side condition simply does not license — the same shape [`Self::free_named`]'s
    /// `u16::MAX` sentinel check treats as nothing-to-do.
    ///
    /// A nullable heap local reaches here holding a stack record on the paths where it never
    /// took a store of its own — an ordinary state for it, not a wrong free.  Removing this
    /// guard turns `1085-ret-buffer-passthrough-free` red with a `#306` refusal at
    /// `OpFreeRefIfDistinct`, which is the falsification record for it.
    ///
    /// The third guard is @FR-H-Free's other side condition: a store a CALLER marked
    /// protected-from-free for the duration of a call is not this frame's to release.  A `&`
    /// parameter's write-back displaces whatever the caller's binding named, and the callee
    /// cannot see whether the caller OWNS that store — a plain heap parameter forwarded into a
    /// `&` one aliases a store owned two frames up, and freeing it there is a use-after-free
    /// plus a double free with the real owner's own release (loft#1287).  The call site marks
    /// the store it does not own; this is where the mark is honoured.
    pub fn free_displaced(&mut self, displaced: &DbRef, witness: &DbRef) {
        if displaced.store_nr == witness.store_nr {
            return;
        }
        // @FR-H-Free — `store(r) ≠ 0`.  Guarded by `stack_store_at_zero` for the same reason
        // `free_named`'s `#306` check is: store 0 is only the eval stack once the interpreter
        // has claimed it, and a durable/embedded `Stores` numbers its first real store 0.
        if displaced.store_nr == 0 && self.stack_store_at_zero {
            return;
        }
        if (displaced.store_nr as usize) < self.allocations.len()
            && self.allocations[displaced.store_nr as usize].is_free_protected()
        {
            return;
        }
        self.free_named(displaced, "");
    }

    /**
    Like [`free`], but includes the loft variable name in `LOFT_STORE_LOG` output.
    Generated native code calls this variant via `OpFreeRef(stores, var, "var_name")`.
    */
    pub fn free_named(&mut self, db: &DbRef, name: &str) {
        // u16::MAX is the null-sentinel used by OpNullRefSentinel for inline-ref temporaries
        // that were never assigned a real store.  Nothing to free in this case.
        if db.store_nr == u16::MAX {
            return;
        }
        let al = db.store_nr;
        // `LOFT_TRACE_DB` — the free half of the OpDatabase trace.  `already_free`
        // is the load-bearing column: a free of a slot that is still IN USE means
        // the DbRef reaching this call is not the current owner's, so the slot is
        // about to be handed out while someone still holds it.  Reading the
        // allocation trace alone cannot tell that apart from an ordinary reuse.
        if crate::keys::trace_db() {
            let was_free =
                (al as usize) < self.allocations.len() && self.allocations[al as usize].free;
            eprintln!("[db] free #{al} name={name} already_free={was_free}");
        }
        // #405 — a wrong/stale free can carry an out-of-range store_nr (e.g. a
        // stack-NRVO loop-local's dep slot read on a not-taken branch before it
        // is initialised). An out-of-range nr is never a live store, so refuse it
        // loudly rather than panicking the whole runtime (generalises the #306
        // stack-store guard). Debug builds still trip the debug_assert below so
        // the wrong-free site surfaces for a codegen root fix.
        #[cfg(not(debug_assertions))]
        if al as usize >= self.allocations.len() {
            // Warn once per process — the wrong-free can recur every loop
            // iteration, so a per-occurrence print would flood stderr.
            use std::sync::atomic::{AtomicBool, Ordering};
            static WARNED: AtomicBool = AtomicBool::new(false);
            if !WARNED.swap(true, Ordering::Relaxed) {
                crate::loft_eprintln!(
                    "loft: BUG (#405): refused free of out-of-range store #{al} \
                     (allocations.len={}, rec={}, pos={}, var='{name}') — wrong/stale \
                     ref; further such frees are silently refused this run",
                    self.allocations.len(),
                    db.rec,
                    db.pos,
                );
            }
            return;
        }
        debug_assert!(al < self.allocations.len() as u16, "Incorrect store");
        // #306 — store 0 is the eval-stack store; freeing it destroys every
        // live frame and lets the allocator recycle slot 0 as a heap store.
        // A ref to a stack-allocated record (OpCreateStack) must never be
        // whole-store freed: refuse loudly so the wrong-free site surfaces
        // instead of corrupting the entire runtime.
        if al == 0 && self.stack_store_at_zero {
            // Count it before printing: stderr is where this went to die.  A test harness
            // reads the counter and fails the script that raised it (loft#920).
            crate::keys::note_stack_free_refusal();
            // Name the SITE that attempted it.  `rec`/`pos`/`var` are routinely
            // 0/0/'' here — the wrong free comes from a temp, so they identify
            // nothing, and three attempts on loft#920 were sent down the wrong
            // path by a message that named no site.  A bare `pc=` was not enough
            // either: it is a position in the WHOLE bytecode stream, stdlib
            // included, and `introspect` prints only the user file's, so nothing
            // a reader has in hand can resolve it.  Resolve it here through the
            // same published span table the crash report uses.
            // The NEAREST span is wanted here rather than a covering one: a rough
            // location beats none when the alternative is a bare pc.  It is labelled
            // with the distance back to the entry, because unlabelled it reads as an
            // exact position and the map is sparse enough to be several statements
            // out — inside the stdlib, a different file entirely (loft#1262).
            let at = crate::crash_report::nearest_source_loc_for_pc(self.alloc_pc).map_or_else(
                || format!("pc={}", self.alloc_pc),
                |(recorded_at, p)| {
                    let back = self.alloc_pc.saturating_sub(recorded_at);
                    let how_near = if back == 0 {
                        String::new()
                    } else {
                        format!(", nearest span pc-{back}")
                    };
                    format!(
                        "{}:{}:{} (pc={}{how_near})",
                        p.file, p.line, p.pos, self.alloc_pc
                    )
                },
            );
            // The opcode is exact, so print both: the line orients the reader, the op
            // says what actually ran.
            let op = crate::crash_report::last_op_name();
            let op = if op.is_empty() {
                String::new()
            } else {
                format!(", op={op}")
            };
            crate::loft_eprintln!(
                "loft: BUG (#306): refused to free the stack store (#0) \
                 (rec={}, pos={}, var='{name}', at {at}{op}) — a stack-record ref was \
                 treated as an owned heap store",
                db.rec,
                db.pos,
            );
            // Record it as a free site even though the free was REFUSED: the
            // refusal keeps store 0 alive, but whatever produced this ref is
            // still wrong, and a later strict-stores access report naming this
            // PC is what ties the two halves together.  Returning before the
            // `strict_note_free` below meant `LOFT_STRICT_STORES=1` — the
            // instrument this fault's own message recommends — could never see
            // the one free that matters.
            if crate::keys::strict_stores() {
                crate::keys::strict_note_free(al, self.alloc_pc, name);
            }
            return;
        }
        // @PLN130 F8 — remember WHERE a store died, so a later access through a stale
        // reference can name the free that killed it rather than only the corpse.
        if crate::keys::strict_stores() && !self.allocations[al as usize].free {
            crate::keys::strict_note_free(al, self.alloc_pc, name);
        }
        let store = &mut self.allocations[al as usize];
        if store.free {
            return; // Already freed — no-op (replaces Issue #120 tolerance hack).
        }
        // Plan-57 Phase C: const/global stores are PINNED — never freed (they
        // live for the whole program).  This replaces the `ref_count = u32::MAX/2`
        // sentinel + the `ref_count > 1` guard below as the ref-count is removed.
        if store.pinned {
            return;
        }
        // Plan-57 Phase C: the Stores ref-count is removed.  Every non-pinned
        // store is single-owner (closure-captured cells are owned by the closure
        // record's cascade, not rc — see Phase B), so `free_named` always frees.
        // (Pinned const/global stores returned above.)
        // P259 commit 4: cascade-free closure-record DbRef attributes.
        // When the store being freed holds a `__closure_*` record, each
        // ADOPTED `DbRef` field references a store the record is the sole
        // owner of: the closure's captured `__cell_<T>` (C74 limits a mutated
        // cell to one capturing closure), or a `Reference` / collection
        // capture the defining frame owned and handed over — the frame's own
        // `OpFreeRef` is suppressed for exactly those (`scopes.rs`
        // `captured_ref`), so this cascade is their single free, and that is
        // what lets an escaping factory closure outlive the frame (#323).
        // Walk those fields, read each 12-byte stored DbRef, and recursively
        // free_named.  There is no ref-count (plan-57 phase C removed it):
        // when the target was already freed the recursive call hits the
        // `store.free` no-op above.
        //
        // A BORROWED capture (`dbref_borrow`, #682) is skipped: its store
        // belongs to a parameter's caller or to the vector a projection local
        // views into, both of which outlive this record.  Freeing it here
        // handed the caller a dangling World and surfaced as a panic thousands
        // of ops later in whatever function next touched it.
        //
        // Gated on the type name's `__closure_` prefix because:
        // - Only closure records hold cells via Parts::DbRef.
        // - User code can't define identifiers with `__` prefix
        //   (loft parser rejects), so the prefix check is leak-free.
        // - Cascading every Parts::DbRef field would break P213
        //   ChildRec storage and any future DbRef-holding struct.
        let cascade_targets: Vec<DbRef> = {
            let store_ref = &self.allocations[al as usize];
            let known_type = store_ref.known_type;
            if known_type != u16::MAX
                && self.types[known_type as usize]
                    .name
                    .starts_with("__closure_")
            {
                let dbref_positions: Vec<u16> =
                    if let Parts::Struct(fields) = &self.types[known_type as usize].parts {
                        fields
                            .iter()
                            .filter(|f| self.dbref_is_adopted(f.content))
                            .map(|f| f.position)
                            .collect()
                    } else {
                        Vec::new()
                    };
                dbref_positions
                    .iter()
                    .map(|&fpos| {
                        let off = db.pos + u32::from(fpos);
                        let store_nr = store_ref.get_u32_raw(db.rec, off) as u16;
                        let rec = store_ref.get_u32_raw(db.rec, off + 4);
                        let pos = store_ref.get_u32_raw(db.rec, off + 8);
                        DbRef { store_nr, rec, pos }
                    })
                    .collect()
            } else {
                Vec::new()
            }
        };
        match std::env::var("LOFT_STORES").as_deref() {
            Ok("log") => {
                let active = self.allocations.iter().filter(|s| !s.free).count();
                let label = if name.is_empty() { "" } else { name };
                crate::loft_eprintln!(
                    "[store] - free   #{al} {label:>12} | active={:<4} max={}",
                    active,
                    self.max
                );
            }
            Ok("timeline") => {
                // @PLN103 P3 — print the SAME `<store_nr>.<seq>` id the alloc printed, so
                // a reader matches alloc↔free across slot reuse. `?` = a free with no live
                // alloc record (a double-free or a pre-timeline store).
                let (seq, live_after) = TIMELINE.with(|t| {
                    let mut t = t.borrow_mut();
                    t.total_free += 1;
                    let s = t.live.remove(&al);
                    (s, t.live.len())
                });
                let seq = seq.map_or_else(|| "?".to_string(), |s| s.to_string());
                let label = if name.is_empty() { "·" } else { name };
                crate::loft_eprintln!(
                    "[timeline] free  #{al}.{seq}  {label:<14} live={live_after}"
                );
            }
            _ => {}
        }
        // S36: clear the lock before marking free.
        let store = &mut self.allocations[al as usize];
        store.unlock();
        // LOFT_POISON=1 (@PLN54 S3) or LOFT_LOG=poison_free: overwrite the
        // freed buffer with a recognisable pattern so subsequent stale-DbRef
        // reads hit loud garbage (0xDEADBEEF repeated) instead of whatever
        // bytes the allocator happens to have left.  Skip the size-header
        // word (offset 0..8) so the bitmap/housekeeping can still read the
        // "freed" marker; start poisoning from offset 8.  The env flag is the
        // dedicated front door (works on BOTH backends — native calls this
        // same free); the struct flag is the LOFT_LOG=poison_free path.
        // A FILE-BACKED store's memory IS the mmap'd file: poisoning it
        // persists 0xDEADBEEF into durable state (the store_persist reload
        // read back an empty hash under LOFT_POISON).  The detector targets
        // in-memory stale reads; durable bytes are out of its scope.
        if (self.poison_free || crate::keys::poison_enabled()) && !store.is_file_backed() {
            let cap_bytes = store.capacity_words() as usize * 8;
            if cap_bytes > 8 {
                unsafe {
                    let base = store.ptr.add(8);
                    // Write 0xDEADBEEF to every i32-aligned word past the
                    // size header.  Use a byte-level loop to avoid worrying
                    // about alignment requirements on the raw pointer.
                    const POISON: [u8; 4] = [0xEF, 0xBE, 0xAD, 0xDE];
                    for off in 0..(cap_bytes - 8) {
                        *base.add(off) = POISON[off & 3];
                    }
                }
            }
        }
        // loft#752 — a BOUND store's file IS the live arena, so give the tail
        // back at the moment the binding ends.  The arena grows by 7/3 and
        // never shrinks on its own, so the file left behind is a rung on a
        // ladder rather than a measure of its content: it can sit 57% above
        // what it holds, two builds a rung apart differ by 133% with identical
        // records, and 40 000 and 60 000 features produced byte-identical
        // files.  loft#710 decided a persisted store's size must follow its
        // content and fixed the IMAGE-write path; this is the same decision on
        // the path that fix never reached, and the reported cost of not making
        // it was a consumer concluding that key ORDER cost them 2.3x when both
        // of their numbers were rungs.
        //
        // `store_reclaim`'s rule — the runtime cannot tell a permanent drop
        // from a lull, so the PROGRAM says when (@PLN123 A3) — is about the
        // middle of a run, where a reclaim can be paid back at 7/3 by the next
        // claim.  Here there is no next claim: the store is being released.
        // That is the one moment the runtime does know, so a generator's output
        // file is content-sized whether or not its author has heard of
        // `store_reclaim`.
        //
        // It must run BEFORE `free = true`, which is what `usage()` reads to
        // early-out — after it, the chain walk reports mark 0 / incomplete and
        // `shrink_to` (rightly) refuses.  `shrink_to` also declines a read-only
        // store, an incomplete walk, and one carrying a durable `.dmeta`
        // sidecar whose recorded length and CRC must not be truncated behind
        // its back, so every case this must not touch says no inside that call.
        if store.is_file_backed() && !store.read_only {
            store.reclaim_tail();
        }
        store.free = true;
        // LOFT_UAF: record the freed slot so the dispatch loop can scan, after
        // this op, for live variables that still read it (a premature free).
        if crate::keys::uaf_any_enabled() {
            self.uaf_freed_this_op.push(al);
        }
        // LOFT_UAF_GEN (c): bump the slot's generation so a DbRef minted at the old
        // gen is detectably stale if read after this free (+ any later reuse).
        if crate::keys::uaf_gen_enabled() {
            crate::keys::uaf_bump_gen(al);
        }
        // S29: mark slot as free in the bitmap so database_named()
        // can reuse it without LIFO ordering.
        self.set_free_bit(al);
        self.trim_free_top(al);
        // P259 commit 4: cascade-free the captured-cell DbRefs collected
        // above.  Done AFTER the closure record's own free so that
        // a recursive cascade on a closure-record cell sees this slot
        // as already-freed and does not re-enter.  Skip the null
        // sentinel pattern (store_nr=0, rec=0) which is the default
        // value written by `set_default_value` for unset DbRef fields.
        for target in cascade_targets {
            if target.store_nr != 0 || target.rec != 0 {
                self.free_named(&target, "<cascade>");
            }
        }
    }

    /// S29: Find the lowest free slot index below `max` using the `free_bits` bitmap.
    /// Returns `self.max` when no freed slot is available (caller must grow the Vec).
    fn find_free_slot(&self) -> u16 {
        // @PLN118 arc D — LOFT_NO_SLOT_REUSE=1: never reclaim a freed slot; always
        // grow. A diagnostic stopgap that proves whether the corruption is
        // slot-reuse-while-referenced (if the null vanishes, it is). Off by default.
        if crate::keys::no_slot_reuse() {
            return self.max;
        }
        for (wi, &word) in self.free_bits.iter().enumerate() {
            if word != 0 {
                let bit = word.trailing_zeros() as u16;
                let slot = wi as u16 * 64 + bit;
                if slot < self.max {
                    return slot;
                }
            }
        }
        self.max
    }

    /// S29: Set bit `slot` in `free_bits`, growing the Vec as needed.
    fn set_free_bit(&mut self, slot: u16) {
        let wi = slot as usize / 64;
        let bi = slot as usize % 64;
        while self.free_bits.len() <= wi {
            self.free_bits.push(0);
        }
        self.free_bits[wi] |= 1u64 << bi;
    }

    /// Give a slot borrowed via [`adopt_store`](Stores::adopt_store) back to
    /// the pool.  [`take_store`](Stores::take_store) deliberately does NOT — it
    /// is written for a store handed out to OUTLIVE the table (the REPL session
    /// store), where the slot should stay reserved — so a caller using a slot
    /// as scratch has to say so.  Pinned by `slot_recycling_tests`.
    pub(crate) fn release_slot(&mut self, slot: u16) {
        self.set_free_bit(slot);
        self.trim_free_top(slot);
    }

    /// Bring `max` back down when `slot` was the top one and is now free.
    ///
    /// `max` is the watermark `database_named` grows the table from, so a slot
    /// released at the top has to lower it or the table only ever climbs. The
    /// walk continues downward because several top slots can be free at once —
    /// a call that borrowed two of them (@PLN119's call arena) releases them one
    /// at a time, and stopping after the first would leave the other stranded
    /// above the mark.
    ///
    /// Ordinarily invisible: the free bitmap hands a released slot straight back,
    /// so a table that never came down still allocated nothing extra. It becomes
    /// load-bearing under `LOFT_NO_SLOT_REUSE` / `LOFT_STRICT_STORES`, where a
    /// released slot is deliberately never recycled and `max` is the ONLY thing
    /// that can come down — without this, a loop calling a placed library ran out
    /// of the 65535 store slots after ~32k iterations while the same loop
    /// in-process sat flat at four.
    /// The walk stops on the free BITMAP rather than on the store's own `free`
    /// flag, and the two are not the same question. `take_store` leaves a
    /// sentinel that IS flagged free but whose bit stays clear on purpose — the
    /// slot is handed out to outlive the table, so its number is reserved. Trim
    /// past one of those and the next allocation lands on a number somebody
    /// still holds.
    fn trim_free_top(&mut self, slot: u16) {
        if self.max > 0 && slot == self.max - 1 {
            self.max -= 1;
            while self.max > 0 && self.slot_bit_free(self.max - 1) {
                self.max -= 1;
            }
        }
    }

    /// Is `slot` marked available in the free bitmap — the authoritative record
    /// of "this number may be handed out"?
    fn slot_bit_free(&self, slot: u16) -> bool {
        let (wi, bi) = (slot as usize / 64, slot as usize % 64);
        self.free_bits
            .get(wi)
            .is_some_and(|w| w & (1u64 << bi) != 0)
    }

    /// S29: Clear bit `slot` in `free_bits` (slot is now active).
    fn clear_free_bit(&mut self, slot: u16) {
        let wi = slot as usize / 64;
        let bi = slot as usize % 64;
        if wi < self.free_bits.len() {
            self.free_bits[wi] &= !(1u64 << bi);
        }
    }

    /// @PLN16 M1a — the lowest store-slot index guaranteed unused: above every
    /// allocated slot *and* the `max` watermark.  A value built at or above this floor
    /// occupies slots that are free here, which is what lets the debugger's whole-value
    /// heap edit graft it in with **no `DbRef` remap**.
    #[must_use]
    pub fn high_water(&self) -> u16 {
        (self.allocations.len() as u16).max(self.max)
    }

    /// @PLN16 M1a — force this (throwaway *build*) `Stores`' next store allocations to
    /// land at slot `floor` and above.  Pads `allocations` to `floor` with in-use
    /// placeholders and marks every slot below `floor` in-use, so [`find_free_slot`]
    /// returns `floor` and `database_named` pushes there.  Used by the debugger heap
    /// edit so the value's stores occupy slots that are free in the *live* paused
    /// store, enabling the no-remap graft ([`adopt_value_stores`](Self::adopt_value_stores)).
    /// The placeholders are never read — the edit's literal is self-contained, so the
    /// constructor only touches its own (real, recompiled) const stores and the new
    /// value-stores — they exist only so the slot indices line up.
    pub fn raise_floor(&mut self, floor: u16) {
        while (self.allocations.len() as u16) < floor {
            let mut placeholder = Store::new(2);
            placeholder.free = false; // in-use marker; bytes are never read
            self.allocations.push(placeholder);
        }
        // Clearing every free bit makes `find_free_slot` return `max`; setting `max =
        // floor` (== allocations.len()) makes the next claim a push at exactly `floor`.
        self.free_bits.clear();
        self.max = floor;
        if self.max > self.peak {
            self.peak = self.max;
        }
    }

    /// @PLN16 M1a — graft the value-stores a debugger heap edit built on `build` into
    /// this (live paused) `Stores`.  `build` raised its floor above this store's
    /// high-water then constructed the new value there, so every value-store sits on a
    /// slot that is **free here** — the move needs no `DbRef` remap (the root and its
    /// whole internal graph keep their slot numbers, valid here unchanged).
    ///
    /// Each in-use `build` slot in `[floor, build.max)` is moved here at the same index
    /// (its `Drop` defused by swapping in a freed sentinel); a slot `build` claimed then
    /// freed mid-construction is skipped (it is free here too).  Slots below `floor`
    /// that this store lacks are padded with free sentinels so indices line up.
    pub fn adopt_value_stores(&mut self, build: &mut Stores, floor: u16) {
        let top = build.allocations.len() as u16;
        while (self.allocations.len() as u16) < top {
            let idx = self.allocations.len() as u16;
            self.allocations.push(Store::new_freed_sentinel());
            self.set_free_bit(idx); // a slot this store never had is free here
        }
        for slot in floor..top {
            let si = slot as usize;
            if build.allocations[si].free {
                continue; // a transient the constructor claimed then freed
            }
            let moved = std::mem::replace(&mut build.allocations[si], Store::new_freed_sentinel());
            self.allocations[si] = moved;
            self.clear_free_bit(slot);
            if slot + 1 > self.max {
                self.max = slot + 1;
                if self.max > self.peak {
                    self.peak = self.max;
                }
            }
        }
    }

    /// Tell the memory ceiling what every store type is called, so a refused growth
    /// can name the type that filled the heap instead of printing a bare id.
    ///
    /// A whole-schema sweep rather than a hook on each naming site: the schema is
    /// small, the sweep runs once per program, and it cannot miss a type the way a
    /// per-site hook can.  Inert when no ceiling is set — there is no report to build
    /// then, so there is nothing to pay for.
    pub fn publish_type_names(&self) {
        if crate::store_budget::limit() == 0 {
            return;
        }
        for (kt, t) in self.types.iter().enumerate() {
            if let Ok(kt) = u16::try_from(kt) {
                crate::store_budget::note_type_name(kt, &t.name);
            }
        }
    }

    /// Collect a description for every leaked store at program exit, grouped BY TYPE,
    /// most-leaked first.
    ///
    /// Mirrors `State::collect_store_leaks` (which operates on the
    /// interpreter's `State`) but lives on `Stores` so the **native**
    /// runtime can run the same check — the generated `main` bootstrap
    /// calls this when `LOFT_NATIVE_LEAK_CHECK` is set so leak
    /// regressions surface on `--native` as well as `--interpret`.
    /// Same filtering: skip the stack store when one occupies slot 0
    /// (`stack_store_at_zero` — interp only; the native runtime's
    /// slot 0 is an ordinary heap store and MUST be checked, #490),
    /// locked constants / worker borrows, and `const_refs`.
    ///
    /// @P317 — the previous per-store `N(bc:created_at)` listing (truncated to 5 by
    /// the leak-check preview) buried the signal in store numbers; naming the culprit
    /// directly (e.g. `kt=68 ChunkKey×6026`) is what pinpointed the @P317 native
    /// ref-local leak.  Used by `LOFT_NATIVE_LEAK_CHECK` (native) and
    /// `LOFT_STORES=timeline` (interp).
    #[must_use]
    pub fn collect_store_leaks(&self) -> Vec<String> {
        let mut by_type: std::collections::BTreeMap<(u16, &str), usize> =
            std::collections::BTreeMap::new();
        for (s_nr, s) in self.allocations.iter().enumerate() {
            if s_nr == 0 && self.stack_store_at_zero {
                continue; // stack store — always alive
            }
            if s.is_locked() || self.const_refs.iter().any(|cr| cr.store_nr == s_nr as u16) {
                continue;
            }
            if !s.free {
                let tn = self
                    .types
                    .get(s.known_type as usize)
                    .map_or("?", |t| t.name.as_str());
                *by_type.entry((s.known_type, tn)).or_default() += 1;
            }
        }
        let mut leaked: Vec<((u16, &str), usize)> = by_type.into_iter().collect();
        leaked.sort_by_key(|&(_, n)| std::cmp::Reverse(n)); // most-leaked first
        leaked
            .into_iter()
            .map(|((kt, tn), n)| format!("kt={kt} {tn}×{n}"))
            .collect()
    }

    /**
    Validate if a reference is already freed before.
    # Panics
    When the store was already freed before.
    */
    pub fn valid(&self, db: &DbRef) {
        if db.store_nr == u16::MAX {
            return; // null-sentinel: never allocated, always valid-as-null
        }
        debug_assert!(
            db.store_nr < self.allocations.len() as u16,
            "Incorrect store"
        );
        // Note: accessing a freed store can still happen when a closure captures
        // a variable whose store was freed by copy_record's source-free.
        // The rc system prevents double-free; this access is benign (reads stale data
        // that will be overwritten).  A full fix requires inc_rc on closure capture.
    }

    /// Re-initialise a REUSED slot to a clean empty store.  If the slot still
    /// holds a file-backed (mmap) store from a `store_persist_bind`, replace it
    /// with a fresh anonymous store instead of `init()`-ing THROUGH the mmap:
    /// `init()` writes the empty-store header into the mapped file, blanking the
    /// persisted store's in-memory view so a later bind into the reused slot
    /// reads an empty hash (the on-disk bytes, unsynced, survive — so a fresh
    /// process still reads the data).  Dropping the old store flushes + closes
    /// the mmap.  #513.
    fn reinit_reused_slot(&mut self, slot: usize) {
        if self.allocations[slot].is_file_backed() {
            self.allocations[slot] = Store::new(100);
        } else {
            self.allocations[slot].init();
        }
    }

    pub fn clear(&mut self, db: &DbRef) {
        let slot = db.store_nr;
        // Clear any stale lock before reinitialising — OpDatabase may
        // reinitialise a store that was previously locked by a const
        // parameter in a prior function call within the same loop iteration.
        // never unlock a PINNED (const/global) store.
        if !self.allocations[slot as usize].pinned {
            self.allocations[slot as usize].unlock();
        }
        // OpDatabase may adopt a store its variable freed at the end of the
        // previous loop iteration (the slot still holds the stale DbRef).
        // Adoption IS ownership: mark the store in use and unlink it from
        // the free bitmap, or `find_free_slot` hands the SAME slot to the
        // next fresh allocation — two owners, and the second's writes wipe
        // the first's record (#348: a File record clobbered by a sibling
        // call's result vector).  #513: a file-backed slot is replaced, not
        // init()'d through the mmap (see reinit_reused_slot).
        self.reinit_reused_slot(slot as usize);
        self.allocations[slot as usize].free = false;
        self.clear_free_bit(slot);
        // free_named's top-slot trim may have dropped `max` BELOW this slot
        // (the stale frame ref outlives the watermark).  Restore it, or
        // find_free_slot returns `max` == this very slot and the fresh-alloc
        // path `init()`s the just-adopted live store.
        if slot >= self.max {
            self.max = slot + 1;
            if self.max > self.peak {
                self.peak = self.max;
            }
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn type_claim(&self, tp: u16) -> u32 {
        u32::from(self.types[tp as usize].size).div_ceil(8)
    }

    pub fn claim(&mut self, db: &DbRef, size: u32) -> DbRef {
        let store = &mut self.allocations[db.store_nr as usize];
        let rec = store.claim(size);
        DbRef {
            store_nr: db.store_nr,
            rec,
            pos: 8,
        }
    }

    #[must_use]
    pub fn null(&mut self) -> DbRef {
        self.database(u32::MAX)
    }

    /// Like [`null`], but includes the loft variable name in `LOFT_STORE_LOG` output.
    /// Generated native code calls this for each `DbRef` variable declaration.
    pub fn null_named(&mut self, name: &str) -> DbRef {
        self.database_named(u32::MAX, name)
    }

    /// `#[inline]` because generated `--native` code calls this for every element read
    /// across a crate boundary with no LTO — see `vector::get_vector`. Inlining also
    /// lets the `strict_stores()` check below hoist out of a loop instead of being
    /// re-tested per element.
    #[must_use]
    #[inline]
    pub fn store(&self, r: &DbRef) -> &Store {
        let s = &self.allocations[r.store_nr as usize];
        // @PLN130 F8 — under LOFT_STRICT_STORES the slot is never recycled, so a freed
        // store here is an unambiguous use-after-free rather than a maybe-reused slot.
        // Available in release builds too: the debug-only print below never ran in the
        // release binary the probes use, which is why a live UAF read as ordinary data.
        if crate::keys::strict_stores() && s.free {
            crate::keys::strict_store_violation(
                r.store_nr,
                r.rec,
                r.pos,
                "read",
                self.types
                    .get(s.known_type as usize)
                    .map_or("?", |t| t.name.as_str()),
                s.created_at,
                s.last_op_at,
                self.alloc_pc,
            );
        }
        #[cfg(debug_assertions)]
        if s.free && !crate::keys::strict_stores() {
            crate::loft_eprintln!(
                "[store] ACCESS FREED store #{} rec={} pos={} — data will be garbage",
                r.store_nr,
                r.rec,
                r.pos
            );
        }
        s
    }

    /// C60 Step 3 (path 2c, piece 1): build a fresh vector of u32
    /// rec-nrs from the hash's records, sorted ascending by key.
    ///
    /// Called by the `on=4` hash-iteration arm in `OpIterate` at
    /// runtime.  The returned DbRef points at a header record whose
    /// offset-4 word is the data-record number; the data record's
    /// offset-4 word is the element count (n), and offset 8 onwards
    /// holds n `u32` rec-nrs at 4-byte stride.
    ///
    /// **Layout matches `Ordered`-style vectors** (see
    /// `src/state/io.rs:777` and `src/vector.rs:448`) — that's why the
    /// `step` handler for on=4 can walk this vector with the same
    /// u32-stride logic Ordered uses, yielding
    /// `DbRef{store=hash_store, rec=<u32>, pos=8}` per iteration.
    ///
    /// Note: `elem_store` is NOT encoded in the scratch; the runtime
    /// retains the original hash's `store_nr` via the companion
    /// iterator-local allocated by `parse_for_iter_setup`.
    #[allow(dead_code)]
    pub fn build_hash_sorted_vec(&mut self, hash_ref: &DbRef, tp: u16) -> DbRef {
        // A holder with no record — reached through `nullref` — holds no collection, and
        // has no store to build a scratch in: the scratch is `nullref` too, which `iterate`,
        // `step` and `free_iteration_scratch` all read as empty (the same test opens each).
        // Collecting the records first resolved `allocations[u16::MAX]`.
        if hash_ref.rec == 0 {
            return DbRef::NULL;
        }
        let keys = self.types[tp as usize].keys.clone();
        let recs = crate::hash::records_sorted(hash_ref, &self.allocations, &keys);
        self.build_ref_scratch(hash_ref, &recs)
    }

    /// Like `build_hash_sorted_vec` but in raw bucket-walk order, skipping the
    /// O(n log n) key sort.  Used to feed `for e in h par(...)`: the parallel
    /// queue preserves input order, but a hash has no user-meaningful order, so
    /// sorting the records only to hand them straight to worker threads is
    /// wasted work.  Iteration order therefore differs from sequential
    /// `for e in h` (which is key-ordered) — acceptable for a hash.
    pub fn build_hash_unsorted_vec(&mut self, hash_ref: &DbRef, _tp: u16) -> DbRef {
        // An absent holder has no scratch — see `build_hash_sorted_vec`.
        if hash_ref.rec == 0 {
            return DbRef::NULL;
        }
        let recs = crate::hash::records(hash_ref, &self.allocations);
        self.build_ref_scratch(hash_ref, &recs)
    }

    /// @PLN48 — the Radix counterpart, feeding the same Ordered (on=3) iteration
    /// path via `build_rec_scratch`.  Unlike a hash, a radix tree has a **natural
    /// order** (its in-order walk is key order — Morton/Z-order for a spatial
    /// index), so `radix_db::records` already yields the records sorted: no O(n log n)
    /// key sort, just the O(n) tree walk.  The `tp` is unused for the same reason.
    pub fn build_radix_sorted_vec(&mut self, coll: &DbRef, _tp: u16) -> DbRef {
        // An absent holder has no scratch — see `build_hash_sorted_vec`.
        if coll.rec == 0 {
            return DbRef::NULL;
        }
        let recs = crate::radix_db::records(coll, &self.allocations);
        self.build_rec_scratch(coll, &recs)
    }

    /// @PLN105 Phase 3 — collect an `index<T[key]>` (red-black tree) in ascending key order into an
    /// iterable scratch rec-nr vector, for the deliver pre-flatten. Mirrors `tree::count`'s in-order
    /// walk (`first` → `next`), keyed off the node type's left-child BYTE position
    /// (`Stores::fields`); result iterated via the same Ordered (on=3) path as the hash/radix
    /// scratches.
    pub fn build_index_sorted_vec(&mut self, coll: &DbRef, tp: u16) -> DbRef {
        // An absent holder has no scratch — see `build_hash_sorted_vec`.
        if coll.rec == 0 {
            return DbRef::NULL;
        }
        let left = self.fields(tp);
        if left == u16::MAX {
            return self.build_rec_scratch(coll, &[]);
        }
        let recs = {
            let store = crate::keys::store(coll, &self.allocations);
            let mut recs = Vec::new();
            let mut cur = tree::first(coll, left, &self.allocations).rec;
            while cur != 0 {
                recs.push(cur);
                let r = DbRef {
                    store_nr: coll.store_nr,
                    rec: cur,
                    pos: u32::from(left),
                };
                cur = tree::next(store, &r);
            }
            recs
        };
        self.build_rec_scratch(coll, &recs)
    }

    /// @PLN48 S3 — a `spatial` range slice as an iterable scratch vector, feeding the
    /// same Ordered (on=3) path as `build_radix_sorted_vec`.  Records whose Morton code
    /// lies in `[from, till]` (or `[from, ∞)` when `has_till == 0`), in natural order,
    /// capped at `limit` (`< 0` = no cap).  Backs `xs[(x,y)..]`, `xs[(x,y)..:n]`, and the
    /// bounding box `xs[(x1,y1)..(x2,y2)]`.  Coordinates arrive as a fixed `MAX_AXES`-wide
    /// triple; only the collection's own `keys.len()` axes are read (a 2D collection
    /// ignores `fz`/`tz`), so the same ABI serves 1D…3D slices.
    /// A trie PREFIX slice as an iterable scratch vector, the trie's twin of
    /// `build_radix_range_vec` and feeding the same on=4 scratch path.  Every record
    /// whose key begins with `pre`, in key order, capped at `limit` (`< 0` = no cap).
    /// Backs `t["kerk"..]` and `t["kerk"..:n]`.
    ///
    /// There is no `till` here and that is the point: a prefix is the whole query, so
    /// the caller never constructs the successor string a `sorted` range would need.
    pub fn build_trie_prefix_vec(&mut self, coll: &DbRef, tp: u16, pre: &str, limit: i64) -> DbRef {
        // An absent holder has no scratch — see `build_hash_sorted_vec`.
        if coll.rec == 0 {
            return DbRef::NULL;
        }
        let keys = self.types[tp as usize].keys.clone();
        let cap = (limit >= 0).then_some(limit as usize);
        let recs = crate::trie_db::prefix(coll, &self.allocations, &keys, pre.as_bytes(), cap);
        self.build_rec_scratch(coll, &recs)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_radix_range_vec(
        &mut self,
        coll: &DbRef,
        tp: u16,
        fx: i64,
        fy: i64,
        fz: i64,
        has_till: i64,
        tx: i64,
        ty: i64,
        tz: i64,
        limit: i64,
    ) -> DbRef {
        // An absent holder has no scratch — see `build_hash_sorted_vec`.
        if coll.rec == 0 {
            return DbRef::NULL;
        }
        let keys = self.types[tp as usize].keys.clone();
        let n = keys.len().min(crate::radix_db::MAX_AXES);
        let from = [fx, fy, fz];
        let till = [tx, ty, tz];
        let cap = (limit >= 0).then_some(limit as usize);
        // Two different queries share this entry point, and `has_till` is the fact
        // that tells them apart. A CLOSED box promises containment (loft#800); the
        // OPEN forms (`xs[(x,y)..]`, `xs[(x,y)..:n]`) promise an outward walk from the
        // point, which is what four docs and the catalogue entry all describe.
        //
        // @PLN136 — the box form walks the box rather than the code interval between
        // its corners. The interval is a superset that Z-order threads in and out of:
        // reading it and filtering costs every record in it, which on the degenerate
        // shapes a map issues is 1.46 M records read to return 4 k. `box_range` seeks
        // over each gap instead, and its cap bounds the WALK, so the two reasons the
        // old composition had to cap AFTER filtering are both gone.
        //
        // loft#1002 — the open forms used to route to `range`, the one-directional walk,
        // so they answered the Z-order TAIL: only records whose code is >= the query's.
        // Half the neighbourhood was structurally unreachable, `..:n` under-delivered by
        // however close the query sat to the end of the curve, and a query past every
        // record answered nothing at all. `near_range` is the two-cursor walk the surface
        // was always described as, and the outward walk that already existed in
        // `spatial::near` with no way for a loft program to reach it.
        let recs = if has_till != 0 {
            crate::radix_db::box_range(coll, &self.allocations, &keys, &from[..n], &till[..n], cap)
        } else {
            crate::radix_db::near_range(coll, &self.allocations, &keys, &from[..n], cap)
        };
        self.build_rec_scratch(coll, &recs)
    }

    /// Materialise `recs` (live hash/radix rec-nrs) into a rec-nr scratch vector that
    /// the Ordered `on=4` iteration path walks.
    ///
    /// The scratch's header records the SOURCE (records) store_nr at offset 8, so the
    /// `on=4` yield resolves each element in that store regardless of where the scratch
    /// itself lives.  A WRITABLE source keeps the scratch co-located (source == scratch
    /// store — the fast path).  A READ-ONLY/exposed source cannot be claimed into (and
    /// its buffer must not move), so the scratch goes in a fresh dedicated store while
    /// the elements still resolve in the exposed source (expose-iteration-scratch.md).
    fn build_rec_scratch(&mut self, hash_ref: &DbRef, recs: &[u32]) -> DbRef {
        // Pick the scratch store: a read-only/exposed source needs a separate writable
        // home (claiming into it would panic and could move its buffer, invalidating the
        // exposed view).  `database(1)` hands back a fresh owned store.
        let scratch_store = if self.store(hash_ref).read_only {
            self.database(1).store_nr
        } else {
            hash_ref.store_nr
        };
        let scratch_ref = DbRef {
            store_nr: scratch_store,
            rec: 0,
            pos: 0,
        };
        let n = recs.len();
        // 8-byte header + n * 4 bytes of u32 rec-nrs, rounded up to 8-byte
        // words (store claim granularity).
        let vec_words = ((n as u32) * 4 + 8).div_ceil(8);
        let vec_words = vec_words.max(1);
        let vec_cr = self.claim(&scratch_ref, vec_words);
        let vec_rec = vec_cr.rec;
        // 2-word header: offset 4 = rec-nr vector, offset 8 = SOURCE (records) store_nr,
        // read by the `on=4` yield (`vector::step_ordered` with `sourced`).
        let header_cr = self.claim(&scratch_ref, 2);
        let header_rec = header_cr.rec;
        {
            let store = self.store_mut(&scratch_ref);
            store.set_u32_raw(vec_rec, 4, n as u32);
            for (i, &rec_nr) in recs.iter().enumerate() {
                let base = 8 + (i as u32) * 4;
                store.set_u32_raw(vec_rec, base, rec_nr);
            }
            store.set_u32_raw(header_rec, 4, vec_rec);
            store.set_u32_raw(header_rec, 8, u32::from(hash_ref.store_nr));
            store.set_u32_raw(
                header_rec,
                crate::vector::SCRATCH_TAG_FLD,
                crate::vector::scratch_tag(4),
            );
        }
        DbRef {
            store_nr: scratch_store,
            rec: header_rec,
            pos: 4,
        }
    }

    /// The `build_rec_scratch` sibling for elements that are `(record, offset)` pairs
    /// rather than record numbers — a hash's arena entries (@PLN135 arc H).
    ///
    /// Same shape, 8 bytes per element instead of 4, and the header's third word says
    /// so: `vector::step_ordered` reads that width rather than being told, so one
    /// stepper serves the arena form and the rec-number form (radix, index) alike.
    fn build_ref_scratch(&mut self, hash_ref: &DbRef, refs: &[DbRef]) -> DbRef {
        // Pick the scratch store: a read-only/exposed source needs a separate writable
        // home (claiming into it would panic and could move its buffer, invalidating the
        // exposed view).  `database(1)` hands back a fresh owned store.
        let scratch_store = if self.store(hash_ref).read_only {
            self.database(1).store_nr
        } else {
            hash_ref.store_nr
        };
        let scratch_ref = DbRef {
            store_nr: scratch_store,
            rec: 0,
            pos: 0,
        };
        let n = refs.len();
        // 8-byte header + n * 8 bytes of (rec, pos) pairs.
        let vec_words = ((n as u32) * 8 + 8).div_ceil(8).max(1);
        let vec_cr = self.claim(&scratch_ref, vec_words);
        let vec_rec = vec_cr.rec;
        // 2-word header: offset 4 = the pair vector, offset 8 = SOURCE store_nr,
        // offset 12 = the element width (8 — the marker `step_ordered` reads).
        let header_cr = self.claim(&scratch_ref, 2);
        let header_rec = header_cr.rec;
        {
            let store = self.store_mut(&scratch_ref);
            store.set_u32_raw(vec_rec, 4, n as u32);
            for (i, r) in refs.iter().enumerate() {
                let base = 8 + (i as u32) * 8;
                store.set_u32_raw(vec_rec, base, r.rec);
                store.set_u32_raw(vec_rec, base + 4, r.pos);
            }
            store.set_u32_raw(header_rec, 4, vec_rec);
            store.set_u32_raw(header_rec, 8, u32::from(hash_ref.store_nr));
            store.set_u32_raw(
                header_rec,
                crate::vector::SCRATCH_TAG_FLD,
                crate::vector::scratch_tag(8),
            );
        }
        DbRef {
            store_nr: scratch_store,
            rec: header_rec,
            pos: 4,
        }
    }

    /// Release an iteration scratch at loop exit — the counterpart of
    /// [`build_rec_scratch`](Self::build_rec_scratch), and the one home for the
    /// rule that a scratch's lifetime is **the loop**, in either home it can
    /// have.  Called from both backends' `OpFreeScratch`.
    ///
    /// A read-only source's scratch lives in a store of its own and goes back
    /// whole.  A writable source keeps it CO-LOCATED, and that case used to be
    /// a no-op, on the reasoning that its records go back when the source store
    /// dies.  They do — but only when it dies, and a collection outlives the
    /// loops that read it, so `for r in h` leaked one snapshot per pass: 4
    /// bytes per element, plus two records, every time.  Invisible in a short
    /// program (the arena's slack absorbs it, and the process ends); unbounded
    /// in a loop that iterates, and permanent for a BOUND store, whose arena is
    /// a file that outlives the process — 16 runs of a program that only READ a
    /// 4,000-record hash grew its file 2.33× with nothing else touching it
    /// (loft#727).
    ///
    /// The two records hold raw rec-nrs and own nothing, so deleting each is
    /// the whole release — no cascade, and the elements the loop yielded are
    /// records in the SOURCE store, untouched by this.
    ///
    /// **Idempotent, and it has to be.** Two sites free the same scratch — the
    /// loop epilogue and the scope exit that catches a `return` out of the loop
    /// — and only the interpreter nulls the variable between them; the native
    /// emitter drops that store, so both calls arrive carrying the same live
    /// `DbRef`. The dedicated case is idempotent through [`Self::free`]'s
    /// already-freed no-op; the co-located case is idempotent through
    /// `is_claimed_record`, which reads the block header a `delete` has just
    /// made negative. A second call is a no-op, not a double free.
    ///
    /// Declines, leaving the scratch where it is, when the store cannot take a
    /// delete: freed, or pinned read-only / free-protected inside the loop body
    /// (`expose(…)` mid-iteration), where `delete` asserts.  That is the old
    /// behaviour for those cases, never worse.
    pub fn free_iteration_scratch(&mut self, scratch: &DbRef) {
        if scratch.rec == 0 || scratch.store_nr as usize >= self.allocations.len() {
            return;
        }
        let store = &self.allocations[scratch.store_nr as usize];
        if store.free || !store.is_claimed_record(scratch.rec) {
            return;
        }
        // The record must still BE this scratch's header before a single one of its
        // fields is believed.
        //
        // `is_claimed_record` says the block is live; it does not say it is still
        // ours. A scratch that has already been released leaves its block on the
        // free list, and the next claim takes it — after which the reads below are
        // somebody else's bytes. That was survivable only while nothing large
        // re-claimed it promptly; @PLN135 arc H made a hash's first insert claim an
        // arena CHUNK, which lands on exactly such a block, and the mismatch below
        // then read `elem_store` out of chunk payload and FREED THE WHOLE STORE the
        // collection lives in — every entry gone between one closure invocation and
        // the next, with no error anywhere (`v2_two_clients_with_spectator_routing`).
        //
        // So the header is self-identifying, and a record that does not carry the tag
        // is left alone. Refusing costs at most the scratch's own two blocks, once;
        // acting on a foreign record costs the store.
        if !crate::vector::is_scratch_header(store, scratch) {
            return;
        }
        // Offset 8 of the header (`pos` is 4) — the store the ELEMENTS live in.
        // Different from the scratch's own store exactly when the source was
        // read-only and the scratch got a dedicated one.
        if store.get_u32_raw(scratch.rec, scratch.pos + 4) as u16 != scratch.store_nr {
            self.free(scratch);
            return;
        }
        if store.read_only || store.is_free_protected() {
            return;
        }
        // Offset 4 of the header — the rec-nr vector.  Read before the header
        // goes back, and refused unless it still names a claimed block.
        let vec_rec = store.get_u32_raw(scratch.rec, scratch.pos);
        let vec_live = vec_rec != scratch.rec && store.is_claimed_record(vec_rec);
        let store = &mut self.allocations[scratch.store_nr as usize];
        store.delete(scratch.rec);
        if vec_live {
            store.delete(vec_rec);
        }
    }

    pub fn store_mut(&mut self, r: &DbRef) -> &mut Store {
        // @PLN130 F8 — see `store`. A write through a dead reference is the worse half:
        // with slot reuse it lands in whatever now owns the memory.
        if crate::keys::strict_stores() && self.allocations[r.store_nr as usize].free {
            let s = &self.allocations[r.store_nr as usize];
            crate::keys::strict_store_violation(
                r.store_nr,
                r.rec,
                r.pos,
                "write",
                self.types
                    .get(s.known_type as usize)
                    .map_or("?", |t| t.name.as_str()),
                s.created_at,
                s.last_op_at,
                self.alloc_pc,
            );
        }
        #[cfg(debug_assertions)]
        if self.allocations[r.store_nr as usize].free && !crate::keys::strict_stores() {
            crate::loft_eprintln!(
                "[store] WRITE TO FREED store #{} rec={} pos={} — corruption",
                r.store_nr,
                r.rec,
                r.pos
            );
        }
        &mut self.allocations[r.store_nr as usize]
    }

    /// Lock the store that contains the record pointed to by `r` —
    /// user-facing `d#lock = true` semantics.  Sets the HARD `read_only`
    /// flag so subsequent writes / claims / deletes panic immediately;
    /// reads remain legal.  Use `set_free_protected` directly when only
    /// frees need blocking (e.g. the fn-call deep-copy bracket from
    /// @P290 — currently unused but kept available).
    pub fn lock_store(&mut self, r: &DbRef) {
        if r.rec != 0 && (r.store_nr as usize) < self.allocations.len() {
            debug_assert!(
                !self.allocations[r.store_nr as usize].free,
                "Locking a freed store (store_nr={}, rec={})",
                r.store_nr, r.rec
            );
            let origin = format!("lock_store(store_nr={}, rec={})", r.store_nr, r.rec);
            self.allocations[r.store_nr as usize].lock_with_origin(origin);
            // This is the ONLY user route to the lock — `n_set_store_lock`, which both
            // backends call for `d#lock = true` — so it is where the author's lock is
            // told apart from the const store and a worker borrow (loft#1405).
            self.allocations[r.store_nr as usize].user_locked = true;
        }
    }

    /// Unlock the store that contains the record pointed to by `r` —
    /// counterpart of `lock_store`.  Clears the user-facing read_only
    /// lock; leaves `free_protected` untouched.
    pub fn unlock_store(&mut self, r: &DbRef) {
        if r.rec != 0 && (r.store_nr as usize) < self.allocations.len() {
            self.allocations[r.store_nr as usize].unlock();
        }
    }

    /// Return whether the store containing the record pointed to by `r` is locked.
    #[must_use]
    pub fn is_store_locked(&self, r: &DbRef) -> bool {
        r.rec != 0
            && (r.store_nr as usize) < self.allocations.len()
            && self.allocations[r.store_nr as usize].is_locked()
    }

    /// Deep-copy a struct record from a worker's `Stores` into a pre-allocated
    /// destination in this (main) `Stores`.
    ///
    /// Uses a temporary "graft": the worker's source store is swapped into
    /// `self.allocations` at its `store_nr` index so that `copy_block` and
    /// `copy_claims` can reach both source and destination through the same
    /// `Stores` instance.  After copying the graft is swapped back out.
    pub fn copy_from_worker(
        &mut self,
        src_ref: &DbRef,
        dest: &DbRef,
        worker_stores: &mut Stores,
        tp: u16,
    ) {
        let ws = src_ref.store_nr as usize;

        // Extend allocations so the worker's store index is reachable.
        while self.allocations.len() <= ws {
            self.allocations.push(Store::new(100));
        }

        // Graft the worker's store in.
        std::mem::swap(
            &mut self.allocations[ws],
            &mut worker_stores.allocations[ws],
        );

        // Raw byte copy + deep-copy of owned sub-fields (text, nested refs).
        let size = u32::from(self.size(tp));
        self.copy_block(src_ref, dest, size);
        self.copy_claims(src_ref, dest, tp);

        // Un-graft: put the worker's store back.
        std::mem::swap(
            &mut self.allocations[ws],
            &mut worker_stores.allocations[ws],
        );
    }

    /// Plan-06 phase 2 step 2b — narrow rebase path for structs whose
    /// fields are entirely self-contained (no text, no DbRef sub-fields).
    /// `copy_block`s the struct bytes from the worker store into the
    /// dest record without invoking `copy_claims` — there's nothing to
    /// deep-copy.  The graft + ungraft swap is replaced by a single
    /// graft (the worker's store is borrowed via the existing slot
    /// for the duration of the copy_block, then swapped back).
    ///
    /// Caller MUST verify `Stores::has_owned_sub_fields(tp) == false`
    /// before calling — otherwise the dest's text/DbRef fields will
    /// reference invalid positions/store_nrs after the worker's stores
    /// are dropped at thread join.
    ///
    /// This is the "no copy_claims" version of `copy_from_worker`,
    /// applicable to ~40 % of typical struct returns (those without
    /// text or nested references — the common case for compute-heavy
    /// workloads returning numeric records).
    pub fn copy_from_worker_unowned(
        &mut self,
        src_ref: &DbRef,
        dest: &DbRef,
        worker_stores: &mut Stores,
        tp: u16,
    ) {
        debug_assert!(
            !self.has_owned_sub_fields(tp),
            "copy_from_worker_unowned called with owned-fields struct (tp={tp})",
        );

        let ws = src_ref.store_nr as usize;
        while self.allocations.len() <= ws {
            self.allocations.push(Store::new(100));
        }

        // Graft the worker's store in (so copy_block can read from it
        // through the parent's allocations table).
        std::mem::swap(
            &mut self.allocations[ws],
            &mut worker_stores.allocations[ws],
        );

        let size = u32::from(self.size(tp));
        self.copy_block(src_ref, dest, size);
        // No copy_claims — caller verified the struct is self-contained.

        // Un-graft.
        std::mem::swap(
            &mut self.allocations[ws],
            &mut worker_stores.allocations[ws],
        );
    }

    /// Produce a light-worker view — the parent stores borrowed read-only, plus a
    /// small set of **self-allocated** writable scratch stores for the worker's own
    /// allocations.  @PLN108 R1: the scratch is self-allocated per call (no external
    /// `WorkerPool` slice), so this composes with any spawner — `thread::scope` OR
    /// rayon — which is what lets the borrow become the single, always-on path (R2).
    ///
    /// Seeds `4` scratch stores, matching the retired `WorkerPool`.  The
    /// dispenser-driven `queue_ref` path wants **zero** scratch (its output slots come
    /// from the parent-namespace dispenser, and any pre-seeded scratch would push the
    /// worker's stack store into the dispenser's index range) — it calls
    /// [`clone_for_light_worker_with_scratch`](Self::clone_for_light_worker_with_scratch)
    /// with `0`.
    ///
    /// # Safety
    /// The original `Stores` must outlive the worker — guaranteed by the scoped join
    /// of the spawner (`thread::scope` or rayon's `pool.install`, both block until
    /// every worker has returned before the parent's borrow ends).
    pub unsafe fn clone_for_light_worker(&self) -> WorkerStores {
        // A light worker STARTS with this many writable scratch stores (matching the
        // retired `WorkerPool` seed of 4 × 1024).  It grows beyond this on demand
        // (freed-slot reuse / new worker-local stores), so it is only a reservation.
        const SCRATCH_STORES: usize = 4;
        unsafe { self.clone_for_light_worker_with_scratch(SCRATCH_STORES) }
    }

    /// Light-worker view with an explicit scratch-store count — see
    /// [`clone_for_light_worker`](Self::clone_for_light_worker).  Pass `0` for the
    /// `queue_ref` dispenser path: with no scratch, the worker's `allocations` are
    /// exactly the `parent_len` borrows, so its stack store lands at `parent_len`
    /// (push-at-end) — below the shared slot dispenser's floor (`parent_len + 1`),
    /// which therefore never climbs into a live worker-local slot.
    ///
    /// # Safety
    /// Same contract as [`clone_for_light_worker`](Self::clone_for_light_worker): the
    /// parent `Stores` must outlive the worker.
    pub unsafe fn clone_for_light_worker_with_scratch(
        &self,
        scratch_stores: usize,
    ) -> WorkerStores {
        const SCRATCH_WORDS: u32 = 1024;
        // Borrow ALL stores — the input vector may reference any store.
        let mut allocations: Vec<Store> = self
            .allocations
            .iter()
            .map(|s| {
                if s.free {
                    Store::new_freed_sentinel()
                } else {
                    unsafe { s.borrow_locked_for_light_worker() }
                }
            })
            .collect();
        // Self-allocated writable scratch: free slots the worker allocates into.
        // `Store::new` is already `free` + `init`'d, so these land in `free_bits` below
        // exactly as the old pool-seeded `Store::new(cap)` did.
        for _ in 0..scratch_stores {
            allocations.push(Store::new(SCRATCH_WORDS));
        }
        // Build free_bits: main-thread freed slots + all pool slots.
        let mut free_bits: Vec<u64> = Vec::new();
        for (i, s) in allocations.iter().enumerate() {
            if s.free {
                let word = i / 64;
                let bit = i % 64;
                while free_bits.len() <= word {
                    free_bits.push(0);
                }
                free_bits[word] |= 1u64 << bit;
            }
        }
        WorkerStores::new(Stores {
            types: self.types.clone(),
            names: self.names.clone(),
            allocations,
            records_created: self.records_created,
            stores_allocated: self.stores_allocated,
            alloc_pc: self.alloc_pc,
            stack_store_at_zero: self.stack_store_at_zero,
            // @PLN129 arc A — a worker inherits NO lazy binding, deliberately. A
            // miss inside a `par` arm answers absent rather than reaching for a
            // source: fetching would put I/O on every parallel arm at once,
            // against one file or one connection, and would insert into a store
            // the parent later reconciles. Fault before the `par`, not inside it.
            lazy_sources: std::collections::HashMap::new(),
            lazy_errors: std::collections::HashMap::new(),
            paged_refusal: None,
            lazy_driver_allocs: None,
            files: Vec::new(),
            max: (self.allocations.len() + scratch_stores) as u16,
            peak: (self.allocations.len() + scratch_stores) as u16,
            free_bits,
            bridge_text_dest: None,
            const_refs: self.const_refs.clone(),
            last_parse_errors: Vec::new(),
            last_json_errors: Vec::new(),
            // The worker inherits the parent's program context, so a `par`
            // inside a `par` worker can dispatch in turn.  Without it the
            // nested dispatch finds no context and aborts the run
            // ("parallel_queue called outside State::execute()").  The pointers
            // stay valid because the parent joins its workers before its own
            // `execute()` returns — the lifetime rule `ParallelCtx` already
            // documents — and they are read-only: the worker only reads the
            // same bytecode, library and `Data` its parent is running.
            parallel_ctx: self.parallel_ctx.clone(),
            par_buffer_stack: Vec::new(),
            par_text_buffer_stack: Vec::new(),
            par_ref_buffer_stack: Vec::new(),
            par_narrow_buffer_stack: Vec::new(),
            par_fn_buffer_stack: Vec::new(),
            par_fn_native_buffer_stack: Vec::new(),
            logger: self.logger.clone(),
            had_fatal: false,
            runtime_error: None,
            // #255 / @PLN9: a parallel worker's file ops must resolve paths the
            // same way as the main thread — carry the anchor + mode.
            source_dir: self.source_dir.clone(),
            program_relative: self.program_relative,
            frame_yield: false,
            poison_free: self.poison_free,
            disable_slot_reuse: self.disable_slot_reuse,
            uaf_freed_this_op: Vec::new(),
            worker_slot_dispenser: None,
            worker_allocated_indices: Vec::new(),
            report_asserts: false,
            assert_results: Vec::new(),
            user_args: Vec::new(),
            #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
            start_time: self.start_time,
            #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
            start_time_ms: self.start_time_ms,
            call_stack_snapshot: Vec::new(),
            variables_snapshot: Vec::new(),
            closure_map: std::collections::HashMap::new(),
            jnull_sentinel: None,
        })
    }

    #[must_use]
    pub fn store_nr(&self, nr: u16) -> &Store {
        &self.allocations[nr as usize]
    }

    /// #306 diagnostic (`LOFT_TRACE_CR`) — bounds-checked mirror of the
    /// `copy_claims` walk.  Reports (instead of faulting on) the first broken
    /// interior edges of `rec`'s claim graph: a text offset or vector record
    /// id outside its store's buffer, a freed/out-of-range store, or an
    /// insane vector length.  Diagnostic only; never dereferences unchecked.
    pub fn validate_claims(&self, rec: &DbRef, tp: u16, path: &str, problems: &mut u32) {
        if *problems > 8 {
            return;
        }
        if rec.store_nr as usize >= self.allocations.len() {
            crate::loft_eprintln!(
                "[cr-check] {path}: ref #{}.{},{} — store out of range",
                rec.store_nr,
                rec.rec,
                rec.pos
            );
            *problems += 1;
            return;
        }
        let store = &self.allocations[rec.store_nr as usize];
        if store.free {
            crate::loft_eprintln!(
                "[cr-check] {path}: ref #{}.{},{} — store FREED",
                rec.store_nr,
                rec.rec,
                rec.pos
            );
            *problems += 1;
            return;
        }
        let cap = store.capacity_words();
        if rec.rec >= cap || u64::from(rec.rec) * 8 + u64::from(rec.pos) >= u64::from(cap) * 8 {
            crate::loft_eprintln!(
                "[cr-check] {path}: ref #{}.{},{} — record beyond store capacity ({cap} words)",
                rec.store_nr,
                rec.rec,
                rec.pos
            );
            *problems += 1;
            return;
        }
        if (tp as usize) >= self.types.len() {
            crate::loft_eprintln!("[cr-check] {path}: type {tp} out of range");
            *problems += 1;
            return;
        }
        match &self.types[tp as usize].parts {
            Parts::Base if tp == 5 => {
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur != 0 && cur >= cap {
                    crate::loft_eprintln!(
                        "[cr-check] {path}: text offset {cur} beyond store #{} capacity {cap}",
                        rec.store_nr
                    );
                    *problems += 1;
                }
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in fields {
                    self.validate_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: rec.rec,
                            pos: rec.pos + u32::from(f.position),
                        },
                        f.content,
                        &format!("{path}.{}", f.name),
                        problems,
                    );
                }
            }
            Parts::Vector(v) | Parts::Sorted(v, _) => {
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    return;
                }
                if cur >= cap {
                    crate::loft_eprintln!(
                        "[cr-check] {path}: vector rec {cur} beyond store #{} capacity {cap}",
                        rec.store_nr
                    );
                    *problems += 1;
                    return;
                }
                let len = store.get_u32_raw(cur, 4);
                let size = u32::from(self.size(*v));
                if u64::from(len) * u64::from(size) > u64::from(cap) * 8 {
                    crate::loft_eprintln!(
                        "[cr-check] {path}: vector rec {cur} len {len} (elem size {size}) \
                         exceeds store #{} capacity {cap} words",
                        rec.store_nr
                    );
                    *problems += 1;
                    return;
                }
                for i in 0..len.min(16) {
                    self.validate_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: cur,
                            pos: 8 + size * i,
                        },
                        *v,
                        &format!("{path}[{i}]"),
                        problems,
                    );
                }
            }
            Parts::ChildRec(ct) => {
                let r = store.get_u32_raw(rec.rec, rec.pos);
                if r == 0 {
                    return;
                }
                if r >= cap {
                    crate::loft_eprintln!(
                        "[cr-check] {path}: child rec {r} beyond store #{} capacity {cap}",
                        rec.store_nr
                    );
                    *problems += 1;
                    return;
                }
                self.validate_claims(
                    &DbRef {
                        store_nr: rec.store_nr,
                        rec: r,
                        pos: 8,
                    },
                    *ct,
                    &format!("{path}->child"),
                    problems,
                );
            }
            Parts::Enum(values) => {
                // Mirrors `copy_claims`' Enum arm (direct index, no -1 shift).
                let e_nr = store.get_byte(rec.rec, rec.pos, -1);
                if e_nr >= 0 && (e_nr as usize) < values.len() {
                    let etp = values[e_nr as usize].0;
                    if etp != u16::MAX {
                        self.validate_claims(rec, etp, path, problems);
                    }
                }
            }
            Parts::Hash(v, _) => {
                // Bounds-checked hash walk (the per-element kind for_each_owned_child
                // trusts; here it is guard-before-deref).  Mirrors hash.rs: the root
                // word holds the bucket-record claim; bucket layout is word 0 = room,
                // buckets from `BUCKET0`, `elms = (room - RESERVED_WORDS) * 2`.
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    return;
                }
                if cur >= cap {
                    crate::loft_eprintln!(
                        "[cr-check] {path}: hash bucket rec {cur} beyond store #{} capacity {cap}",
                        rec.store_nr
                    );
                    *problems += 1;
                    return;
                }
                // `room` is the bucket record's SIZE HEADER, which hash.rs deliberately
                // doubles as data (`src/hash.rs`: "fld 0 : size header (word count =
                // `room`); see `Store::record_words`").  Read it through the header
                // accessor: `get_u32_raw` is the FIELD accessor and asserts `fld >= 4`
                // under armed debug-assertions, because fld 0..4 IS the header it is
                // meant to sit above — so reading word 0 with it panicked the
                // debug-assertions gate ("Fld 0 is outside of record N").
                let room = store.record_words(cur);
                if room < crate::hash::RESERVED_WORDS || u64::from(room) > u64::from(cap) {
                    crate::loft_eprintln!(
                        "[cr-check] {path}: hash bucket rec {cur} insane room {room} (cap {cap})",
                    );
                    *problems += 1;
                    return;
                }
                let elms = (room - crate::hash::RESERVED_WORDS) * 2;
                // @PLN135 arc H — a slot holds an arena INDEX for a hash that owns its
                // entries, and a record number for one that borrows them (a secondary
                // index).  The stride the table records is the discriminator, and the
                // chunk directory has to be reached before any slot can be decoded —
                // guarded first, like everything else here, because this walk exists
                // to survive a corrupt store rather than to assume a sound one.
                let stride = crate::hash::stride(store, cur);
                let dir = store.get_u32_raw(cur, crate::arena::DIR_FLD);
                if stride != 0 && dir >= cap {
                    crate::loft_eprintln!(
                        "[cr-check] {path}: hash arena directory {dir} beyond store #{} capacity {cap}",
                        rec.store_nr
                    );
                    *problems += 1;
                    return;
                }
                for i in 0..elms {
                    let slot =
                        store.get_u32_raw(cur, crate::hash::BUCKET0 + i * crate::hash::SLOT_BYTES);
                    if slot == 0 {
                        continue;
                    }
                    // The slot's high bit says it names a RECORD rather than an arena
                    // index (`hash::SLOT_RECORD`) — one table can hold both, so this
                    // is decided per slot. Decoding a tagged record as an index would
                    // read a chunk that does not exist and skip a live entry.
                    let (entry, pos) = if stride == 0 || slot & crate::hash::SLOT_RECORD != 0 {
                        (
                            slot & !crate::hash::SLOT_RECORD,
                            crate::store::RECORD_PAYLOAD,
                        )
                    } else {
                        match crate::arena::slot(store, cur, slot, stride) {
                            Some(found) => found,
                            None => continue,
                        }
                    };
                    if entry >= cap {
                        crate::loft_eprintln!(
                            "[cr-check] {path}: hash entry rec {entry} beyond store #{} capacity {cap}",
                            rec.store_nr
                        );
                        *problems += 1;
                        if *problems > 8 {
                            return;
                        }
                        continue;
                    }
                    self.validate_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: entry,
                            pos,
                        },
                        *v,
                        &format!("{path}{{#{entry}}}"),
                        problems,
                    );
                }
            }
            // An ARRAY / ORDERED holds a record of element REFERENCES.  The root word is
            // the claim; each slot is a rec number into this store.  Bounds-check both,
            // then validate the elements — this walk exists to run over a heap that may
            // already be corrupt, so every number is guarded before it is followed.
            Parts::Array(v) | Parts::Ordered(v, _) => {
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    return;
                }
                if cur >= cap {
                    crate::loft_eprintln!(
                        "[cr-check] {path}: array rec {cur} beyond store #{} capacity {cap}",
                        rec.store_nr
                    );
                    *problems += 1;
                    return;
                }
                let len = store.get_u32_raw(cur, 4);
                if u64::from(len) * 4 > u64::from(cap) * 8 {
                    crate::loft_eprintln!(
                        "[cr-check] {path}: array rec {cur} len {len} exceeds store #{} \
                         capacity {cap} words",
                        rec.store_nr
                    );
                    *problems += 1;
                    return;
                }
                for i in 0..len.min(16) {
                    let elm = store.get_u32_raw(cur, 8 + 4 * i);
                    if elm == 0 {
                        continue;
                    }
                    if elm >= cap {
                        crate::loft_eprintln!(
                            "[cr-check] {path}[{i}]: array element rec {elm} beyond store #{} \
                             capacity {cap}",
                            rec.store_nr
                        );
                        *problems += 1;
                        return;
                    }
                    self.validate_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: elm,
                            pos: 8,
                        },
                        *v,
                        &format!("{path}[{i}]"),
                        problems,
                    );
                }
            }
            // A RADIX / TRIE root word claims the tree record.  The LEAVES belong to the
            // primary collection and are validated through it, so this checks the spine
            // itself is in range and stops — the same split `for_each_owned_child` makes
            // (it keeps `cur` and no per-leaf children).
            Parts::Radix(_, _) | Parts::Trie(_, _) => {
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur != 0 && cur >= cap {
                    crate::loft_eprintln!(
                        "[cr-check] {path}: tree rec {cur} beyond store #{} capacity {cap}",
                        rec.store_nr
                    );
                    *problems += 1;
                }
            }
            // A stored 12-byte DbRef points OUT of this store, so the only thing checkable
            // here without following it into another allocation is its store number.
            // Following it is what `validate_claims` is called ON, one level up.
            Parts::DbRef => {
                let sn = store.get_u32_raw(rec.rec, rec.pos);
                if sn != u32::from(u16::MAX) && sn as usize >= self.allocations.len() {
                    crate::loft_eprintln!(
                        "[cr-check] {path}: stored DbRef names store #{sn}, out of range"
                    );
                    *problems += 1;
                }
            }
            // Deliberately nothing, and each for its own reason — not a catch-all.
            //
            // `Index` BORROWS its rows from the primary collection: `for_each_owned_child`
            // answers `empty(None, …)` for it, so there is no claim of its own to check
            // and walking into the primary's rows would double-report every problem.
            //
            // The narrow-integer kinds and a non-text `Base` hold VALUES, not claims:
            // there is no record number in them to be out of range, so a walk has nothing
            // to say.  (`Base` at `tp == 5` is `text`, which DOES hold an offset and is
            // checked in the first arm above.)
            Parts::Index(_, _, _)
            | Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::Int(_, _)
            | Parts::ShortRaw(_, _)
            | Parts::Base => {}
        }
    }

    /// `tp` is the CONTAINER type (`Vector` / `Sorted`), not the content type — the
    /// same convention as `array_body` / `hash_body` / `index_body`, so the keystone
    /// walk keys on the container's `Parts` and the content type is read back out.
    pub(super) fn copy_claims_seq_vector(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let content_tp = match &self.types[tp as usize].parts {
            Parts::Vector(v) | Parts::Sorted(v, _) => *v,
            other => {
                panic!("copy_claims_seq_vector called with non-vector type {tp} (parts: {other:?})")
            }
        };
        let length = vector::length_vector(rec, &self.allocations);
        let size = u32::from(self.size(content_tp));
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        if cur == 0 {
            self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
            return;
        }
        let into = self.store_mut(to).claim(1 + (size * length).div_ceil(8));
        debug_assert!(
            i32::try_from(into).is_ok(),
            "vector allocation offset overflow: {into}"
        );
        self.store_mut(to).set_u32_raw(to.rec, to.pos, into);
        // DESTINATION build: one bulk copy of the whole element block (elements are
        // INLINE in the container, unlike array/hash/index where each is a separate
        // record).  Keep it — the keystone only enumerates the SOURCE; it does not
        // build the destination.
        self.copy_block(
            &DbRef {
                store_nr: rec.store_nr,
                rec: cur,
                pos: 4,
            },
            &DbRef {
                store_nr: to.store_nr,
                rec: into,
                pos: 4,
            },
            length * size + 4,
        );
        // SOURCE enumeration reads the keystone walk — the single home of the
        // per-`Parts` source layout.  Its `Vector`/`Sorted` arm yields one child per
        // element at `pos = 8 + size*i` with `owning_elem: None` (inline, no separate
        // record).  We iterate it ALONGSIDE the bulk copy above (two passes over the
        // same elements, deliberately not merged — the bulk copy moves the bytes, this
        // pass deep-copies the nested claims).  The bulk copy laid the destination out
        // byte-identically, so each element sits at the SAME offset in `into`; reuse
        // `child.pos` for the destination instead of recomputing `8 + size*i`.
        let children = self.for_each_owned_child(rec, tp).children;
        debug_assert_eq!(
            u32::try_from(children.len()).unwrap_or(u32::MAX),
            length,
            "keystone element count disagrees with the vector length header (tp={tp})"
        );
        for child in children {
            self.copy_claims(
                &child.child,
                &DbRef {
                    store_nr: to.store_nr,
                    rec: into,
                    pos: child.child.pos,
                },
                child.child_tp,
            );
        }
    }

    /// `tp` is the CONTAINER type (`Array` / `Ordered`), not the content type: the
    /// keystone walk is keyed on the container's `Parts`.  The content type is read
    /// back out of it, so the caller no longer has to know which of the two it is.
    pub(super) fn copy_claims_array_body(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let content_tp = match &self.types[tp as usize].parts {
            Parts::Array(v) | Parts::Ordered(v, _) => *v,
            other => {
                panic!("copy_claims_array_body called with non-array type {tp} (parts: {other:?})")
            }
        };
        let length = vector::length_vector(rec, &self.allocations);
        let size = u32::from(self.size(content_tp));
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        if cur == 0 {
            self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
            return;
        }
        // @P309 — claim by element COUNT (header word + one 4-byte rec-id
        // slot per element, 2 slots/word), NOT by `cur` (the source
        // structure's rec-id, which is meaningless as a size), and WRITE THE
        // LENGTH HEADER (offset 4) — without it the copied `array`/`ordered`
        // read back as length 0 (silent data loss; e.g. a `sorted<T>` field
        // becomes an `ordered<T>` secondary index when an `index<T>` exists,
        // and deep-copying the owning struct lost its elements).
        let into = self.store_mut(to).claim(1 + length.div_ceil(2));
        self.store_mut(to).set_u32_raw(to.rec, to.pos, into);
        self.store_mut(to).set_u32_raw(into, 4, length);
        // SOURCE enumeration reads the keystone walk — the single home of the
        // per-`Parts` source layout.  Its `Array`/`Ordered` arm computes the same
        // `get_u32_raw(cur, 8 + i * 4)` this loop used to, and hands each element
        // record back as `owning_elem`, so the fold is position-for-position.
        // The DESTINATION build below (claim → `copy_block` → rec-id slot → recurse)
        // stays per-kind, including the @P309 length header written above.
        let elems: Vec<u32> = self
            .for_each_owned_child(rec, tp)
            .children
            .into_iter()
            .filter_map(|c| c.owning_elem)
            .collect();
        // The keystone reads the same `length_vector` header, so the counts are one
        // fact seen twice.  Assert it rather than let a future divergence write the
        // header for N and fill N-1 slots (the @P309 shape) — the nightly
        // debug-assertions gate runs this over the whole interpreter corpus.
        debug_assert_eq!(
            u32::try_from(elems.len()).unwrap_or(u32::MAX),
            length,
            "keystone element count disagrees with the vector length header (tp={tp})"
        );
        for (i, elm) in elems.into_iter().enumerate() {
            let i = u32::try_from(i).unwrap_or(u32::MAX);
            // @PLN102 heap copy/alias audit — an `elm == 0` slot is an ABSENT element
            // (no record). Every runtime path keeps `length` in sync with the filled
            // slots (append/copy fill, `#remove` compacts, nullable elements stay inline
            // so a null is never a 0 rec-id), so this is unreachable today — but the walk
            // must NOT dereference record 0 (it is reserved). Preserve the hole (slot ← 0)
            // and skip the deref, keeping positions and the length header intact rather
            // than fabricating an element from record 0. Mirrors the free-side skip below.
            if elm == 0 {
                self.store_mut(to).set_u32_raw(into, 8 + 4 * i, 0);
                continue;
            }
            // A record is an 8-byte header followed by `size` payload bytes, and
            // `claim` takes 8-byte WORDS — so the element needs `1 + size/8`
            // words, and its payload is copied from offset 8 for the full `size`.
            // Claiming `size.div_ceil(8)` left out the header word, and copying
            // `size - 4` from offset 4 both started 4 bytes early and stopped 4
            // bytes short: for `It { k: integer, v: integer }` that copied `k` and
            // never `v`, and `v`'s slot fell outside the record, so reading it
            // returned the NEXT record's header — `k` correct, `v` a plausible
            // large integer (`(claim << 32) | length`).  The `copy_claims` recursion
            // just below always used offset 8; these two now agree with it.
            let new = self.store_mut(to).claim(1 + size.div_ceil(8));
            self.copy_block(
                &DbRef {
                    store_nr: rec.store_nr,
                    rec: elm,
                    pos: 8,
                },
                &DbRef {
                    store_nr: to.store_nr,
                    rec: new,
                    pos: 8,
                },
                size,
            );
            self.store_mut(to).set_u32_raw(into, 8 + 4 * i, new);
            self.copy_claims(
                &DbRef {
                    store_nr: rec.store_nr,
                    rec: elm,
                    pos: 8,
                },
                &DbRef {
                    store_nr: to.store_nr,
                    rec: new,
                    pos: 8,
                },
                content_tp,
            );
        }
    }

    /// Deep-copy a `hash<T[key]>` field from `rec` into `to`.  `tp` is the
    /// HASH type (not the content type).
    ///
    /// @P318 — re-INSERT each entry into an emptied destination via `hash::add`
    /// rather than copying the bucket array slot-for-slot.  A hash's bucket
    /// layout (slot count + offsets, now including a per-hash seed word) lives
    /// only in `src/hash.rs`; `room` comes from the record's SIZE HEADER
    /// (offset 0) — and `Store::claim` may hand back a block LARGER than
    /// requested (it only splits when the surplus exceeds 1/3; see
    /// `Store::claim_block`).  The old slot-for-slot copy laid entries out for
    /// the SOURCE's `room`, but the destination record's header (its own
    /// `room`) could differ, so `hash::find` later probed `key % dest_elms` —
    /// a DIFFERENT start slot — and missed entries.  That surfaced only as a
    /// wrong / NON-deterministic result (whether `claim` over-sizes depends on
    /// free-list state) and was immune to `zero_claim` (the probe START shifts,
    /// not just the slack).  Re-inserting rebuilds the destination consistently
    /// with its own `room`, exactly as the source was built — mirroring
    /// `copy_claims_index_body`.  `hash::add` also maintains the length word and
    /// rehash invariants, subsuming the earlier @P317 length-word and @P290
    /// loop-bound fixes.
    pub(super) fn copy_claims_hash_body(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        // Start the destination as an empty hash; `hash::add` claims the bucket
        // record on the first insert and rehashes (growing `room`) as it fills.
        self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
        if cur == 0 {
            return;
        }
        let content_tp = match &self.types[tp as usize].parts {
            Parts::Hash(c, _) => *c,
            other => {
                panic!("copy_claims_hash_body called with non-hash type {tp} (parts: {other:?})")
            }
        };
        let size = u32::from(self.size(content_tp));
        let keys = self.types[tp as usize].keys.clone();
        // Source-bucket enumeration reads the SAME keystone walk `remove_claims`
        // uses (`for_each_owned_child` → `hash::records`), so the bucket layout
        // lives in ONE place.  Each child carries its element record in
        // `owning_elem`; re-insert that entry into the emptied destination.
        // loft#730 — re-insert in SOURCE RECORD ORDER, not bucket order.
        //
        // The destination claims sequentially, so the order entries are visited
        // IS the physical order of the rebuilt file. Bucket order is key-hash
        // order, which bears no relation to how the source was laid out — so a
        // rebuild scattered whatever locality the generator had built in, and a
        // reader paging a spatially-coherent working set paid for it. Measured
        // on a real 162 MB store, one wide viewport: 31 129 600 bytes read
        // before compaction and 45 613 056 after, while the file itself had
        // HALVED. Compaction is meant to make reads cheaper, and it was making
        // the one that matters more expensive.
        //
        // Sorting by the source element record restores the source's order, so
        // compaction changes a store's SIZE without changing its shape.
        // @PLN135 arc H — an entry is a SLOT, so `owning_elem` is `None` and the
        // source order is the arena's own: `(chunk record, offset)` sorts to the order
        // the entries were handed out, because a chunk is filled before the next is
        // appended.  The destination allocates its slots through the same arena, so
        // visiting in that order rebuilds the source's physical layout.
        let mut children = self.for_each_owned_child(rec, tp).children;
        children.sort_by_key(|c| (c.child.rec, c.child.pos));
        let stride = crate::hash::stride_for(size);
        for child in children {
            let src_db = child.child;
            // The destination's slot comes from ITS arena, not from a claim: an entry
            // has no header word and no back-pointer of its own any more (the chunk
            // carries the owner).  Allocate, copy the payload, deep-copy the nested
            // claims, then re-insert by key.
            let new_db = crate::hash::alloc_entry(to, stride, to.rec, &mut self.allocations);
            self.copy_block(&src_db, &new_db, size);
            self.copy_claims(&src_db, &new_db, content_tp);
            hash::add(to, &new_db, &mut self.allocations, &keys);
        }
    }

    /// Deep-copy a `Radix` collection: rebuild the destination tree by re-inserting
    /// each source element.  A radix tree cannot be byte-copied — node ids and the
    /// container relocate — so, like `copy_claims_hash_body`, it re-inserts entry by
    /// entry through the same keystone walk `remove_claims` uses.
    pub(super) fn copy_claims_radix_body(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        // Start the destination as an empty tree; `radix_db::add` claims + grows it.
        self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        if cur == 0 {
            return;
        }
        let content_tp = match &self.types[tp as usize].parts {
            Parts::Radix(c, _) => *c,
            other => panic!("copy_claims_radix_body called with non-radix type {tp} ({other:?})"),
        };
        let size = u32::from(self.size(content_tp));
        let keys = self.types[tp as usize].keys.clone();
        for child in self.for_each_owned_child(rec, tp).children {
            let Some(elm) = child.owning_elem else {
                continue;
            };
            // Element layout (record_new): header, back-pointer at 4, payload at 8.
            let new = self.store_mut(to).claim(1 + size.div_ceil(8));
            self.store_mut(to).set_u32_raw(new, 4, to.rec);
            let src_db = DbRef {
                store_nr: rec.store_nr,
                rec: elm,
                pos: 8,
            };
            let new_db = DbRef {
                store_nr: to.store_nr,
                rec: new,
                pos: 8,
            };
            self.copy_block(&src_db, &new_db, size);
            self.copy_claims(&src_db, &new_db, content_tp);
            radix_db::add(to, &new_db, &mut self.allocations, &keys);
        }
    }

    /// Deep-copy a `trie<T[k]>` field from `rec` into `to`.
    ///
    /// The Radix twin, and deliberately a twin rather than a shared body with a kind
    /// switch: the two differ exactly where the kinds differ — whose `add` re-inserts
    /// each element — and a shared body would put a trie concept in the spatial path,
    /// which is the coupling this design removes.
    pub(super) fn copy_claims_trie_body(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        // Start the destination as an empty tree; `trie_db::add` claims + grows it.
        self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        if cur == 0 {
            return;
        }
        let content_tp = match &self.types[tp as usize].parts {
            Parts::Trie(c, _) => *c,
            other => panic!("copy_claims_trie_body called with non-trie type {tp} ({other:?})"),
        };
        let size = u32::from(self.size(content_tp));
        let keys = self.types[tp as usize].keys.clone();
        for child in self.for_each_owned_child(rec, tp).children {
            let Some(elm) = child.owning_elem else {
                continue;
            };
            // Element layout (record_new): header, back-pointer at 4, payload at 8.
            let new = self.store_mut(to).claim(1 + size.div_ceil(8));
            self.store_mut(to).set_u32_raw(new, 4, to.rec);
            let src_db = DbRef {
                store_nr: rec.store_nr,
                rec: elm,
                pos: 8,
            };
            let new_db = DbRef {
                store_nr: to.store_nr,
                rec: new,
                pos: 8,
            };
            self.copy_block(&src_db, &new_db, size);
            self.copy_claims(&src_db, &new_db, content_tp);
            crate::trie_db::add(to, &new_db, &mut self.allocations, &keys);
        }
    }

    /// Collect all record numbers in an RB-tree index by in-order traversal.
    /// `rec` points to the i32 tree-root field; `left` is `self.fields(index_tp)`.
    pub(super) fn collect_index_nodes(&self, rec: &DbRef, left: u16) -> Vec<u32> {
        let mut nodes = Vec::new();
        let mut curr = tree::first(rec, left, &self.allocations).rec;
        while curr != 0 {
            nodes.push(curr);
            curr = tree::next(
                &self.allocations[rec.store_nr as usize],
                &DbRef {
                    store_nr: rec.store_nr,
                    rec: curr,
                    pos: u32::from(left),
                },
            );
        }
        nodes
    }

    /// Deep-copy an `index<T>` field from `rec` into `to`.
    /// `tp` is the index type (not the content type).
    pub(super) fn copy_claims_index_body(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
        if cur == 0 {
            self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
            return;
        }
        let left = self.fields(tp);
        let content_tp = match &self.types[tp as usize].parts {
            Parts::Index(c, _, _) => *c,
            other => {
                panic!("copy_claims_index_body called with non-index type {tp} (parts: {other:?})")
            }
        };
        let size = u32::from(self.size(content_tp));
        let keys = self.types[tp as usize].keys.clone();
        // SOURCE enumeration reads the keystone walk — the single home of the
        // per-`Parts` source layout — exactly as `remove_claims` and
        // `copy_claims_hash_body` do.  The keystone's `Index` arm makes the same
        // `collect_index_nodes(rec, left)` call and hands each node back as
        // `owning_elem`, so this is position-for-position the walk it replaces.
        // The DESTINATION build (claim → back-pointer → `copy_block` → recurse →
        // `tree::add`) stays per-kind: unifying it is how @P318/@P309 come back.
        let nodes: Vec<u32> = self
            .for_each_owned_child(rec, tp)
            .children
            .into_iter()
            .filter_map(|c| c.owning_elem)
            .collect();
        // Initialize the destination tree root to empty before inserting.
        self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
        for src_node in nodes {
            // Allocate element record in the destination store.
            let dst_node = self.store_mut(to).claim(1 + size.div_ceil(8));
            // Back-reference to the destination parent record (offset 4).
            self.store_mut(to).set_u32_raw(dst_node, 4, to.rec);
            // Bulk-copy element data bytes (pos=8, size bytes).
            self.copy_block(
                &DbRef {
                    store_nr: rec.store_nr,
                    rec: src_node,
                    pos: 8,
                },
                &DbRef {
                    store_nr: to.store_nr,
                    rec: dst_node,
                    pos: 8,
                },
                size,
            );
            // Deep-copy nested claims (strings, sub-structures).
            self.copy_claims(
                &DbRef {
                    store_nr: rec.store_nr,
                    rec: src_node,
                    pos: 8,
                },
                &DbRef {
                    store_nr: to.store_nr,
                    rec: dst_node,
                    pos: 8,
                },
                content_tp,
            );
            // Insert into the destination tree; tree::add initialises nav fields.
            tree::add(
                to,
                &DbRef {
                    store_nr: to.store_nr,
                    rec: dst_node,
                    pos: 8,
                },
                left,
                &mut self.allocations,
                &keys,
            );
        }
    }

    /**
    Copy string fields and substructures from `rec` to `to`.
    # Panics
    When a field points to a spatial structure.
    */
    pub fn copy_claims(&mut self, rec: &DbRef, to: &DbRef, tp: u16) {
        // TODO prevent copying secondary structures
        match &self.types[tp as usize].parts {
            Parts::Base if tp == 5 => {
                // text — P220: discriminate null source from empty-string
                // source.  `get_str(0)` returns `STRING_NULL` ("\0"), so the
                // old `s.is_empty()` check never fired for a null source
                // (s would be "\0", not "") — but it DID fire for a
                // genuinely empty `""` source, writing 0 (the null
                // sentinel) into the destination and silently
                // re-classifying the value as null.  Result: a `""`
                // element copied through this path read back as `null`.
                // Discriminate on the source `cur` instead.
                let store = self.store(rec);
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
                } else {
                    let s = store.get_str(cur);
                    let into = self.store_mut(to);
                    let s_pos = into.set_str(s);
                    into.set_u32_raw(to.rec, to.pos, s_pos);
                }
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in fields.clone() {
                    self.copy_claims(
                        &DbRef {
                            store_nr: rec.store_nr,
                            rec: rec.rec,
                            pos: rec.pos + u32::from(f.position),
                        },
                        &DbRef {
                            store_nr: to.store_nr,
                            rec: to.rec,
                            pos: to.pos + u32::from(f.position),
                        },
                        f.content,
                    );
                }
            }
            Parts::Vector(_) | Parts::Sorted(_, _) => {
                // Pass the CONTAINER type: the keystone walk keys on it, and the helper
                // reads the content type back out (same shape as array/hash/index).
                self.copy_claims_seq_vector(rec, to, tp);
            }
            Parts::ChildRec(content_kt) => {
                // P213: read source rec-id; if 0 (empty / non-capturing),
                // clear destination.  Otherwise claim a fresh record in
                // dest's Store, byte-copy the child's payload, recurse on
                // the child's nested heap fields, and write the new rec-id
                // into dest's field.
                let src_rec = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if src_rec == 0 {
                    self.store_mut(to).set_u32_raw(to.rec, to.pos, 0);
                } else {
                    let content_kt = *content_kt;
                    let size = u32::from(self.size(content_kt));
                    let new_rec = self.allocations[to.store_nr as usize].claim(size);
                    let src_db = DbRef {
                        store_nr: rec.store_nr,
                        rec: src_rec,
                        pos: 8,
                    };
                    let new_db = DbRef {
                        store_nr: to.store_nr,
                        rec: new_rec,
                        pos: 8,
                    };
                    // Cross-store byte copy of payload.
                    if rec.store_nr == to.store_nr {
                        self.store_mut(to)
                            .copy_block(src_rec, 8, new_rec, 8, size as isize);
                    } else {
                        // @PLAN53 cluster 3: sound disjoint-borrow cross-store copy
                        // (src_db.store_nr == rec.store_nr != to.store_nr in this else).
                        self.copy_block_cross_store(
                            src_db.store_nr,
                            src_rec,
                            8,
                            to.store_nr,
                            new_rec,
                            8,
                            size as isize,
                        );
                    }
                    // Deep-copy nested heap fields (text, Reference, etc.).
                    self.copy_claims(&src_db, &new_db, content_kt);
                    self.store_mut(to).set_u32_raw(to.rec, to.pos, new_rec);
                }
            }
            Parts::Array(_) | Parts::Ordered(_, _) => {
                // Pass the CONTAINER type: the keystone walk is keyed on it, and the
                // helper reads the content type back out (same shape as `Hash`/`Index`).
                self.copy_claims_array_body(rec, to, tp);
            }
            Parts::Hash(_, _) => {
                self.copy_claims_hash_body(rec, to, tp);
            }
            Parts::Trie(_, _) => self.copy_claims_trie_body(rec, to, tp),
            Parts::Radix(_, _) => self.copy_claims_radix_body(rec, to, tp),
            Parts::Index(_, _, _) => self.copy_claims_index_body(rec, to, tp),
            Parts::Enum(values) => {
                let e_nr = self.store(rec).get_byte(rec.rec, rec.pos, -1);
                // @PLN25 — `get_byte(.., -1)` applies a -1 shift, so a valid variant is
                // `e_nr` in `0..values.len()` (stored byte 1 = variant 0).  A null/absent
                // inline enum reads NEGATIVE here (stored 0 → -1, an absent source rec →
                // i32::MIN) and carries no payload claims; skip rather than index `values`
                // out of bounds (matches `validate_claims`'s `>= 0` arm).  Covers
                // `vr[i] = null` and whole-vector copy of a null element.
                if e_nr >= 0 && (e_nr as usize) < values.len() {
                    let tp = values[e_nr as usize].0;
                    // Do not copy claims on simple enumerate types.
                    if tp != u16::MAX {
                        self.copy_claims(rec, to, tp);
                    }
                }
            }
            // The kinds with nothing to claim, each saying why — the arm set is
            // EXHAUSTIVE on purpose, so a new `Parts` variant fails the build here
            // instead of silently joining the not-copied set.  Its sibling
            // `validate_claims` was made exhaustive for the same reason a day earlier;
            // this is the keystone Cluster C folded every source enumeration onto, so a
            // kind missing from it is a kind the deep-copy walks past.
            //
            // A non-text `Base` and the four narrow-integer widths hold a VALUE in the
            // record's own bytes.  The block copy that precedes this walk already moved
            // those bytes; there is no second record to claim.
            Parts::Base
            | Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::ShortRaw(_, _)
            | Parts::Int(_, _) => {}
            // A stored 12-byte `DbRef` — the closure half of a `fn(…)` struct field — is
            // deliberately NOT re-pointed: the copy keeps the source's pointer and ALIASES
            // its closure record, which is why a bound fn-ref field read is marked
            // `skip_free` (`tests/scripts/85-poison-return-tail-uaf.loft` t5).
            //
            // ⚠ That is sound only because the copy can never outlive the source, and
            // what guarantees it lives in ANOTHER file: #318 refuses a capturing-closure
            // holder as a return value, as a collection element and as a field of another
            // struct, and a fn-ref struct field admits ONE capture shape program-wide.
            // Measured 2026-08-22 — every escape route is refused at compile time, and
            // the two legal copies (passed down by value; copied out of an inner scope)
            // answer correctly on both backends under `LOFT_POISON=1` with store churn.
            // Relax any of those refusals and this arm becomes a use-after-free with
            // nothing at this site to catch it.
            Parts::DbRef => {}
        }
    }

    /// @P317 debug — `LOFT_LOG=copy_check` (or `LOFT_COPY_CHECK=1`): after a
    /// deep-copy, walk SOURCE and DESTINATION in parallel and warn about every
    /// nested vector/array/hash length that DIFFERS — the signature of a
    /// `copy_claims` bug that silently inflates or truncates a nested
    /// collection (the kind valgrind can't see and that surfaces only as a
    /// wrong / non-deterministic result).  Read once, cached; zero cost off.
    #[inline]
    #[allow(clippy::unused_self)] // `&self` is for ergonomic call sites; the flag is a cached static
    pub(crate) fn copy_check_enabled(&self) -> bool {
        static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *FLAG.get_or_init(|| {
            std::env::var("LOFT_COPY_CHECK").is_ok()
                || std::env::var("LOFT_LOG")
                    .is_ok_and(|v| v.split([',', ':', ' ']).any(|p| p.trim() == "copy_check"))
        })
    }

    /// Does `tp` own nested heap worth recursing into?  Primitive vector
    /// elements (u8/integer/single/…) have none, so a length compare at the
    /// vector level suffices — skipping them avoids walking every element of a
    /// multi-thousand-element vector.
    fn has_nested_heap(&self, tp: u16) -> bool {
        matches!(
            self.types[tp as usize].parts,
            Parts::Struct(_)
                | Parts::EnumValue(_, _)
                | Parts::Vector(_)
                | Parts::Sorted(_, _)
                | Parts::Array(_)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Index(_, _, _)
                | Parts::ChildRec(_)
                | Parts::Enum(_)
        ) || tp == 5 // text
    }

    /// @P317 — entry point for the `copy_check` validator.  Compares the
    /// structure of a just-copied `src`/`dst` pair and prints `[copy_check]`
    /// warnings (deduped by field path, so a vector doesn't spam per element).
    /// Warn-and-continue: never panics, so one run gives a full census.
    pub fn report_copy_mismatches(&self, src: &DbRef, dst: &DbRef, tp: u16, label: &str) {
        if !self.copy_check_enabled() {
            return;
        }
        let mut seen = std::collections::HashSet::new();
        self.walk_copy_cmp(src, dst, tp, label.to_string(), &mut seen);
    }

    fn walk_copy_cmp(
        &self,
        src: &DbRef,
        dst: &DbRef,
        tp: u16,
        path: String,
        seen: &mut std::collections::HashSet<String>,
    ) {
        match &self.types[tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                let fields = fields.clone();
                for f in fields {
                    let s = DbRef {
                        store_nr: src.store_nr,
                        rec: src.rec,
                        pos: src.pos + u32::from(f.position),
                    };
                    let d = DbRef {
                        store_nr: dst.store_nr,
                        rec: dst.rec,
                        pos: dst.pos + u32::from(f.position),
                    };
                    self.walk_copy_cmp(&s, &d, f.content, format!("{path}.{}", f.name), seen);
                }
            }
            Parts::Vector(v) | Parts::Sorted(v, _) => {
                let v = *v;
                let sl = vector::length_vector(src, &self.allocations);
                let dl = vector::length_vector(dst, &self.allocations);
                if sl != dl && seen.insert(path.clone()) {
                    crate::loft_eprintln!(
                        "[copy_check] MISMATCH {path}: src_len={sl} dst_len={dl} (vector elem kt={v})"
                    );
                }
                if self.has_nested_heap(v) {
                    let s_cur = self.store(src).get_u32_raw(src.rec, src.pos);
                    let d_cur = self.store(dst).get_u32_raw(dst.rec, dst.pos);
                    if s_cur != 0 && d_cur != 0 {
                        let size = u32::from(self.size(v));
                        for i in 0..sl.min(dl) {
                            let s = DbRef {
                                store_nr: src.store_nr,
                                rec: s_cur,
                                pos: 8 + size * i,
                            };
                            let d = DbRef {
                                store_nr: dst.store_nr,
                                rec: d_cur,
                                pos: 8 + size * i,
                            };
                            self.walk_copy_cmp(&s, &d, v, format!("{path}[]"), seen);
                        }
                    }
                }
            }
            Parts::Array(v) | Parts::Ordered(v, _) => {
                let v = *v;
                let sl = vector::length_vector(src, &self.allocations);
                let dl = vector::length_vector(dst, &self.allocations);
                if sl != dl && seen.insert(path.clone()) {
                    crate::loft_eprintln!(
                        "[copy_check] MISMATCH {path}: src_len={sl} dst_len={dl} (array/ordered kt={v})"
                    );
                }
            }
            Parts::Hash(v, _) => {
                let v = *v;
                let s_cur = self.store(src).get_u32_raw(src.rec, src.pos);
                let d_cur = self.store(dst).get_u32_raw(dst.rec, dst.pos);
                if s_cur == 0 || d_cur == 0 {
                    return;
                }
                let s_room = self.store(src).get_u32_raw(s_cur, 0);
                let d_room = self.store(dst).get_u32_raw(d_cur, 0);
                let s_len = self.store(src).get_u32_raw(s_cur, 4);
                let d_len = self.store(dst).get_u32_raw(d_cur, 4);
                if s_len != d_len && seen.insert(format!("{path}#count")) {
                    crate::loft_eprintln!(
                        "[copy_check] MISMATCH {path}: src_count={s_len} dst_count={d_len} (hash)"
                    );
                }
                // copy_claims_hash_body copies bucket-by-bucket, so when the two
                // tables have equal `room` the bucket index pairs src↔dst.
                if s_room == d_room && self.has_nested_heap(v) {
                    let elms = s_room.saturating_sub(1) * 2;
                    for i in 0..elms {
                        let se = self.store(src).get_u32_raw(s_cur, 8 + 4 * i);
                        let de = self.store(dst).get_u32_raw(d_cur, 8 + 4 * i);
                        if se != 0 && de != 0 {
                            let s = DbRef {
                                store_nr: src.store_nr,
                                rec: se,
                                pos: 8,
                            };
                            let d = DbRef {
                                store_nr: dst.store_nr,
                                rec: de,
                                pos: 8,
                            };
                            self.walk_copy_cmp(&s, &d, v, format!("{path}[]"), seen);
                        }
                    }
                }
            }
            Parts::ChildRec(content_kt) => {
                let content_kt = *content_kt;
                let sc = self.store(src).get_u32_raw(src.rec, src.pos);
                let dc = self.store(dst).get_u32_raw(dst.rec, dst.pos);
                if sc != 0 && dc != 0 {
                    let s = DbRef {
                        store_nr: src.store_nr,
                        rec: sc,
                        pos: 8,
                    };
                    let d = DbRef {
                        store_nr: dst.store_nr,
                        rec: dc,
                        pos: 8,
                    };
                    self.walk_copy_cmp(&s, &d, content_kt, path, seen);
                }
            }
            Parts::Enum(values) => {
                let e_nr = self.store(src).get_byte(src.rec, src.pos, -1);
                if let Some(&(vtp, _)) = values.get(e_nr as usize)
                    && vtp != u16::MAX
                {
                    // Re-borrow-safe: `vtp` is copied out before recursing.
                    self.walk_copy_cmp(src, dst, vtp, path, seen);
                }
            }
            _ => {}
        }
    }

    /**
    Remove claimed data for a record. Both strings and substructures are freed.
    It will not free the record itself because that might be a part of a vector.

    Reads the [`Stores::for_each_owned_child`] keystone for every cascade kind
    (struct / enum / vector / sorted / array / ordered / hash / index / childrec):
    one walk recurses into each owned child, frees the per-element record it lived
    in, then frees the container block and clears the field pointer.  Only the text
    leaf and the (unimplemented) `Radix` teardown stay special-cased.
    # Panics
    When a field points to a spatial structure (teardown unimplemented).
    */
    /// Mark the collection slot `dest` addresses as ABSENT rather than empty (loft#1150).
    ///
    /// Zero is the EMPTY collection, so a slot left at its zero-init cannot mean absence;
    /// `DbRef::ABSENT_REC` is the id reserved for it (loft#917), which
    /// `vector::is_absent_collection` reads raw and every other reader maps back to `0`.
    ///
    /// Used where a keyed assignment's SOURCE is absent: the destination keeps its store —
    /// there is nowhere else for a later `+=` to build — and reads back `null` rather than
    /// `[]`, which is the distinction `xs == null` and `xs == []` rest on.  Called from both
    /// `OpReplaceKeyed` bodies, which is why it lives here rather than twice over.
    pub fn mark_collection_absent(&mut self, dest: &DbRef) {
        if dest.store_nr == u16::MAX || dest.rec == 0 || dest.pos == 0 {
            return;
        }
        self.store_mut(dest)
            .set_u32_raw(dest.rec, dest.pos, DbRef::ABSENT_REC);
    }

    pub fn remove_claims(&mut self, rec: &DbRef, tp: u16) {
        self.remove_claims_mode(rec, tp, false);
    }

    /// Release a SECONDARY VIEW of a linked collection group: drop this route to
    /// the records, never the records (loft#898).
    ///
    /// Two keyed fields over one element type in one struct are two views of a
    /// single record set, so exactly one of them owns it.  Clearing the other
    /// through [`Self::remove_claims`] freed the shared records and left the
    /// primary reading freed memory — a length that still says 2 over bytes that
    /// read back as `4294967296:null`.
    pub fn remove_claims_view(&mut self, rec: &DbRef, tp: u16) {
        self.remove_claims_mode(rec, tp, true);
    }

    /// `OpClearKeyed`'s release, decoding the [`CLEAR_KEYED_VIEW`] bit its `tp`
    /// operand carries.
    ///
    /// The bit is set by the parser, which is the only layer that can ask the
    /// schema whether the field being cleared is a secondary view. Both backends
    /// call THIS — the interpreter through the `#rust` template in
    /// `default/01_code.loft`, the native generator through
    /// `codegen_runtime::OpClearKeyed` — so the decode exists once and the two
    /// cannot drift.
    pub fn remove_claims_keyed(&mut self, rec: &DbRef, raw_tp: u16) {
        self.remove_claims_mode(
            rec,
            raw_tp & !CLEAR_KEYED_VIEW,
            raw_tp & CLEAR_KEYED_VIEW != 0,
        );
    }

    fn remove_claims_mode(&mut self, rec: &DbRef, tp: u16, borrowed: bool) {
        // loft#796 — the record itself must be inside its store before anything reads
        // its fields.  The per-edge guards in `for_each_owned_child` catch a rotten
        // POINTER; this catches a rotten RECORD, which is the same fault one level up
        // and the one that names the caller rather than a field of a field.
        if rec.store_nr != u16::MAX
            && (rec.store_nr as usize) < self.allocations.len()
            && u64::from(rec.pos) >= u64::from(self.store(rec).capacity_words()) * 8
        {
            self.refuse_owned_edge(rec, tp, rec.pos, "record (position)");
            return;
        }
        // A null/absent container (the `store_nr == u16::MAX` sentinel) owns nothing to tear
        // down — no-op, mirroring `free_named`'s own guard. The `for_each_owned_child` keystone
        // guards absent CHILDREN (`cur != 0`), but not a null CONTAINER; without this a nullable
        // DbRef passed as the container would index `allocations[u16::MAX]` and panic. The
        // invariant now holds HERE, not by every caller pre-checking the container.
        if rec.store_nr == u16::MAX {
            return;
        }
        // A view's own text/leaf fields do not exist — it is a collection, never a
        // scalar — so the borrowed mode only ever reaches the cascade arm below.
        match &self.types[tp as usize].parts {
            Parts::Base if tp == 5 => {
                // Text leaf: free the string record and clear the pointer.  Not a
                // cascade kind (no owned children), so it stays out of the keystone.
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur != 0 {
                    let cap = self.store(rec).capacity_words();
                    // (d) cluster-462 stale-interior-claim guard — the slice the
                    // copy-source detectors miss. The DESTINATION record's text-field
                    // pointer is read here and `delete()`-ed; if the dst slot was reused
                    // and this field holds a STALE pointer past the store's end, the
                    // delete reads a record header OOB → SIGSEGV (release skips the
                    // bounds debug_assert). Under LOFT_UAF* name the bad field and SKIP
                    // the delete (leak, not crash) so the run continues and the site is
                    // located — this is what catches the #462 sim.loft:3546 fault.
                    if crate::keys::uaf_any_enabled() && cur >= cap {
                        let rec_ok = self.store(rec).valid(rec.rec, rec.pos);
                        let (sfree, skt) = {
                            let a = &self.allocations[rec.store_nr as usize];
                            (a.free, a.known_type)
                        };
                        thread_local! {
                            static RPT: std::cell::RefCell<std::collections::HashSet<(u16, u32, u32)>> =
                                std::cell::RefCell::new(std::collections::HashSet::new());
                        }
                        if RPT.with(|s| s.borrow_mut().insert((rec.store_nr, rec.rec, rec.pos))) {
                            crate::loft_eprintln!(
                                "[uaf-claim] remove_claims: TEXT field at store #{} rec={} pos={} \
                                 holds STALE pointer cur={cur} past store end (cap_words={cap}; \
                                 slot free={sfree} known_type={skt} rec_pos_valid={rec_ok}) — dst \
                                 record has a dangling interior claim (reused-slot or borrowed-ref \
                                 over-free); skipping delete to avoid SIGSEGV",
                                rec.store_nr,
                                rec.rec,
                                rec.pos,
                            );
                        }
                        self.store_mut(rec).set_u32_raw(rec.rec, rec.pos, 0);
                    } else {
                        let store = self.store_mut(rec);
                        store.delete(cur);
                        store.set_u32_raw(rec.rec, rec.pos, 0);
                    }
                }
            }
            // `Radix` teardown is unimplemented; the keystone yields nothing for
            // it, so guard explicitly to preserve the loud failure (a silent no-op
            // would leak).
            // Every owned-child cascade kind (Struct/Enum/Vector/Sorted/Array/
            // Ordered/Hash/Index/ChildRec) reads the SINGLE keystone walk: recurse
            // into each child, free the per-element record it lived in (Array/Hash/
            // Index), then free the container block and clear the field pointer.
            // The historical loop-bound / slot-drift / length-header bugs
            // (@P290/@P306/@P318/@P309) lived in the per-dispatcher copies of this
            // walk; reading it once removes them by construction.
            _ => {
                let walk = self.owned_walk(rec, tp, borrowed);
                for c in walk.children {
                    // @PLN102 heap-free audit — an `owning_elem == Some(0)` slot is an
                    // ABSENT element record (Array/Ordered/Hash/Index). Recursing into it
                    // would walk reserved record 0 as if it were a live element, and the
                    // `delete(0)` below would free it. Unreachable today (the length/slots
                    // invariant holds it off), guarded by construction so a future desync
                    // can never fault here. Mirrors the copy-side skip in
                    // `copy_claims_array_body`.
                    if c.owning_elem == Some(0) {
                        continue;
                    }
                    self.remove_claims_mode(&c.child, c.child_tp, c.borrowed);
                    if let Some(elm) = c.owning_elem {
                        self.store_mut(rec).delete(elm);
                    }
                }
                for extra in walk.extra_recs {
                    self.store_mut(rec).delete(extra);
                }
                if let Some(cur) = walk.container_rec {
                    self.store_mut(rec).delete(cur);
                }
                if walk.zero_field {
                    self.store_mut(rec).set_u32_raw(rec.rec, rec.pos, 0);
                }
            }
        }
    }

    /// `LOFT_WATCH_STORE` (cluster-462 write-watch) — read-only sibling of `remove_claims`:
    /// walk the record's text fields and return the first whose stored pointer is
    /// out-of-bounds (`>= capacity_words`, i.e. would fault a later `delete`/`get_str`),
    /// as `(field_pos, bad_pointer)`. Never deletes, never recurses through a bad pointer
    /// (it only follows OWNED-CHILD edges, which `for_each_owned_child` derives from the
    /// type, not from heap pointers — so the walk itself cannot fault). Used to NAME the
    /// copy that first wrote a garbage text-pointer into the watched store.
    #[must_use]
    pub fn first_oob_text(&self, rec: &DbRef, tp: u16) -> Option<(u32, u32)> {
        match &self.types[tp as usize].parts {
            Parts::Base if tp == 5 => {
                let store = self.store(rec);
                let cur = store.get_u32_raw(rec.rec, rec.pos);
                if cur != 0 && cur >= store.capacity_words() {
                    Some((rec.pos, cur))
                } else {
                    None
                }
            }
            _ => {
                let walk = self.for_each_owned_child(rec, tp);
                for c in walk.children {
                    if let Some(hit) = self.first_oob_text(&c.child, c.child_tp) {
                        return Some(hit);
                    }
                }
                None
            }
        }
    }

    /// `LOFT_WATCH_STORE` write-watch — call right AFTER a copy/append that may have
    /// written `to` (type `tp`). If the watched store now holds an out-of-bounds text
    /// pointer, report the writing op (pc/line via `crash_report`) + whether the SOURCE
    /// already had the bad pointer (`src_oob` distinguishes propagated-from-source vs
    /// introduced-here). `ctx` names the path. No-op unless `to.store_nr` is watched.
    pub fn watch_oob_text(&self, to: &DbRef, tp: u16, src: Option<&DbRef>, ctx: &str) {
        if crate::keys::watch_store() != Some(to.store_nr) {
            return;
        }
        let Some((bad_pos, bad_cur)) = self.first_oob_text(to, tp) else {
            return;
        };
        let src_oob = src.and_then(|s| self.first_oob_text(s, tp));
        let (pc, _op, _d) = crate::crash_report::last_context();
        let line = crate::crash_report::source_loc_for_pc(pc).map_or(0, |p| p.line);
        let cap = self.store(to).capacity_words();
        thread_local! {
            static RPT_W: std::cell::RefCell<std::collections::HashSet<(u16, u32, u32)>> =
                std::cell::RefCell::new(std::collections::HashSet::new());
        }
        if RPT_W.with(|s| s.borrow_mut().insert((to.store_nr, to.rec, bad_pos))) {
            let (s_nr, s_rec, s_pos) = src.map_or((u16::MAX, 0, 0), |s| (s.store_nr, s.rec, s.pos));
            crate::loft_eprintln!(
                "[watch-store/{ctx}] OOB text-ptr now in watched store #{}: dst rec={} field \
                 pos={bad_pos} holds ptr={bad_cur} (cap_words={cap}, tp={tp}) — src=#{s_nr}(rec={s_rec},\
                 pos={s_pos}) src_oob_text={src_oob:?} — at pc={pc} (line {line})",
                to.store_nr,
                to.rec,
            );
        }
    }

    /// @PLAN38 — bind the Store at `slot` to a file at `path`, returning
    /// `true` on success.  Dryopea-driven "the hash IS the file" entry
    /// point: a freshly-allocated container (e.g. `hash<X[k]>::new()`)
    /// can be re-rooted onto disk in one call, after which all mutations
    /// are durable via mmap/msync without any explicit save loop.
    ///
    /// Two modes, selected by whether the file already exists:
    ///
    /// 1. **Fresh file (path does not exist or is empty):** the current
    ///    in-memory Store's bytes are padded out to ≥ 1024 words with a
    ///    valid trailing free block, written to disk via
    ///    `MmapStorage::open` + `resize`, then `Store::open` mmaps it
    ///    back.  Caller-side `DbRef`s into this slot stay valid because
    ///    the on-disk record layout is byte-identical to the in-memory
    ///    one we just copied.
    ///
    /// 2. **Existing file:** `Store::open(path)` validates the SIGNATURE
    ///    and rebuilds the free-list; the in-memory state at this slot is
    ///    dropped in favour of the on-disk image.  Caller-side `DbRef`s
    ///    remain valid IFF the on-disk record at `rec=1` describes the
    ///    same type as the in-memory container the caller just made —
    ///    this is the load-on-startup path and assumes the consumer
    ///    pinned the type at allocation time (typical pattern: allocate
    ///    an empty `hash<X[k]>`, immediately call `bind_path`, then read
    ///    keys back).
    ///
    /// Failure modes (return `false`):
    /// - `slot >= self.allocations.len()`.
    /// - Path is empty / not valid UTF-8 / I/O error writing the snapshot.
    /// - Existing-file branch encounters a bad SIGNATURE (caught via
    ///   `std::panic::catch_unwind`, since `Store::open` panics on
    ///   format mismatch).
    ///
    /// Metadata preserved across the swap: `ref_count`, `known_type`,
    /// `free`, `created_at`, `last_op_at`, `lock_origin` (the bookkeeping
    /// the `Stores` collection needs to keep the slot accounted for in
    /// the bitmap and the rc walk).  Cleared on the new Store:
    /// `claims` (init() wipes it; matches `database_named` post-swap
    /// state), `borrowed`, `read_only`.
    ///
    /// Off when the `mmap` feature is disabled: returns `false`.
    #[cfg(feature = "mmap")]
    pub fn bind_path(&mut self, slot: u16, path: &std::path::Path) -> bool {
        let slot_idx = slot as usize;
        if slot_idx >= self.allocations.len() {
            return false;
        }
        let path_str = match path.to_str() {
            Some(s) if !s.is_empty() => s,
            _ => return false,
        };

        // Distinguish fresh vs existing by file size — a valid loft Store
        // file is always ≥ 8 bytes (SIGNATURE + free-space index).  A
        // smaller or missing file is "fresh" and triggers the
        // snapshot-then-mmap path.
        let exists = std::fs::metadata(path).is_ok_and(|m| m.len() >= 8);

        // Preserve the slot's bookkeeping across the swap.
        let preserved = {
            let s = &self.allocations[slot_idx];
            (s.known_type, s.free, s.created_at, s.last_op_at, s.pinned)
        };

        if !exists {
            // @PLN134 — lay the tries out BEFORE the snapshot. This is the image a
            // reader will page, and a trie numbered in insertion order costs it 27
            // pages of 64 KB per prefix query instead of 3. Semantics-preserving, so
            // the live store simply keeps the better layout too.
            let laid = self.relayout_trees(slot);
            if laid > 0 && std::env::var_os("LOFT_LOADER_STATS").is_some() {
                crate::loft_eprintln!("store_persist_bind: laid out {laid} tree(s) for paging");
            }
            // FRESH PATH — snapshot current bytes, size the image to the data,
            // write to disk, then re-open via mmap.
            let src_words = self.allocations[slot_idx].capacity_words();
            let snapshot = {
                let s = &self.allocations[slot_idx];
                let raw =
                    unsafe { std::slice::from_raw_parts(s.base_ptr(), (src_words as usize) * 8) };
                raw.to_vec()
            };
            // loft#710: the arena's CAPACITY is not what the store holds — the
            // tail past the last record is whatever the last 7/3 growth
            // over-allocated, and persisting it made the file size a fact about
            // how the store was BUILT rather than what is in it.  Size the image
            // from the high-water mark instead.
            //
            // Not to the mark exactly: a bound store stays live, and growth
            // multiplies by 7/3, so a store with no slack left pays a 2.33×
            // file resize on its very next claim — measurably WORSE than the
            // tail we removed.  An eighth keeps ordinary post-bind activity off
            // that cliff while staying a function of the content, which is the
            // property the issue is about: two builds of the same data now land
            // within ~5% of each other instead of 1.84× apart.
            //
            // Never below the floor `Store::open` expects, and a chain the walk
            // cannot follow falls back to the whole capacity — the file is never
            // shortened on a guess.
            //
            // The eighth is NOT clamped to the arena's current capacity, and
            // that is a fix, not an omission (@PLN123).  This read
            // `.min(src_words)` — "never larger than the capacity we would have
            // written before" — which was a safe conservatism while capacity was
            // always comfortably above the mark.  `store_reclaim` (arc A) trims
            // capacity TO the mark, so the clamp collapsed the eighth to zero
            // for exactly the stores that had just been tidied, and the first
            // claim after binding then paid the 7/3 ladder the paragraph above
            // exists to avoid.  Measured on a 2,000-record hash: reclaim-then-
            // bind wrote 187,784 bytes and the first READ of the collection took
            // it to 438,160 — 2.07x LARGER than never reclaiming at all
            // (211,256), because iterating a keyed collection claims its
            // key-sorted snapshot inside the store.  The slack is deliberate;
            // it has to survive a caller who tidied up first.
            let live_end = store_image_live_end(&snapshot, src_words).unwrap_or(src_words);
            let target_words =
                crate::store::slack_target(live_end).max(crate::store::MIN_BOUND_WORDS);
            let padded = match build_padded_store_image(&snapshot, src_words, target_words) {
                Some(b) => b,
                None => return false,
            };
            if std::fs::write(path, &padded).is_err() {
                return false;
            }
        }

        // Both branches: open the file via Store::open.  Wrapped in
        // catch_unwind because Store::open panics on signature mismatch
        // for the existing-file branch — we surface that as a clean
        // `false` return instead of crashing the interpreter.
        let new_store = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::store::Store::open(path_str)
        }));
        let mut new_store = match new_store {
            Ok(s) => s,
            Err(_) => return false,
        };

        // Re-apply preserved metadata onto the new Store.  Slot bitmap sees
        // continuity; the on-disk bytes carry the user data.
        new_store.set_known_type(preserved.0);
        new_store.free = preserved.1;
        new_store.set_created_at(preserved.2);
        new_store.last_op_at = preserved.3;
        new_store.pinned = preserved.4;

        self.allocations[slot_idx] = new_store;
        // @PLN123 B3 — the OTHER load moment, and the one the plan is really
        // about: a bound store is what a long-lived store normally is.  Only on
        // the EXISTING-file branch — the fresh branch is a WRITE, where a
        // program's interior references are still live and moving records would
        // dangle them (§ B2).
        if exists && Self::compact_on_load_enabled() {
            self.compact_bound_store(slot, path);
        }
        // @PLN97 3b.5 — record the layout identity beside the store
        // (`<path>.dschema`) so a later (possibly remote) working-set load can
        // reject a mismatched layout BEFORE range-reading foreign bytes at
        // schema-derived offsets. Best-effort: a store without a sidecar just
        // falls back to the post-copy `store_verify` backstop on load.
        if preserved.0 != u16::MAX {
            let id = crate::schema_sidecar::LayoutIdentity::of(self, &[preserved.0]);
            if id.write_beside(path).is_err() && std::env::var_os("LOFT_LOADER_STATS").is_some() {
                crate::loft_eprintln!(
                    "store_persist_bind: could not write layout sidecar beside {}",
                    path.display()
                );
            }
        }
        true
    }

    /// @PLN134 — write a persisted IMAGE of the collection in `slot`, laid out
    /// for PAGING, and leave the live store exactly where it was.
    ///
    /// The difference from [`bind_path`](Self::bind_path) is the whole point.
    /// Binding snapshots the live bytes, so every record keeps its position and
    /// the caller's `DbRef`s stay valid — a guarantee `store_persist_bind`
    /// documents and must keep. But position IS the layout: a record number is a
    /// word offset, so records claimed in insertion order are scattered across
    /// the image, and a prefix query that returns 20 of them touches ~20 pages of
    /// 64 KB to read a few hundred bytes each.
    ///
    /// So this writes a REBUILT copy instead. The rebuild re-claims every record
    /// in its collection's own walk order — KEY order for a `trie` — and the copy
    /// is a scratch store nobody holds a reference into, which is what makes
    /// moving records legal here and not in the bind path. Measured on a 74 692
    /// word vocabulary, one 20-record prefix query: 22 requests / 1.25 MB as
    /// built, against 4 / 0.26 MB from this image.
    ///
    /// Nodes are laid out too, by the same [`relayout_trees`](Self::relayout_trees)
    /// pass binding runs — on the COPY, so the live tree keeps its numbering.
    ///
    /// The image is sized to its content with no growth slack: binding leaves an
    /// eighth because the file goes on being the live arena, and this file is a
    /// artefact to read or ship. Answers `false` on any I/O or format error, and
    /// on a store with no schema to walk — never a panic, matching every other
    /// store builtin.
    #[cfg(feature = "mmap")]
    pub fn persist_copy(&mut self, slot: u16, path: &std::path::Path) -> bool {
        use crate::store::PRIMARY;
        let idx = slot as usize;
        if idx >= self.allocations.len() {
            return false;
        }
        let (tp, root_words) = {
            let s = &self.allocations[idx];
            if s.free || s.known_type == u16::MAX {
                return false;
            }
            let header = *s.addr::<i32>(PRIMARY, 0);
            if header <= 0 {
                return false;
            }
            (s.known_type, header as u32)
        };
        // A shape the rebuild cannot carry would come out of the copy wrong, and
        // wrong is worse than refused — the same gate compaction applies.
        if !self.type_is_compactable(tp, &mut Vec::new()) {
            return false;
        }
        let Ok(dst_slot) = self.rebuild_into_scratch(slot, tp, root_words) else {
            return false;
        };
        // On the COPY: this is the image a reader pages, and the live tree keeps
        // the numbering its own references were handed out against.
        let laid = self.relayout_trees(dst_slot);
        let rebuilt = self.take_store(dst_slot);
        self.release_slot(dst_slot);

        let src_words = rebuilt.capacity_words();
        let snapshot = {
            let raw =
                unsafe { std::slice::from_raw_parts(rebuilt.base_ptr(), (src_words as usize) * 8) };
            raw.to_vec()
        };
        drop(rebuilt);
        let live_end = store_image_live_end(&snapshot, src_words).unwrap_or(src_words);
        let target_words = live_end.max(crate::store::MIN_BOUND_WORDS);
        let Some(padded) = build_padded_store_image(&snapshot, src_words, target_words) else {
            return false;
        };
        if std::fs::write(path, &padded).is_err() {
            return false;
        }
        // Same sidecar the bind path writes, for the same reason: a paged or
        // remote loader gates on it BEFORE range-reading foreign bytes, and an
        // image written by this call is exactly the one that gets served.
        let id = crate::schema_sidecar::LayoutIdentity::of(self, &[tp]);
        if id.write_beside(path).is_err() && std::env::var_os("LOFT_LOADER_STATS").is_some() {
            crate::loft_eprintln!(
                "store_persist_copy: could not write layout sidecar beside {}",
                path.display()
            );
        }
        if std::env::var_os("LOFT_LOADER_STATS").is_some() {
            crate::loft_eprintln!(
                "store_persist_copy: wrote {} bytes, laid out {laid} tree(s) for paging",
                padded.len()
            );
        }
        true
    }

    /// No-op shim when the `mmap` feature is disabled — the image writer needs
    /// the same file support binding does.
    #[cfg(not(feature = "mmap"))]
    #[allow(clippy::unused_self)]
    pub fn persist_copy(&mut self, _slot: u16, _path: &std::path::Path) -> bool {
        false
    }

    /// No-op shim when the `mmap` feature is disabled.  Always returns
    /// `false` so consumers branch into their non-mmap fallback (today,
    /// JSON via `text as Struct`).  Avoids `cfg`-gating every caller.
    #[cfg(not(feature = "mmap"))]
    #[allow(clippy::unused_self)]
    pub fn bind_path(&mut self, _slot: u16, _path: &std::path::Path) -> bool {
        false
    }

    /// Load a persisted store image at `path` into `slot`, HEAP-backed — the
    /// portable, non-durable counterpart of [`bind_path`](Stores::bind_path).
    /// Unlike `bind_path` there is no fresh/create branch (you load an
    /// EXISTING store) and no mmap, so it works on **every** backend — this is
    /// the piece wasm lacked.  The slot's empty store is replaced by a heap
    /// store copied from the file, keeping the slot bookkeeping.  Returns
    /// `false` on a missing / truncated / wrong-format file (via the
    /// `Store::load` signature check under `catch_unwind`), never a misread.
    /// @PLN97 arc G Phase 1 (#522).
    pub fn load_path(&mut self, slot: u16, path: &std::path::Path) -> bool {
        let slot_idx = slot as usize;
        if slot_idx >= self.allocations.len() {
            return false;
        }
        if !self.schema_gate_ok(slot, path) {
            return false;
        }
        let path_str = match path.to_str() {
            Some(s) if !s.is_empty() => s,
            _ => return false,
        };
        // load needs an existing, plausibly-valid store file (≥ the header) — on
        // EITHER filesystem, so a `--html` page can read a pack out of its own page
        // tree rather than being told there is no such file (@PLN146 F4).
        if !crate::store::image_at_least(path_str, 16) {
            return false;
        }

        // Preserve the slot's bookkeeping across the swap (mirrors bind_path).
        let preserved = {
            let s = &self.allocations[slot_idx];
            (s.known_type, s.free, s.created_at, s.last_op_at, s.pinned)
        };

        // Store::load panics on a bad signature / unreadable file — surface
        // that as a clean `false` instead of crashing the interpreter.
        let new_store = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::store::Store::load(path_str)
        }));
        let mut new_store = match new_store {
            Ok(s) => s,
            Err(_) => return false,
        };

        new_store.set_known_type(preserved.0);
        new_store.free = preserved.1;
        new_store.set_created_at(preserved.2);
        new_store.last_op_at = preserved.3;
        new_store.pinned = preserved.4;

        self.allocations[slot_idx] = new_store;
        // @PLN123 B2 — the load moment is the one place records may move (see
        // `compact_slot`).  ON by default; `LOFT_NO_COMPACT_ON_LOAD` turns it off
        // and leaves the loaded store exactly as the file had it, which is the
        // first bisect step for a wrong answer or a stall inside `store_load`.
        if Self::compact_on_load_enabled() && std::env::var_os("LOFT_LOADER_STATS").is_some() {
            // Say WHY when it does not run: a refusal that reads the same as a
            // dense store is a refusal nobody can test or diagnose.
            match self.compact_slot(slot) {
                Ok(words) => {
                    crate::loft_eprintln!("store_load: compacted #{slot} — {words} words returned")
                }
                Err(why) => crate::loft_eprintln!("store_load: not compacted #{slot} — {why}"),
            }
        } else if Self::compact_on_load_enabled() {
            let _ = self.compact_slot(slot);
        }
        true
    }

    /// @PLN123 B3 — compact a store that was just BOUND to an existing file,
    /// and put the smaller image back on disk.
    ///
    /// The mmap case of compaction-at-load.  `compact_slot` leaves the slot
    /// holding a dense HEAP store, so the file still carries the old bytes and
    /// has to be rewritten and re-mapped — "the hash IS the file" has to keep
    /// being true.
    ///
    /// Every failure path leaves the caller with a correct store:
    /// - compaction declines → nothing happened, the mapped store stands;
    /// - the write fails → the ORIGINAL file is untouched (the image goes to a
    ///   temp file and is renamed over, so the file is never half-written), and
    ///   the store is re-bound from it;
    /// - the re-open fails → the same re-bind, from the file that is now the
    ///   compacted one.
    ///
    /// The re-bind on failure is what keeps the contract: a bound collection
    /// must come back file-backed or not at all, never as a heap store that
    /// silently stops persisting.
    #[cfg(feature = "mmap")]
    fn compact_bound_store(&mut self, slot: u16, path: &std::path::Path) {
        let stats = std::env::var_os("LOFT_LOADER_STATS").is_some();
        let saved = match self.compact_slot(slot) {
            Ok(words) => words,
            Err(why) => {
                if stats {
                    crate::loft_eprintln!("store_persist_bind: not compacted #{slot} — {why}");
                }
                return;
            }
        };
        // The slot now holds a dense heap store; put it on disk and map it back.
        //
        // loft#730 — at the CONTENT size plus the format's eighth, not at the
        // arena's capacity. `compact_slot` builds the dense store by claiming
        // into a fresh arena, so its capacity is whatever the arena's own 7/3
        // growth last landed on — and writing that raw put the growth tail
        // straight back into the file the compaction had just earned. Measured
        // on a 100-record store: compaction brought the live bytes from
        // 5 468 440 to 3 227 032 (1.008x its content, a perfect rebuild) while
        // the file only went 6 152 000 -> 5 172 832, because 1 945 792 bytes of
        // arena tail were written out with it. The next pass then read the file
        // as dense and declined, so the loss was permanent.
        //
        // This is the same fact `bind_path` already applies to the image it
        // writes, and it is deliberately the same call: the eighth exists
        // because a store trimmed to exactly its content pays a 2.33x resize on
        // its very next claim, and merely READING a keyed collection claims a
        // snapshot inside it (loft#710, loft#727).
        // `build_padded_store_image` is what makes the shorter image a valid
        // store: it copies up to the last live record and writes ONE free block
        // covering the rest, so the free chain never points past EOF. A raw
        // slice of the arena would carry a free list describing bytes the file
        // no longer has.
        let image = {
            let s = &self.allocations[slot as usize];
            let cap = s.capacity_words();
            let raw = unsafe { std::slice::from_raw_parts(s.base_ptr(), (cap as usize) * 8) };
            let live_end = store_image_live_end(raw, cap).unwrap_or(cap);
            let target = crate::store::slack_target(live_end)
                .max(crate::store::MIN_BOUND_WORDS)
                .min(cap);
            match build_padded_store_image(raw, cap, target) {
                Some(b) => b,
                // Sizing failed (a malformed chain): write what compaction
                // produced rather than lose it.
                None => raw.to_vec(),
            }
        };
        let rebound = write_image_atomic(path, &image).is_ok() && self.rebind(slot, path);
        if !rebound {
            // Could not land the compacted image.  Re-bind from whatever the
            // file holds — the original bytes if the write never happened, the
            // compacted ones if it did — so the collection is file-backed
            // either way.
            if !self.rebind(slot, path) && stats {
                crate::loft_eprintln!(
                    "store_persist_bind: #{slot} could not be re-bound after compaction"
                );
            }
            return;
        }
        if stats {
            crate::loft_eprintln!("store_persist_bind: compacted #{slot} — {saved} words returned");
        }
    }

    /// Re-open `path` and adopt it into `slot`, keeping the slot's bookkeeping.
    /// The tail both branches of [`bind_path`] end in, and the recovery step
    /// [`compact_bound_store`](Self::compact_bound_store) leans on.
    #[cfg(feature = "mmap")]
    fn rebind(&mut self, slot: u16, path: &std::path::Path) -> bool {
        let Some(path_str) = path.to_str() else {
            return false;
        };
        let preserved = {
            let s = &self.allocations[slot as usize];
            (s.known_type, s.free, s.created_at, s.last_op_at, s.pinned)
        };
        let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::store::Store::open(path_str)
        }));
        let Ok(mut store) = opened else {
            return false;
        };
        store.set_known_type(preserved.0);
        store.free = preserved.1;
        store.set_created_at(preserved.2);
        store.last_op_at = preserved.3;
        store.pinned = preserved.4;
        self.allocations[slot as usize] = store;
        true
    }

    /// Is compaction-on-load switched on?  @PLN123 B3 turns it ON by default —
    /// `LOFT_NO_COMPACT_ON_LOAD` opts out, the same shape as loft's other
    /// default-on behaviours.  Read once.
    ///
    /// It can be a default because it is gated: a dense store is measured and
    /// left alone (`compact_slot`), so the common case pays one chain walk on a
    /// path that already walks the chain to rebuild the free tree.
    fn compact_on_load_enabled() -> bool {
        static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *FLAG.get_or_init(|| std::env::var_os("LOFT_NO_COMPACT_ON_LOAD").is_none())
    }

    /// Can a collection of type `tp` be rebuilt by [`Stores::copy_claims`]?
    ///
    /// Exhaustive over `Parts` with **no catch-all**, so a variant added later
    /// is a compile error here rather than a silently-skipped shape — the whole
    /// point of the check is that an unhandled kind must REFUSE, and a
    /// `_ => true` would refuse nothing.  `Radix` is the one live refusal:
    /// `copy_claims` panics on it and the `for_each_owned_child` keystone
    /// returns an empty walk, so a spatial index would come back empty.
    /// loft#730 — can a value of `tp` own a heap record? Decides whether a
    /// vector's ELEMENTS must be walked or the container alone is the whole
    /// story, which is the difference between visiting 5 000 records and
    /// 5 600 000 of them on a real store.
    fn type_owns_heap(&self, tp: u16, seen: &mut Vec<u16>) -> bool {
        if tp as usize >= self.types.len() || seen.contains(&tp) {
            return false;
        }
        seen.push(tp);
        match &self.types[tp as usize].parts {
            // text and reference are the two heap-owning primitives.
            Parts::Base => matches!(tp, 5 | 6),
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => fields
                .clone()
                .iter()
                .any(|f| self.type_owns_heap(f.content, seen)),
            Parts::Byte(..)
            | Parts::Short(..)
            | Parts::Int(..)
            | Parts::ShortRaw(..)
            | Parts::Enum(_)
            | Parts::DbRef => false,
            _ => true,
        }
    }

    /// @PLN134, @PLN136 — lay every radix TREE in `slot` out for PAGING, and answer
    /// how many.
    ///
    /// A trie's node ids are handed out in INSERTION order, which puts a root→leaf
    /// path nowhere near itself. Measured over 978 842 words, one prefix query then
    /// touches 27 pages of 64 KB to read ~330 bytes of nodes — so a reader that
    /// range-reads the image fetches 1.7 MB to answer a keystroke, and downloading
    /// the whole thing is cheaper by the fourth one. Renumbered van Emde Boas the
    /// same query touches 2.8.
    ///
    /// Runs when an IMAGE IS WRITTEN, because that is the copy a reader pages and
    /// the one moment the whole tree is in hand. It rewrites node ids only — the
    /// records, their order and every answer are untouched — so a caller that keeps
    /// using the store afterwards sees no difference beyond a faster walk.
    ///
    /// Both kinds that share the tree, because both are read the same way: a `trie`
    /// answers a prefix and a `spatial` answers a bounding box, and the numbering
    /// that decides what either costs is one numbering. A capped box query over 3.2 M
    /// real map points touches 222 pages of 64 KB as built and 3.6 renumbered
    /// (@PLN136) — the trie's 27 → 2.8, in a shape the trie's measurement did not
    /// cover.
    ///
    /// Only trees in `slot` are laid out. The walk can reach another store through
    /// a `ChildRec` or a cross-store element, and rewriting a tree the caller did
    /// not ask to persist would be a surprise, so the pass drops those. It does not
    /// descend through a tree's own ELEMENTS either — a trie of tries is not a shape
    /// worth spending a million walk steps to rule out.
    pub(crate) fn relayout_trees(&mut self, slot: u16) -> u32 {
        let idx = slot as usize;
        if idx >= self.allocations.len() {
            return 0;
        }
        let tp = self.allocations[idx].known_type;
        if tp == u16::MAX {
            return 0;
        }
        // The DATA walk is skipped entirely where the SCHEMA cannot hold a tree.
        // Persisting is a whole-image write on every kind of store, and a walk over
        // a million hash elements to conclude "no trees here" would be a cost every
        // one of them pays for a kind it does not have. The type graph answers the
        // same question in the number of TYPES.
        if !self.type_has_tree(tp, &mut Vec::new()) {
            return 0;
        }
        let root = DbRef {
            store_nr: slot,
            rec: crate::store::PRIMARY,
            pos: 8,
        };
        // Collected first, applied second: the walk reads the schema through `&self`
        // and the pass needs `&mut Store`.
        let mut found = Vec::new();
        let mut budget = 200_000u32;
        self.tree_roots(&root, tp, &mut budget, &mut found);
        let mut done = 0;
        for at in found {
            if at.store_nr != slot {
                continue;
            }
            let store = &mut self.allocations[at.store_nr as usize];
            let tree = store.get_u32_raw(at.rec, at.pos);
            if tree != 0 && crate::radix_tree::rtree_relayout(store, tree) {
                done += 1;
            }
        }
        done
    }

    /// Can a value of `tp` contain a radix TREE — a `trie` or a `spatial` — at all?
    ///
    /// A question about the SCHEMA, so it costs the number of types rather than the
    /// number of records — which is what lets [`relayout_trees`](Self::relayout_trees)
    /// decline instantly on the stores that have no tree in them.
    fn type_has_tree(&self, tp: u16, seen: &mut Vec<u16>) -> bool {
        if tp as usize >= self.types.len() || seen.contains(&tp) {
            return false;
        }
        seen.push(tp);
        match &self.types[tp as usize].parts {
            // Both kinds ARE the tree, so neither is descended into further here.
            Parts::Trie(_, _) | Parts::Radix(_, _) => true,
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => fields
                .clone()
                .iter()
                .any(|f| self.type_has_tree(f.content, seen)),
            Parts::Vector(c)
            | Parts::Sorted(c, _)
            | Parts::Array(c)
            | Parts::Ordered(c, _)
            | Parts::Hash(c, _)
            | Parts::ChildRec(c) => self.type_has_tree(*c, seen),
            Parts::Index(c, _, _) => self.type_has_tree(*c, seen),
            // Spelled out rather than closed with `_`: a new collection kind that
            // can hold a trie must be named here, and the compiler is what says so.
            // DATABASE.md § "Adding or changing a collection kind".
            Parts::Base
            | Parts::Byte(..)
            | Parts::Short(..)
            | Parts::Int(..)
            | Parts::ShortRaw(..)
            | Parts::Enum(_)
            | Parts::DbRef => false,
        }
    }

    /// Every radix-tree field reachable from `rec` — a `trie` or a `spatial` — as
    /// the `DbRef` holding its tree id.
    ///
    /// `budget` stops the walk on a pathological graph, the same way
    /// [`intra_record_slack`](Self::intra_record_slack) bounds its own: running out
    /// lays out fewer trees, which costs pages on a query and never correctness.
    fn tree_roots(&self, rec: &DbRef, tp: u16, budget: &mut u32, out: &mut Vec<DbRef>) {
        if *budget == 0 || tp as usize >= self.types.len() {
            return;
        }
        *budget -= 1;
        match &self.types[tp as usize].parts {
            Parts::Trie(_, _) | Parts::Radix(_, _) => out.push(*rec),
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in fields.clone() {
                    // A field that owns no heap record cannot hold a collection, so
                    // the descent stops at the scalars rather than at every field.
                    if self.type_owns_heap(f.content, &mut Vec::new()) {
                        let at = DbRef {
                            store_nr: rec.store_nr,
                            rec: rec.rec,
                            pos: rec.pos + u32::from(f.position),
                        };
                        self.tree_roots(&at, f.content, budget, out);
                    }
                }
            }
            _ => {
                for c in self.for_each_owned_child(rec, tp).children {
                    self.tree_roots(&c.child, c.child_tp, budget, out);
                }
            }
        }
    }

    /// loft#730 — words of INTRA-RECORD slack reachable from `rec`: for every
    /// vector container, its record size minus what its length actually needs.
    ///
    /// This is the quantity `usage()` cannot see. `usage()` walks the block
    /// chain and reports free space BETWEEN records; a vector left at 7/4 of its
    /// content by `Store::resize` is a fully claimed, entirely live record, so a
    /// store made of nothing else reports as dense — which is why compaction
    /// declined the very shape it would have paid best on.
    ///
    /// A LOWER bound by construction, like the interior estimate it joins:
    /// `budget` stops the walk on a pathological graph and the shortfall only
    /// ever makes the gate more reluctant, never less.
    fn intra_record_slack(&self, rec: &DbRef, tp: u16, budget: &mut u32) -> u32 {
        if *budget == 0 || tp as usize >= self.types.len() {
            return 0;
        }
        *budget -= 1;
        match &self.types[tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                let fields = fields.clone();
                let mut total = 0u32;
                for f in &fields {
                    if self.type_owns_heap(f.content, &mut Vec::new()) {
                        let at = DbRef {
                            store_nr: rec.store_nr,
                            rec: rec.rec,
                            pos: rec.pos + u32::from(f.position),
                        };
                        total =
                            total.saturating_add(self.intra_record_slack(&at, f.content, budget));
                    }
                }
                total
            }
            Parts::Vector(v) | Parts::Sorted(v, _) => {
                let v = *v;
                let cur = self.store(rec).get_u32_raw(rec.rec, rec.pos);
                if cur == 0 {
                    return 0;
                }
                let length = vector::length_vector(rec, &self.allocations);
                let esize = u32::from(self.size(v));
                // What `copy_claims_seq_vector` would claim for the same content.
                let need = 1 + esize.saturating_mul(length).div_ceil(8);
                let have = self.store(rec).record_words(cur);
                let mut total = have.saturating_sub(need);
                if self.type_owns_heap(v, &mut Vec::new()) {
                    for i in 0..length {
                        let at = DbRef {
                            store_nr: rec.store_nr,
                            rec: cur,
                            pos: 8 + esize * i,
                        };
                        total = total.saturating_add(self.intra_record_slack(&at, v, budget));
                        if *budget == 0 {
                            break;
                        }
                    }
                }
                total
            }
            // Keyed / per-element containers: the keystone already knows how to
            // enumerate one child per ENTRY, which is the granularity wanted.
            Parts::Array(_)
            | Parts::Ordered(_, _)
            | Parts::Hash(_, _)
            | Parts::Index(_, _, _)
            | Parts::ChildRec(_) => {
                let walk = self.for_each_owned_child(rec, tp);
                let mut total = 0u32;
                for c in walk.children {
                    total =
                        total.saturating_add(self.intra_record_slack(&c.child, c.child_tp, budget));
                    if *budget == 0 {
                        break;
                    }
                }
                total
            }
            _ => 0,
        }
    }

    fn type_is_compactable(&self, tp: u16, seen: &mut Vec<u16>) -> bool {
        if tp as usize >= self.types.len() {
            return false;
        }
        if seen.contains(&tp) {
            return true; // already accepted on this path — a cycle, not a new shape
        }
        seen.push(tp);
        let ok = match &self.types[tp as usize].parts {
            Parts::Base
            | Parts::Byte(..)
            | Parts::Short(..)
            | Parts::Int(..)
            | Parts::ShortRaw(..)
            | Parts::Enum(_) => true,
            // A stored DbRef names another store, and this rebuild only moves
            // records WITHIN one.  Nothing would rewrite a foreign pointer at
            // the far end, so refuse rather than guess.
            Parts::DbRef => false,
            // A `trie` rebuilds through `copy_claims_trie_body`, which re-inserts
            // every element into a fresh tree — and does it in KEY order, which is
            // the placement @PLN134 wants an image to have. It was refused here
            // when the kind landed (#804), appended to `Radix`'s refusal rather
            // than reasoned about on its own; the deep copy it needs already
            // existed and was already used for every other copy of a trie.
            // `a_trie_survives_the_rebuild_and_comes_out_in_key_order` is the
            // proof, and it asserts placement as well as every answer.
            Parts::Trie(..) => self.type_is_compactable(
                match &self.types[tp as usize].parts {
                    Parts::Trie(v, _) => *v,
                    _ => unreachable!("matched Trie"),
                },
                seen,
            ),
            // A `spatial` rebuilds through `copy_claims_radix_body`, which re-inserts
            // every element into a fresh tree — in MORTON order, which is the
            // placement @PLN136 measured. It was refused here while nobody had taken
            // that measurement; over 3.2 M real map points, one capped box query
            // reads 204 pages of 64 KB from an as-inserted image and 1.7 from a
            // Morton-ordered one. `a_spatial_survives_the_rebuild_and_comes_out_in
            // _morton_order` is the proof, and it asserts placement as well as every
            // answer.
            Parts::Radix(..) => self.type_is_compactable(
                match &self.types[tp as usize].parts {
                    Parts::Radix(v, _) => *v,
                    _ => unreachable!("matched Radix"),
                },
                seen,
            ),
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => fields
                .clone()
                .iter()
                .all(|f| self.type_is_compactable(f.content, seen)),
            Parts::Vector(v)
            | Parts::Sorted(v, _)
            | Parts::Array(v)
            | Parts::Ordered(v, _)
            | Parts::Hash(v, _)
            | Parts::Index(v, _, _)
            | Parts::ChildRec(v) => self.type_is_compactable(*v, seen),
        };
        seen.pop();
        ok
    }

    /// Rebuild the collection rooted in `slot` into a fresh SCRATCH slot, and
    /// answer that slot.  The caller owns it: `take_store` + `release_slot`.
    ///
    /// The rebuild is a deep copy through [`Stores::copy_claims`], so the
    /// destination's records are claimed in the order each kind's own walk hands
    /// them out — which for a `trie` is KEY order (`trie_db::records` walks the
    /// tree in-order), and that is the placement a paged prefix query is cheap
    /// on. It is the same pass for every caller; what differs is what they do
    /// with the result, so it lives here once rather than in each.
    ///
    /// This moves records by construction. It is therefore safe only where the
    /// caller can account for every interior `DbRef` — see [`compact_slot`]'s
    /// note, which is the reasoning in full.
    ///
    /// [`compact_slot`]: Self::compact_slot
    fn rebuild_into_scratch(
        &mut self,
        slot: u16,
        tp: u16,
        root_words: u32,
    ) -> Result<u16, &'static str> {
        use crate::store::PRIMARY;
        // The destination's FIRST claim is the root, so it lands at PRIMARY —
        // where the source has it, and where the caller's reference points.
        let mut dst = crate::store::Store::new_in_use(root_words + PRIMARY + 1);
        let dst_root = dst.claim(root_words);
        if dst_root != PRIMARY {
            return Err("a fresh store did not hand out PRIMARY first");
        }
        // The size the destination's claim ACTUALLY handed out, which is not
        // always what was asked for: `claim_block` only splits a block that is
        // more than a third larger than the request, so a request that lands
        // within that margin of the whole arena keeps the whole arena.  Here it
        // always does — the arena is `root_words + 2` and the request is
        // `root_words`.  Writing `root_words` back would shrink the block by the
        // words it owns, and the words past the shortened header become a block
        // whose own header is zero, which `claim_scan` walks by adding zero to
        // its position: an unbounded loop inside the next `claim`, on a store
        // the program only asked to LOAD (`store_load` never returned).
        let dst_header = *dst.addr::<i32>(PRIMARY, 0);
        // Carry the root block's raw bytes first: `copy_claims` rebuilds the
        // heap children and rewrites the pointers into them, but says nothing
        // about any inline bytes sharing the record.
        let dst_slot = self.adopt_store(dst);
        let src = DbRef {
            store_nr: slot,
            rec: PRIMARY,
            pos: 8,
        };
        let to = DbRef {
            store_nr: dst_slot,
            rec: PRIMARY,
            pos: 8,
        };
        self.copy_block_cross_store(
            slot,
            PRIMARY,
            0,
            dst_slot,
            PRIMARY,
            0,
            (root_words as isize) * 8,
        );
        // Re-assert the destination's own block header: the raw copy above
        // brought the SOURCE's size word with it, and the two blocks are the
        // same size only when the destination's claim happened not to round up.
        *self.allocations[dst_slot as usize].addr_mut::<i32>(PRIMARY, 0) = dst_header;
        self.copy_claims(&src, &to, tp);
        // The scratch carries the schema it was rebuilt from. `compact_slot` sets
        // this on the way out and could leave it until then; a caller that runs a
        // SCHEMA-driven pass over the scratch cannot — `relayout_trees` reads
        // `known_type` and silently does nothing without it, which is a layout
        // quietly not applied rather than an error.
        self.allocations[dst_slot as usize].set_known_type(tp);
        Ok(dst_slot)
    }

    /// @PLN123 B2 — rebuild the collection rooted in `slot` into a densely
    /// packed store and swap it in.  Returns the words the arena shrank by; 0
    /// when compaction did not run or bought nothing.
    ///
    /// **Sound only where an interior `DbRef` cannot be live**, which is the
    /// LOAD path and nowhere else.  Compaction moves records, and a `DbRef` is
    /// a POSITION — `(store_nr, rec, pos)` — so anything holding one into this
    /// store's interior is left naming the wrong record.  Those references live
    /// in interpreter stack slots and native locals; nothing can enumerate or
    /// rewrite them.
    ///
    /// `bind_path`'s WRITE branch is emphatically not such a place: a program
    /// keeps element references across `store_persist_bind`, and the reason
    /// that works today is exactly what compaction removes — the image is a
    /// byte-for-byte copy, so every record keeps its position.  The load path
    /// is safe for a stronger reason than "probably nobody holds one": it
    /// already replaces the slot's bytes wholesale, so an interior reference
    /// held across it is already meaningless.
    ///
    /// The collection ROOT does survive — the collection variable is a `DbRef`
    /// at `(slot, PRIMARY, 8)` — so it must not move.  It does not: the root
    /// record is the destination's first claim, and a fresh store hands out
    /// `PRIMARY` first.
    fn compact_slot(&mut self, slot: u16) -> Result<u32, &'static str> {
        use crate::store::PRIMARY;
        let idx = slot as usize;
        if idx >= self.allocations.len() {
            return Err("slot out of range");
        }
        let (tp, before_words, root_words) = {
            let s = &self.allocations[idx];
            if s.free {
                return Err("store is free");
            }
            if s.read_only || s.is_borrowed() {
                return Err("store is read-only or borrowed");
            }
            if s.has_durable_sidecar() {
                // Same reason `shrink_to` refuses (F4): the sidecar records the
                // file's byte length and CRC, so rewriting the image behind its
                // back reports a healthy store as corrupt.
                return Err("a durable sidecar records this file's length and CRC");
            }
            if s.known_type == u16::MAX {
                return Err("store records no type, so there is no schema to walk");
            }
            let header = *s.addr::<i32>(PRIMARY, 0);
            if header <= 0 {
                return Err("the root block is free or malformed");
            }
            (s.known_type, s.capacity_words(), header as u32)
        };
        if !self.type_is_compactable(tp, &mut Vec::new()) {
            return Err("the collection holds a shape the rebuild cannot carry");
        }
        // The gate, read BEFORE anything is built: a rebuild that recovers
        // nothing still costs a full copy of every record.
        //
        // The estimate is the INTERIOR — the free space between surviving
        // records — and it is a LOWER BOUND on what a rebuild returns, not a
        // prediction of it.  Measured, recovery runs consistently ABOVE it:
        //
        //     inner  2%  ->  refused          inner 17%  ->  24% recovered
        //     inner  9%  ->  refused          inner 26%  ->  33% recovered
        //     inner  0%  ->   0% recovered    inner 43%  ->  57% recovered
        //
        // because a rebuild ALSO right-sizes live structures that `inner` counts
        // as data — a hash keeps the bucket array it grew to at its peak, and a
        // fresh one is sized for what survived.  So the gate is conservative by
        // construction: it never fires where nothing is to be had, and declines
        // some cases that would have paid moderately.
        //
        // An eighth is the threshold because that is the slack the image format
        // already carries deliberately (`bind_path` sizes an image at the
        // high-water mark PLUS an eighth, loft#710) — a chosen bias for a
        // DEFAULT-ON behaviour, where declining a marginal win costs a little
        // space and taking a marginal loss costs every load.
        {
            let u = self.allocations[idx].usage();
            if !u.walk_complete {
                return Err("the block chain does not tile the store");
            }
            if u.live_end_words <= crate::store::MIN_BOUND_WORDS {
                // A bound store's image is padded up to this floor regardless,
                // so there is no file to give back; a heap store this small is
                // under 8 KB.  Either way the rebuild would buy nothing.
                return Err("the store is at or below the image floor");
            }
            let interior = u.live_end_words.saturating_sub(u.claimed_words);
            // loft#730 — the interior is only HALF of what a rebuild returns,
            // and on a store built by appending it is the smaller half. A vector
            // left at 7/4 of its content by `Store::resize` is a fully claimed,
            // entirely live record, so a store made of nothing else is dense by
            // the measure above and was declined — the shape compaction pays
            // best on was the one shape it refused. Measured on a 100-record
            // store: interior said 2%, and rebuilding returned 41%.
            let mut budget = 200_000u32;
            let slack = self.intra_record_slack(
                &DbRef {
                    store_nr: slot,
                    rec: PRIMARY,
                    pos: 8,
                },
                tp,
                &mut budget,
            );
            if interior.saturating_add(slack).saturating_mul(8) <= u.live_end_words {
                return Err(
                    "free space between records and slack inside them are together under the \
                     eighth the image format already allows",
                );
            }
        }

        let dst_slot = self.rebuild_into_scratch(slot, tp, root_words)?;

        let mut rebuilt = self.take_store(dst_slot);
        // `take_store` leaves a freed sentinel but does NOT set the slot's free
        // BIT — it is written for a store that is handed out to outlive the
        // table, not for a scratch slot.  Compaction runs once per load, so
        // without this the slot number is burned every time: `find_free_slot`
        // only ever returns a slot whose bit is SET, so the sentinel would sit
        // there forever and `allocations` would grow per load.  The scratch
        // slot is ours, so releasing it is ours too.
        self.release_slot(dst_slot);
        let after_words = rebuilt.capacity_words();
        if after_words >= before_words {
            // Nothing gained — leave the loaded store exactly as it is.
            return Err("already dense");
        }
        // The slot's bookkeeping is the slot's, not the buffer's.
        let s = &self.allocations[idx];
        rebuilt.set_known_type(tp);
        rebuilt.free = s.free;
        rebuilt.set_created_at(s.created_at);
        rebuilt.last_op_at = s.last_op_at;
        rebuilt.pinned = s.pinned;
        self.allocations[idx] = rebuilt;
        Ok(before_words - after_words)
    }

    /// Adopt an in-memory, already-authenticity-verified byte buffer as the store
    /// in `slot` — the slice counterpart of [`Stores::load_path`] (a heap copy
    /// keeping the slot's bookkeeping).  Returns `false` (a clean reject, never a
    /// misread/crash) when the slot is out of range or the buffer is not a
    /// well-formed store image (bad signature / too small).  The caller MUST have
    /// established the bytes' authenticity first — see [`Stores::load_url_verified`].
    /// @PLN97 arc G Phase 0.
    pub fn load_bytes(&mut self, slot: u16, bytes: &[u8]) -> bool {
        let slot_idx = slot as usize;
        if slot_idx >= self.allocations.len() {
            return false;
        }
        // Preserve the slot's bookkeeping across the swap (mirrors load_path).
        let preserved = {
            let s = &self.allocations[slot_idx];
            (s.known_type, s.free, s.created_at, s.last_op_at, s.pinned)
        };
        let mut new_store = match crate::store::Store::from_bytes(bytes) {
            Some(s) => s,
            None => return false,
        };
        new_store.set_known_type(preserved.0);
        new_store.free = preserved.1;
        new_store.set_created_at(preserved.2);
        new_store.last_op_at = preserved.3;
        new_store.pinned = preserved.4;
        self.allocations[slot_idx] = new_store;
        true
    }

    /// @PLN97 arc G Phase 0 — load a persisted store IMAGE over HTTP(S) from a
    /// TRUSTED source into `slot`, establishing authenticity BEFORE the bytes are
    /// adopted: fetch the whole image, verify its SHA-256 against the caller-
    /// pinned `sha256_hex`, and only on a match adopt it in memory (the bytes
    /// never touch disk).  A fetch error OR a hash mismatch REFUSES the load
    /// (returns `false`, adopts nothing) — the same fetch→verify→trust discipline
    /// the registry install path uses (`integrity::verify_sha256`), bridged
    /// onto the store loader.  `url` may be `http(s)://` or `file://`
    /// (offline / testing).  This is the whole-file counterpart of the paged
    /// `load_key(s)` / `load_range` loaders; per-page authenticity for the paged
    /// path is a separate (Merkle-tree) problem, out of Phase 0's scope.
    ///
    /// Available wherever [`crate::net::fetch_bytes`] can fetch — including the
    /// browser (loft#678).  It must track its TRUSTED twin
    /// [`load_url`](Stores::load_url) exactly: a target that can fetch a whole image
    /// but cannot PIN its hash offers only the weaker of the two, which is the wrong
    /// asymmetry to ship to a page loading third-party bytes.
    #[cfg(any(
        feature = "registry",
        all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))
    ))]
    pub fn load_url_verified(&mut self, slot: u16, url: &str, sha256_hex: &str) -> bool {
        let bytes = match crate::net::fetch_bytes(url) {
            Ok(b) => b,
            Err(e) => {
                crate::loft_eprintln!("store loader: refusing {url} — fetch failed: {e}");
                return false;
            }
        };
        if let Err(e) = crate::integrity::verify_sha256(&bytes, sha256_hex) {
            crate::loft_eprintln!("store loader: refusing {url} — {e}");
            return false;
        }
        self.load_bytes(slot, &bytes)
    }

    /// @PLN97 arc G Phase 0 — load a whole store IMAGE over HTTP(S)/`file://` from
    /// a **TRUSTED** source (no authenticity check) into `slot` — the *instant*
    /// counterpart of [`Stores::load_url_verified`].  Skips the SHA-256 pin (you
    /// trust the origin) but is still **structurally safe**: `load_bytes` →
    /// `Store::from_bytes` runs `validate_structure`, so a corrupt/malformed
    /// image is rejected (`false`), never adopted — the heap invariant holds
    /// regardless.  Use `load_url_verified` when the source is untrusted.
    ///
    /// Available wherever [`crate::net::fetch_bytes`] can fetch: any native build
    /// with the `registry` client, AND the browser (`--html`) target, where the
    /// fetch is bridged to JS `fetch()` via the asyncify host import — so the
    /// loft-visible `store_load_url_trusted(r, url) -> boolean` is identical on
    /// both, per the same-internal-API requirement.
    #[cfg(any(
        feature = "registry",
        all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))
    ))]
    pub fn load_url(&mut self, slot: u16, url: &str) -> bool {
        let bytes = match crate::net::fetch_bytes(url) {
            Ok(b) => b,
            Err(e) => {
                crate::loft_eprintln!("store loader: {url} — fetch failed: {e}");
                return false;
            }
        };
        self.load_bytes(slot, &bytes)
    }

    /// @PLN97 arc G Phase 2 — load a store IMAGE from a local file that may be
    /// **UNTRUSTED** into `slot`, the structurally-validated counterpart of
    /// [`Stores::load_path`].  Reads the whole file and adopts it via
    /// `load_bytes` → `Store::from_bytes`, which runs the always-on
    /// `validate_structure` gate — so a crafted / corrupt file cannot hang
    /// (0-size block) or drive a heap over-read; it is rejected (`false`).
    /// `load_path` (the trusted path) validates only in debug and is faster;
    /// use this for a file whose provenance you don't control.
    pub fn load_path_untrusted(&mut self, slot: u16, path: &std::path::Path) -> bool {
        if !self.schema_gate_ok(slot, path) {
            return false;
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(_) => return false,
        };
        self.load_bytes(slot, &bytes)
    }

    /// loft#700 — does the store at `path` have the layout this program will READ it
    /// with?
    ///
    /// A whole-image load keeps the target slot's `known_type` and reinterprets the
    /// file's bytes through it, and records are fixed-stride: add one field to a stored
    /// struct and every older record is read at the new, larger stride.  Nothing
    /// announced it — `len()` on the added collection returned wild values
    /// (`510277628`, `2135492622`) and iterating one read arbitrary memory.  A consumer
    /// shipped a field on the strength of a probe that happened to use a layout where
    /// the overrun landed on zeroes.
    ///
    /// The `.dschema` sidecar already records the layout the file was written with, and
    /// the paged loaders already gate on it — this is the same gate on the whole-image
    /// path.  A store with no sidecar (legacy, or written by another tool) still loads:
    /// the check can only report what it knows, and refusing every unlabelled store
    /// would break programs that are reading their own correct data.
    fn schema_gate_ok(&self, slot: u16, path: &std::path::Path) -> bool {
        let known_type = self.allocations[slot as usize].known_type;
        if known_type == u16::MAX {
            return true; // untyped store — no identity to compare against
        }
        let current = crate::schema_sidecar::LayoutIdentity::of(self, &[known_type]);
        // An unreadable sidecar (an I/O failure, not a layout answer) tells us nothing
        // about the layout, so it neither passes nor fails the store: keep the previous
        // behaviour rather than refuse a load on a filesystem hiccup.
        let Ok(verdict) = crate::schema_sidecar::check_beside(path, &current) else {
            return true;
        };
        if verdict.is_raw_safe() {
            return true;
        }
        // Say what changed.  A refused load otherwise reads as "the file is missing",
        // which sends the reader to the wrong half of their program.
        let detail = match &verdict {
            crate::schema_sidecar::SchemaVerdict::Changed(diff) => {
                let mut parts = Vec::new();
                if !diff.changed.is_empty() {
                    parts.push(format!("reshaped: {}", diff.changed.join(", ")));
                }
                if !diff.added.is_empty() {
                    parts.push(format!("only in this program: {}", diff.added.join(", ")));
                }
                if !diff.dropped.is_empty() {
                    parts.push(format!("only in the store: {}", diff.dropped.join(", ")));
                }
                parts.join("; ")
            }
            _ => "its recorded layout could not be read".to_string(),
        };
        crate::loft_eprintln!(
            "store_load: refusing {} — it was written with a different layout than this \
             program reads it with, so its records would be read at the wrong stride \
             ({detail}).  Rebuild the store with this program, or load it with the \
             version that wrote it.",
            path.display()
        );
        false
    }

    /// @PLN97 3b.5 — the layout-identity gate for a working-set load. Returns
    /// `false` (reject) when the store's `.dschema` sidecar records a layout that
    /// differs from THIS program's collection type — so the loader never
    /// range-reads foreign-layout bytes at schema-derived offsets (a silent
    /// corruption / wild-pointer hazard the post-hoc `store_verify` should not be
    /// the only guard against). An ABSENT sidecar (a legacy / pre-3b.5 store) or
    /// an untyped local store passes; `store_verify` stays the backstop. `path`
    /// is a local file or an `http(s)://` URL — the sidecar is read from beside
    /// it (`<path>.dschema`), over the same transport.
    #[cfg(paged_store)]
    fn layout_gate_ok(&mut self, path: &str, local: &DbRef) -> bool {
        let known_type = self.allocations[local.store_nr as usize].known_type;
        if known_type == u16::MAX {
            return true; // untyped store — no identity to compare against
        }
        let current = crate::schema_sidecar::LayoutIdentity::of(self, &[known_type]);
        let verdict = crate::paged_reader::check_sidecar(path, &current);
        if verdict.is_raw_safe() {
            return true;
        }
        // A layout mismatch is a rare, important safety event — always surface it
        // so a rejected load is not silently mistaken for "key absent". Through
        // `refuse_paged` rather than its own `eprintln`, so the program can read
        // it back too: a refusal reachable by one route and not another is the
        // same silence in a different place (loft#802).
        let reason = format!(
            "its recorded layout differs from this program's (verdict: \
             {verdict:?}); not range-reading foreign bytes"
        );
        self.refuse_paged(path, &reason);
        false
    }

    /// @PLN97 arc G Phase 4 / 3b.7 — load the entries with integer key in
    /// `[lo, hi]` from a persisted SORTED collection into the (empty) local
    /// `sorted<T[k]>`, fetching only the pages the range walk touches. The
    /// range-friendly counterpart of `load_key(s)` — routing's tile-window
    /// fetch. Returns the count loaded; refuses a non-sorted / non-copyable
    /// collection. A Sorted collection is a sorted INLINE vector, so the source
    /// range is already ordered: build `local`'s vector directly in key order
    /// (no per-element sort), then relocate each element's heap graph. Root + key
    /// schema come from `local`'s live type (same collection type ⇒ same
    /// structural root position in the image), NOT the raw bytes.
    #[cfg(paged_store)]
    pub fn load_range(&mut self, local: &DbRef, path: &str, lo: i64, hi: i64) -> i64 {
        if !self.layout_gate_ok(path, local) {
            return 0;
        }
        use crate::paged_reader::{PageSource, PagedReader};
        let Ok(source) = PageSource::open(path) else {
            self.refuse_paged(path, "it cannot be opened as a paged source");
            return 0;
        };
        let mut reader = PagedReader::new(source);
        let tp = self.allocations[local.store_nr as usize].known_type;
        if tp == u16::MAX {
            self.refuse_paged(path, "the target collection's store has no recorded type");
            return 0;
        }
        // Range needs an ordered collection — `Sorted`, or its promoted `Ordered`
        // form (a sorted collection whose element type is shared, renamed by
        // `finish()`; without this a promoted sorted refused even as a plain local).
        //
        // NOT descended into a struct field like the hash loaders (#632): a paged
        // range over a sorted collection declared as a struct field reads the flat
        // element vector at the field's claim slot, which holds for a plain sorted
        // but NOT for a promoted/linked `ordered` field (its elements sit behind an
        // indirection the positional reader can't follow — whole-image `store_load`
        // handles it). Since promotion depends on unrelated program structure,
        // supporting the field form here would work or silently mis-read depending
        // on whether the element type is shared. So a sorted collection declared as
        // a field refuses AUDIBLY (its `known_type` is the wrapper struct) — the
        // remaining @PLN97 arc G work. Hash-as-field IS supported (Hash never
        // promotes, so its field layout is stable).
        let Some(Parts::Sorted(content_tp, _) | Parts::Ordered(content_tp, _)) =
            self.types.get(tp as usize).map(|t| &t.parts)
        else {
            let reason = self.wrong_collection_reason(tp, "a sorted collection");
            self.refuse_paged(path, &reason);
            return 0;
        };
        let content_tp = *content_tp;
        if !self.is_copyable_entry(content_tp) {
            let reason = self.not_copyable_reason(content_tp);
            self.refuse_paged(path, &reason);
            return 0;
        }
        let keys = self.keys(tp).to_vec();
        let esize = u32::from(self.size(content_tp));
        if esize == 0 {
            return 0;
        }
        let lo_c = [crate::keys::Content::Long(lo)];
        let hi_c = [crate::keys::Content::Long(hi)];
        let (src_rec, positions) = crate::paged_reader::sorted_range_positions(
            &mut reader,
            local.rec,
            local.pos,
            esize,
            &lo_c,
            &hi_c,
            &keys,
        );
        let count = positions.len() as u32;
        if count == 0 {
            return 0;
        }
        let store_nr = local.store_nr;
        // Build the local sorted vector: header (8 bytes) + count elements.
        let words = (8 + count * esize).div_ceil(8).max(2);
        let vec_rec = self.allocations[store_nr as usize].claim(words);
        self.allocations[store_nr as usize].zero_fill(vec_rec);
        self.allocations[store_nr as usize].set_u32_raw(vec_rec, 4, count); // length
        self.allocations[store_nr as usize].set_u32_raw(local.rec, local.pos, vec_rec);

        for (i, &epos) in positions.iter().enumerate() {
            let dst_pos = 8 + (i as u32) * esize;
            // Copy the element's field bytes (esize is 4-aligned — all fields are).
            let mut b = 0u32;
            while b + 4 <= esize {
                let w = reader.u32_at(src_rec, epos + b);
                self.allocations[store_nr as usize].set_u32_raw(vec_rec, dst_pos + b, w);
                b += 4;
            }
            // Relocate this element's heap graph into local.
            self.relocate_ptr_fields(
                &mut reader,
                vec_rec,
                dst_pos,
                src_rec,
                epos,
                content_tp,
                store_nr,
            );
        }

        if std::env::var_os("LOFT_LOADER_STATS").is_some() {
            crate::loft_eprintln!(
                "store_load_range: [{lo},{hi}] loaded={count} bytes_fetched={} file={}",
                reader.provider().bytes_fetched(),
                reader.size()
            );
        }
        i64::from(count)
    }

    /// Verify a store-rooted collection `r`'s heap graph is structurally sound —
    /// every interior pointer (text offset, vector/child rec, hash bucket +
    /// entries) stays within its store's bounds. Reuses the DEFENSIVE
    /// [`validate_claims`](Stores::validate_claims) walk (guard-before-deref —
    /// it never faults on a wild pointer, it NAMES the broken edge). Its type
    /// comes from the live schema (`known_type`). Returns `true` when sound;
    /// `false` (with `[cr-check]` reasons on stderr) otherwise.
    ///
    /// This is the instrument that makes the relocating working-set copy
    /// (Phase 3b) *checkable*: after a `store_load*`, `store_verify(local)`
    /// proves the copy left no pointer aimed outside the store.
    pub fn verify_graph_ok(&self, r: &DbRef) -> bool {
        let tp = self.allocations[r.store_nr as usize].known_type;
        if tp == u16::MAX {
            return false;
        }
        let tp = self.type_at(r, tp);
        let mut problems = 0u32;
        self.validate_claims(r, tp, "store_verify", &mut problems);
        problems == 0
    }

    /// loft#790 — the type of what `r` actually POINTS AT, given the type its
    /// store was allocated for.
    ///
    /// A store knows one type: the record allocated into it. That is the right
    /// answer for `store_verify(f)` and the wrong one for `store_verify(f.people)`
    /// — a collection declared as a struct FIELD lives in the WRAPPER's store, so
    /// the walk read the hash root as a `Firm` and reported a corruption that was
    /// not there (the bucket rec `1701667649` is `eman`, the wrapper's `name`
    /// text read as a pointer).
    ///
    /// The two are told apart by `pos`, which was there all along: a ref to the
    /// record itself starts at the payload (8), and a ref to a field carries that
    /// field's byte offset. So the field is looked up by offset — no guessing
    /// which of several collections was meant, and no reliance on the wrapper
    /// having exactly one.
    fn type_at(&self, r: &DbRef, tp: u16) -> u16 {
        if r.pos <= 8 {
            return tp; // the record itself
        }
        let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) = &self.types[tp as usize].parts
        else {
            return tp;
        };
        let Ok(offset) = u16::try_from(r.pos - 8) else {
            return tp;
        };
        fields
            .iter()
            .find(|f| f.position == offset && !f.name.starts_with('#'))
            .map_or(tp, |f| f.content)
    }

    /// Give back the free tail of the store holding `slot` — the runtime half
    /// of `store_reclaim(collection)`.  Returns the BYTES handed back to the
    /// filesystem (or to the allocator, for a heap store); 0 when there was
    /// nothing to give, which is also what an out-of-range slot answers.
    ///
    /// The program says when, and that is the whole design (@PLN123 A3): only a
    /// live set that drops far below its peak AND STAYS THERE has anything to
    /// give back, and whether a drop is permanent is something the program
    /// knows and the runtime cannot infer.  Measured here, steady-state churn
    /// held capacity flat across 36,000 insert+remove pairs and a refill reused
    /// every byte — an always-on reclaimer would walk, coalesce and find
    /// nothing, for everyone.
    ///
    /// Nothing on the free path is touched, so `delete` stays O(1): the walk
    /// happens here, on a call the program asked for.
    pub fn reclaim_store(&mut self, slot: u16) -> i64 {
        match self.allocations.get_mut(slot as usize) {
            Some(store) => i64::from(store.reclaim_tail()) * 8,
            None => 0,
        }
    }

    /// @PLN126 — flush what has been written to a bound store's file and drop it from
    /// the resident set, answering the bytes dropped.  The loft-callable half of
    /// [`crate::store::Store::release_resident`].
    ///
    /// A sibling of [`Self::reclaim_store`] and deliberately not a mode of it: that one
    /// changes the file's LENGTH and is a decision about disk, this one changes only
    /// what is resident and is a decision about memory.  A caller wants one or the
    /// other, never "some of both".
    pub fn release_store(&mut self, slot: u16) -> i64 {
        match self.allocations.get_mut(slot as usize) {
            Some(store) => i64::try_from(store.release_resident()).unwrap_or(i64::MAX),
            None => 0,
        }
    }

    /// True when a field's type stores its value INLINE (a fixed-width scalar),
    /// so a working-set copy can move it as raw bytes with no pointer to
    /// relocate. Text (type 5) and Reference (type 6) are POINTERS; vectors /
    /// nested structs / keyed collections are heap-owned — all need the
    /// relocating graph-copy (3b.2+), so they are NOT inline.
    #[cfg(paged_store)]
    fn is_inline_scalar(&self, tp: u16) -> bool {
        match self.types[tp as usize].parts {
            Parts::Int(..) | Parts::Byte(..) | Parts::Short(..) | Parts::ShortRaw(..) => true,
            // Base covers the numeric primitives AND text(5) / Reference(6);
            // only the numerics are inline.
            Parts::Base => !matches!(tp, 5 | 6),
            _ => false,
        }
    }

    /// True when a field can be moved by the working-set copy today: an inline
    /// scalar (raw word-copy), a `text` (relocated string, 3b.2), or a
    /// `vector<scalar>` (relocated flat inner record, 3b.3). Text and a
    /// scalar-vector are BOTH a single flat sub-record behind a `u32` pointer, so
    /// they relocate with identical code. A `vector<struct>` / `vector<text>` /
    /// nested struct / reference still needs the recursive copy (3b.4+).
    #[cfg(paged_store)]
    fn is_copyable_field(&self, tp: u16) -> bool {
        if self.is_inline_scalar(tp) || tp == 5 {
            return true;
        }
        match &self.types[tp as usize].parts {
            // vector<scalar> (flat inner record) OR vector<copyable struct>
            // (copy inner + relocate each element, 3b.4b). vector<text> /
            // vector<vector> are NOT handled — the element pointers would dangle.
            Parts::Vector(e) => {
                self.is_inline_scalar(*e)
                    || (matches!(
                        self.types[*e as usize].parts,
                        Parts::Struct(_) | Parts::EnumValue(_, _)
                    ) && self.is_copyable_field(*e))
            }
            // An INLINE nested struct: copyable when every one of its own fields
            // is (its `text`/`vector` fields relocate at a nested offset, 3b.4).
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                fields.iter().all(|f| self.is_copyable_field(f.content))
            }
            // An enum's value is a TAG BYTE stored inline, and a struct-enum
            // variant's payload lives in the record beside it — `copy_claims`
            // recurses on the SAME position for both.  So an enum relocates
            // exactly when every variant it could be holding does, and a simple
            // variant (`u16::MAX`, no payload) always does.  Without this arm
            // every enum FIELD made a paged load refuse the whole collection,
            // and the refusal blamed a `vector<text>` the type did not have —
            // an asset pack's `bl_kind: BlobKind` was enough to lose the range
            // read (@PLN146 F2).
            Parts::Enum(values) => values
                .iter()
                .all(|(vt, _)| *vt == u16::MAX || self.is_copyable_field(*vt)),
            _ => false,
        }
    }

    /// Report that a paged working-set load REFUSED the request and loaded
    /// nothing. Every refusal in `layout_gate_ok` / `load_key(s)` /
    /// `load_key_text` / `load_range` speaks through here, because these loaders
    /// report failure as `false` / `0`, which is exactly what an ABSENT KEY
    /// looks like — so a silent refusal reads as "the data isn't there" and
    /// sends the caller hunting for missing data instead of an unsupported
    /// shape (#632).
    ///
    /// Two channels, because stderr is not one a program can read. The message
    /// is for the person, and [`paged_refusal`](Stores::paged_refusal) is for
    /// the lazy fetch, which turns it into `store_lazy_error` (loft#802). A
    /// refusal that only printed left the program itself with nothing but the
    /// `null`.
    #[cfg(paged_store)]
    fn refuse_paged(&mut self, path: &str, reason: &str) {
        crate::loft_eprintln!(
            "store loader: refusing {path} — {reason}; loaded NOTHING (a refusal, \
             not an absent key)"
        );
        self.paged_refusal = Some(format!("refusing {path} — {reason}"));
    }

    /// The refusal reason when the bound store does not root the collection kind
    /// the loader needs (`want` names it, e.g. "a hash"). Spells out the cause
    /// because the type name alone does not suggest the fix.
    ///
    /// THREE causes, and telling them apart is the whole value of the message.
    /// When the root is a keyed collection of the WRONG KIND, the shape is
    /// already what the author wrote and no declaration change can help — the
    /// answer is `store_load`, and saying "declare it as an annotated local"
    /// sends them to rewrite a line that is correct (a `trie` local was told
    /// exactly that, loft#802). When the root is a `trie` the answer is neither:
    /// a trie IS pageable (@PLN134), just not by the loader that asked, so the
    /// cure is the trie's own call and sending the author to `store_load` would
    /// cost them the whole saving — and the same is true of a `spatial` since
    /// @PLN136. When the root is anything else, it is the #632 wrapper-struct trap
    /// and the declaration IS the fix.
    #[cfg(paged_store)]
    fn wrong_collection_reason(&self, tp: u16, want: &str) -> String {
        let name = self.type_name(tp);
        let parts = self.types.get(tp as usize).map(|t| &t.parts);
        if matches!(parts, Some(Parts::Trie(..))) {
            return format!(
                "its bound store roots `{name}`, not {want} — a trie is keyed on \
                 TEXT, so read it with `store_load_key_text` for one key or \
                 `store_load_prefix` for a prefix"
            );
        }
        if matches!(parts, Some(Parts::Radix(..))) {
            return format!(
                "its bound store roots `{name}`, not {want} — a spatial index is \
                 keyed on COORDINATES, so read it with `store_load_box` for a \
                 bounding box"
            );
        }
        let keyed = matches!(
            parts,
            Some(Parts::Hash(..) | Parts::Sorted(..) | Parts::Ordered(..) | Parts::Index(..))
        );
        if keyed {
            format!(
                "its bound store roots `{name}`, not {want} — that kind cannot be \
                 read a page at a time, so read it whole with `store_load` (or \
                 `store_load_url_trusted` for a URL)"
            )
        } else {
            format!(
                "its bound store roots `{name}`, not {want} — a collection declared \
                 as a struct FIELD records the WRAPPER STRUCT as the store's type \
                 (#632), so declare it as an annotated local (`h: hash<T[k]> = []`) \
                 for paged loads, or read it whole with `store_load`"
            )
        }
    }

    /// The refusal reason when an entry has a field the working-set copy cannot
    /// relocate into the local store.
    ///
    /// It NAMES the field. The message used to blame a `vector<text>` /
    /// `vector<vector>` whatever the real cause was, which sent an author looking
    /// for a field their type did not have — an `enum` field was refused and
    /// reported as a nested vector for as long as the enum arm was missing from
    /// [`Stores::is_copyable_field`]. A refusal a reader cannot act on is barely
    /// better than a silent one.
    #[cfg(paged_store)]
    fn not_copyable_reason(&self, content_tp: u16) -> String {
        let culprit = match &self.types[content_tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => fields
                .iter()
                .find(|f| !self.is_copyable_field(f.content))
                .map(|f| format!("`{}: {}`", f.name, self.type_name(f.content))),
            _ => None,
        };
        let which = culprit.unwrap_or_else(|| "a field".to_string());
        format!(
            "entry type `{}` has a field the working-set copy cannot relocate: \
             {which} — its contents live behind pointers this copy would leave \
             dangling in the local store; read the collection whole with \
             `store_load` instead, which carries every field",
            self.type_name(content_tp)
        )
    }

    /// @PLN97 arc G / #632 — the keyed-collection type a paged loader should use,
    /// given the bound store's `known_type` and the target ref.
    ///
    /// When the store roots the collection DIRECTLY (`known_type` is
    /// `Hash`/`Sorted`/`Index` — the annotated-local idiom `h: hash<T[k]> = []`),
    /// that IS the type. When the collection is declared as a struct FIELD
    /// (`struct Wrap { data: hash<T[k]> }`), the store roots the WRAPPER struct and
    /// the collection is the field the target ref points at — `local.pos` is that
    /// field's byte offset, exactly where `find_hash_entry` reads the collection
    /// root claim in both the image and the target. So descend to the field at
    /// `local.pos` whose type is a keyed collection. `u16::MAX` when `tp` is a
    /// struct with no keyed-collection field there (a genuinely wrong shape).
    pub(crate) fn collection_type_of_store(&self, tp: u16) -> u16 {
        // An untyped store has nothing to descend from, and `u16::MAX` is not an
        // index. The paged callers happened to check first; @PLN129's do not,
        // and a guard in the caller is a guard someone can forget.
        if tp == u16::MAX || tp as usize >= self.types.len() {
            return u16::MAX;
        }
        // `Ordered` is the PROMOTED form of `Sorted` (`finish()` renames a
        // sorted collection whose element is shared by >1 container), so a
        // collection that is `Sorted` in a small program is `Ordered` in one where
        // its element is reused — both are the same keyed collection here.
        //
        // `Trie` and `Radix` belong for the same reason `Hash` does and `Sorted`
        // does not: their field form is a 4-byte claim slot holding the tree record
        // id, and neither promotes, so the layout a paged descent reads at
        // `local.pos` is the same whether the collection was declared as a local or
        // as a struct field (@PLN134, @PLN136). A `sorted` field can be promoted to
        // `ordered` by unrelated program structure, which is why `load_range`
        // refuses that form audibly.
        let is_keyed = |t: u16| {
            matches!(
                self.types[t as usize].parts,
                Parts::Hash(..)
                    | Parts::Sorted(..)
                    | Parts::Ordered(..)
                    | Parts::Index(..)
                    | Parts::Trie(..)
                    | Parts::Radix(..)
            )
        };
        match &self.types[tp as usize].parts {
            _ if is_keyed(tp) => tp,
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                // The target ref's `pos` is the collection's claim slot, NOT the
                // struct field offset, so it can't select the field. A wrapper with
                // exactly ONE keyed-collection field is the reported idiom (`struct
                // Wrap { data: hash<T[k]> }`) and unambiguous; use it. A struct with
                // several keyed fields can't be disambiguated yet → refuse.
                let mut keyed = fields.iter().filter(|f| is_keyed(f.content));
                match (keyed.next(), keyed.next()) {
                    (Some(f), None) => f.content,
                    _ => u16::MAX,
                }
            }
            _ => u16::MAX,
        }
    }

    /// True when an entry of type `content_tp` can be partially loaded today —
    /// every field is [copyable](Stores::is_copyable_field). Otherwise the
    /// collection is refused (safe-refusal, never a broken heap).
    #[cfg(paged_store)]
    fn is_copyable_entry(&self, content_tp: u16) -> bool {
        match &self.types[content_tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                fields.iter().all(|f| self.is_copyable_field(f.content))
            }
            _ => self.is_copyable_field(content_tp),
        }
    }

    #[cfg(paged_store)]
    pub fn load_key(&mut self, local: &DbRef, path: &str, key: i64) -> bool {
        self.load_keys(local, path, std::slice::from_ref(&key)) > 0
    }

    /// @PLN129 arc A — bind a COLLECTION to a lazy source, so a lookup that
    /// misses consults it instead of answering "absent".
    ///
    /// Per collection (`persons` and `companies` are different tables), so the
    /// key is the collection's root ref. Rebinding replaces; binding is
    /// configuration and may be set before the collection holds anything.
    ///
    /// Answers whether the binding was ACCEPTED. A source that can never serve
    /// this collection's kind is refused HERE rather than at some later lookup,
    /// because that is a static property of the pair and is knowable with no I/O
    /// — and the alternative is what loft#802 reports: `true`, then `null`
    /// forever, with the refusal on stderr where no program can read it.
    pub fn bind_lazy(&mut self, coll: &DbRef, source: &str) -> bool {
        if let Some(why) = self.unservable_kind(coll, source) {
            self.lazy_sources
                .remove(&(coll.store_nr, coll.rec, coll.pos));
            crate::loft_eprintln!("store_bind_lazy: refusing `{source}` — {why}");
            self.lazy_refuse((coll.store_nr, coll.rec, coll.pos), &why);
            return false;
        }
        // @PLN129 arc D — pin the source's identity at bind. Rebinding RE-pins,
        // which is how a caller says "I accept the new world" after drift.
        let pin = Self::source_pin(source);
        self.lazy_sources.insert(
            (coll.store_nr, coll.rec, coll.pos),
            crate::database::LazyBinding {
                source: source.to_string(),
                pin,
                // @PLN129 arc B step 7 — a rebind re-decides the schema too: it
                // says "a different world now", and the schema is part of it.
                check: crate::database::SchemaCheck::Unchecked,
            },
        );
        // The FAULT channel is deliberately NOT cleared here, and it took a suite failure
        // to see why.  A rebind re-decides the source and the schema, so "the faults
        // belong to a source this collection no longer reads" sounds right — but the
        // faults describe the CONTENTS, not the source, and the contents do not reset:
        // whatever the previous binding materialised is still resident, minus the rows
        // that failed.  Clearing on rebind therefore answers "healthy" for a working set
        // that is missing data, which is the silent wrong answer this channel exists to
        // prevent.  `tests/scripts/129-lazy-bind.loft` pins exactly that shape — fail,
        // rebind, succeed — and only `store_lazy_clear` may clear it.
        true
    }

    /// Why this source can NEVER serve this collection, or `None` when the pair
    /// is at least possible.
    ///
    /// Static, and that is the point: a `.store` IMAGE is read by the paged
    /// loader, which serves a `hash` (integer- or text-keyed), a `trie` (one
    /// text-keyed descent, @PLN134) and a `spatial` (a bounding-box walk over the
    /// same tree, @PLN136). Every other keyed kind is refused at the first lookup
    /// and would be refused at every lookup after it, so the answer is already
    /// known when the binding is made and costs no I/O to give.
    ///
    /// A kind that becomes servable and is not removed from here keeps refusing a
    /// binding that would now work, which is why the two move in ONE change.
    ///
    /// A DATABASE source is NOT judged here. Its refusals depend on the schema
    /// on the other end — a column that is missing, a type that will not map —
    /// and `fetch_from_sql` interrogates that once, on the first fault. Refusing
    /// a `trie` bound to sqlite would also be WRONG: `derive_select` gives a
    /// trie `sorted`'s shape and serves it (`sql_query.rs`). The kinds a paged
    /// image can serve and the kinds SQL can serve are different sets, and this
    /// is only about the first.
    fn unservable_kind(&self, coll: &DbRef, source: &str) -> Option<String> {
        if !matches!(
            crate::database::lazy::LazySource::of(source),
            crate::database::lazy::LazySource::File(_)
        ) {
            return None;
        }
        let root_tp = self.allocations[coll.store_nr as usize].known_type;
        // An untyped store has no kind to judge; the loader's own gate handles
        // it, and refusing here on no evidence would break a working bind.
        if root_tp == u16::MAX {
            return None;
        }
        let tp = self.collection_type_of_store(root_tp);
        if matches!(
            self.types.get(tp as usize).map(|t| &t.parts),
            Some(Parts::Hash(..) | Parts::Trie(..) | Parts::Radix(..))
        ) {
            return None;
        }
        Some(format!(
            "a lazily-bound `.store` image is read a page at a time, which only a \
             `hash`, a `trie` or a `spatial` supports, and `{}` is none of them — \
             read it whole with `store_load` (or `store_load_url_trusted` for a \
             URL), which carries every kind",
            self.type_name(root_tp)
        ))
    }

    /// The identity a local source is pinned to: `(len, mtime_secs)`.
    ///
    /// `None` for anything that cannot be stat'ed cheaply — an `http(s)` URL, or
    /// a path that does not exist yet. No pin means no drift check, never a
    /// silent pass: an unreachable source is already arc C's business.
    fn source_pin(source: &str) -> Option<(u64, u64, u64)> {
        if source.starts_with("http://") || source.starts_with("https://") {
            return None;
        }
        let md = std::fs::metadata(source).ok()?;
        let mtime = u64::try_from(
            md.modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos(),
        )
        .unwrap_or(u64::MAX);
        // The inode is what catches a rename-replace: the replacement keeps its
        // own mtime, which can predate the bind, so a time comparison alone reads
        // as "unchanged".
        #[cfg(unix)]
        let ino = {
            use std::os::unix::fs::MetadataExt;
            md.ino()
        };
        #[cfg(not(unix))]
        let ino = 0u64;
        Some((md.len(), mtime, ino))
    }

    /// @PLN129 arc C — why a fetch could not reach this collection's
    /// source, or `""` when it is healthy. The FIRST such reason, kept.
    ///
    /// Empty is the honest answer for a collection that was never bound: nothing
    /// has failed. A caller distinguishing "absent" from "unreachable" asks this
    /// AFTER a lookup answered null — the lookup itself cannot say (C80).
    #[must_use]
    pub fn lazy_error(&self, coll: &DbRef) -> String {
        self.lazy_errors
            .get(&(coll.store_nr, coll.rec, coll.pos))
            .map(|(_, reason)| reason.clone())
            .unwrap_or_default()
    }

    /// @PLN129 arc C — how many fetches for this collection could not reach its
    /// source. `0` is healthy.
    ///
    /// The magnitude behind [`lazy_error`]: it answers "how incomplete am I",
    /// which is the question stickiness exists to make answerable.
    #[must_use]
    pub fn lazy_faults(&self, coll: &DbRef) -> i64 {
        self.lazy_errors
            .get(&(coll.store_nr, coll.rec, coll.pos))
            .map_or(0, |(n, _)| i64::try_from(*n).unwrap_or(i64::MAX))
    }

    /// @PLN129 arc C — acknowledge this collection's fetch failures. True when
    /// there was something to clear.
    ///
    /// The ONLY thing that clears them, deliberately: a caller saying "I have
    /// seen this" is a different event from a later fetch happening to succeed.
    pub fn lazy_clear(&mut self, coll: &DbRef) -> bool {
        self.lazy_errors
            .remove(&(coll.store_nr, coll.rec, coll.pos))
            .is_some()
    }

    /// The lazy source bound to this collection, if any.
    #[must_use]
    pub fn lazy_source(&self, coll: &DbRef) -> Option<String> {
        self.lazy_sources
            .get(&(coll.store_nr, coll.rec, coll.pos))
            .map(|b| b.source.clone())
    }

    /// @PLN129 arc D — has the source moved under this binding since it was
    /// pinned? `false` when there is no pin (nothing to compare).
    #[must_use]
    fn lazy_drifted(&self, coll: &DbRef) -> bool {
        let Some(b) = self.lazy_sources.get(&(coll.store_nr, coll.rec, coll.pos)) else {
            return false;
        };
        let Some(pinned) = b.pin else { return false };
        Self::source_pin(&b.source).is_none_or(|now| now != pinned)
    }

    /// @PLN129 arc A — the MISS path: the one place the outside world is
    /// consulted.
    ///
    /// `find` already answers a miss with `rec: 0`, so this changes nothing for
    /// an unbound collection. For a bound one it fetches that single entry,
    /// which INSERTS it into the collection (`load_one` ends in `hash::add`),
    /// and re-runs the lookup so the answer comes from the collection either
    /// way.
    ///
    /// Re-running rather than returning the fetched ref is deliberate: the
    /// collection stays the single source of truth for what is resident, which
    /// is what makes identity fall out of it (README § the resident set IS the
    /// cache). A path that answered from the fetch directly could hand back a
    /// record the collection does not hold.
    ///
    /// **@PLN133 S8 — the miss and the fetch are separate calls, and their two
    /// callers make the decision between them.** `Stores` cannot run a loft
    /// function and `State` can, so a fetch that IS loft code needs the
    /// `&mut Stores` borrow released between the two — which is only possible
    /// where the borrow is taken. The retry is unchanged; only who sequences it
    /// moved.
    pub(crate) fn fetch_missing(
        &mut self,
        data: &DbRef,
        db: u16,
        key: &[crate::keys::Content],
    ) -> DbRef {
        let Some(source) = self.lazy_source(data) else {
            return DbRef {
                store_nr: data.store_nr,
                rec: 0,
                pos: 0,
            };
        };
        // @PLN129 arc D — the source must be the one this collection was pinned
        // to. A store is a consistent image; a live file is not, and two faults
        // in one traversal reading different worlds is a silent lie about what
        // was read. Measured before the check existed: swapping the file between
        // two lookups returned `grace` then `ALAN-v2` with nothing reporting it.
        //
        // Refuse rather than serve the new world: continuing would MIX two
        // versions inside one collection, which is worse than an absence. Rebind
        // to accept the new world deliberately.
        if self.lazy_drifted(data) {
            let slot = (data.store_nr, data.rec, data.pos);
            let entry = self.lazy_errors.entry(slot).or_insert((
                0,
                format!("`{source}` changed since it was bound — rebind to accept the new version"),
            ));
            entry.0 += 1;
            return DbRef {
                store_nr: data.store_nr,
                rec: 0,
                pos: 0,
            };
        }
        // @PLN129 arc B step 1 — through the SOURCE seam. Which source this is
        // is the binding's business; what a fetch can ANSWER is the same three
        // outcomes whatever it is.
        let src = crate::database::lazy::LazySource::of(&source);
        let slot = (data.store_nr, data.rec, data.pos);
        match self.fetch_from_source(data, db, &src, key) {
            // NOT cleared. A success says nothing about what an earlier failure
            // already lost, and reporting "healthy" after a partial traversal is
            // exactly the silent-wrong-answer this channel exists to prevent.
            crate::database::lazy::Fetched::Inserted => self.find(data, db, key),
            // Reached it and the key was not there — a genuine absence. It leaves
            // the record untouched: reachability NOW does not undo an earlier loss.
            crate::database::lazy::Fetched::Absent => DbRef {
                store_nr: data.store_nr,
                rec: 0,
                pos: 0,
            },
            // @PLN129 arc C — a failed fetch is TWO different facts, and answering
            // `null` for both is the failure this channel exists to prevent: "no
            // such person" is stable and true, "the source is unreachable" is
            // neither. Sticky: count every failure, keep the FIRST reason.
            crate::database::lazy::Fetched::Unreachable(reason) => {
                let entry = self.lazy_errors.entry(slot).or_insert((0, reason));
                entry.0 += 1;
                DbRef {
                    store_nr: data.store_nr,
                    rec: 0,
                    pos: 0,
                }
            }
        }
    }

    /// @PLN97 arc G Phase 3b.6 — load ONE TEXT-keyed entry from a persisted
    /// `hash<T[textkey]>` (e.g. a place-name or z/x/y tile-id index), fetching
    /// only the pages the lookup touches. Same working-set fetch as
    /// [`load_key`](Stores::load_key), keyed by a string: `find_hash_entry`
    /// hashes the `Content::Str`, and the entry's text key is compared over the
    /// reader. Returns false when absent / unreadable / not an integer-or-text
    /// -keyed copyable hash.
    #[cfg(paged_store)]
    pub fn load_key_text(&mut self, local: &DbRef, path: &str, key: &str) -> bool {
        if !self.layout_gate_ok(path, local) {
            return false;
        }
        use crate::paged_reader::{PageSource, PagedReader};
        let Ok(source) = PageSource::open(path) else {
            self.refuse_paged(path, "it cannot be opened as a paged source");
            return false;
        };
        let mut reader = PagedReader::new(source);
        let root_tp = self.allocations[local.store_nr as usize].known_type;
        if root_tp == u16::MAX {
            self.refuse_paged(path, "the target collection's store has no recorded type");
            return false;
        }
        let tp = self.collection_type_of_store(root_tp); // field form → the field's hash (#632)
        // @PLN134 — a TRIE serves an exact text lookup too, by one root→leaf
        // descent. Which kind decides how the record is LOCATED and how it is
        // linked once copied; everything between is the same working-set copy.
        let (content_tp, kind) = match self.types.get(tp as usize).map(|t| &t.parts) {
            Some(Parts::Hash(c, _)) => (*c, PagedKind::Hash),
            Some(Parts::Trie(c, _)) => (*c, PagedKind::Trie),
            _ => {
                let reason = self.wrong_collection_reason(root_tp, "a hash or a trie");
                self.refuse_paged(path, &reason);
                return false;
            }
        };
        if !self.is_copyable_entry(content_tp) {
            let reason = self.not_copyable_reason(content_tp);
            self.refuse_paged(path, &reason);
            return false;
        }
        let keys = self.keys(tp).to_vec();
        let key_content = [crate::keys::Content::Str(crate::keys::Str::new(key))];
        let found = self.load_one(&mut reader, local, content_tp, &keys, &key_content, kind);
        // The same meter the batch forms carry, so the two are comparable — which is
        // the whole question a caller weighing this against
        // [`load_keys_text`](Stores::load_keys_text) is asking (loft#1064). Off unless
        // asked.
        if std::env::var_os("LOFT_LOADER_STATS").is_some() {
            crate::loft_eprintln!(
                "store_load_key_text: key={key:?} found={found} bytes_fetched={} requests={} depth={} distinct={} file={}",
                reader.provider().bytes_fetched(),
                reader.provider().requests(),
                reader.provider().round_trips(),
                reader.provider().distinct_bytes(),
                reader.size()
            );
        }
        found
    }

    /// @PLN134 — load every entry whose TEXT key begins with `pre` from a persisted
    /// `trie<T[k]>` image into the (empty) local trie, fetching only the pages the
    /// prefix walk touches. Returns the count loaded.
    ///
    /// The capability the trie kind exists for, now over a link: `t["kerk"..:20]`
    /// costs a root→leaf descent plus the records it returns — ~3.8 pages of a
    /// laid-out image (@PLN134 step 2) against a 5.9 MB whole-image download. A
    /// phone typing one letter should not pull the vocabulary.
    ///
    /// `limit < 0` means no cap, matching `build_trie_prefix_vec`. The cap bounds
    /// the WALK, not just the answer: `paged_reader::trie_prefix_recs` stops
    /// stepping at the cap, so the untaken tail is never fetched.
    #[cfg(paged_store)]
    pub fn load_prefix(&mut self, local: &DbRef, path: &str, pre: &str, limit: i64) -> i64 {
        if !self.layout_gate_ok(path, local) {
            return 0;
        }
        use crate::paged_reader::{PageSource, PagedReader};
        let Ok(source) = PageSource::open(path) else {
            self.refuse_paged(path, "it cannot be opened as a paged source");
            return 0;
        };
        let mut reader = PagedReader::new(source);
        let root_tp = self.allocations[local.store_nr as usize].known_type;
        if root_tp == u16::MAX {
            self.refuse_paged(path, "the target collection's store has no recorded type");
            return 0;
        }
        let tp = self.collection_type_of_store(root_tp);
        let Some(Parts::Trie(content_tp, _)) = self.types.get(tp as usize).map(|t| &t.parts) else {
            let reason = self.wrong_collection_reason(root_tp, "a trie");
            self.refuse_paged(path, &reason);
            return 0;
        };
        let content_tp = *content_tp;
        if !self.is_copyable_entry(content_tp) {
            let reason = self.not_copyable_reason(content_tp);
            self.refuse_paged(path, &reason);
            return 0;
        }
        let keys = self.keys(tp).to_vec();
        let cap = (limit >= 0).then(|| usize::try_from(limit).unwrap_or(usize::MAX));
        let found = crate::paged_reader::trie_prefix_recs(
            &mut reader,
            local.rec,
            local.pos,
            pre,
            &keys,
            cap,
        );
        let mut loaded = 0i64;
        for rec in found {
            if self.copy_entry(
                &mut reader,
                local,
                content_tp,
                &keys,
                (rec, crate::store::RECORD_PAYLOAD),
                PagedKind::Trie,
            ) {
                loaded += 1;
            }
        }
        if std::env::var_os("LOFT_LOADER_STATS").is_some() {
            crate::loft_eprintln!(
                "store_load_prefix: {pre:?} limit={limit} loaded={loaded} bytes_fetched={} requests={} file={}",
                reader.provider().bytes_fetched(),
                reader.provider().requests(),
                reader.size()
            );
        }
        loaded
    }

    /// @PLN136 — load every entry inside the bounding box `[from, till]` from a
    /// persisted `spatial<T[…]>` image into the (empty) local spatial collection,
    /// fetching only the pages the box walk touches. Returns the count loaded.
    ///
    /// The capability the spatial kind exists for, now over a link. Measured on a
    /// 3.2 M-point map index (a 158 MB image, 2532 pages of 64 KB): one capped
    /// viewport query touches 3.6 pages of nodes and 1.7 of records, and panning the
    /// map costs ~1.5 pages a step against 426 from an image written as inserted.
    ///
    /// **Two bounds, and the plan is about both.** `limit < 0` means no cap, matching
    /// `build_radix_range_vec`; the cap bounds the WALK, so the untaken tail is never
    /// fetched. And the BOX bounds it: the Morton interval between two corners is a
    /// superset that Z-order threads in and out of, so a walk that read the interval
    /// and filtered would fetch 1.46 M records' pages to return 4 k on the wide,
    /// shallow viewport a map actually issues.
    ///
    /// A corner given the other way round names the same box, as the resident
    /// surface reads it. Extra coordinates past the collection's own axes are
    /// ignored, and too few refuse — the same ABI a 1-D…3-D slice lowers through.
    #[cfg(paged_store)]
    pub fn load_box(
        &mut self,
        local: &DbRef,
        path: &str,
        from: &[i64],
        till: &[i64],
        limit: i64,
    ) -> i64 {
        if !self.layout_gate_ok(path, local) {
            return 0;
        }
        use crate::paged_reader::{PageSource, PagedReader};
        let Ok(source) = PageSource::open(path) else {
            self.refuse_paged(path, "it cannot be opened as a paged source");
            return 0;
        };
        let mut reader = PagedReader::new(source);
        let root_tp = self.allocations[local.store_nr as usize].known_type;
        if root_tp == u16::MAX {
            self.refuse_paged(path, "the target collection's store has no recorded type");
            return 0;
        }
        let tp = self.collection_type_of_store(root_tp);
        let Some(Parts::Radix(content_tp, _)) = self.types.get(tp as usize).map(|t| &t.parts)
        else {
            let reason = self.wrong_collection_reason(root_tp, "a spatial index");
            self.refuse_paged(path, &reason);
            return 0;
        };
        let content_tp = *content_tp;
        if !self.is_copyable_entry(content_tp) {
            let reason = self.not_copyable_reason(content_tp);
            self.refuse_paged(path, &reason);
            return 0;
        }
        let keys = self.keys(tp).to_vec();
        if from.len() < keys.len() || till.len() < keys.len() {
            self.refuse_paged(
                path,
                &format!(
                    "the box has {} and {} coordinates but `{}` has {} axes — a \
                     corner must name every one of them",
                    from.len(),
                    till.len(),
                    self.type_name(tp),
                    keys.len()
                ),
            );
            return 0;
        }
        let cap = (limit >= 0).then(|| usize::try_from(limit).unwrap_or(usize::MAX));
        let found = crate::paged_reader::spatial_box_recs(
            &mut reader,
            local.rec,
            local.pos,
            from,
            till,
            &keys,
            cap,
        );
        let mut loaded = 0i64;
        for rec in found {
            if self.copy_entry(
                &mut reader,
                local,
                content_tp,
                &keys,
                (rec, crate::store::RECORD_PAYLOAD),
                PagedKind::Radix,
            ) {
                loaded += 1;
            }
        }
        if std::env::var_os("LOFT_LOADER_STATS").is_some() {
            crate::loft_eprintln!(
                "store_load_box: {from:?}..{till:?} limit={limit} loaded={loaded} bytes_fetched={} requests={} file={}",
                reader.provider().bytes_fetched(),
                reader.provider().requests(),
                reader.size()
            );
        }
        loaded
    }

    /// @PLN97 arc G Phase 3a — load the requested integer keys' entries from a
    /// persisted HASH image at `path` into the empty local hash `local`,
    /// fetching only the pages the lookups touch. Returns the count actually
    /// found (keys absent in the remote are silently skipped). The paged reader
    /// is opened ONCE and shared across all keys, so its LRU cache is reused.
    /// See [`load_key`](Stores::load_key) for the single-key form. Same
    /// FLAT-struct restriction (scalar fields only — no relocation yet).
    #[cfg(paged_store)]
    pub fn load_keys(&mut self, local: &DbRef, path: &str, keys_vals: &[i64]) -> i64 {
        let key_sets: Vec<Vec<crate::keys::Content>> = keys_vals
            .iter()
            .map(|&kv| vec![crate::keys::Content::Long(kv)])
            .collect();
        self.load_key_sets(local, path, &key_sets, "store_load_keys")
    }

    /// loft#1064 — the TEXT-keyed batch loader: the plural of
    /// [`load_key_text`](Stores::load_key_text), and the text twin of
    /// [`load_keys`](Stores::load_keys).
    ///
    /// It exists because looping the single-key form is not the same call. Each
    /// `load_key_text` opens its OWN reader, so the bucket table page is fetched
    /// again for every key: a ring of twenty asset names around a player cost
    /// twenty traversals and twenty copies of the same 64 KiB table — about
    /// 2.5 MB to fetch perhaps 100 KB of art. One reader pays it once.
    ///
    /// Text-keyed collections are what asset packs are (`"page/mobs"`,
    /// `"hit.ogg"`), and a ring prefetched at a load or level boundary is exactly
    /// the shape the batch form is for.
    #[cfg(paged_store)]
    pub fn load_keys_text(&mut self, local: &DbRef, path: &str, keys_vals: &[&str]) -> i64 {
        let key_sets: Vec<Vec<crate::keys::Content>> = keys_vals
            .iter()
            .map(|k| vec![crate::keys::Content::Str(crate::keys::Str::new(k))])
            .collect();
        self.load_key_sets(local, path, &key_sets, "store_load_keys_text")
    }

    /// The batch loader both plural forms are: open the paged reader ONCE, resolve
    /// the schema once, and materialise every requested key through the shared page
    /// cache. Returns the count actually found (an absent key is skipped, not an
    /// error).
    ///
    /// One body rather than one per key type, so the warm pipeline below — the part
    /// that turns O(N) serial round trips into three batches — cannot be present on
    /// the integer form and missing from the text one.
    ///
    /// A **hash** locates by arithmetic on the key's own bytes, so every bucket
    /// address is known before any of them is read and the warms apply. A **trie**
    /// locates by descent, where each step's address comes from the node the step
    /// before it read; there is nothing to batch, and the win is the shared cache —
    /// the upper nodes every key walks through are fetched once instead of N times.
    #[cfg(paged_store)]
    fn load_key_sets(
        &mut self,
        local: &DbRef,
        path: &str,
        key_sets: &[Vec<crate::keys::Content>],
        stats_label: &str,
    ) -> i64 {
        if !self.layout_gate_ok(path, local) {
            return 0;
        }
        use crate::paged_reader::{PageSource, PagedReader};
        // `path` is a local file OR an `http(s)://` URL — the paged reader pulls
        // only the pages a lookup touches, from disk or over the network (#517).
        let Ok(source) = PageSource::open(path) else {
            self.refuse_paged(path, "it cannot be opened as a paged source");
            return 0;
        };
        let mut reader = PagedReader::new(source);

        // Schema from the LIVE type of `local` (never reverse-engineered bytes).
        // `known_type` roots the collection directly (annotated local) OR the
        // wrapper struct when the collection is a field (#632) — resolve to the
        // collection either way.
        let root_tp = self.allocations[local.store_nr as usize].known_type;
        if root_tp == u16::MAX {
            self.refuse_paged(path, "the target collection's store has no recorded type");
            return 0;
        }
        let tp = self.collection_type_of_store(root_tp);
        // SAFE REFUSAL (3b.1) — `load_one` can copy an entry whose fields are
        // inline-scalar (raw word-copy) or `text` (relocated string, 3b.2). A
        // vector / nested / reference field still needs the recursive relocating
        // copy (3b.3+); until then, refuse the whole collection (load nothing)
        // rather than build a heap with a dangling pointer. `store_verify` would
        // catch a broken copy — this makes sure one is never built.
        //
        // @PLN134 — a TRIE serves an exact lookup too, by one root→leaf descent,
        // the same kind split [`load_key_text`](Stores::load_key_text) makes.
        let (content_tp, kind) = match self.types.get(tp as usize).map(|t| &t.parts) {
            Some(Parts::Hash(c, _)) => (*c, PagedKind::Hash),
            Some(Parts::Trie(c, _)) => (*c, PagedKind::Trie),
            _ => {
                let reason = self.wrong_collection_reason(root_tp, "a hash or a trie");
                self.refuse_paged(path, &reason);
                return 0;
            }
        };
        if !self.is_copyable_entry(content_tp) {
            let reason = self.not_copyable_reason(content_tp);
            self.refuse_paged(path, &reason);
            return 0;
        }
        let keys = self.keys(tp).to_vec();

        if kind == PagedKind::Hash {
            // loft#782 — WARM the addresses already known, so round trips stop scaling with the
            // key count.
            //
            // The reads only LOOK sequential. A bucket slot is arithmetic from the key's hash
            // (the table's `claim`/`elms`/`seed` are the same for every key), and once the
            // lookups return, every entry's address is known and the bodies are independent.
            // Three batched fetches replace ~2N demand-paged ones.
            //
            // Measured on a 7.2 MB image with a scattered 40-key selection over a 15 ms/request
            // link, which is the shape that makes depth visible — a selection covering most of
            // the file has nothing to batch, because `prefetch_run` already coalesced it.
            //
            // It cannot make a load wrong or fetch more: warming only fills the page table with
            // pages the walk would have fetched anyway, and a resident page is never re-fetched.
            let addrs =
                crate::paged_reader::hash_bucket_addrs(&mut reader, local.rec, local.pos, key_sets);
            reader.warm(&addrs);

            // The candidate entries, warmed BEFORE the lookups rather than after.
            //
            // This is the step that actually moves the clock, and leaving it out is why an
            // earlier attempt measured nothing: `find_hash_entry` reads the entry record to
            // COMPARE the key, so locating already pulls each entry's page — one serial round
            // trip per key — and a warm placed after the lookups finds every page resident and
            // does nothing. The bucket slots are warm now, so reading the candidate record
            // numbers out of them is free, and those pages can go in one batch.
            let cands: Vec<(u64, usize)> = addrs
                .iter()
                .filter_map(|&(off, _)| {
                    let b = reader.resolve(off, 4);
                    let rec = u32::from_ne_bytes([b[0], b[1], b[2], b[3]]);
                    (rec != 0)
                        .then(|| (u64::from(rec) * 8, crate::paged_reader::PAGE_SIZE.min(4096)))
                })
                .collect();
            reader.warm(&cands);

            // Pass 1 — locate. Independent per key, now against warm bucket pages.
            let found: Vec<(u32, u32)> = key_sets
                .iter()
                .map(|k| {
                    crate::paged_reader::find_hash_entry(
                        &mut reader,
                        local.rec,
                        local.pos,
                        k,
                        &keys,
                    )
                })
                .collect();

            // Two EXACT warms rather than one speculative one: headers first (the size word
            // says how long a record is), then exactly those bodies. Guessing a page per entry
            // was tried and reverted — it fetched a page the walk did not need, and loft#729
            // already established that buying depth with bytes just moves the cost.
            // @PLN135 arc H — an entry is a SLOT at a known stride, so its extent is
            // arithmetic and there is no per-entry header word to fetch first: one warm
            // over exactly the bytes the copy reads replaces the header-then-body pair.
            let stride = crate::hash::stride_for(u32::from(self.size(content_tp)));
            let bodies: Vec<(u64, usize)> = found
                .iter()
                .filter(|&&(m, _)| m != 0)
                .map(|&(m, pos)| (u64::from(m) * 8 + u64::from(pos), stride as usize))
                .collect();
            reader.warm(&bodies);
        }

        // Pass 2 — materialise. `load_one` re-locates the entry; for a hash that is
        // now a warm-page read, and for a trie it is the descent itself.
        let mut loaded = 0i64;
        for k in key_sets {
            if self.load_one(&mut reader, local, content_tp, &keys, k, kind) {
                loaded += 1;
            }
        }

        // Observability for the "bytes fetched ≪ file" invariant: at scale N
        // keys touch O(N) pages, not O(file). Off unless asked.
        if std::env::var_os("LOFT_LOADER_STATS").is_some() {
            crate::loft_eprintln!(
                "{stats_label}: asked={} loaded={loaded} bytes_fetched={} requests={} depth={} distinct={} file={} reads={:?}",
                key_sets.len(),
                reader.provider().bytes_fetched(),
                reader.provider().requests(),
                reader.provider().round_trips(),
                reader.provider().distinct_bytes(),
                reader.size(),
                reader.read_histogram()
            );
        }
        loaded
    }

    /// Find one key in the paged image and, if present, FLAT-copy its entry record
    /// into `local` and link it via the verified `hash::add` / `trie_db::add`. The
    /// entry's field words (fld 8 .. size·8) hold only scalars, so a straight
    /// word copy into a fresh claim is correct — no internal `rec` pointers to
    /// relocate. Returns whether the key was found.
    #[cfg(paged_store)]
    fn load_one(
        &mut self,
        reader: &mut crate::paged_reader::PagedReader<crate::paged_reader::PageSource>,
        local: &DbRef,
        content_tp: u16,
        keys: &[crate::keys::Key],
        key_content: &[crate::keys::Content],
        kind: PagedKind,
    ) -> bool {
        // `(record, offset)`: a trie entry is still a record whose payload starts at
        // byte 8; a hash entry is a slot inside an arena chunk (@PLN135 arc H).
        let matched = match kind {
            PagedKind::Hash => crate::paged_reader::find_hash_entry(
                reader,
                local.rec,
                local.pos,
                key_content,
                keys,
            ),
            // A trie keys on ONE text column, so the probe is that column's string;
            // any other spelling cannot match and locates nothing.
            PagedKind::Trie => match key_content.first() {
                Some(crate::keys::Content::Str(s)) => (
                    crate::paged_reader::trie_find_rec(reader, local.rec, local.pos, s.str(), keys),
                    crate::store::RECORD_PAYLOAD,
                ),
                _ => (0, 0),
            },
            // A spatial index is reached by a BOX, never by one key: `store_load_box`
            // is its entry point and locates its own records, so nothing routes a
            // single-key lookup here. Locating nothing is the honest answer.
            PagedKind::Radix => (0, 0),
        };
        if matched.0 == 0 {
            return false;
        }
        self.copy_entry(reader, local, content_tp, keys, matched, kind)
    }

    /// Copy the ALREADY-LOCATED entry record `matched` out of the paged image into
    /// `local`, relocate its heap graph, and link it into the local collection.
    ///
    /// Split from [`load_one`](Stores::load_one) because a prefix walk locates many
    /// records in one pass and must not re-descend per record — locating and copying
    /// are separate costs, and only the first depends on the query shape.
    #[cfg(paged_store)]
    fn copy_entry(
        &mut self,
        reader: &mut crate::paged_reader::PagedReader<crate::paged_reader::PageSource>,
        local: &DbRef,
        content_tp: u16,
        keys: &[crate::keys::Key],
        matched: (u32, u32),
        kind: PagedKind,
    ) -> bool {
        let (src_rec, src_pos) = matched;
        // How many bytes the entry occupies, and where the local copy goes.  A trie
        // entry is a record: its header says how long it is, and the copy claims one
        // of its own.  A hash entry is an arena SLOT (@PLN135 arc H): its width is the
        // element type's stride — there is no header to read — and the local copy is a
        // slot handed out by the destination's own arena, never a claim, or the two
        // halves of this collection would disagree about what an entry is.
        let (bytes, entry) = match kind {
            PagedKind::Hash => {
                let stride = crate::hash::stride_for(u32::from(self.size(content_tp)));
                (
                    stride,
                    crate::hash::alloc_entry(local, stride, local.rec, &mut self.allocations),
                )
            }
            // Both tree kinds store an element as a claimed RECORD with its payload
            // at offset 8, so the copy is the same claim; only the linker differs.
            PagedKind::Trie | PagedKind::Radix => {
                let size = reader.record_words(src_rec);
                if size < 2 {
                    return false;
                }
                let store = &mut self.allocations[local.store_nr as usize];
                let new_rec = store.claim(size);
                store.zero_fill(new_rec);
                (
                    (size * 8).saturating_sub(crate::store::RECORD_PAYLOAD),
                    DbRef {
                        store_nr: local.store_nr,
                        rec: new_rec,
                        pos: crate::store::RECORD_PAYLOAD,
                    },
                )
            }
        };
        // 1) Move the entry's field words verbatim. Inline scalars are
        //    now correct; every `text` field still holds the SOURCE store's
        //    string-record id (a dangling pointer) — fixed in step 2.
        // One fetch for the entry's field words, unpacked locally — the twin of
        // the bulk read in `copy_flat_subrecord` (loft#729).
        let wordwise = std::env::var_os("LOFT_LOADER_WORDWISE").is_some();
        let buf = if wordwise {
            Vec::new()
        } else {
            reader.resolve(u64::from(src_rec) * 8 + u64::from(src_pos), bytes as usize)
        };
        let mut off = 0u32;
        while off + 4 <= bytes {
            let i = off as usize;
            let w = if wordwise {
                reader.u32_at(src_rec, src_pos + off)
            } else {
                u32::from_ne_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]])
            };
            self.allocations[local.store_nr as usize].set_u32_raw(entry.rec, entry.pos + off, w);
            off += 4;
        }

        // 2) 3b.2–3b.4b — relocate the entry's pointer fields (text / vector /
        //    nested / vector<struct>) so the local copy owns its whole graph.
        self.relocate_ptr_fields(
            reader,
            entry.rec,
            entry.pos,
            src_rec,
            src_pos,
            content_tp,
            local.store_nr,
        );
        // The verified linker for the kind, never a hand-rolled insert: a working-set
        // copy that built its own bucket or node would be a second implementation of
        // the structure the collection is checked against.
        match kind {
            PagedKind::Hash => crate::hash::add(local, &entry, &mut self.allocations, keys),
            PagedKind::Trie => crate::trie_db::add(local, &entry, &mut self.allocations, keys),
            PagedKind::Radix => crate::radix_db::add(local, &entry, &mut self.allocations, keys),
        }
        true
    }

    /// Recursively relocate the POINTER fields of struct `tp` whose (already
    /// flat-copied) data starts at byte `dst_fb` in local record `dst_rec`, with
    /// the source starting at `src_fb` in `src_rec` read over `reader`. The two
    /// field-bases are separate because a hash entry copies same-offset (both 8)
    /// while a Sorted element copies from `8 + i·esize` in the source vector into
    /// a slot at a different local position. Handles: text / vector<scalar> (flat
    /// sub-record copy), inline nested struct (recurse, deeper base), vector<struct>
    /// (copy inner + recurse each element). Inline scalars are already correct.
    #[cfg(paged_store)]
    #[allow(clippy::too_many_arguments)] // dst/src (rec,base) + reader + tp + store — all essential
    fn relocate_ptr_fields<P: crate::paged_reader::PageProvider>(
        &mut self,
        reader: &mut crate::paged_reader::PagedReader<P>,
        dst_rec: u32,
        dst_fb: u32,
        src_rec: u32,
        src_fb: u32,
        tp: u16,
        store_nr: u16,
    ) {
        let fields = match &self.types[tp as usize].parts {
            Parts::Struct(f) | Parts::EnumValue(_, f) => f.clone(),
            _ => return,
        };
        for f in fields {
            let pos = u32::from(f.position);
            let (dst_off, src_off) = (dst_fb + pos, src_fb + pos);
            let ftp = f.content;
            if ftp == 5
                || matches!(self.types[ftp as usize].parts, Parts::Vector(e) if self.is_inline_scalar(e))
            {
                // Flat sub-record pointer (text / vector<scalar>).
                let src_sub = self.src_ptr(reader, dst_rec, dst_off, src_rec, src_off, store_nr);
                if src_sub != 0 {
                    // `text` is one byte per unit; a vector<scalar> is its element.
                    let esz = if ftp == 5 {
                        1
                    } else if let Parts::Vector(e) = self.types[ftp as usize].parts {
                        u32::from(self.size(e))
                    } else {
                        0
                    };
                    let new_sub = self.copy_flat_subrecord(reader, src_sub, store_nr, esz);
                    self.allocations[store_nr as usize].set_u32_raw(dst_rec, dst_off, new_sub);
                }
            } else if matches!(
                self.types[ftp as usize].parts,
                Parts::Struct(_) | Parts::EnumValue(_, _)
            ) {
                // Inline nested struct: same records, deeper bases.
                self.relocate_ptr_fields(reader, dst_rec, dst_off, src_rec, src_off, ftp, store_nr);
            } else if let Parts::Vector(elem_tp) = self.types[ftp as usize].parts {
                // vector<struct>: copy the inner record + recurse each element.
                let src_sub = self.src_ptr(reader, dst_rec, dst_off, src_rec, src_off, store_nr);
                if src_sub != 0 {
                    let new_sub = self.copy_vector_of_struct(reader, src_sub, elem_tp, store_nr);
                    self.allocations[store_nr as usize].set_u32_raw(dst_rec, dst_off, new_sub);
                }
            }
        }
    }

    /// loft#782 — the SOURCE addresses of every sub-record an element points at, read out
    /// of the already-copied local bytes.
    ///
    /// This is the phase that dominates a keyed load's depth once the lookups are batched:
    /// a `text` or nested vector is one round trip EACH, discovered while walking. loft#783
    /// is what makes batching them possible — the element block is copied locally first,
    /// pointers included, so every address is known before the walk rather than during it.
    ///
    /// Walks exactly the shape `relocate_ptr_fields` walks, so the two cannot disagree
    /// about which pointers exist; a null contributes nothing.
    #[cfg(paged_store)]
    fn collect_sub_records(
        &self,
        dst_rec: u32,
        dst_fb: u32,
        tp: u16,
        store_nr: u16,
        out: &mut Vec<(u64, usize)>,
    ) {
        let fields = match &self.types[tp as usize].parts {
            Parts::Struct(f) | Parts::EnumValue(_, f) => f.clone(),
            _ => return,
        };
        for f in fields {
            let dst_off = dst_fb + u32::from(f.position);
            let ftp = f.content;
            if matches!(
                self.types[ftp as usize].parts,
                Parts::Struct(_) | Parts::EnumValue(_, _)
            ) {
                self.collect_sub_records(dst_rec, dst_off, ftp, store_nr, out);
            } else if ftp == 5 || matches!(self.types[ftp as usize].parts, Parts::Vector(_)) {
                let sub = self.allocations[store_nr as usize].get_u32_raw(dst_rec, dst_off);
                if sub != 0 {
                    out.push((u64::from(sub) * 8, 8));
                }
            }
        }
    }

    /// The SOURCE pointer stored in a field, read from the local copy rather than
    /// re-fetched over the reader (loft#783).
    ///
    /// Every caller of [`relocate_ptr_fields`](Stores::relocate_ptr_fields) flat-copies the
    /// element's bytes into `dst` BEFORE relocating its pointers, and a pointer word is
    /// part of those bytes — so the value is already in local memory, and asking the reader
    /// for it again is a 4-byte paged request for something we hold. That request is one
    /// per pointer field per ELEMENT: on a 100×250 `vector<Named>` it was 25 000 of the
    /// 75 893 four-byte reads a load issued.
    ///
    /// `LOFT_LOADER_WORDWISE=1` takes the old route, so the two can be compared on one
    /// binary — the same escape hatch loft#729's bulk-body read kept.
    #[cfg(paged_store)]
    fn src_ptr<P: crate::paged_reader::PageProvider>(
        &mut self,
        reader: &mut crate::paged_reader::PagedReader<P>,
        dst_rec: u32,
        dst_off: u32,
        src_rec: u32,
        src_off: u32,
        store_nr: u16,
    ) -> u32 {
        if std::env::var_os("LOFT_LOADER_WORDWISE").is_some() {
            return reader.u32_at(src_rec, src_off);
        }
        self.allocations[store_nr as usize].get_u32_raw(dst_rec, dst_off)
    }

    /// Copy a FLAT record (a string or a `vector<scalar>` inner record — no
    /// interior pointers) at `src_rec` into a fresh local claim; the size header
    /// (fld 0) is set by `claim`, so copy the length + payload from fld 4 on.
    #[cfg(paged_store)]
    fn copy_flat_subrecord<P: crate::paged_reader::PageProvider>(
        &mut self,
        reader: &mut crate::paged_reader::PagedReader<P>,
        src_rec: u32,
        store_nr: u16,
        elem_size: u32,
    ) -> u32 {
        // loft#783 — the size word (fld 0) and the length (fld 4) are ADJACENT, and every
        // flat sub-record needs both. Read as ONE 8-byte request instead of two 4-byte
        // ones: on a 100×250 `vector<Named>` that was 25 000 requests, the second of the
        // three this path issued per element. Same bytes, same page, half the calls.
        let head = reader.resolve(u64::from(src_rec) * 8, 8);
        let ssz = u32::from_ne_bytes([head[0], head[1], head[2], head[3]]);
        let head_len = u32::from_ne_bytes([head[4], head[5], head[6], head[7]]);
        // loft#729 — claim what the LENGTH needs, not what the source's CAPACITY
        // is.
        //
        // A record that grew in place is left at 7/4 of the size that triggered
        // the growth (`Store::resize`), and that amortisation is never given
        // back — so a persisted store carries it, and a loader that claims
        // `record_words` faithfully reproduces it in the working set. It is
        // invisible to compaction, which reclaims free space BETWEEN records and
        // cannot see slack INSIDE one: on the store below, compaction correctly
        // reports "interior free space is under the eighth the image format
        // already allows" while every vector record is up to 1.75x its content.
        //
        // Measured on a real 162 MB store, the 42-tile working set written back
        // out: 4 890 240 bytes claiming capacity, 2 198 904 claiming length —
        // 2.22x, with the content digest unchanged. That is the bulk of the
        // amplification reported in loft#729, and it is memory the browser pays
        // for every viewport.
        //
        // `min(ssz)` because the source is the authority on what can be read: a
        // length that disagrees with the record size must never widen the copy.
        let want = if elem_size > 0 {
            let len = u64::from(head_len);
            let bytes = 8 + len * u64::from(elem_size);
            u32::try_from(bytes.div_ceil(8)).unwrap_or(ssz).min(ssz)
        } else {
            ssz
        };
        let new = self.allocations[store_nr as usize].claim(want.max(2));
        self.allocations[store_nr as usize].zero_fill(new);
        // loft#729 — fetch the record's body ONCE, then unpack it locally.
        // Reading it word by word issued one `resolve` per 4 bytes: on a real
        // 162 MB store a single 42-key viewport made 1 073 065 of them and
        // 101 of everything else, so every byte of a working set arrived
        // through a 4-byte request. That is what made the reader's cost
        // page-shaped — a word-at-a-time walk pays a page of rounding wherever
        // it lands, and no span-sized optimisation below it can bite on a
        // request that is never bigger than a word.
        let body = usize::try_from((want * 8).saturating_sub(4)).unwrap_or(0);
        let wordwise = std::env::var_os("LOFT_LOADER_WORDWISE").is_some();
        let buf = if wordwise {
            Vec::new()
        } else {
            reader.resolve(u64::from(src_rec) * 8 + 4, body)
        };
        let mut sf = 4u32;
        while sf + 4 <= want * 8 {
            let i = (sf - 4) as usize;
            let w = if wordwise {
                reader.u32_at(src_rec, sf)
            } else {
                u32::from_ne_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]])
            };
            self.allocations[store_nr as usize].set_u32_raw(new, sf, w);
            sf += 4;
        }
        new
    }

    /// Copy a `vector<struct>` inner record: first the flat bytes (length + the
    /// contiguous element structs), then relocate EACH element's pointer fields
    /// (its text / vector / nested graph). Elements are at `8 + i·elem_size`.
    #[cfg(paged_store)]
    fn copy_vector_of_struct<P: crate::paged_reader::PageProvider>(
        &mut self,
        reader: &mut crate::paged_reader::PagedReader<P>,
        src_inner: u32,
        elem_tp: u16,
        store_nr: u16,
    ) -> u32 {
        let esize = u32::from(self.size(elem_tp));
        let new_inner = self.copy_flat_subrecord(reader, src_inner, store_nr, esize);
        let length = reader.u32_at(src_inner, 4);
        if esize == 0 {
            return new_inner;
        }
        // loft#782 — warm every element's sub-record in ONE batch before relocating, so the
        // walk stops paying a round trip per element. The addresses come from the local
        // copy (loft#783), so collecting them costs the reader nothing.
        //
        // Headers only, deliberately. Warming the BODIES too needs each sub-record's size
        // word, and reading 1 000 of those put back the per-element four-byte read that
        // loft#783 had just removed (109 → 1 222 on the guard) while measuring no further
        // depth win — the bodies were already on the warmed pages. A batch that costs a
        // read per element to save a page nobody needed is the trade in reverse.
        let mut heads = Vec::new();
        for i in 0..length {
            self.collect_sub_records(new_inner, 8 + i * esize, elem_tp, store_nr, &mut heads);
        }
        reader.warm(&heads);

        for i in 0..length {
            // element i's data starts at byte 8 + i·esize in BOTH the copied
            // inner record and the source (this is a copy, so same offsets).
            let fb = 8 + i * esize;
            self.relocate_ptr_fields(reader, new_inner, fb, src_inner, fb, elem_tp, store_nr);
        }
        new_inner
    }

    /// Read a `vector<integer>` (`keys_vec`) into an `i64` slice and load those
    /// keys via [`load_keys`](Stores::load_keys). Used by BOTH backends — the
    /// interpreter handler and the `#rust` codegen body.
    #[cfg(paged_store)]
    pub fn load_keys_vec(&mut self, local: &DbRef, path: &str, keys_vec: &DbRef) -> i64 {
        let vals = self.int_vector(keys_vec);
        self.load_keys(local, path, &vals)
    }

    /// Read a `vector<text>` (`keys_vec`) and load those keys via
    /// [`load_keys_text`](Stores::load_keys_text) — the text twin of
    /// [`load_keys_vec`](Stores::load_keys_vec). Used by BOTH backends: the
    /// interpreter handler and the `#rust` codegen body (loft#1064).
    #[cfg(paged_store)]
    pub fn load_keys_text_vec(&mut self, local: &DbRef, path: &str, keys_vec: &DbRef) -> i64 {
        // Owned first: reading the keys borrows `self`, and loading them mutates it.
        let owned = self.str_vector(keys_vec);
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        self.load_keys_text(local, path, &refs)
    }

    /// The strings a `vector<text>` holds — the text twin of
    /// [`int_vector`](Stores::int_vector), keeping the element layout in one place.
    /// A `text` element is a 4-byte string-record offset at `8 + i·4`.
    ///
    /// Not `text_vector`: that name is taken by the WRITE direction (build a
    /// `vector<text>` from a `&[String]`), and the two would read alike.
    #[cfg(paged_store)]
    fn str_vector(&self, v: &DbRef) -> Vec<String> {
        let length = crate::vector::length_vector(v, &self.allocations);
        let inner = self.store(v).get_u32_raw(v.rec, v.pos);
        (0..length)
            .map(|i| {
                let off = self.store(v).get_u32_raw(inner, 8 + i * 4);
                self.store(v).get_str(off).to_string()
            })
            .collect()
    }

    /// The `i64` values a `vector<integer>` holds — the single vector-reading home
    /// for the paged loaders, so the element layout lives in one place. `integer`
    /// elements are i64 at `8 + i·8` within the vector's inner record.
    #[cfg(paged_store)]
    fn int_vector(&self, v: &DbRef) -> Vec<i64> {
        let length = crate::vector::length_vector(v, &self.allocations);
        let inner = self.store(v).get_u32_raw(v.rec, v.pos);
        (0..length)
            .map(|i| self.store(v).get_int(inner, 8 + i * 8))
            .collect()
    }

    /// Read two `vector<integer>` corners and load the box they name via
    /// [`load_box`](Stores::load_box) — the vector-reading twin of
    /// [`load_keys_vec`](Stores::load_keys_vec), used by BOTH backends.
    ///
    /// A vector rather than a fixed `(x1, y1, x2, y2)`: the same call then serves a
    /// 1-D, 2-D and 3-D collection, which is what the resident slice already does
    /// through its `MAX_AXES`-wide ABI.
    #[cfg(paged_store)]
    pub fn load_box_vec(
        &mut self,
        local: &DbRef,
        path: &str,
        from: &DbRef,
        till: &DbRef,
        limit: i64,
    ) -> i64 {
        let (f, t) = (self.int_vector(from), self.int_vector(till));
        self.load_box(local, path, &f, &t, limit)
    }
}

/// The word where the store image's last ACTIVE record ends — its high-water
/// mark.  Walks the record chain (an i32 size word at each block's word
/// position; abs = block size in words, negative = free), so a trailing free
/// block is not counted.  `None` when the bytes do not describe a walkable
/// chain that terminates exactly at `src_words` (corrupt size word, zero-size
/// block, wrong layout) — the caller then persists the image verbatim rather
/// than trusting a walk it could not complete.
///
/// loft#710: a persisted store used to be the arena's whole CAPACITY, and the
/// tail of that capacity is whatever the last growth (7/3) over-allocated —
/// 43% of the reported 7 MB file was one trailing free block of zeros.  Because
/// that has nothing to do with the data, two builds of the SAME records
/// differed 1.84× by construction order alone (where each happened to land
/// relative to a growth step) and, worse, the size did not move when the
/// content changed by 1.8×.  Ending the image at the high-water mark makes it
/// track the data.  Interior free blocks still remain: reclaiming those means
/// relocating records and rewriting every `DbRef`, which is compaction, not
/// this.
#[cfg(feature = "mmap")]
fn store_image_live_end(src: &[u8], src_words: u32) -> Option<u32> {
    // PRIMARY = 1 is the first record's word position.
    let mut rec: u32 = 1;
    let mut live_end: u32 = 1;
    while rec < src_words {
        let off = (rec as usize) * 8;
        if off + 4 > src.len() {
            return None;
        }
        let sz = i32::from_le_bytes([src[off], src[off + 1], src[off + 2], src[off + 3]]);
        if sz == 0 {
            return None;
        }
        let step = sz.unsigned_abs();
        rec = rec.checked_add(step)?;
        if sz > 0 {
            live_end = rec; // an active block: the image must reach past it
        }
    }
    // Chain must terminate exactly at the end of the source, else the image is
    // malformed or does not follow the loft Store layout.
    if rec == src_words {
        Some(live_end)
    } else {
        None
    }
}

/// @PLAN38 — build the Store byte image to persist, exactly `target_words`
/// long, with the record chain still valid.
///
/// Longer than the live data (the `bind_path` floor, or an active record at the
/// tail): the remainder becomes one trailing free block.  Exactly as long: the
/// chain ends where the last record does, with no free tail at all — which is
/// the loft#710 case, where the tail was 43% of the file.  Either way the
/// result is walkable, and the free list is not in the image: `Store::open`
/// rebuilds it from this chain, so dropping a trailing free block loses
/// nothing.
///
/// Returns `None` if the source bytes don't describe a walkable record chain,
/// or if `target_words` would cut into live data.
#[cfg(feature = "mmap")]
fn build_padded_store_image(src: &[u8], src_words: u32, target_words: u32) -> Option<Vec<u8>> {
    if (src.len() as u32) < src_words.saturating_mul(8) {
        return None;
    }
    let live_end = store_image_live_end(src, src_words)?;
    if target_words < live_end {
        return None; // would truncate a live record
    }
    let out_bytes = (target_words as usize) * 8;
    let mut out = vec![0u8; out_bytes];
    let copy = (live_end as usize) * 8;
    out[..copy].copy_from_slice(&src[..copy]);

    if target_words > live_end {
        // One free block covers everything past the last record, whether that
        // space came from padding up to the floor or from the arena's own slack.
        let free_words = target_words - live_end;
        let size = -i32::try_from(free_words).ok()?;
        let off = (live_end as usize) * 8;
        out[off..off + 4].copy_from_slice(&size.to_le_bytes());
    }
    Some(out)
}

/// Write `bytes` to `path` without ever leaving a half-written file: build it
/// beside the target and rename over.  Rename is atomic, so a reader either
/// sees the old image or the new one — never a truncated store.  @PLN123 B3.
#[cfg(feature = "mmap")]
fn write_image_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut tmp = path.to_path_buf();
    let mut name = tmp
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    name.push(".compact.tmp");
    tmp.set_file_name(name);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod p318_hash_deepcopy {
    use crate::database::Parts;
    use crate::database::Stores;
    use crate::keys::DbRef;

    /// @P318 — deep-copying a struct whose `hash<T[k]>` field is populated must
    /// keep the hash FIND-CONSISTENT even when the destination bucket record is
    /// OVER-sized.  `Store::claim` returns a block up to 1/3 larger than
    /// requested without splitting (`claim_block`), and a hash reads its `room`
    /// (bucket count, `elms = (room-2)*2`) from that size header — so the old
    /// slot-for-slot copy laid the dest buckets out for the SOURCE room while
    /// `find` later probed `key % dest_elms` (a DIFFERENT start slot) and missed
    /// entries.  The gap `(room, room*4/3]` is tiny for small rooms (e.g. (9,12],
    /// (17,22]), so a freed record easily lands in it.  This test forces the
    /// over-size deterministically — claim `big, gap=room+1, big` contiguously
    /// then delete the middle, so `fl_take_ge(room)` returns the gap block — and
    /// asserts every key survives the deep copy.
    /// Positive control for the hash arm of `validate_claims` (the walk behind
    /// `store_verify`) — a green check is only evidence once the detector is
    /// known to FIRE. Build a sound hash (must validate clean), then corrupt the
    /// root pointer to an out-of-range bucket record and confirm the walk NAMES
    /// the broken edge instead of faulting on it — exactly what a bad relocation
    /// would leave behind (a source rec-number larger than the small local store).
    #[test]
    // @PLN54 S5(b) — the DA gate: `validate_claims`' GRACEFUL out-of-range
    // detection (name the broken edge, don't fault) is a RELEASE-mode contract.
    // Under debug-assertions the lower-level `Store::addr` bounds `debug_assert!`
    // (store.rs) fail-fasts on the dangling read FIRST — an equally valid, louder
    // signal, but not the graceful path this positive control asserts.  So skip
    // it under DA (where the fail-fast is the correct behavior), run it otherwise.
    #[cfg_attr(
        debug_assertions,
        ignore = "graceful dangling-walk is release-mode; DA addr() fail-fasts first"
    )]
    fn verify_graph_catches_a_dangling_pointer() {
        let mut stores = Stores::new();
        let cell = stores.structure("VCell", -1);
        stores.field(cell, "k", 0); // integer
        stores.field(cell, "v", 0); // integer
        stores.finish();
        let hash_tp = stores.hash(cell, &["k".to_string()]);
        let holder = stores.structure("VHolder", -1);
        stores.field(holder, "h", hash_tp);
        stores.finish();

        let words = |sz: u16| 1 + ((u32::from(sz) + 7) >> 3);
        let cell_words = words(stores.size(cell));
        let holder_words = words(stores.size(holder));

        let root = stores.database(holder_words);
        let h = DbRef {
            store_nr: root.store_nr,
            rec: root.rec,
            pos: root.pos, // field `h` at struct-position 0
        };
        for k in 0..5i64 {
            let e = stores.database(cell_words);
            stores.store_mut(&e).set_int(e.rec, e.pos, k);
            stores.store_mut(&e).set_int(e.rec, e.pos + 8, k + 100);
            stores.set_keyed(&h, &e, hash_tp, false);
        }

        // Sound to start — no false positive.
        let mut problems = 0u32;
        stores.validate_claims(&h, hash_tp, "ctl", &mut problems);
        assert_eq!(problems, 0, "a well-formed hash must validate clean");

        // Corrupt: aim the hash root at a bucket record beyond the store.
        stores.store_mut(&h).set_u32_raw(h.rec, h.pos, 99_999);
        let mut broken = 0u32;
        stores.validate_claims(&h, hash_tp, "ctl", &mut broken);
        assert!(
            broken > 0,
            "validate_claims MUST catch an out-of-range bucket pointer (positive control), \
             not fault on it"
        );
    }

    #[test]
    fn hash_deepcopy_survives_oversized_dest_bucket() {
        let mut stores = Stores::new();
        let cell_tp = stores.structure("CellP318", -1);
        stores.field(cell_tp, "ck", 0); // integer
        stores.field(cell_tp, "payload", 0); // integer
        stores.finish();
        let hash_tp = stores.hash(cell_tp, &["ck".to_string()]);
        let holder_tp = stores.structure("HolderP318", -1);
        stores.field(holder_tp, "h", hash_tp);
        stores.finish();

        let words = |sz: u16| 1 + ((u32::from(sz) + 7) >> 3);
        let cell_words = words(stores.size(cell_tp));
        let holder_words = words(stores.size(holder_tp));

        // --- source Holder with a populated hash (n=14 -> room 17) ---
        let src = stores.database(holder_words);
        let src_h = DbRef {
            store_nr: src.store_nr,
            rec: src.rec,
            pos: src.pos, // field `h` is at struct-position 0
        };
        let n: i64 = 14;
        for k in 0..n {
            let v = stores.database(cell_words);
            stores.store_mut(&v).set_int(v.rec, v.pos, k); // ck = k
            stores.store_mut(&v).set_int(v.rec, v.pos + 8, k + 1000); // payload
            stores.set_keyed(&src_h, &v, hash_tp, false);
        }
        let cur = stores.store(&src_h).get_u32_raw(src_h.rec, src_h.pos);
        let room = stores.store(&src_h).record_words(cur);
        assert!(room >= 3, "source room {room} too small to over-size");

        // --- destination Holder + an engineered over-size free block ---
        let dst = stores.database(holder_words);
        let dst_h = DbRef {
            store_nr: dst.store_nr,
            rec: dst.rec,
            pos: dst.pos,
        };
        {
            let s = &mut stores.allocations[dst.store_nr as usize];
            let _p1 = s.claim(room * 8); // big
            let gap = s.claim(room + 1); // gap-sized: room < room+1 <= room*4/3 (room>=3)
            let _p3 = s.claim(room * 8); // big — pins the gap so delete can't merge it
            s.delete(gap); // free block of size room+1; the smallest free >= room
        }

        // --- deep-copy the populated hash field into the dest ---
        stores.copy_claims(&src_h, &dst_h, hash_tp);

        // --- every key must still be found, with the right payload ---
        let keys = stores.types[hash_tp as usize].keys.clone();
        let mut missing = 0;
        for k in 0..n {
            let probe = stores.database(cell_words);
            stores.store_mut(&probe).set_int(probe.rec, probe.pos, k);
            let key = crate::keys::get_key(&probe, &stores.allocations, &keys);
            let found = crate::hash::find(&dst_h, &stores.allocations, &keys, &key);
            if found.rec == 0 {
                missing += 1;
            } else {
                let pay = stores.store(&found).get_int(found.rec, found.pos + 8);
                assert_eq!(pay, k + 1000, "wrong payload for key {k}");
            }
        }
        assert_eq!(
            missing, 0,
            "deep-copied hash lost {missing}/{n} entries (source room {room})"
        );
    }

    /// @PLN102 heap-free audit — the null/absent container is a no-op at the free/teardown
    /// chokepoints, safe by CONSTRUCTION not by caller convention. Without the `remove_claims`
    /// guard this indexes `allocations[u16::MAX]` (`for_each_owned_child` reads `store(rec)`) and
    /// OOB-panics; the assertion is that it does not. `free_named`/`free` already guarded — pinned
    /// here too so a future null-container caller can never reintroduce the panic.
    #[test]
    fn free_and_teardown_no_op_on_the_null_sentinel() {
        let mut stores = Stores::new();
        let cell = stores.structure("NCell", -1);
        stores.field(cell, "x", 0); // integer
        stores.finish();
        // None of these may panic; each is a no-op on the `store_nr == u16::MAX` sentinel.
        stores.remove_claims(&DbRef::NULL, cell);
        stores.free_named(&DbRef::NULL, "");
        stores.free(&DbRef::NULL);
    }

    /// @PLN102 heap copy/alias audit — positive control for the `elm == 0` (absent element)
    /// guards in `copy_claims_array_body` (copy) and `remove_claims` (free). A `vector<S>`
    /// field alongside an `index<S[k]>` over the SAME `S` becomes a record-per-element `Array`;
    /// every runtime path keeps its length in sync with the filled slots, so a 0 rec-id slot
    /// is unreachable in practice. Here we hand-build that state (slot0 = real element,
    /// slot1 = 0) and assert the walks treat the 0 slot as an absent hole — the copy preserves
    /// it as 0 (never fabricating an element from reserved record 0) and the free skips it
    /// (never `delete(0)` nor recursing into record 0). Proven to FAIL without the copy guard:
    /// the absent slot then copies as a bogus non-zero record id.
    #[test]
    fn array_copy_and_free_skip_an_absent_element_slot() {
        let mut stores = Stores::new();
        let cell = stores.structure("ACell", -1);
        stores.field(cell, "k", 0); // integer key
        stores.field(cell, "v", 0); // integer
        stores.finish();
        let vec_tp = stores.vector(cell);
        // An `index<ACell[k]>` field marks `ACell` LINKED, so the sibling `vector<ACell>`
        // is laid out record-per-element (`Array`) rather than inline (`Vector`).
        let idx_tp = stores.index(cell, &[("k".to_string(), false)]);
        let holder = stores.structure("AHolder", -1);
        stores.field(holder, "list", vec_tp);
        stores.field(holder, "ix", idx_tp);
        stores.finish();

        assert!(
            matches!(stores.types[vec_tp as usize].parts, Parts::Array(_)),
            "setup: a linked vector<ACell> must lay out as an Array (got {:?})",
            stores.types[vec_tp as usize].parts
        );

        let words = |sz: u16| 1 + ((u32::from(sz) + 7) >> 3);
        let cell_words = words(stores.size(cell));
        let holder_words = words(stores.size(holder));
        let Parts::Struct(fields) = &stores.types[holder as usize].parts else {
            unreachable!("holder is a struct")
        };
        let list_pos = u32::from(fields[0].position);

        // Source holder whose Array container has slot0 = a real element, slot1 = 0 (absent).
        let src = stores.database(holder_words);
        let list_src = DbRef {
            store_nr: src.store_nr,
            rec: src.rec,
            pos: src.pos + list_pos,
        };
        let elem = stores.store_mut(&src).claim(cell_words);
        stores.store_mut(&src).set_int(elem, 4, 7); // element payload
        let cur = stores.store_mut(&src).claim(2); // container: header word + one slot word (len 2)
        stores.store_mut(&src).set_u32_raw(cur, 4, 2); // length header = 2
        stores.store_mut(&src).set_u32_raw(cur, 8, elem); // slot0 → real element
        stores.store_mut(&src).set_u32_raw(cur, 12, 0); // slot1 → ABSENT (the guarded state)
        stores
            .store_mut(&list_src)
            .set_u32_raw(list_src.rec, list_src.pos, cur);

        // COPY — the absent slot must copy as a hole (0), never a record fabricated from rec 0.
        let dest = stores.database(holder_words);
        let list_dest = DbRef {
            store_nr: dest.store_nr,
            rec: dest.rec,
            pos: dest.pos + list_pos,
        };
        stores.copy_claims(&list_src, &list_dest, vec_tp);
        let dcur = stores
            .store(&list_dest)
            .get_u32_raw(list_dest.rec, list_dest.pos);
        assert_ne!(dcur, 0, "copied Array container must exist");
        assert_eq!(
            stores.store(&list_dest).get_u32_raw(dcur, 4),
            2,
            "copied length header preserved"
        );
        assert_ne!(
            stores.store(&list_dest).get_u32_raw(dcur, 8),
            0,
            "slot0 (real element) is copied"
        );
        assert_eq!(
            stores.store(&list_dest).get_u32_raw(dcur, 12),
            0,
            "slot1 (absent element) stays a hole — copy MUST NOT dereference record 0"
        );

        // FREE — teardown must not recurse into / delete record 0 for the absent slots
        // (both the hand-built source and the copied destination carry one).
        stores.remove_claims(&src, holder);
        stores.free(&dest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Stores;

    /// Teardown: `remove_claims` on a trie must free every element record AND the tree.
    ///
    /// The gate for step 4 of `doc/claude/plans/text-keyed-trie.md`, and it guards a
    /// fault that was real: `for_each_owned_child` had no `Trie` arm, so a trie fell into
    /// its catch-all, the keystone yielded no children and no container, and
    /// `remove_claims` freed NOTHING — every element and the tree itself leaked, with
    /// nothing said.  `claims_count` is the observable, because `claim` inserts and
    /// `delete` removes: a teardown that works returns it to where it started.
    #[test]
    fn a_trie_frees_every_element_and_its_tree() {
        let mut stores = Stores::new();
        // struct Word { w: text }
        let word_tp = stores.structure("Word", -1);
        stores.field(word_tp, "w", 5); // text
        stores.finish();
        let trie_tp = stores.trie(word_tp, "w");
        assert_ne!(trie_tp, u16::MAX, "the trie type registered");
        stores.determine_keys();

        // A record holding one trie field at payload offset 0.
        let coll = stores.database(4);
        let store_nr = coll.store_nr;
        let baseline = stores.allocations[store_nr as usize].claims_count();

        let keys = stores.types[trie_tp as usize].keys.clone();
        assert_eq!(keys.len(), 1, "a trie registers exactly one key");
        for w in ["kerkstraat", "kerk", "lonneker", "kerf"] {
            let elm = {
                let s = &mut stores.allocations[store_nr as usize];
                let ptr = s.set_str(w);
                let rec = s.claim(4);
                s.set_u32_raw(rec, 8, ptr);
                rec
            };
            let e = DbRef {
                store_nr,
                rec: elm,
                pos: 8,
            };
            crate::trie_db::add(&coll, &e, &mut stores.allocations, &keys);
        }
        assert_eq!(
            crate::trie_db::count(&coll, &stores.allocations),
            4,
            "four words are resident"
        );
        assert!(
            stores.allocations[store_nr as usize].claims_count() > baseline,
            "the inserts claimed records"
        );

        stores.remove_claims(&coll, trie_tp);

        assert_eq!(
            stores.allocations[store_nr as usize].get_u32_raw(coll.rec, coll.pos),
            0,
            "the field is cleared, so a re-bind starts empty"
        );
        assert_eq!(
            stores.allocations[store_nr as usize].claims_count(),
            baseline,
            "every element record and the tree itself are freed"
        );
    }
}
