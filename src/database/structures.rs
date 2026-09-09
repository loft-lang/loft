// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)
//! Structure allocation, initialization, field get/set, parsing operations.

use crate::database::{Field, Parts, Stores};

/// `LOFT_TRACE_VADD=1` — trace `vector_add` stride resolution (read once;
/// `vector_add` is runtime-hot, a per-call `env::var` lookup is not).
fn vadd_trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_TRACE_VADD").is_ok())
}
use crate::keys::DbRef;
use crate::store::Store;
use crate::vector;
use crate::{hash, keys, tree};
use std::collections::HashSet;

/// Why a field is being given its absent value — which decides what that value may COST.
///
/// The two are the same for every type whose absence is a bit pattern, and differ only for
/// `text`, whose empty value has to be interned. `set_default_value` runs per RECORD on the
/// allocation path, where the literal or the walker that follows writes every field anyway;
/// making it intern there is pure garbage (measured: +78 % wall, +91 % peak heap over 400 000
/// three-text-field rows). What a reader actually sees is written by the walker, per FIELD,
/// and that is the call that has to be right.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Absent {
    /// Pre-fill for a freshly claimed record: a value nothing may MISREAD, on the
    /// understanding that the caller overwrites it.
    Prefill,
    /// The value that stays. A JSON key written `null`, one the document omits, and a parse
    /// that failed all leave the field exactly as this call writes it.
    Final,
}

/// Walker-native diagnostic for `walk_parsed_into` failures.
///
/// `at` is a byte offset into the original input; `path` is the
/// dotted-key / `[index]` path to the failing node.  `format.rs`
/// converts these into the user-visible `"line N:M path:X"` shape
/// using `crate::json::line_col_of`.
pub(super) struct WalkErr {
    pub at: usize,
    pub path: Vec<String>,
}

/// The part of a collection's `Parts` that `insert_record` dispatches on, copied out so the
/// arms can borrow `self` mutably without cloning a `Struct`'s field list per insert
/// (@PLN157 § V-k).
#[derive(Clone, Copy)]
enum InsertKind {
    Vector,
    Sorted(u16),
    Array,
    Hash(u16),
    Index(u16),
    Ordered,
    Trie,
    Radix,
    Other,
}

impl InsertKind {
    fn of(parts: &Parts) -> Self {
        match parts {
            Parts::Vector(_) => Self::Vector,
            Parts::Sorted(c, _) => Self::Sorted(*c),
            Parts::Array(_) => Self::Array,
            Parts::Hash(c, _) => Self::Hash(*c),
            Parts::Index(c, _, _) => Self::Index(*c),
            Parts::Ordered(_, _) => Self::Ordered,
            Parts::Trie(_, _) => Self::Trie,
            Parts::Radix(_, _) => Self::Radix,
            _ => Self::Other,
        }
    }
}

impl Stores {
    /**
    # Panics
    When requesting a record on a non-structure
    */
    /// @PLN25 single-payload — for a FIELD (`field != MAX`) inside a `__nullable<S>` parent,
    /// redirect to the inline `payload`'s dense `S` (the `key_owner` struct) + add the payload
    /// base to the record ref, so the field resolves on dense `S` instead of the enum top level
    /// (whose field 0 is the discriminant).  Returns `(resolved_parent_tp, adjusted_ref)`.
    /// A `field == MAX` call (creating the element record itself, which IS the enum) and any
    /// non-nullable parent are returned unchanged.  Shared by `record_new` / `record_finish` so
    /// the create + finalize halves agree on the type/offset.
    fn nullable_field_parent(&self, data: &DbRef, parent_tp: u16, field: u16) -> (u16, DbRef) {
        if field != u16::MAX {
            let owner = self.key_owner(parent_tp);
            if owner != parent_tp {
                let mut adj = *data;
                adj.pos += u32::from(self.key_base(parent_tp));
                return (owner, adj);
            }
        }
        (parent_tp, *data)
    }

    /// The type of the sub-record `record_new` / `record_finish` operate on: the parent
    /// itself when `field == u16::MAX` (the element record IS the parent), otherwise that
    /// field's own content type.
    ///
    /// One derivation, shared by both halves, so the record that gets created and the record
    /// that gets inserted cannot disagree about their type.
    ///
    /// # Panics
    /// When the parent record does not declare `field`. `field_type` answers `u16::MAX` for
    /// a miss, which is a not-found sentinel and not a type — letting it through reaches the
    /// type table as an index and reports `index out of bounds: … the index is 65535`, naming
    /// neither the type nor the field it failed to find (loft#977).
    fn sub_record_type(&self, parent_tp: u16, field: u16) -> u16 {
        if field == u16::MAX {
            return parent_tp;
        }
        let tp = self.field_type(parent_tp, field);
        assert!(
            (tp as usize) < self.types.len(),
            "field {field} of '{}' has no storage in that type — a record operation named a \
             field the type does not declare",
            self.types
                .get(parent_tp as usize)
                .map_or("<unknown type>", |t| t.name.as_str())
        );
        tp
    }

    /// Create a fresh record for a collection element / nullable field and return its `DbRef`.
    ///
    /// # Panics
    /// Panics on an unsupported `parent_tp`/`field` parts kind (an internal invariant
    /// violation — the parser only emits `OpNewRecord` for collection/struct field types).
    pub fn record_new(&mut self, data: &DbRef, parent_tp: u16, field: u16) -> DbRef {
        // @PLN101 Slice 0 — count every heap record allocation (the cost value structs remove).
        self.records_created += 1;
        // @PLN157 § V-k — the commonest shape, `v += [x]` on a plain vector: `field` is
        // `u16::MAX`, so the three lookups below answer the identity, and the element is
        // an inline slot `vector_append` claims.  Taking it here, before the general
        // dispatch, is the difference the `lock` row measured (the append machinery was
        // 40 % of it); `set_default_value` still runs on the caller's side of this.
        if field == u16::MAX
            && let Parts::Vector(c) = self.types[parent_tp as usize].parts
        {
            return vector::vector_append(data, u32::from(self.size(c)), &mut self.allocations);
        }
        // @PLN25 single-payload: when creating a sub-record for a FIELD inside a
        // `__nullable<S>` element (a nested collection/struct), the field lives in the inline
        // `payload` (dense S), not at the enum's top level (field 0 there is the discriminant,
        // which has no sub-structure → `field_type` = MAX → OOB).  Redirect a `__nullable<S>`
        // parent to the payload's struct + base offset so the field resolution + sub-record
        // allocation target the dense `S` — the `key_owner` redirect.  Only for `field != MAX`:
        // a `field == MAX` call creates the element record ITSELF (which IS the enum), so it
        // must keep `parent_tp`.  Non-nullable parents are unchanged (`key_owner` = identity).
        let (parent_tp, data_owned) = self.nullable_field_parent(data, parent_tp, field);
        let data = &data_owned;
        // The top-level (`field == u16::MAX`) case is the parent itself; see `sub_record_type`.
        let tp = self.sub_record_type(parent_tp, field);
        let d = self.field_ref(data, parent_tp, field);
        match self.types[tp as usize].parts {
            Parts::Sorted(c, _) => {
                vector::sorted_new(&d, u32::from(self.size(c)), &mut self.allocations)
            }
            Parts::Vector(c) => {
                // The outer slot strides by the element type's own size — for a
                // nested vector that is the 4-byte rec-id handle.  #475 used to
                // special-case this to the INNER scalar width (min 4) because the
                // element type registered as the level-collapsed inner scalar, so
                // `size(c)` read the wrong thing; now that `vector_element_type`
                // registers a real `vector<inner>`, `size(c)` IS the handle and
                // the special case would re-introduce the very mismatch it fixed.
                vector::vector_append(&d, u32::from(self.size(c)), &mut self.allocations)
            }
            // @PLN135 arc H — a hash's entries live packed in a chunked arena instead
            // of one store record each: no per-entry header word, no `Store::claim`
            // rounding, and — what the measurement was actually about — entries dense
            // enough that a random lookup reads one cache line instead of walking a
            // working set 2–3x the payload.  The owning-collection back-pointer moves
            // with them, to byte 4 of the CHUNK, because every slot in a chunk shares
            // an owner and `database::search` reads that offset to decide a record is
            // live.  Allocation happens here, before the constructor writes any field,
            // exactly as the per-entry claim did.
            // ...EXCEPT when a sibling collection VIEWS these same records through
            // a 4-byte record id.  A `vector` / `sorted` over an element type that
            // another keyed field also holds becomes an `array` / `ordered`, whose
            // slots store `rec.rec` alone and are read back at a hard-coded
            // `pos = 8` (`vector::ordered_find`) — an address that only names an
            // element OWNING its record.  A packed entry lives at a stride INSIDE a
            // shared record, so its position is not recoverable from the id and
            // every entry in one chunk reads back as that chunk's first: right
            // length, wrong rows, silently (loft#843).
            //
            // `linked` is set exactly where that conversion happens
            // (`types.rs::finish_type`), so it IS the condition — and such a hash
            // allocates one record per entry, the way every hash did before the
            // arena.  `hash::add` then finds no table of its own and builds one
            // with stride 0, which is already how it records "these records are
            // somebody else's": it borrows them and frees nothing.
            Parts::Hash(c, _) if !self.types[c as usize].linked => {
                let stride = crate::hash::stride_for(u32::from(self.size(c)));
                crate::hash::alloc_entry(&d, stride, data.rec, &mut self.allocations)
            }
            Parts::Array(c)
            | Parts::Ordered(c, _)
            | Parts::Index(c, _, _)
            | Parts::Radix(c, _)
            | Parts::Trie(c, _)
            | Parts::Hash(c, _) => {
                let rec = self.claim(&d, 1 + ((u32::from(self.size(c)) + 7) >> 3));
                self.store_mut(&rec).set_u32_raw(rec.rec, 4, data.rec);
                rec
            }
            // loft#715 — say WHO handed us the type when a bridge call is on the
            // stack.  A type index that resolves to a non-structure is a program
            // error from loft code and an ABI/type-table mismatch from a shared
            // library, and the two read identically without this.
            _ => match crate::extensions::current_shared_bridge() {
                Some(bridge) => panic!(
                    "Cannot add to none-structure '{}' — raised inside {bridge}. \
                     The library passed a type index this loft's table does not \
                     map to a structure, which means it was built against a \
                     different type layout; rebuild the package's native-auto/ \
                     artifact.",
                    self.types[tp as usize].name
                ),
                None => panic!(
                    "Cannot add to none-structure '{}'",
                    self.types[tp as usize].name
                ),
            },
        }
    }

    /**
    # Panics
    When the implementation is not yet written
    */
    pub fn record_finish(&mut self, data: &DbRef, rec: &DbRef, parent_tp: u16, field: u16) {
        // @PLN157 § V-k — the twin of `record_new`'s short path: a plain vector's finish is
        // the length bump and nothing else (no siblings to link, no key to place).
        if field == u16::MAX && matches!(self.types[parent_tp as usize].parts, Parts::Vector(_)) {
            vector::vector_finish(data, &mut self.allocations);
            return;
        }
        // @PLN25 single-payload: mirror `record_new`'s nullable-field redirect so the
        // create + finalize halves agree on the type/offset (a FIELD inside a `__nullable<S>`
        // element resolves on the payload's dense `S`, not the enum top level).
        let (parent_tp, data_owned) = self.nullable_field_parent(data, parent_tp, field);
        let data = &data_owned;
        // The top-level (`field == u16::MAX`) case is the parent itself; see `sub_record_type`.
        let tp = self.sub_record_type(parent_tp, field);
        let d = self.field_ref(data, parent_tp, field);
        // loft#1226 — does this field SHARE its records with a sibling?  A member of a linked
        // group does (@FR-Col-Group: several routes to ONE record set), and a displaced record
        // is then still held by the other member, so a dedup here must UNLINK ONLY.
        //
        // `secondary` already carries exactly that instruction, but it was describing the
        // collection's ROLE IN THIS CALL rather than who holds the record: the sibling inserts
        // below pass `true`, and the PRIMARY insert passed `false` unconditionally — including
        // when the field written is itself a group member.  So `g.ordered += [v]; g.by_nm +=
        // [v]` freed the record the first append had put in BOTH members, while the vector went
        // on holding it: every `text` field of that record read `null` and nothing reported.
        // The `integer` beside it survived, because only the nested heap was released.
        //
        // Non-empty `other_indexes` is the membership test — including the leading `u16::MAX`
        // marker form, which says this field is a VIEW of records another field also holds, and
        // is the direction that most needs the guard.
        let shares_records = field != u16::MAX
            && matches!(
                &self.types[parent_tp as usize].parts,
                Parts::Struct(fields) | Parts::EnumValue(_, fields)
                    if fields.get(field as usize).is_some_and(|f| !f.other_indexes.is_empty())
            );
        self.insert_record(&d, rec, tp, shares_records);
        self.link_siblings(data, rec, parent_tp, field);
    }

    /// Put a record ONE member of a linked group already holds into every OTHER member —
    /// the sibling half of [`Self::record_finish`] on its own (`@FR-Col-Group`: a record
    /// entering through any member is in every member).
    ///
    /// An element-level write through the vector member — `w.es[i] = e`, or the `Some`
    /// half of `w.es[i] = e` on a `vector<E?>` — keeps the record's IDENTITY and changes
    /// its contents, so the keyed views that index it by key are unlinked before the write
    /// and handed the record again after it.  The primary already holds it, and a
    /// `record_finish` here would append the record to the vector a second time.
    /// `OpLinkRecord` is the op that reaches this, emitted by the parser beside the write.
    pub fn link_record_siblings(&mut self, data: &DbRef, rec: &DbRef, parent_tp: u16, field: u16) {
        // A record that is not there links nowhere: the element place of an out-of-range
        // index reads null, and the write before this was already dropped on it.  Tested
        // BEFORE any store is resolved — an absent read answers `DbRef::NULL`, whose store
        // number names no store.
        if rec.rec == 0 || data.rec == 0 {
            return;
        }
        let (parent_tp, data_owned) = self.nullable_field_parent(data, parent_tp, field);
        self.link_siblings(&data_owned, rec, parent_tp, field);
    }

    /// The sibling walk itself, over a parent already redirected through
    /// [`Self::nullable_field_parent`]: every field named by `other_indexes` is handed
    /// the record as a SECONDARY insert (index-only, never freeing what it displaces).
    fn link_siblings(&mut self, data: &DbRef, rec: &DbRef, parent_tp: u16, field: u16) {
        if field != u16::MAX
            && let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                self.types[parent_tp as usize].parts.clone()
        {
            let f = &fields[field as usize];
            let o = &f.other_indexes;
            {
                for fld_nr in o {
                    // A leading `u16::MAX` marks this field as a VIEW of records
                    // another field also holds — read by the JSON walk to skip
                    // default-initialising it. It is a marker, not a field
                    // number, so it is SKIPPED rather than treated as the end of
                    // the list: that is what lets a view maintain its siblings
                    // too, and an insert then means the same thing whichever of
                    // the collections it is spelled through (@FR-Col-Group).
                    if *fld_nr == u16::MAX {
                        continue;
                    }
                    let sibling_content = fields[*fld_nr as usize].content;
                    // @PLN25 Scope B — a keyed index over a shared NULLABLE array indexes only
                    // the non-null records: when the sibling keyed element is `__nullable<S>`
                    // and this record is the `Null` variant (discriminant 1 at byte offset 0),
                    // skip the keyed insert.  The null stays in the vector (the primary insert
                    // at the top of this fn) but is unreachable by key and cannot collide with a
                    // real empty/zero-key element.  Inert for dense (non-nullable) keyed elements.
                    // Index ONLY a `Some` record (discriminant 2).  A null element is either
                    // the zeroed/absent slot (discriminant 0) or the explicit `Null` variant
                    // (discriminant 1); both are skipped.
                    if self.absent_nullable_record(self.content(sibling_content), rec) {
                        continue;
                    }
                    let o = self.field_ref(data, parent_tp, *fld_nr);
                    // Secondary index for a sibling field — index-only, never
                    // delete the displaced record (the primary collection owns it).
                    self.insert_record(&o, rec, sibling_content, true);
                }
            }
        }
    }

    /// @P305 — true when the keyed field at byte offset `byte_off` in
    /// struct / enum-value type `struct_tp` is cross-linked with a sibling
    /// index (the multi-index case: two-or-more keyed fields sharing an
    /// element type are auto-linked in `types.rs`).  `OpSetKeyed` lacks the
    /// struct + field context to maintain the sibling indexes, so the parser
    /// falls back to the (non-corrupting) update-only path for these.
    #[must_use]
    pub fn keyed_field_is_linked(&self, struct_tp: u16, byte_off: u16) -> bool {
        if (struct_tp as usize) >= self.types.len() {
            return false;
        }
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[struct_tp as usize].parts
        {
            for f in fields {
                if f.position == byte_off {
                    return !f.other_indexes.is_empty();
                }
            }
        }
        false
    }

    /// The content type of the field at byte offset `byte_off` in struct /
    /// enum-value type `struct_tp`, or `None` when there is no such field.
    ///
    /// The offset is what the IR carries (`OpGetField`'s second argument), so this is
    /// how a nested field access resolves the struct it reads out of.
    #[must_use]
    pub fn field_content_at(&self, struct_tp: u16, byte_off: u16) -> Option<u16> {
        if (struct_tp as usize) >= self.types.len() {
            return None;
        }
        let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
            &self.types[struct_tp as usize].parts
        else {
            return None;
        };
        fields
            .iter()
            .find(|f| f.position == byte_off)
            .map(|f| f.content)
    }

    /// loft#1152 — insert every record the vector `primary` holds into the group member
    /// `view`, whose collection type is `view_tp`.
    ///
    /// `Stores::record_finish` is the chokepoint that maintains a linked group: it walks the
    /// field's `other_indexes` and inserts the record into every sibling, so every route that
    /// adds records ONE AT A TIME keeps the members agreeing.  A whole-vector write does not
    /// pass through it — `OpAppendVector` reaches `vector_add` → `vector_add_array`, which
    /// moves the records in bulk — so the views stayed empty and nothing said so: `len`
    /// answered `0` and a lookup answered `null`, both legal values for an empty group.
    ///
    /// The records are NOT copied.  A group is several routes to a SINGLE record set, so the
    /// view is handed the primary's own element records by id, exactly as `record_finish`
    /// hands them over — which is what makes a write through the vector visible through the
    /// view.  `secondary: true` says so: the view indexes, and never frees.
    ///
    /// Reads the array through `is_linked`, not through the caller's word: a grouped element
    /// type is promoted to `Parts::Array` (a u32 rec-id per slot) by `finish_type`, and on an
    /// UNPROMOTED vector those same bytes are inline payload, so walking them as ids would
    /// hand `insert_record` addresses built out of field data.
    pub fn index_group_records(&mut self, primary: &DbRef, view: &DbRef, view_tp: u16) {
        if (view_tp as usize) >= self.types.len() {
            return;
        }
        let elem = self.content(view_tp);
        if elem == u16::MAX || !self.is_linked(elem) {
            return;
        }
        let length = vector::length_vector(primary, &self.allocations);
        if length == 0 {
            return;
        }
        let arr = keys::store(primary, &self.allocations).get_u32_raw(primary.rec, primary.pos);
        if arr == 0 {
            return;
        }
        for i in 0..length {
            let rec = keys::store(primary, &self.allocations).get_u32_raw(arr, 8 + 4 * i);
            // A null element stays out of the index: it is reachable in the vector by
            // position and has no key to be found under.
            if rec == 0 {
                continue;
            }
            let elem_ref = DbRef {
                store_nr: primary.store_nr,
                rec,
                pos: 8,
            };
            self.insert_record(view, &elem_ref, view_tp, true);
        }
    }

    /// loft#1159 — the FIELD INDEX of the field at byte offset `byte_off` in the struct or
    /// enum-value type `struct_tp`, or `None` when no field sits there.
    ///
    /// A field ref names a byte POSITION, while every group question — `other_indexes`, the
    /// sibling walk in [`Self::record_finish`] — is asked by field NUMBER. The two are
    /// related only through the field list, so the translation lives here beside the list
    /// rather than being re-derived at each caller.
    #[must_use]
    pub fn field_index_at(&self, struct_tp: u16, byte_off: u16) -> Option<u16> {
        if (struct_tp as usize) >= self.types.len() {
            return None;
        }
        let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
            &self.types[struct_tp as usize].parts
        else {
            return None;
        };
        fields
            .iter()
            .position(|f| f.position == byte_off)
            .map(|i| i as u16)
    }

    /// loft#898 — the members of the linked collection group the keyed field at
    /// `byte_off` belongs to, as `(byte_off, collection_tp, is_view)` per member,
    /// INCLUDING the field itself. Empty when the field is not in a group.
    ///
    /// Ordering a clear needs three facts the parser cannot read off the
    /// expression: which member owns the records, where each sibling sits in the
    /// struct, and what type each one is. `types.rs` records all three — a
    /// leading `u16::MAX` in `other_indexes` marks a VIEW, the rest of the list
    /// names the other members by field number — so this reads them out in one
    /// place rather than having the caller re-walk the schema per question.
    #[must_use]
    pub fn keyed_group_members(&self, struct_tp: u16, byte_off: u16) -> Vec<(u16, u16, bool)> {
        if (struct_tp as usize) >= self.types.len() {
            return Vec::new();
        }
        let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
            &self.types[struct_tp as usize].parts
        else {
            return Vec::new();
        };
        let Some(me) = fields.iter().position(|f| f.position == byte_off) else {
            return Vec::new();
        };
        if fields[me].other_indexes.is_empty() {
            return Vec::new();
        }
        // The list on the field names the OTHER members; add itself to get the
        // whole group. A `u16::MAX` entry is the view MARKER, not a field number.
        let mut nrs: Vec<u16> = vec![me as u16];
        nrs.extend(
            fields[me]
                .other_indexes
                .iter()
                .copied()
                .filter(|n| *n != u16::MAX),
        );
        nrs.sort_unstable();
        nrs.dedup();
        nrs.iter()
            .filter_map(|n| {
                let f = fields.get(*n as usize)?;
                Some((
                    f.position,
                    f.content,
                    f.other_indexes.first() == Some(&u16::MAX),
                ))
            })
            .collect()
    }

    pub(super) fn field_ref(&self, data: &DbRef, parent_tp: u16, field: u16) -> DbRef {
        if field == u16::MAX {
            *data
        } else if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[parent_tp as usize].parts
        {
            DbRef {
                store_nr: data.store_nr,
                rec: data.rec,
                pos: data.pos + u32::from(fields[field as usize].position),
            }
        } else {
            *data
        }
    }

    /// Back `reserve(h, n)` on a hash: size its bucket table for `n` entries up front
    /// so filling it does not rebuild the table on the way (@PLN135 arc C).
    ///
    /// Capacity only — the collection's contents and its `len` are untouched, and a
    /// count the table already covers does nothing.  A non-hash `tp` cannot reach here
    /// (the parser only emits `OpReserveHash` for a hash), so it is a silent no-op
    /// rather than a fault.
    pub fn reserve_hash(&mut self, data: &DbRef, count: i64, tp: u16) {
        let Parts::Hash(c, _) = self.types[tp as usize].parts else {
            return;
        };
        let keys = self.types[tp as usize].keys.clone();
        // Stride 0 for a hash whose records a sibling `array` / `ordered` also
        // holds: those entries are allocated one per record (see `record_new`),
        // and stride 0 is how a table records that it BORROWS them. Sizing the
        // table with a real stride here would claim an arena the entries never
        // come from, and make the teardown believe it owns them (loft#843).
        let stride = if self.types[c as usize].linked {
            0
        } else {
            crate::hash::stride_for(u32::from(self.size(c)))
        };
        crate::hash::reserve(data, count, stride, &mut self.allocations, &keys);
    }

    pub(super) fn insert_record(&mut self, data: &DbRef, rec: &DbRef, tp: u16, secondary: bool) {
        // The kind and its content id are two words; cloning the whole `Parts` (a
        // `Struct`'s field list included) per insert was 1.5 % of the `lock` row
        // (@PLN157 § V-k).  The arms borrow `self` mutably, so the match is on a copy of
        // exactly what they read.
        let kind = InsertKind::of(&self.types[tp as usize].parts);
        match kind {
            InsertKind::Vector => {
                vector::vector_finish(data, &mut self.allocations);
            }
            InsertKind::Sorted(c) => {
                let size = u32::from(self.size(c));
                vector::sorted_finish(
                    data,
                    size,
                    &self.types[tp as usize].keys,
                    &mut self.allocations,
                );
            }
            InsertKind::Array => {
                let reference = vector::vector_append(data, 4, &mut self.allocations);
                self.store_mut(data)
                    .set_u32_raw(reference.rec, reference.pos, rec.rec);
                vector::vector_finish(data, &mut self.allocations);
            }
            InsertKind::Hash(c) => {
                // @P306 — replace any existing record with this key (dedup).
                self.dedup_keyed(data, rec, tp, c, secondary);
                let keys = self.types[tp as usize].keys.clone();
                hash::add(data, rec, &mut self.allocations, &keys);
            }
            InsertKind::Index(c) => {
                // @P306 — replace any existing record with this key (dedup);
                // tree::add otherwise rejects the duplicate and keeps the old.
                self.dedup_keyed(data, rec, tp, c, secondary);
                let left = self.fields(tp);
                let keys = self.types[tp as usize].keys.clone();
                tree::add(data, rec, left, &mut self.allocations, &keys);
            }
            InsertKind::Ordered => {
                vector::ordered_finish(
                    data,
                    rec,
                    &self.types[tp as usize].keys,
                    &mut self.allocations,
                );
            }
            InsertKind::Trie => {
                // Same no-dedup contract as the spatial side: two records may share a
                // key, differing in the id suffix, and land adjacent (`r8b`).
                let keys = self.types[tp as usize].keys.clone();
                crate::trie_db::add(data, rec, &mut self.allocations, &keys);
            }
            InsertKind::Radix => {
                // @PLN48 S2 — no dedup: two records may share a cell (they differ in
                // the id suffix and land adjacent), which is what a spatial index
                // needs.  A future `radix<T[k]>` map surface can layer dedup on top.
                let keys = self.types[tp as usize].keys.clone();
                crate::radix_db::add(data, rec, &mut self.allocations, &keys);
            }
            InsertKind::Other => (),
        }
    }

    /// @PLAN53 cluster 3: sound cross-store byte copy.  Copies `len` bytes from
    /// `src_idx`'s record (`src_rec` @ `from_pos`) into `dst_idx`'s record
    /// (`dst_rec` @ `to_pos`).  `get_disjoint_mut` produces the two store borrows
    /// from disjoint sub-ranges of `allocations`, so neither overlaps the whole
    /// slice — the old `from_ref`/`from_mut` reborrow pair formed a `&[Store]`
    /// and a `&mut [Store]` over the same slice simultaneously, which Miri's
    /// Stacked Borrows (correctly) rejects as UB.  Behaviour is identical (same
    /// bytes copied).  Caller guarantees `src_idx != dst_idx`.
    #[allow(clippy::too_many_arguments)] // low-level (idx, rec, pos) src+dst+len copy descriptor
    pub(crate) fn copy_block_cross_store(
        &mut self,
        src_idx: u16,
        src_rec: u32,
        from_pos: isize,
        dst_idx: u16,
        dst_rec: u32,
        to_pos: isize,
        len: isize,
    ) {
        let [src_store, dst_store] = self
            .allocations
            .get_disjoint_mut([src_idx as usize, dst_idx as usize])
            .expect("copy_block_cross_store: src and dst store indices must be distinct");
        let src_store: &Store = src_store;
        src_store.copy_block_between(src_rec, from_pos, dst_store, dst_rec, to_pos, len);
    }

    /// P213: claim a fresh record in `host_field`'s Store of size matching
    /// `content_kt`, deep-copy `src`'s payload + nested heap fields into
    /// it, then write the new rec-id u32 into `host_field`.  Used by
    /// capturing-closure-in-struct-field writes to co-locate the
    /// parent-Store closure record in host's Store.  The new record's
    /// lifetime is bound to the host: `Parts::ChildRec(content_kt)`'s
    /// cascade in `copy_claims` / `remove_claims` deep-copies and frees
    /// it whenever the host is copied or freed.
    pub fn claim_child_rec(&mut self, host_field: &DbRef, src: &DbRef, content_kt: u16) {
        if src.rec == 0 {
            return;
        }
        let size = u32::from(self.size(content_kt));
        let new_rec = self.allocations[host_field.store_nr as usize].claim(size);
        let new_db = DbRef {
            store_nr: host_field.store_nr,
            rec: new_rec,
            pos: 8,
        };
        // Cross-store byte copy of the child record's payload.
        if host_field.store_nr == src.store_nr {
            self.store_mut(host_field).copy_block(
                src.rec,
                src.pos as isize,
                new_rec,
                new_db.pos as isize,
                size as isize,
            );
        } else {
            self.copy_block_cross_store(
                src.store_nr,
                src.rec,
                src.pos as isize,
                host_field.store_nr,
                new_rec,
                new_db.pos as isize,
                size as isize,
            );
        }
        // Deep-copy nested heap fields (text, Reference, vector, ...).
        self.copy_claims(src, &new_db, content_kt);
        // Write the new rec-id into host's field as a u32.
        self.store_mut(host_field)
            .set_u32_raw(host_field.rec, host_field.pos, new_rec);
    }

    /// P213: read the rec-id u32 at `host_field` and construct a `DbRef`
    /// pointing at element [0]'s standard struct payload start (`pos=8`)
    /// in the same Store as the host.  Returns the null sentinel
    /// (`store_nr=u16::MAX, rec=0, pos=0`) when the rec-id is 0
    /// (non-capturing / default-init case).
    #[must_use]
    pub fn ref_from_child_rec(&self, host_field: &DbRef) -> DbRef {
        let store = keys::store(host_field, &self.allocations);
        let rec = store.get_u32_raw(host_field.rec, host_field.pos);
        if rec == 0 {
            DbRef::NULL
        } else {
            DbRef {
                store_nr: host_field.store_nr,
                rec,
                pos: 8,
            }
        }
    }

    /// Make `db` (dest) hold `o_db` (src)'s content — the aliasing-safe vector
    /// "deliver into buffer" the return machinery needs.  When `db` and `o_db`
    /// name the SAME backing vector (the NRVO case where a returned local still
    /// ALIASES the function's buffer — `out` borrows `__vdb_1`, and the buffer is
    /// the return slot), the content is ALREADY in place: clearing first would
    /// DESTROY it, which is the `clear(buf); append(buf, out)` self-copy that
    /// silently returned an empty vector.  Same vector → no-op.  Distinct vectors
    /// → clear dest and append src (`vector_add` snapshots a same-store source).
    pub fn vector_replace(&mut self, db: &DbRef, o_db: &DbRef, known: u16) {
        // An ABSENT source copies NOTHING, and leaves the destination ABSENT — the same
        // rule `Stores::replace_keyed` carries for the keyed kinds (loft#1150), in the same
        // words, because it is the same question one collection kind over.  `vector_add`
        // alone reads an absent source as a zero LENGTH and returns, which leaves the
        // destination holding the empty store the bind allocated for it: `b = a` with `a`
        // absent then answered an empty vector where `a == null` (loft#1319).  Emptiness and
        // absence are different values, and only the whole-value replace may turn one into
        // the other — an `a += b` must leave `a` alone.
        if o_db.store_nr == u16::MAX {
            vector::clear_vector(db, &mut self.allocations);
            self.mark_collection_absent(db);
            return;
        }
        let dest_rec = keys::store(db, &self.allocations).get_u32_raw(db.rec, db.pos);
        let src_rec = keys::store(o_db, &self.allocations).get_u32_raw(o_db.rec, o_db.pos);
        if db.store_nr == o_db.store_nr && dest_rec != 0 && dest_rec == src_rec {
            return;
        }
        vector::clear_vector(db, &mut self.allocations);
        self.vector_add(db, o_db, known);
    }

    /// @PLN157 § V-m — the slot of one fused scalar append: `vector_append` claims it (and
    /// grows on the ladder), the caller writes it, and the length bump lands on the same
    /// resolved store.  `None` for a null or absent vector, which the four-op path also
    /// left untouched (`OpNewRecord` answered the null slot and every op after it declined).
    #[inline]
    fn append_slot(&mut self, db: &DbRef, size: u32) -> Option<DbRef> {
        let slot = vector::vector_append(db, size, &mut self.allocations);
        (slot.rec != 0).then_some(slot)
    }

    /// The length bump of [`Self::append_slot`]: `slot.rec` IS the vector record.
    #[inline]
    fn append_done(store: &mut Store, slot: &DbRef) {
        let len = store.get_u32_raw(slot.rec, 4);
        store.set_u32_raw(slot.rec, 4, len + 1);
    }

    pub fn append_i64(&mut self, db: &DbRef, v: i64) {
        if let Some(slot) = self.append_slot(db, 8) {
            let store = self.store_mut(&slot);
            store.set_int(slot.rec, slot.pos, v);
            Self::append_done(store, &slot);
        }
    }

    pub fn append_i32(&mut self, db: &DbRef, v: i32) {
        if let Some(slot) = self.append_slot(db, 4) {
            let store = self.store_mut(&slot);
            store.set_i32_raw(slot.rec, slot.pos, v);
            Self::append_done(store, &slot);
        }
    }

    pub fn append_f64(&mut self, db: &DbRef, v: f64) {
        if let Some(slot) = self.append_slot(db, 8) {
            let store = self.store_mut(&slot);
            store.set_float(slot.rec, slot.pos, v);
            Self::append_done(store, &slot);
        }
    }

    pub fn append_f32(&mut self, db: &DbRef, v: f32) {
        if let Some(slot) = self.append_slot(db, 4) {
            let store = self.store_mut(&slot);
            store.set_single(slot.rec, slot.pos, v);
            Self::append_done(store, &slot);
        }
    }

    /// A boolean or a plain enum: one byte, no sentinel (`OpSetBoolean` / `OpSetEnum`'s
    /// `set_byte(…, 0, v)`).
    pub fn append_byte(&mut self, db: &DbRef, v: i32) {
        if let Some(slot) = self.append_slot(db, 1) {
            let store = self.store_mut(&slot);
            store.set_byte(slot.rec, slot.pos, 0, v);
            Self::append_done(store, &slot);
        }
    }

    pub fn append_u32(&mut self, db: &DbRef, v: u32) {
        if let Some(slot) = self.append_slot(db, 4) {
            let store = self.store_mut(&slot);
            store.set_u32_raw(slot.rec, slot.pos, v);
            Self::append_done(store, &slot);
        }
    }

    pub fn vector_add(&mut self, db: &DbRef, o_db: &DbRef, known: u16) {
        // `LOFT_TRACE_VADD=1` prints one line per vector concat/append-copy
        // with the resolved stride — the instrument that settled the nested
        // `a += b` stride bug (rows strode by the 4-byte field-slot size
        // instead of the read path's clamped content size).
        if vadd_trace_enabled() {
            crate::loft_eprintln!(
                "[vadd] db={db:?} o_db={o_db:?} known={} size={} linked={} parts={:?}",
                known,
                self.size(known),
                self.is_linked(known),
                self.types.get(known as usize).map(|t| t.parts.clone())
            );
        }
        let o_length = vector::length_vector(o_db, &self.allocations);
        if o_length == 0 {
            // The other vector has no data
            return;
        }
        // @PLN90 phase 1 — make the copy visible. A non-empty source means `vector_add`
        // deep-copies `o_length` elements into the destination store: a real structure
        // copy. `LOFT_COPY_DUMP` reports it with the element count (the runtime size — the
        // "hundreds of MB just to be sure" the user cannot see today). No source line here:
        // `Stores` has no `State`; the source location is the compile-time decision's job
        // (COPY_DIAGNOSTICS.md phase 2).
        if keys::copy_dump_enabled() {
            crate::loft_eprintln!("[copy] vector-append elements={o_length}  tp={known}");
        }
        // @P376 — when `known` is "linked" (multiple containers share this
        // content type), `vector<known>` was promoted to `Array(known)` in
        // `Stores::finish_type`: storage is a u32 rec-id per element pointing
        // to a separate record of type `known`, not an inline `size(known)`
        // payload.  The inline byte-copy below would read garbage past the
        // 4-byte source pointer slots; route Array→Array appends through a
        // dedicated path that mirrors `out += [elem]` (claim fresh element
        // record in dest store, byte-copy from source's element record,
        // deep-copy nested heap, append u32 rec-id to dest's array).
        if self.is_linked(known) {
            self.vector_add_array(db, o_db, known, o_length);
            return;
        }
        // Snapshot the source record number BEFORE any resize: if `db` and `o_db` share the
        // same backing store the resize inside `vector_append` / `vector_set_size` may
        // reallocate the vector and invalidate `o_rec`.  Reading it after the resize would
        // reference freed memory, silently producing corrupt data.
        let o_rec = keys::store(o_db, &self.allocations).get_u32_raw(o_db.rec, o_db.pos);
        // Element stride — the element type's own size, for every row shape.
        // A VECTOR-typed row (`vector<vector<T>>`) is the inner vector's 4-byte
        // handle, which is exactly `size(known)` now that the element type
        // registers as a real `vector<inner>` (`Data::vector_element_type`)
        // rather than the level-collapsed inner scalar.  The former
        // `size(content(known)).max(4)` reproduced the READ path's own
        // mis-derivation; both now read this one fact.
        let size = u32::from(self.size(known));
        // If source and destination share the same backing vector record, copy source elements
        // to a local buffer first so the resize cannot invalidate the source pointer.
        let same_vec = db.store_nr == o_db.store_nr && o_rec != 0 && {
            let dest_rec = keys::store(db, &self.allocations).get_u32_raw(db.rec, db.pos);
            dest_rec == o_rec
        };
        let snapshot: Vec<u8> = if same_vec {
            let store = keys::store(o_db, &self.allocations);
            let byte_len = o_length as usize * size as usize;
            (0..byte_len)
                .map(|i| *store.addr::<u8>(o_rec, 8 + i as u32))
                .collect()
        } else {
            Vec::new()
        };
        let new_db = vector::vector_append(db, size, &mut self.allocations);
        let append_pos = new_db.pos;
        // Claim more than 1 record if needed for the actual copy.
        self.vector_set_size(db, o_length, size);
        // `vector_set_size` may have relocated the destination record.
        // `new_db.rec` captured from `vector_append` is stale after relocation;
        // re-read the current rec from the field slot (which `vector_set_size`
        // keeps up to date) before we use it for the byte copy.  Element
        // offset (`append_pos`) is layout-stable across relocation.
        let dest_rec = keys::store(db, &self.allocations).get_u32_raw(db.rec, db.pos);
        let new_db = DbRef {
            store_nr: db.store_nr,
            rec: dest_rec,
            pos: append_pos,
        };
        if same_vec {
            // Write from the pre-resize snapshot; `new_db.rec` is already the correct
            // (possibly reallocated) destination record after `vector_set_size`.
            let store = keys::mut_store(db, &mut self.allocations);
            for (i, &byte) in snapshot.iter().enumerate() {
                *store.addr_mut::<u8>(new_db.rec, new_db.pos + i as u32) = byte;
            }
        } else if db.store_nr == o_db.store_nr {
            // Re-read o_rec after resize in case it moved (non-self-append same-store case).
            let o_rec = keys::store(o_db, &self.allocations).get_u32_raw(o_db.rec, o_db.pos);
            keys::mut_store(db, &mut self.allocations).copy_block(
                o_rec,
                8,
                new_db.rec,
                new_db.pos as isize,
                o_length as isize * size as isize,
            );
        } else {
            // @PLAN53 cluster 3: two different data structures in distinct stores —
            // copy via the sound disjoint-borrow helper instead of an aliasing reborrow.
            self.copy_block_cross_store(
                o_db.store_nr,
                o_rec,
                8,
                db.store_nr,
                new_db.rec,
                new_db.pos as isize,
                o_length as isize * size as isize,
            );
        }
        // After the raw byte copy, slot indices for text and sub-structure fields in each
        // appended element still point into the source store.  Deep-copy those claims so
        // that the destination owns independent copies and is not affected when the source
        // vector is freed.
        for i in 0..o_length {
            self.copy_claims(
                &DbRef {
                    store_nr: o_db.store_nr,
                    rec: o_rec,
                    pos: 8 + size * i,
                },
                &DbRef {
                    store_nr: db.store_nr,
                    rec: new_db.rec,
                    pos: new_db.pos + size * i,
                },
                known,
            );
            // LOFT_WATCH_STORE — flag the element if this append left a garbage text-ptr.
            self.watch_oob_text(
                &DbRef {
                    store_nr: db.store_nr,
                    rec: new_db.rec,
                    pos: new_db.pos + size * i,
                },
                known,
                Some(&DbRef {
                    store_nr: o_db.store_nr,
                    rec: o_rec,
                    pos: 8 + size * i,
                }),
                "vector_add",
            );
        }
    }

    /// @P376 — Array(content) → Array(content) append.  When the content
    /// type is "linked" the vector storage is a u32-per-element rec-id
    /// table pointing to separately-claimed element records (see
    /// `Stores::finish_type` for the Vector→Array promotion).  Mirror what
    /// the IR emits for `out += [elem]`: claim a fresh element record in
    /// the destination's store, byte-copy the source record's payload,
    /// deep-copy nested heap, then push the new rec-id into the dest's
    /// array via `vector_append` (size=4) + `vector_finish`.
    fn vector_add_array(&mut self, db: &DbRef, o_db: &DbRef, known: u16, o_length: u32) {
        let elem_size = u32::from(self.size(known));
        // Source array record (u32 rec-id per slot, header at offset 0/4).
        let o_rec = keys::store(o_db, &self.allocations).get_u32_raw(o_db.rec, o_db.pos);
        if o_rec == 0 {
            return;
        }
        // Words to claim per element record: matches `record_new`'s Array
        // arm — 1 header word + ceil(content_size / 8) data words.
        let elem_words = 1 + elem_size.div_ceil(8);
        for i in 0..o_length {
            let src_rec = keys::store(o_db, &self.allocations).get_u32_raw(o_rec, 8 + 4 * i);
            // Append a slot to the destination array (4-byte rec-id stride);
            // `vector_append` allocates / resizes the dest vec_rec as needed.
            let slot = vector::vector_append(db, 4, &mut self.allocations);
            if src_rec == 0 {
                // Preserve null elements as null slots in the destination.
                self.store_mut(db).set_u32_raw(slot.rec, slot.pos, 0);
            } else {
                // Claim a fresh element record in the destination's store and
                // copy the source record's data + nested heap into it.
                let new_rec = self.allocations[db.store_nr as usize].claim(elem_words);
                if db.store_nr == o_db.store_nr {
                    self.store_mut(db)
                        .copy_block(src_rec, 8, new_rec, 8, elem_size as isize);
                } else {
                    // @PLAN53 cluster 3: sound disjoint-borrow cross-store copy.
                    self.copy_block_cross_store(
                        o_db.store_nr,
                        src_rec,
                        8,
                        db.store_nr,
                        new_rec,
                        8,
                        elem_size as isize,
                    );
                }
                self.copy_claims(
                    &DbRef {
                        store_nr: o_db.store_nr,
                        rec: src_rec,
                        pos: 8,
                    },
                    &DbRef {
                        store_nr: db.store_nr,
                        rec: new_rec,
                        pos: 8,
                    },
                    known,
                );
                self.store_mut(db).set_u32_raw(slot.rec, slot.pos, new_rec);
                // LOFT_WATCH_STORE — flag this element if the append left a garbage text-ptr.
                self.watch_oob_text(
                    &DbRef {
                        store_nr: db.store_nr,
                        rec: new_rec,
                        pos: 8,
                    },
                    known,
                    Some(&DbRef {
                        store_nr: o_db.store_nr,
                        rec: src_rec,
                        pos: 8,
                    }),
                    "vector_add_array",
                );
            }
            vector::vector_finish(db, &mut self.allocations);
        }
    }

    pub fn vector_set_size(&mut self, db: &DbRef, adding: u32, size: u32) {
        let store = keys::mut_store(db, &mut self.allocations);
        let mut vec_rec = store.get_u32_raw(db.rec, db.pos);
        let length = store.get_u32_raw(vec_rec, 4);
        if adding > 1 {
            let new_vec = store.resize(vec_rec, ((length + adding) * size + 15) / 8);
            if new_vec != vec_rec {
                store.set_u32_raw(db.rec, db.pos, new_vec);
                // track the relocation so the length write below lands
                // in the current record instead of the freed one.
                vec_rec = new_vec;
            }
        }
        store.set_u32_raw(vec_rec, 4, length + adding);
    }

    /// Walk a [`crate::json::Parsed`] tree into the record at
    /// `to`, dispatching on the target type's [`Parts`] variant.
    /// Returns `Ok(())` on success, `Err(WalkErr)` with byte offset
    /// + dotted path on a shape/type mismatch.
    ///
    /// `at` is the byte offset in the source text where this value
    /// was parsed (for struct fields, the field's key offset; for
    /// array elements / top-level, 0 — `Parsed::Array` doesn't carry
    /// per-element offsets).  Used to populate the `at` field of
    /// `WalkErr` when a leaf hits a type mismatch, so users see the
    /// real "line N:M path:X" instead of "line 1:1 path:X".
    ///
    /// Schema-driven counterpart to the parser-side
    /// [`crate::json::parse_with(text, Dialect::Lenient)`] — the
    /// parser stays schema-free; all type dispatch lives here.
    /// Together they form the only `text → struct` path in the
    /// crate; the legacy hand-rolled scanner was removed when
    /// every Parts arm gained walker coverage.
    #[allow(clippy::ptr_arg, clippy::too_many_arguments)] // path push/pop; arg-count is intrinsic to the dispatch
    pub(super) fn walk_parsed_into(
        &mut self,
        parsed: &crate::json::Parsed,
        tp: u16,
        rec_tp: u16,
        field: u16,
        to: &DbRef,
        path: &mut Vec<String>,
        at: usize,
    ) -> Result<(), WalkErr> {
        // `null` at any target position resets to the target's default — mirrors the
        // legacy scanner's first-line behaviour and keeps round-tripping correct.  Which
        // default is the FIELD's question, not the type's, and
        // [`Self::write_absent_value`] is where that question is answered.
        if matches!(parsed, crate::json::Parsed::Null) {
            self.write_absent_value(tp, rec_tp, field, to);
            return Ok(());
        }
        let mismatch = || WalkErr {
            at,
            path: path.clone(),
        };
        match self.types[tp as usize].parts.clone() {
            Parts::Base => self.walk_primitive_into(parsed, tp, to, path, at),
            Parts::Sorted(c, _)
            | Parts::Vector(c)
            | Parts::Array(c)
            | Parts::Ordered(c, _)
            | Parts::Hash(c, _)
            | Parts::Radix(c, _)
            | Parts::Trie(c, _)
            | Parts::Index(c, _, _) => {
                let crate::json::Parsed::Array(items) = parsed else {
                    return Err(mismatch());
                };
                // @P357: an EMPTY JSON array must still zero the collection
                // header.  The per-item loop below is the ONLY thing that
                // initialises the vector/array field (the first `record_new`
                // writes its header) — so with zero items the field keeps
                // whatever bytes the recycled store record held, reading back
                // as a phantom non-zero length (e.g. `json_parse("[]").item(0)`
                // returning a garbage object, and `len` reporting 8 after a
                // run of earlier parses populated then freed that block).  The
                // `Parts::Null` arm above already calls `set_default_value` for
                // exactly this reason; do the same when the array is empty.
                if items.is_empty() {
                    // @P373: write the default to the COLLECTION FIELD's slot,
                    // not to `to` — which for a struct field is the struct base
                    // (field 0), so `set_default_value(tp, to)` zeroed the FIRST
                    // field's bytes and corrupted it (e.g. `{"name":"b","items":[]}`
                    // read `name` back as "").  A collection field reaches here
                    // via walk_parsed_struct's `else` branch with `to` = base +
                    // `field` = its index, exactly as the non-empty path below
                    // feeds `record_new(to, rec_tp, field)`.  `field_ref` maps
                    // (to, rec_tp, field) → the field slot, and returns `*to`
                    // unchanged when `field == u16::MAX` (top-level `json_parse("[]")`,
                    // the @P357 case), so that path is preserved.
                    let slot = self.field_ref(to, rec_tp, field);
                    self.set_default_value(tp, &slot);
                }
                for (idx, item) in items.iter().enumerate() {
                    path.push(format!("[{idx}]"));
                    let res = self.record_new(to, rec_tp, field);
                    self.walk_parsed_into(item, c, c, u16::MAX, &res, path, at)?;
                    self.record_finish(to, &res, rec_tp, field);
                    path.pop();
                }
                Ok(())
            }
            Parts::Struct(object) | Parts::EnumValue(_, object) => {
                // A type-tagged constructor `Type{…}` unwraps to its body for a
                // struct target — the tag is informational (the schema fixes the
                // type).  A plain object reads as fields directly, so old
                // un-tagged dumps still load: this is the disambiguation a single
                // `Constructor` node buys us over the collapsed-object shape.
                let body = match parsed {
                    crate::json::Parsed::Constructor(_tag, _, inner) => inner.as_ref(),
                    other => other,
                };
                self.walk_parsed_struct(body, tp, to, &object, path, at)
            }
            Parts::Enum(fields) => {
                // Accepted shapes:
                //   - `Str("Tag")` / `Ident("Tag")` / `Ident("Enum.Tag")` — unit
                //   - `Constructor("Tag", _, body)` — variant with payload (the
                //     `Tag { … }` shape from the Lenient parser)
                //   - `Object([("Tag", _, body)])` — the legacy collapsed shape,
                //     still accepted so older `{"Tag":{…}}` dumps keep loading
                // @PLN25 E2 (A4): a synthetic `__nullable<S>` enum (a `vector<S>`
                // element rewritten so it can be null — see `Data::nullable_enum_for`)
                // deserialises a bare JSON object as the PRESENT case: the object
                // holds S's fields directly, not a variant tag.  Route it to the
                // `Some` variant with the whole object as the payload.  `null` is
                // already handled above (Parsed::Null → `set_default_value` → the
                // absent disc 0).  Without this a present struct object falls to the
                // `_` mismatch arm and the `?` in the array loop aborts the whole
                // parse, dropping every present element (`[{…},null]` → len 0).
                let synth_nullable = self.types[tp as usize].name.starts_with("__nullable<");
                let (name, payload) = match parsed {
                    crate::json::Parsed::Object(_) | crate::json::Parsed::Constructor(..)
                        if synth_nullable =>
                    {
                        ("Some", Some(parsed))
                    }
                    crate::json::Parsed::Str(s) | crate::json::Parsed::Ident(s) => {
                        (s.as_str(), None)
                    }
                    crate::json::Parsed::Constructor(tag, _, body) => {
                        (tag.as_str(), Some(body.as_ref()))
                    }
                    crate::json::Parsed::Object(entries) if entries.len() == 1 => {
                        (entries[0].0.as_str(), Some(&entries[0].2))
                    }
                    _ => return Err(mismatch()),
                };
                // Accept a *qualified* tag (`Enum.Variant`): match on the last
                // segment.  The schema already fixes the enum type, so the prefix
                // is informational — the lenient dialect does not validate it.
                let name = name.rsplit('.').next().unwrap_or(name);
                let mut enum_tp = u16::MAX;
                let val = if name == "null" {
                    0
                } else {
                    // No-match degrades to the null sentinel (0), NOT variant 1:
                    // an unknown tag (e.g. a variant removed by a schema edit)
                    // must read back as null, never silently as a wrong variant.
                    // Preserve-as-much-as-possible across data-structure changes.
                    let mut v = 0;
                    for (f_nr, f) in fields.iter().enumerate() {
                        if f.1 == name {
                            v = f_nr as i32 + 1;
                            enum_tp = f.0;
                            break;
                        }
                    }
                    v
                };
                self.store_mut(to).set_byte(to.rec, to.pos, 0, val);
                // @PLN25 single-payload: a synth `__nullable<S>` `Some` variant holds the
                // object in its inline `payload` dense-`S` field — recurse the body into
                // the payload sub-record (`to.pos + payload offset`), NOT the `Some` variant
                // (whose direct fields are {enum, payload}, not S's).
                if synth_nullable
                    && name == "Some"
                    && enum_tp != u16::MAX
                    && let Some(body) = payload
                {
                    let pinfo = if let Parts::Struct(sf) | Parts::EnumValue(_, sf) =
                        &self.types[enum_tp as usize].parts
                    {
                        sf.iter()
                            .find(|f| f.name == "payload")
                            .map(|f| (f.content, f.position))
                    } else {
                        None
                    };
                    if let Some((pcontent, ppos)) = pinfo {
                        let payload_to = DbRef {
                            store_nr: to.store_nr,
                            rec: to.rec,
                            pos: to.pos + u32::from(ppos),
                        };
                        return self.walk_parsed_into(
                            body,
                            pcontent,
                            rec_tp,
                            field,
                            &payload_to,
                            path,
                            at,
                        );
                    }
                    return Ok(());
                }
                // Variant-with-payload (hand-written struct-enum): recurse so the payload
                // fields land in the same slot as the discriminant byte.
                if let Some(body) = payload
                    && enum_tp != u16::MAX
                    && self.types[enum_tp as usize].size > 1
                {
                    return self.walk_parsed_into(body, enum_tp, rec_tp, field, to, path, at);
                }
                Ok(())
            }
            // The five narrow-integer encodings (@FR-L-Narrow) live in `write_narrow_value`,
            // so this walker and the `JsonValue` one spell a narrow slot's bytes the same way.
            Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::ShortRaw(_, _)
            | Parts::Int(_, _)
            | Parts::IntRaw(_, _) => {
                let Some(n) = parsed.as_i64() else {
                    return Err(mismatch());
                };
                self.write_narrow_value(tp, n, to);
                Ok(())
            }
            // Plan-06 phase 4d.C step 2: stored DbRef pointer has no
            // JSON-representable form — closures don't survive a JSON
            // round-trip.  Surface as a clean error.
            Parts::DbRef => Err(mismatch()),
            // P213: child-record pointer (closures) — same JSON
            // restriction as DbRef.
            Parts::ChildRec(_) => Err(mismatch()),
        }
    }

    /// Schema-driven struct fill from a [`crate::json::Parsed::Object`].
    /// Matches fields by name, recurses into the walker for each
    /// value (passing the field's key byte offset as the recursion's
    /// `at` hint so a leaf type-mismatch reports the field's position),
    /// default-fills any unmentioned field (mirroring the legacy
    /// scanner's "missing field → default" behaviour).
    #[allow(clippy::ptr_arg)] // path needs push/pop, slice not enough
    fn walk_parsed_struct(
        &mut self,
        parsed: &crate::json::Parsed,
        tp: u16,
        to: &DbRef,
        object: &[Field],
        path: &mut Vec<String>,
        at: usize,
    ) -> Result<(), WalkErr> {
        let crate::json::Parsed::Object(entries) = parsed else {
            return Err(WalkErr {
                at,
                path: path.clone(),
            });
        };
        let fld = if to.rec == 0 { 0 } else { to.pos };
        let rec = if to.rec == 0 {
            let size = self.types[tp as usize].size;
            self.store_mut(to).claim(u32::from(size).div_ceil(8))
        } else {
            to.rec
        };
        let mut found_fields: HashSet<&str> = HashSet::new();
        for (name, key_at, value) in entries {
            let mut matched = false;
            for (f_nr, f) in object.iter().enumerate() {
                if f.name == *name {
                    matched = true;
                    path.push(name.clone());
                    let res = if self.content(f.content) == u16::MAX {
                        let slot = DbRef {
                            store_nr: to.store_nr,
                            rec,
                            pos: fld + u32::from(f.position),
                        };
                        self.walk_parsed_into(
                            value,
                            f.content,
                            tp,
                            f_nr as u16,
                            &slot,
                            path,
                            *key_at,
                        )
                    } else {
                        self.walk_parsed_into(value, f.content, tp, f_nr as u16, to, path, *key_at)
                    };
                    res?;
                    path.pop();
                    break;
                }
            }
            if !matched {
                // @P366: an unknown JSON key has no matching struct field.
                // Skip it (lenient-ignore) rather than aborting the element /
                // the whole array.  This matches the dynamic `JsonValue` walker
                // (`populate_struct_from_jsonvalue`), which only visits declared
                // fields and silently tolerates extra keys.  Previously this
                // returned a `WalkErr`, and the array loop's `?` aborted the
                // entire parse → `text as vector<Struct>` returned a silently
                // empty vector (`len == 0`) whenever the JSON carried any field
                // the struct did not declare.
                continue;
            }
            found_fields.insert(name.as_str());
        }
        for (f_nr, f) in object.iter().enumerate() {
            if (f.other_indexes.is_empty() || f.other_indexes[0] != u16::MAX)
                && !found_fields.contains(f.name.as_str())
                && f.name != "enum"
            {
                let slot = DbRef {
                    store_nr: to.store_nr,
                    rec,
                    pos: fld + u32::from(f.position),
                };
                // A key the JSON simply omits is the same question as one written
                // `null`, and gets the same answer (loft#870).
                self.write_absent_value(f.content, tp, f_nr as u16, &slot);
            }
        }
        Ok(())
    }

    /// Schema-driven primitive write.  `tp` is one of the
    /// low-numbered base-type IDs (0 = integer, 1 = long, 2 = single,
    /// 3 = float, 4 = bool, 5 = text, 6 = character).
    /// `at` is the byte offset where this primitive was parsed —
    /// reported on the [`WalkErr`] of any type mismatch so users
    /// see the real position instead of byte 0.
    #[allow(clippy::ptr_arg)] // path needs push/pop, slice not enough
    fn walk_primitive_into(
        &mut self,
        parsed: &crate::json::Parsed,
        tp: u16,
        to: &DbRef,
        path: &mut Vec<String>,
        at: usize,
    ) -> Result<(), WalkErr> {
        let mismatch = || WalkErr {
            at,
            path: path.clone(),
        };
        match tp {
            0 => {
                let Some(n) = parsed.as_i64() else {
                    return Err(mismatch());
                };
                self.store_mut(to).set_int(to.rec, to.pos, n);
                Ok(())
            }
            6 => {
                // `character` is FOUR bytes.  It shared type 0's arm — an 8-byte
                // `set_int` — because the doc comment above called type 6 "Reference";
                // the extra word ran into whatever field the layout put next, so
                // `{"tail":5,"c3":99,"c2":98,"c1":97}` into
                // `T { c1, c2, c3: character, tail: integer }` answered `a`, NUL, NUL, 5.
                // Only the LAST character written survived, which is why document order
                // decided whether the corruption showed. Same spill loft#1014 fixed in
                // `emit_typed_null`, at a site that pre-dated the shared width rule.
                //
                // On the wire a character is the one-character STRING `to_json` writes;
                // a NUMBER is also accepted as its codepoint, which is the only form
                // this arm used to take.
                let cp = if let crate::json::Parsed::Str(t) = parsed {
                    let mut cs = t.chars();
                    match (cs.next(), cs.next()) {
                        (Some(c), None) => u32::from(c),
                        _ => return Err(mismatch()),
                    }
                } else {
                    let Some(n) = parsed.as_i64() else {
                        return Err(mismatch());
                    };
                    u32::try_from(n).map_err(|_| mismatch())?
                };
                #[allow(clippy::cast_possible_wrap)]
                self.store_mut(to).set_i32_raw(to.rec, to.pos, cp as i32);
                Ok(())
            }
            1 => {
                let Some(n) = parsed.as_i64() else {
                    return Err(mismatch());
                };
                self.store_mut(to).set_long(to.rec, to.pos, n);
                Ok(())
            }
            2 => {
                let Some(n) = parsed.as_f64() else {
                    return Err(mismatch());
                };
                #[allow(clippy::cast_possible_truncation)]
                self.store_mut(to).set_single(to.rec, to.pos, n as f32);
                Ok(())
            }
            3 => {
                let Some(n) = parsed.as_f64() else {
                    return Err(mismatch());
                };
                self.store_mut(to).set_float(to.rec, to.pos, n);
                Ok(())
            }
            4 => {
                let crate::json::Parsed::Bool(b) = parsed else {
                    return Err(mismatch());
                };
                self.store_mut(to)
                    .set_byte(to.rec, to.pos, 0, i32::from(*b));
                Ok(())
            }
            5 => {
                // Text accepts only a quoted string — bare
                // identifiers (`Parsed::Ident`) are NOT promoted to
                // text, matching the legacy `match_text` behaviour.
                let crate::json::Parsed::Str(s) = parsed else {
                    return Err(mismatch());
                };
                let text_pos = self.store_mut(to).set_str(s);
                self.store_mut(to).set_u32_raw(to.rec, to.pos, text_pos);
                Ok(())
            }
            _ => Err(mismatch()),
        }
    }

    /// Is the field at `field` of struct `rec_tp` DECLARED nullable?
    ///
    /// Answers `true` — today's behaviour, the null sentinel — whenever the question
    /// does not apply: a top-level or array-element target carries `field == u16::MAX`,
    /// and a non-struct `rec_tp` has no fields to consult. Only a field that a
    /// declaration site actually spelled non-null gets the other answer.
    fn field_declared_nullable(&self, rec_tp: u16, field: u16) -> bool {
        if rec_tp == u16::MAX || field == u16::MAX {
            return true;
        }
        match &self.types[rec_tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                fields.get(field as usize).is_none_or(|f| f.nullable)
            }
            _ => true,
        }
    }

    /// loft#876 — write a field's DECLARED default into `slot`, answering whether there
    /// was one.  `false` means the caller writes the type's absent value as before.
    ///
    /// The value goes in through [`Self::walk_parsed_into`] — the same writer the cast
    /// uses for a key the document DID carry — so a declared default lands exactly as if
    /// the JSON had spelled it, and every field encoding (narrow ranged ints, text
    /// interning, the `Parts` dispatch) is handled in one place rather than restated
    /// here.  A default whose literal does not fit the field's type writes nothing and
    /// answers `false`, so the absent value stays what it was.
    fn write_declared_default(&mut self, rec_tp: u16, field: u16, slot: &DbRef) -> bool {
        // A top-level or array-element target carries `field == u16::MAX` (and a bare value
        // `rec_tp == u16::MAX`): neither names a declared field, and the second is not an
        // index into `types` at all.  The companion of [`Self::field_declared_nullable`],
        // which asks the same question of the same two sentinels; the field is read in
        // place rather than cloned out, because this runs on every absent field of every
        // parsed record (@PLN157 § V-e).
        if rec_tp == u16::MAX || field == u16::MAX {
            return false;
        }
        let (c, content) = {
            let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
                &self.types[rec_tp as usize].parts
            else {
                return false;
            };
            let Some(f) = fields.get(field as usize) else {
                return false;
            };
            let Some(c) = f.default.clone() else {
                return false;
            };
            (c, f.content)
        };
        let parsed = match c {
            // A boolean field is content type 4, whose walker arm accepts only
            // `Parsed::Bool`; every other numeric field reads the integer spelling.
            crate::keys::Content::Long(n) if content == 4 => crate::json::Parsed::Bool(n != 0),
            crate::keys::Content::Long(n) => crate::json::Parsed::Int(n),
            crate::keys::Content::Float(v) => crate::json::Parsed::Number(v),
            crate::keys::Content::Single(v) => crate::json::Parsed::Number(f64::from(v)),
            crate::keys::Content::Str(s) => crate::json::Parsed::Str(s.str().to_string()),
        };
        let mut path = Vec::new();
        self.walk_parsed_into(&parsed, content, rec_tp, field, slot, &mut path, 0)
            .is_ok()
    }

    /// What a slot holds when the document hands it no usable value — an omitted key, an
    /// explicit `null`, or a value the slot's type cannot hold.  All three are the same
    /// question, and this is the ONE place that answers it.
    ///
    /// Enforces @FR-L-Null: a nullable field has the SAME bytes as its not-null form, so
    /// "what does an absent field hold?" is the DECLARATION's question, not the type's.
    ///
    /// A field that DECLARES a default answers with it (loft#876).  Otherwise the type's
    /// absent value stands, which per [`formal/layout.md`] `(L-Null)` is the null sentinel
    /// inside the slot's own bytes when the slot is nullable, and the type's zero when it
    /// is not — a null written into a non-null slot is a value `(N-Decl)` says it cannot
    /// hold (loft#870).
    ///
    /// `tp` is the slot's content type; `rec_tp` and `field` name the declaration to
    /// consult.  A target that is not a declared field — a top-level cast target, a
    /// collection element — carries `field == u16::MAX` and takes the nullable answer.
    ///
    /// ⚠ Both JSON walkers must ask HERE rather than answer for themselves.  Four things
    /// have to agree for an absent field to read back correctly — the declared default, the
    /// not-null integer sentinel, the one-byte width's absence code, and the boolean's —
    /// and a second implementation gets to disagree on each of them independently, with no
    /// error at any of the four.
    ///
    /// [`formal/layout.md`]: ../../../doc/claude/formal/layout.md
    pub fn write_absent_value(&mut self, tp: u16, rec_tp: u16, field: u16, slot: &DbRef) {
        if self.write_declared_default(rec_tp, field, slot) {
            return;
        }
        let nullable = self.field_declared_nullable(rec_tp, field);
        self.set_default_value_nullable(tp, nullable, Absent::Final, slot);
    }

    /// Write `n` into a NARROW integer slot through the encoding its `Parts` declares,
    /// answering whether `tp` was a narrow integer at all — `false` lets a caller fall
    /// through to its own dispatch.
    ///
    /// Enforces @FR-L-Null and @FR-L-Narrow-Enc for the narrow widths — the write twin of
    /// `narrow_is_null`.
    ///
    /// The five encodings disagree about where the null code sits: `Byte` and `Short`
    /// store `value - min + 1` and reserve the raw code for absence, `ShortRaw` stores
    /// `value - min` directly, `Int` is a raw `i32` whose null is `i32::MIN`, and
    /// `IntRaw` is a raw `u32` whose null is `u32::MAX`.  That
    /// makes the encoding part of the slot's LAYOUT, so it lives at one address.  A second
    /// writer re-deriving it does not fail loudly: it writes plausible bytes that decode to
    /// the wrong value, or to absence, for present input.
    pub fn write_narrow_value(&mut self, tp: u16, n: i64, slot: &DbRef) -> bool {
        enum Enc {
            Byte(i32),
            Short(i32),
            ShortRaw(i32),
            Int,
            IntRaw,
        }
        if tp == u16::MAX || tp <= 6 {
            return false;
        }
        // Copy the encoding out before taking the store's mutable borrow.  Every narrow
        // `Parts` carries just `(min, nullable)`, so this clones no field list.
        let enc = match self.types[tp as usize].parts {
            Parts::Byte(from, _) => Enc::Byte(from),
            Parts::Short(from, _) => Enc::Short(from),
            Parts::ShortRaw(from, _) => Enc::ShortRaw(from),
            Parts::Int(_, _) => Enc::Int,
            Parts::IntRaw(_, _) => Enc::IntRaw,
            _ => return false,
        };
        #[allow(clippy::cast_possible_truncation)]
        let v = n as i32;
        let store = self.store_mut(slot);
        // The setters answer whether the value FIT the slot's range; out-of-range is
        // theirs to handle (they store the slot's default) and both walkers have always
        // let that stand.  The answer here is the narrower question the caller asked:
        // was this a narrow slot at all?
        let _fits = match enc {
            Enc::Byte(from) => store.set_byte(slot.rec, slot.pos, from, v),
            Enc::Short(from) => store.set_short(slot.rec, slot.pos, from, v),
            Enc::ShortRaw(from) => store.set_i16_raw(slot.rec, slot.pos, from, v),
            // `i64::MIN` is the wide null, and it must not truncate to a live `i32`.
            Enc::Int => {
                store.set_i32_raw(slot.rec, slot.pos, if n == i64::MIN { i32::MIN } else { v })
            }
            // The unsigned twin, whose absence is the top code — and whose value must
            // NOT go through `v`, an `i32`, or every value past 2147483647 truncates.
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Enc::IntRaw => store.set_u32_raw(
                slot.rec,
                slot.pos,
                if n == i64::MIN { u32::MAX } else { n as u32 },
            ),
        };
        true
    }

    /**
        Write default(null) values on all fields. This should normally only be done while debugging
        as all fields should be set anyway under correctly generated code.
        # Panics
        On inconsistent database definitions.
    */
    pub fn set_default_value(&mut self, tp: u16, rec: &DbRef) {
        self.set_default_value_nullable(tp, true, Absent::Prefill, rec);
    }

    /// [`Self::set_default_value`] for a record the caller is about to HAND OUT — a
    /// `text as Struct` fills its target this way before parsing, so every field the
    /// document does not reach keeps exactly what this writes.  A parse that fails
    /// outright reaches none of them, which is why the difference is not academic:
    /// `#errors` is the channel that says a parse failed, and the value must not
    /// double as that signal (loft#870, and its `text` half loft#875).
    pub fn set_final_default_value(&mut self, tp: u16, rec: &DbRef) {
        self.set_default_value_nullable(tp, true, Absent::Final, rec);
    }

    /// The absent value of a struct FIELD, which is not the same question as the
    /// absent value of its TYPE.
    ///
    /// `integer`(0), `long`(1), `single`(2) and `float`(3) spell absence with a
    /// SENTINEL and share one content type between their `T` and `T?` spellings, so
    /// a type-only answer has to pick one, and [`Self::set_default_value`] picks the
    /// sentinel. Writing that into a field declared non-null puts a null in a slot
    /// DN1 says cannot hold one: the reader answers `null`, the declared type says
    /// otherwise, and `redundant-coalesce` then advises deleting the guard that is
    /// doing the work (loft#870).
    ///
    /// Every other type either has no sentinel to choose (`text`, `boolean`,
    /// `character`, a collection header — all already zero) or carries its own
    /// nullability in its `Parts` (`Byte`/`Short`/`ShortRaw`/`Int`), which is why a
    /// ranged `u8` field was already right while a plain `integer` was not. So
    /// `nullable` changes exactly four arms and nothing else.
    ///
    /// # Panics
    /// On inconsistent database definitions.
    pub fn set_default_value_nullable(
        &mut self,
        tp: u16,
        nullable: bool,
        why: Absent,
        rec: &DbRef,
    ) {
        // @PLN25 — a forward-referenced field's content can still be u16::MAX here (its known_type
        // is not laid out yet — e.g. a `__nullable<S>` element of a forward-ref'd struct, gate-on
        // 371_p375_forward_ref_positions).  It has no per-type default to write, and zero-on-claim
        // already zeroed the record (0 = the correct default: a `null` discriminant for a nullable
        // field, or a zero scalar), so skip rather than OOB-index `self.types[tp]` below.
        if tp == u16::MAX {
            return;
        }
        // An ABSENT destination has no record to default — writing one indexes
        // `allocations[u16::MAX]` and takes the interpreter down. Both spellings of
        // absence count, exactly as on the read side (the null sentinel, and a record id
        // of 0 for a field that is not there). Sibling of the `tp == u16::MAX` guard
        // above: nothing to write INTO rather than nothing to write.
        if rec.is_null() || rec.rec == 0 {
            return;
        }
        if tp <= 6 {
            match tp {
                0 => {
                    let v = if nullable { i64::MIN } else { 0 };
                    self.store_mut(rec).set_int(rec.rec, rec.pos, v);
                }
                6 => {
                    // Content type 6 is a 4-byte u32-raw field (read via `get_u32_raw`,
                    // e.g. a `character` codepoint), NOT an 8-byte integer.  The old
                    // `set_int(i64::MIN)` wrote 8 bytes — its high 4 bytes spilled into
                    // the NEXT slot, which silently corrupts a tightly-sized record when
                    // the field is the LAST one (a trailing `character` in a single-payload
                    // `Some` payload overran the record and clobbered the adjacent free
                    // block's size header → `fl_size` negate-overflow).  `i64::MIN`'s low 4
                    // bytes are 0, so write 0 in exactly 4 bytes — same field value, no spill.
                    self.store_mut(rec).set_i32_raw(rec.rec, rec.pos, 0);
                }
                1 => {
                    let v = if nullable { i64::MIN } else { 0 };
                    self.store_mut(rec).set_long(rec.rec, rec.pos, v);
                }
                2 => {
                    let v = if nullable { f32::NAN } else { 0.0 };
                    self.store_mut(rec).set_single(rec.rec, rec.pos, v);
                }
                3 => {
                    let v = if nullable { f64::NAN } else { 0.0 };
                    self.store_mut(rec).set_float(rec.rec, rec.pos, v);
                }
                4 => {
                    // @PLN17 C73 — a boolean stores tri-state (0 / 1 / 255), and 255 IS
                    // its null.  Writing `false` for an absent NULLABLE boolean made it
                    // the one nullable base type whose absence read back as a VALUE; the
                    // integer, float and text arms beside it all write their sentinel.
                    let v = if nullable { 255 } else { 0 };
                    self.store_mut(rec).set_byte(rec.rec, rec.pos, 0, v);
                }
                5 => {
                    // A text handle of 0 reads back as `null` (`Store::get_str`), so a field
                    // declared plain `text` that KEEPS this value needs an interned empty
                    // string — the same value a struct literal writes for an omitted field
                    // (`OpSetText(rec, off, "")`), so a cast and a literal answer the same
                    // question the same way (loft#875).
                    //
                    // Only for `Absent::Final`. An empty string claims a word, and the
                    // prefill runs per RECORD on the allocation path: interning there cost
                    // +78 % wall and +91 % peak heap on 400 000 rows with three text fields,
                    // every one of them overwritten by the literal that followed.
                    let intern = !nullable && why == Absent::Final;
                    let h = if intern {
                        self.store_mut(rec).set_str("")
                    } else {
                        0
                    };
                    self.store_mut(rec).set_u32_raw(rec.rec, rec.pos, h);
                }
                _ => (),
            }
            return;
        }
        // Read the row's shape into a few numbers first, so the writes below borrow `self`
        // mutably without a clone of `Parts` — which carried the whole field list, on every
        // record allocation (@PLN157 § V-e: ~5 % of the `smooth` row).
        enum Shape {
            Enum,
            Byte(bool),
            Short(bool),
            ShortRaw(i32, bool),
            Int(bool),
            IntRaw(bool),
            Record(usize),
            DbRef,
            ChildRec,
            Base,
            Other,
        }
        let shape = match &self.types[tp as usize].parts {
            Parts::Enum(_) => Shape::Enum,
            Parts::Byte(_, null) => Shape::Byte(*null),
            Parts::Short(_, null) => Shape::Short(*null),
            Parts::ShortRaw(from, null) => Shape::ShortRaw(*from, *null),
            Parts::Int(_, null) => Shape::Int(*null),
            Parts::IntRaw(_, null) => Shape::IntRaw(*null),
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => Shape::Record(fields.len()),
            Parts::DbRef => Shape::DbRef,
            Parts::ChildRec(_) => Shape::ChildRec,
            Parts::Base => Shape::Base,
            _ => Shape::Other,
        };
        match shape {
            Shape::Enum => {
                self.store_mut(rec).set_byte(rec.rec, rec.pos, 0, 0);
            }
            Shape::Byte(null) => {
                self.store_mut(rec)
                    .set_byte(rec.rec, rec.pos, 0, if null { 255 } else { 0 });
            }
            Shape::Short(null) => {
                self.store_mut(rec)
                    .set_short(rec.rec, rec.pos, 0, if null { i32::MIN } else { 0 });
            }
            Shape::ShortRaw(from, null) => {
                self.store_mut(rec).set_i16_raw(
                    rec.rec,
                    rec.pos,
                    from,
                    if null { i32::MIN } else { from },
                );
            }
            Shape::Int(null) => {
                self.store_mut(rec)
                    .set_i32_raw(rec.rec, rec.pos, if null { i32::MIN } else { 0 });
            }
            // The unsigned twin's absence is the TOP code; `i32::MIN` is a legal value
            // of this encoding, so writing it here would store 2147483648 as the null.
            Shape::IntRaw(null) => {
                self.store_mut(rec)
                    .set_u32_raw(rec.rec, rec.pos, if null { u32::MAX } else { 0 });
            }
            Shape::Record(n_fields) => {
                // A record whose default is all zero bytes — no nullable field, no
                // declared default, no text, no variant tag, recursively — is one
                // `zero_range` rather than a walk writing each field's zero.
                if self.heap_facts(tp).1 {
                    let size = self.types[tp as usize].size;
                    if size != u16::MAX {
                        self.store_mut(rec)
                            .zero_range(rec.rec, rec.pos, u32::from(size));
                        return;
                    }
                }
                for f_nr in 0..n_fields {
                    let (position, content, nullable, is_type_tag) = {
                        let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
                            &self.types[tp as usize].parts
                        else {
                            unreachable!()
                        };
                        let f = &fields[f_nr];
                        (
                            f.position,
                            f.content,
                            f.nullable,
                            f.name == "type" && f.position == 0,
                        )
                    };
                    if is_type_tag {
                        self.store_mut(rec)
                            .set_short(rec.rec, rec.pos, 0, i32::from(tp));
                        continue;
                    }
                    let slot = DbRef {
                        store_nr: rec.store_nr,
                        rec: rec.rec,
                        pos: rec.pos + u32::from(position),
                    };
                    if why == Absent::Final && self.write_declared_default(tp, f_nr as u16, &slot) {
                        continue;
                    }
                    self.set_default_value_nullable(content, nullable, why, &slot);
                }
            }
            Shape::Other => {
                // Zero is the EMPTY collection, which is the right absent value for a
                // field that cannot be null — but it is the WRONG one for a field
                // declared `?`, and this arm was the only one in the match that did not
                // ask.  `DbRef::ABSENT_REC` is the third state (loft#917), the same code
                // `mark_collection_absent` writes and `vector::is_absent_collection`
                // reads, so an absent `vector<T>?` reads back `null` rather than `[]` —
                // which is the entire distinction the `?` was written to express.
                //
                // Only for `Absent::Final`, and the reason is the same one the text arm
                // gives: a PREFILL runs per record on the allocation path and is
                // overwritten by whatever fills the field, so marking it absent there
                // makes an empty `[]` the JSON walker DID write read back as null — the
                // handle it leaves at zero is the prefilled one.  A `Final` is a decision
                // that the field is not there.
                let v = if nullable && why == Absent::Final {
                    DbRef::ABSENT_REC
                } else {
                    0
                };
                self.store_mut(rec).set_u32_raw(rec.rec, rec.pos, v);
            }
            // Plan-06 phase 4d.C step 2: default for a 12-byte
            // stored DbRef = the null-DbRef bytes (store_nr=u16::MAX,
            // rec=0, pos=0 — three u32 zeros works since u16::MAX as
            // u32 is 0xFFFF, BUT for "not initialised" we want the
            // sentinel pattern; write all zeros and let the read
            // path treat rec=0 as null).
            Shape::DbRef => {
                let s = self.store_mut(rec);
                s.set_u32_raw(rec.rec, rec.pos, 0);
                s.set_u32_raw(rec.rec, rec.pos + 4, 0);
                s.set_u32_raw(rec.rec, rec.pos + 8, 0);
            }
            // P213: default child-record pointer = 0 (null sentinel /
            // empty / non-capturing).
            Shape::ChildRec => {
                self.store_mut(rec).set_u32_raw(rec.rec, rec.pos, 0);
            }
            Shape::Base => {
                panic!(
                    "not implemented default {:?}",
                    self.types[tp as usize].parts
                );
            }
        }
    }

    /// The record a four-byte child pointer field names, as a VALUE: `nullref` when the
    /// holder has no record or the pointer is zero (`DbRef::or_null`, @FR-L-Null) — a zero
    /// pointer is the slot's spelling of absence and does not leave the slot.
    #[must_use]
    pub fn get_ref(&self, db: &DbRef, fld: u32) -> DbRef {
        if db.rec == 0 {
            return DbRef::NULL;
        }
        let store = self.store(db);
        let res = store.get_u32_raw(db.rec, db.pos + fld);
        DbRef {
            store_nr: db.store_nr,
            rec: res,
            pos: 8,
        }
        .or_null()
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn get_field(db: &DbRef, fld: u32) -> DbRef {
        DbRef {
            store_nr: db.store_nr,
            rec: db.rec,
            pos: db.pos + fld,
        }
    }

    pub fn copy_block(&mut self, from: &DbRef, to: &DbRef, len: u32) {
        unsafe {
            std::ptr::copy(
                self.store(from)
                    .ptr
                    .offset(from.rec as isize * 8 + from.pos as isize),
                self.store_mut(to)
                    .ptr
                    .offset(to.rec as isize * 8 + to.pos as isize),
                len as usize,
            );
        }
        // @PLN154 — this is the return-value slide, the one untyped byte move on the
        // interpreter stack, and the tags have to travel with the bytes: the destination
        // otherwise keeps whatever its previous occupant left, and a callee that returned
        // nothing would read as a value at the caller.  A source with no shadow is real
        // heap data, so it arrives WRITTEN.
        if self.store(to).shadow_armed() {
            let at = (to.rec as isize * 8 + to.pos as isize) as usize;
            let src = (from.rec as isize * 8 + from.pos as isize) as usize;
            let tags = if self.store(from).shadow_armed() {
                self.store(from).shadow_tags(src, len as usize)
            } else {
                // Heap bytes carry no tag, and they are real data: opaque, so they match
                // whatever the destination is read as.
                self.store_mut(to)
                    .shadow_write(at, len as usize, crate::stack_verify::OPAQUE);
                Vec::new()
            };
            if tags.is_empty() {
                return;
            }
            self.store_mut(to).shadow_set_tags(at, &tags);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Stores;

    /// Build `enum Shape { Words { ws: vector<text> }, Nums { ns: vector<float> } }` and
    /// answer `(enum type, Words type, Nums type)`.
    ///
    /// Two variants each holding ONE collection is the arrangement `variant_owning_field`
    /// has to tell apart: a collection field is a 4-byte handle laid down straight after the
    /// discriminant, so both land at the same byte offset and only the content type
    /// separates them.  The tests assert that collision rather than assume it.
    fn shape_enum(stores: &mut Stores) -> (u16, u16, u16) {
        let text_tp = stores.name("text");
        let float_tp = stores.name("float");
        let words_vec = stores.vector(text_tp);
        let nums_vec = stores.vector(float_tp);
        let enum_tp = stores.enumerate("ProbeShape");
        let words_tp = stores.structure("ProbeShape::Words", 1);
        stores.field(words_tp, "ws", words_vec);
        let nums_tp = stores.structure("ProbeShape::Nums", 2);
        stores.field(nums_tp, "ns", nums_vec);
        stores.value(enum_tp, "Words", words_tp);
        stores.value(enum_tp, "Nums", nums_tp);
        stores.finish();
        (enum_tp, words_tp, nums_tp)
    }

    /// The byte position and content type of one field of a built record type.
    fn field_at(stores: &Stores, tp: u16, name: &str) -> (u16, u16) {
        let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
            &stores.types[tp as usize].parts
        else {
            panic!("'{name}' lives in a record type");
        };
        let f = fields
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("no field '{name}'"));
        (f.position, f.content)
    }

    /// loft#977 — a field written through the ENUM type resolves to the VARIANT that
    /// declares it, and the offset alone is not enough to say which.
    ///
    /// Both fields sit at the same byte offset, so an offset-keyed lookup answers the FIRST
    /// variant for both and every write lands in `Words`.  Only the content type separates
    /// them, and the second assertion is the one that fails without it.
    #[test]
    fn a_struct_enum_field_resolves_to_the_variant_that_declares_it() {
        let mut stores = Stores::new();
        let (enum_tp, words_tp, nums_tp) = shape_enum(&mut stores);

        let (ws_pos, ws_tp) = field_at(&stores, words_tp, "ws");
        let (ns_pos, ns_tp) = field_at(&stores, nums_tp, "ns");
        assert_eq!(
            ws_pos, ns_pos,
            "the two variants put their collection at the same offset — that is the point"
        );
        assert_ne!(ws_tp, ns_tp, "and they differ only in content type");

        assert_eq!(
            stores.variant_owning_field(enum_tp, ws_pos, ws_tp),
            words_tp,
            "the text collection resolves to Words"
        );
        assert_eq!(
            stores.variant_owning_field(enum_tp, ns_pos, ns_tp),
            nums_tp,
            "and the float one to Nums, not to the first variant sharing its offset"
        );
    }

    /// A field no variant declares leaves the type UNCHANGED, so the caller keeps whatever
    /// behaviour it had — the redirect may never invent a record type.
    #[test]
    fn an_unclaimed_field_leaves_the_enum_type_alone() {
        let mut stores = Stores::new();
        let (enum_tp, _, nums_tp) = shape_enum(&mut stores);
        let (ns_pos, ns_tp) = field_at(&stores, nums_tp, "ns");

        assert_eq!(
            stores.variant_owning_field(enum_tp, ns_pos + 64, ns_tp),
            enum_tp,
            "no variant has a field there"
        );
        assert_eq!(
            stores.variant_owning_field(enum_tp, ns_pos, u16::MAX),
            enum_tp,
            "no variant has a field of that type"
        );
        let plain = stores.structure("ProbePlain", -1);
        assert_eq!(
            stores.variant_owning_field(plain, 0, ns_tp),
            plain,
            "a plain struct is not an enum and is answered unchanged"
        );
    }

    /// loft#977 — the message when a record operation names a field its parent does not
    /// declare.  `field_type` answers `u16::MAX` there, and letting that reach the type
    /// table reported `index out of bounds: the len is 83 but the index is 65535`, naming
    /// neither the type nor the field.
    ///
    /// No loft program can reach this any more — the parser resolves the variant before
    /// emitting — so the guard has to be produced from here or not at all.
    #[test]
    #[should_panic(expected = "has no storage in that type")]
    fn a_field_the_parent_cannot_hold_is_named_rather_than_indexed() {
        let mut stores = Stores::new();
        let (enum_tp, _, _) = shape_enum(&mut stores);
        let rec = stores.database(8);
        // The enum type itself holds no fields at all, which is exactly what the parser
        // used to hand `record_new` for `c.limbs += [x]`.
        stores.record_new(&rec, enum_tp, 0);
    }
}
