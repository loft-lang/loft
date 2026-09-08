// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)
//! Find/search operations.

use crate::database::{Parts, Stores};
use crate::keys::{Content, DbRef};
use crate::store::RECORD_PAYLOAD;
use crate::vector;
use crate::{hash, keys, tree};
use std::cmp::Ordering;

#[allow(dead_code)]
fn compare(a: &Content, b: &Content) -> Ordering {
    match (a, b) {
        (Content::Long(a), Content::Long(b)) => i64::cmp(a, b),
        (Content::Single(a), Content::Single(b)) => {
            if a > b {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (Content::Float(a), Content::Float(b)) => {
            if a > b {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (Content::Str(a), Content::Str(b)) => str::cmp(a.str(), b.str()),
        _ => panic!("Undefined compare {a:?} vs {b:?}"),
    }
}

impl Stores {
    #[allow(dead_code)]
    pub(super) fn get_key(&self, fld: &DbRef, db: u16, keys: &[(u16, bool)]) -> Vec<Content> {
        let mut key = Vec::new();
        for (k, _) in keys {
            key.push(self.field_content(fld, db, *k));
        }
        key
    }

    /// Byte offset within a record where an `index`'s red-black links start — what `tree`
    /// descends from.  `u16::MAX` when `tp` is not an index.
    ///
    /// The field list is the one `Stores::index_owner` names, so a synth `__nullable<S>`
    /// element reads its links out of the `Some` record the append put them in.
    #[must_use]
    pub fn fields(&self, tp: u16) -> u16 {
        if let Parts::Index(c, _, f) = self.types[tp as usize].parts {
            let c = self.index_owner(c);
            if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                &self.types[c as usize].parts
            {
                8 + fields[f as usize].position
            } else {
                u16::MAX
            }
        } else {
            u16::MAX
        }
    }

    #[must_use]
    pub fn keys(&self, tp: u16) -> &[crate::keys::Key] {
        &self.types[tp as usize].keys
    }

    #[allow(dead_code)]
    pub(super) fn field_content(&self, rec: &DbRef, db: u16, key: u16) -> Content {
        let store = self.store(rec);
        // Resolve key field `key` to its (content type, absolute byte position) via the
        // SAME `key_field` chokepoint `determine_keys` bakes from — so a synth
        // `__nullable<S>` element's payload base is added identically here and there.
        if let Some((content, position)) = self.key_field(db, key) {
            let pos = rec.pos + u32::from(position);
            return match content {
                0 => Content::Long(store.get_int(rec.rec, pos)),
                6 => Content::Long(i64::from(store.get_u32_raw(rec.rec, pos))),
                1 => Content::Long(store.get_long(rec.rec, pos)),
                2 => Content::Single(store.get_single(rec.rec, pos)),
                3 => Content::Float(store.get_float(rec.rec, pos)),
                4 => Content::Long(i64::from(store.get_byte(rec.rec, pos, 0))),
                5 => Content::Str(crate::keys::Str::new(
                    store.get_str(store.get_u32_raw(rec.rec, pos)),
                )),
                _ => {
                    if let Parts::Enum(_) = self.types[content as usize].parts {
                        Content::Long(i64::from(store.get_byte(rec.rec, pos, 0)))
                    } else {
                        panic!(
                            "Unknown key type {} (field {key} of {})",
                            self.types[content as usize].name, self.types[db as usize].name,
                        )
                    }
                }
            };
        }
        Content::Long(0)
    }

    /**
    Find a record on a given key.
    # Panics
    When the given database type doesn't support searcher.
    */
    #[must_use]
    pub(super) fn find_vector(&self, data: &DbRef, c: u16, key: &[Content]) -> DbRef {
        if let Content::Long(v) = key[0] {
            vector::get_vector(
                data,
                u32::from(self.types[c as usize].size),
                v,
                &self.allocations,
            )
        } else {
            DbRef {
                store_nr: data.store_nr,
                rec: if data.rec == 0 || self.store(data).get_u32_raw(data.rec, 4) == 0 {
                    0
                } else {
                    self.store(data).get_u32_raw(data.rec, 0)
                },
                pos: 8,
            }
        }
    }

    pub(super) fn find_array(&self, data: &DbRef, c: u16, key: &[Content]) -> DbRef {
        if let Content::Long(v) = key[0] {
            let res = vector::get_vector(
                data,
                u32::from(self.types[c as usize].size),
                v,
                &self.allocations,
            );
            DbRef {
                store_nr: res.store_nr,
                rec: if res.rec == 0 {
                    0
                } else {
                    self.store(&res).get_u32_raw(res.rec, res.pos)
                },
                pos: 8,
            }
        } else {
            DbRef {
                store_nr: data.store_nr,
                rec: if data.rec == 0 || self.store(data).get_u32_raw(data.rec, 4) == 0 {
                    0
                } else {
                    let rec = self.store(data).get_u32_raw(data.rec, 0);
                    self.store(data).get_u32_raw(rec, 8)
                },
                pos: 8,
            }
        }
    }

    /**
    Find a record on a given key.
    # Panics
    When the given database type doesn't support searching.
    */
    #[must_use]
    pub fn find(&self, data: &DbRef, db: u16, key: &[Content]) -> DbRef {
        // loft#1213 — an ABSENT collection holds no records, so every lookup MISSES.  Without
        // this the arms below dereference the reserved absent id as if it were a record
        // (`rec=4294967295`), which is out of bounds in every store: appending to a keyed
        // field left absent rather than empty panicked in `hash::find` / `vector::sorted_find`
        // / `tree::find`, all three reached from the DEDUP lookup one line inside
        // `insert_keyed_copy`, on both backends.
        //
        // The same answer `Store::collection_rec` gives on the vector side, where every slot
        // dereference goes through that accessor and absent therefore reads as empty.  Its
        // header names this failure exactly — *"a missed site is a SIGSEGV rather than a wrong
        // answer"* — and the keyed family had none of its twenty sites.  Stated ONCE, in the
        // guard arm below, because the question is the collection's and not each kind's: hash,
        // sorted, index, ordered, trie and radix would otherwise each need it, and the one that
        // was forgotten would be the one that crashes.  It sits under the non-collection arm
        // rather than above the whole match so that it answers only where a collection was
        // actually named.
        match &self.types[db as usize].parts {
            // The TYPE question is asked first, and deliberately so.  A type that reaches this
            // arm is a programming error rather than a miss, and the absent test below would
            // otherwise short-circuit past it: a `DbRef` naming no slot is indistinguishable
            // from an absent collection, so a wrong `db` was answered with a silent miss
            // instead of this diagnostic.
            Parts::Base
            | Parts::Struct(_)
            | Parts::Enum(_)
            | Parts::EnumValue(_, _)
            | Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::ShortRaw(_, _)
            | Parts::Int(_, _)
            | Parts::IntRaw(_, _)
            | Parts::DbRef
            | Parts::ChildRec(_) => panic!(
                "find called on non-collection type: {} (db={})",
                self.types[db as usize].name, db
            ),
            // Every arm below names a COLLECTION, so the absent question is the collection's
            // and is asked ONCE here rather than in each kind.
            _ if crate::vector::is_absent_collection(data, &self.allocations) => DbRef {
                store_nr: data.store_nr,
                rec: 0,
                pos: 0,
            },
            Parts::Vector(c) => self.find_vector(data, *c, key),
            Parts::Array(c) => self.find_array(data, *c, key),
            Parts::Sorted(c, _) => {
                let (pos, found) = vector::sorted_find(
                    data,
                    true,
                    self.types[*c as usize].size,
                    &self.allocations,
                    &self.types[db as usize].keys,
                    key,
                );
                if found {
                    DbRef {
                        store_nr: data.store_nr,
                        rec: self.store(data).get_u32_raw(data.rec, data.pos),
                        pos: 8 + pos * u32::from(self.types[*c as usize].size),
                    }
                } else {
                    DbRef {
                        store_nr: data.store_nr,
                        rec: 0,
                        pos: 0,
                    }
                }
            }
            Parts::Ordered(_, _) => {
                let sorted_rec = self.store(data).get_u32_raw(data.rec, data.pos);
                let (pos, found) = vector::ordered_find(
                    data,
                    true,
                    &self.allocations,
                    &self.types[db as usize].keys,
                    key,
                );
                if found {
                    DbRef {
                        store_nr: data.store_nr,
                        rec: self.store(data).get_u32_raw(sorted_rec, 8 + pos * 4),
                        pos: 8,
                    }
                } else {
                    DbRef {
                        store_nr: data.store_nr,
                        rec: 0,
                        pos: 0,
                    }
                }
            }
            Parts::Hash(_, _) => hash::find(data, &self.allocations, self.keys(db), key),
            Parts::Trie(_, _) => crate::trie_db::find(data, &self.allocations, self.keys(db), key),
            Parts::Radix(_, _) => {
                crate::radix_db::find(data, &self.allocations, self.keys(db), key)
            }
            Parts::Index(_, _, _) => self.find_index(data, db, key),
        }
    }

    pub(super) fn find_index(&self, data: &DbRef, db: u16, key: &[Content]) -> DbRef {
        // Where the links live is [`Self::fields`]'s question, asked once: a synth
        // `__nullable<S>` element keeps them in the `Some` record, and recomputing the
        // offset here from the enum's own (absent) field list read `u16::MAX`.
        let left = self.fields(db);
        let rec = tree::find(data, true, left, &self.allocations, self.keys(db), key);
        let mut result = DbRef {
            store_nr: data.store_nr,
            rec,
            pos: 8,
        };
        result.rec = if rec == 0 {
            tree::first(data, left, &self.allocations).rec
        } else {
            tree::next(
                keys::store(&result, &self.allocations),
                &DbRef {
                    store_nr: result.store_nr,
                    rec,
                    pos: u32::from(left),
                },
            )
        };
        let cmp = keys::key_compare(key, &result, &self.allocations, self.keys(db));
        if cmp == Ordering::Equal {
            result
        } else {
            DbRef {
                store_nr: data.store_nr,
                rec: 0,
                pos: 0,
            }
        }
    }

    #[must_use]
    pub fn get_keys(&self, db: u16) -> Vec<u16> {
        match &self.types[db as usize].parts {
            Parts::Vector(_) | Parts::Array(_) => vec![0],
            Parts::Sorted(c, key) | Parts::Ordered(c, key) | Parts::Index(c, key, _) => {
                // Key content TYPES (for `read_key` to pop the right widths).  Route through
                // the `key_field` chokepoint so a synth `__nullable<S>` element resolves the
                // key inside the `Some` payload, and through `key_contents_for_field` so a
                // TUPLE field contributes one entry per element — the same arity
                // `determine_keys` baked.
                key.iter()
                    .filter_map(|(k, _)| self.key_field(*c, *k))
                    .flat_map(|(content, position)| self.key_contents_for_field(content, position))
                    .map(|(content, _)| content)
                    .collect()
            }
            // `Radix` (a `spatial<T[x,y]>`) carries the same `(element, key fields)` shape as
            // `Hash`, and its coordinate axes are as much a key as any other: `read_key` pops
            // one stack value per entry here, so an empty answer pops NOTHING and the very
            // next `get_stack::<DbRef>()` reads a leftover key value as the collection —
            // `sp[3, 3]` then looked up in store #3 (loft#720).  The bytecode was always
            // right (`GetRecord(… no_keys=2)`); only this list disagreed.
            Parts::Trie(c, k) => self
                .key_field(*c, *k)
                .map(|(ct, _)| vec![ct])
                .unwrap_or_default(),
            Parts::Hash(c, key) | Parts::Radix(c, key) => key
                .iter()
                .filter_map(|k| self.key_field(*c, *k))
                .flat_map(|(content, position)| self.key_contents_for_field(content, position))
                .map(|(content, _)| content)
                .collect(),
            // Not a keyed collection — nothing to pop.  Listed out rather than
            // caught by `_`, the way `Stores::remove` lists them: this answer
            // decides an ARITY, so a collection kind missing from it does not
            // lose a feature, it desynchronises the operand stack (loft#720).
            // A `_` let `Radix` go missing silently; spelled out, the next kind
            // added to `Parts` cannot compile until someone decides here.
            Parts::Base
            | Parts::Struct(_)
            | Parts::Enum(_)
            | Parts::EnumValue(_, _)
            | Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::ShortRaw(_, _)
            | Parts::Int(_, _)
            | Parts::IntRaw(_, _)
            | Parts::DbRef
            | Parts::ChildRec(_) => Vec::new(),
        }
    }

    /**
    Validate the structure in any way possible.
    What is still open to validate:
    - individual allocations inside store size
    - length of vector/sorted/array/ordered stays within allocation
    - when called fully; but allow for single vector:
      - allocations linked together correctly (linked from previous and to next)
      - open space validation
      - references of array/ordered/separate to correct allocations
    # Panics
    When the structure is not correct
    */
    pub fn validate(&mut self, data: &DbRef, db: u16) {
        match self.types[db as usize].parts.clone() {
            Parts::Hash(_, _) => {
                hash::validate(data, &self.allocations, &self.types[db as usize].keys);
            }
            Parts::Index(_, _, fields) => {
                tree::validate(
                    data,
                    fields,
                    &self.allocations,
                    &self.types[db as usize].keys,
                );
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in fields {
                    self.validate(
                        &DbRef {
                            store_nr: data.store_nr,
                            rec: data.rec,
                            pos: data.pos + u32::from(f.position),
                        },
                        f.content,
                    );
                }
            }
            _ => (),
        }
    }

    /**
    Get the next record given a specific point in a structure.
    # Panics
    When not in a valid structure
    */
    pub(super) fn next(&self, data: &DbRef, pos: &mut i32, db: u16) -> DbRef {
        match &self.types[db as usize].parts {
            Parts::Vector(c) | Parts::Sorted(c, _) => {
                vector::vector_next(data, pos, self.types[*c as usize].size, &self.allocations);
                self.element_reference(data, *pos)
            }
            Parts::Array(_) => {
                vector::vector_next(data, pos, 4, &self.allocations);
                let r = self.store(data).get_u32_raw(data.rec, data.pos);
                self.db_ref(data, *pos, r)
            }
            Parts::Ordered(_, _) => {
                vector::vector_next(data, pos, 4, &self.allocations);
                if *pos == i32::MAX {
                    return DbRef {
                        store_nr: data.store_nr,
                        rec: 0,
                        pos: 0,
                    };
                }
                let r = self.store(data).get_u32_raw(data.rec, data.pos);
                DbRef {
                    store_nr: data.store_nr,
                    rec: self.store(data).get_u32_raw(r, *pos as u32),
                    pos: 8,
                }
            }
            Parts::Index(_, _, _) => {
                if *pos == i32::MAX {
                    let n = tree::first(data, self.fields(db), &self.allocations);
                    *pos = n.rec as i32;
                    return n;
                }
                let store = keys::store(data, &self.allocations);
                let mut rec = DbRef {
                    store_nr: data.store_nr,
                    rec: *pos as u32,
                    pos: u32::from(self.fields(db)),
                };
                let n = tree::next(store, &rec);
                if n == 0 {
                    return DbRef {
                        store_nr: data.store_nr,
                        rec: 0,
                        pos: 0,
                    };
                }
                *pos = n as i32;
                rec.rec = n;
                rec.pos = 8;
                rec
            }
            Parts::Base
            | Parts::Struct(_)
            | Parts::Enum(_)
            | Parts::EnumValue(_, _)
            | Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::ShortRaw(_, _)
            | Parts::Int(_, _)
            | Parts::IntRaw(_, _)
            | Parts::DbRef
            | Parts::ChildRec(_)
            | Parts::Hash(_, _)
            | Parts::Radix(_, _)
            | Parts::Trie(_, _) => panic!(
                "Undefined iterate on non-collection type: {} (db={})",
                self.types[db as usize].name, db
            ),
        }
    }

    pub(super) fn db_ref(&self, data: &DbRef, pos: i32, r: u32) -> DbRef {
        DbRef {
            store_nr: data.store_nr,
            rec: if pos == i32::MAX {
                0
            } else {
                self.store(data).get_u32_raw(r, pos as u32)
            },
            pos: 8,
        }
    }

    #[must_use]
    pub fn element_reference(&self, data: &DbRef, pos: i32) -> DbRef {
        DbRef {
            store_nr: data.store_nr,
            rec: if pos == i32::MAX {
                0
            } else {
                self.store(data).get_u32_raw(data.rec, data.pos)
            },
            pos: pos as u32,
        }
    }

    /// @P305 — insert-or-replace a value into a keyed collection
    /// (`hash`/`sorted`/`index`), keyed by `value`'s own key field(s).  This
    /// is the runtime of `coll[k] = value`: if a record with `value`'s key
    /// already exists it is removed first (so the key stays unique — dedup),
    /// then `value` is deep-copied into a fresh record and linked in.  Works
    /// uniformly for a LOCAL, struct FIELD, or `&`-param collection because
    /// `coll` is just the resolved collection ref at runtime.  Mirrors the
    /// proven `OpNewRecord` + `OpCopyRecord` + `OpFinishRecord` append
    /// sequence (so it shares its deep-copy + linking machinery) plus a
    /// preceding remove.  `free_source` frees `value`'s store after the copy
    /// when it is a caller temp (the `0x8000` bit, as in `copy_record`).
    pub fn set_keyed(&mut self, coll: &DbRef, value: &DbRef, db: u16, free_source: bool) {
        self.insert_keyed_copy(coll, value, db, coll, db, u16::MAX);
        if free_source
            && value.store_nr != coll.store_nr
            // Sentinel guard: the `allocations[..]` prechecks below index by store_nr, so a null
            // source must short-circuit here (the eventual `free` is already sentinel-safe).
            && value.store_nr != u16::MAX
            && !self.is_stack_store(value.store_nr)
            && !self.allocations[value.store_nr as usize].free
            && !self.allocations[value.store_nr as usize].read_only
            && !self.allocations[value.store_nr as usize].is_free_protected()
        {
            self.free(value);
        }
    }

    /// Place a deep COPY of `value` into the keyed collection `coll` (type `db`), keyed by
    /// `value`'s own key fields, replacing any record already under that key.
    ///
    /// `(parent, parent_tp, field)` name the collection the way [`Self::record_finish`] wants
    /// it — the owning struct ref, its type, and the field index — because that is what
    /// decides whether the record reaches the rest of a linked collection group: the sibling
    /// walk is keyed on the FIELD, and `field == u16::MAX` means "the parent IS the
    /// collection", which is a lone collection with no siblings to maintain.
    ///
    /// This is one home on purpose (loft#1159). The bulk fill and `coll[k] = v` used to
    /// differ in nothing but this, and the difference did not show as two spellings of an
    /// insert — it showed as a linked group whose members disagreed on which records exist.
    pub(crate) fn insert_keyed_copy(
        &mut self,
        coll: &DbRef,
        value: &DbRef,
        db: u16,
        parent: &DbRef,
        parent_tp: u16,
        field: u16,
    ) {
        let content_tp = match self.types[db as usize].parts {
            Parts::Hash(c, _)
            | Parts::Sorted(c, _)
            | Parts::Index(c, _, _)
            | Parts::Ordered(c, _)
            | Parts::Radix(c, _)
            | Parts::Trie(c, _) => c,
            _ => return,
        };
        let keys = self.types[db as usize].keys.clone();
        let key = keys::get_key(value, &self.allocations, &keys);
        let existing = self.find(coll, db, &key);
        if existing.rec != 0 {
            // dedup: free the old record's nested heap, then unlink it.
            self.remove_claims(&existing, content_tp);
            self.remove(coll, &existing, db);
            // A hash record is a SEPARATE store claim, so the unlink above
            // leaves it orphaned — reclaim it (otherwise repeated
            // `coll[k] = v` replaces grow the store unboundedly).  Sorted /
            // index records are inline in the vector / freed by their own
            // `remove`, so no extra delete there.  A `Radix` element holds its
            // own block exactly as a hash element does (the same pair
            // `remove_owned` calls `own_block`), so it needs the same reclaim.
            // @PLN135 arc H — a hash entry's storage goes back to its ARENA; a
            // `Store::delete` here would hand the whole chunk (and every live entry
            // in it) to the free tree.
            if matches!(self.types[db as usize].parts, Parts::Hash(_, _)) {
                hash::free_entry(coll, &existing, &mut self.allocations);
            } else if matches!(
                self.types[db as usize].parts,
                Parts::Radix(_, _) | Parts::Trie(_, _)
            ) {
                self.store_mut(coll).delete(existing.rec);
            }
        }
        // insert: claim a fresh record, deep-copy `value` into it, link by key.
        let new = self.record_new(parent, parent_tp, field);
        let size = u32::from(self.size(content_tp));
        self.copy_block(value, &new, size);
        self.copy_claims(value, &new, content_tp);
        self.record_finish(parent, &new, parent_tp, field);
        // @P317 — LOFT_LOG=copy_check: warn if the keyed deep copy changed any
        // nested collection length (before the source-free below).
        if self.copy_check_enabled() {
            self.report_copy_mismatches(value, &new, content_tp, "set_keyed");
        }
    }

    /// loft#1159 — fill the keyed collection `dest` with a deep COPY of every record the
    /// plain vector `src` holds, keyed by each record's own key fields.
    ///
    /// This is the bulk sibling of [`Self::set_keyed`], and it exists because the two ways of
    /// naming the same records did not mean the same thing.  A vector LITERAL in a keyed
    /// position (`h.a = [E{…}, E{…}]`) is parsed into per-element construction that inserts
    /// each record by key; a vector VALUE (`h.a = rows()`, `h.a += rows()`) is one expression
    /// of type `vector<E>`, and there was no route from it to the same inserts.  `=` reached
    /// `OpReplaceKeyed`, which hands the source to `copy_claims` under the DESTINATION's type
    /// and so walks a vector's storage as if it were a hash / index / trie — `sorted` alone
    /// survived that, because a sorted's own storage IS a sequential vector.  `+=` reached
    /// nothing at all and the statement was discarded.
    ///
    /// `raw_tp`'s `0x8000` bit frees the source store after the copy, exactly as it does for
    /// `OpReplaceKeyed` and `OpSetKeyed`: a fresh-storage right-hand side (`rows()`) is a
    /// temporary nobody else names.  The decode lives here, not in the two op bodies, so the
    /// interpreter and the native generator cannot drift on it.
    ///
    /// The element addressing follows `is_linked` on the element TYPE, which is the same
    /// question `index_group_records` asks and is a property of the type across the whole
    /// program: a record-backed vector holds 4-byte record ids, a plain one holds its records
    /// inline.  Reading the wrong one is how a length that disagrees with its own lookups gets
    /// built, which is the state this replaces.
    pub fn fill_keyed_from_vector(
        &mut self,
        parent: &DbRef,
        src: &DbRef,
        raw_tp: u16,
        parent_tp: u16,
        field: u16,
    ) {
        let free_source = raw_tp & 0x8000 != 0;
        let tp = raw_tp & 0x7FFF;
        if (tp as usize) >= self.types.len() || src.is_null() || src.store_nr == u16::MAX {
            return;
        }
        // `field == u16::MAX` means the parent IS the collection — the convention
        // `record_finish` and `record_new` already use for a lone collection.
        let dest = self.field_ref(parent, parent_tp, field);
        let dest = &dest;
        let content_tp = self.content(tp);
        if content_tp == u16::MAX {
            return;
        }
        let length = vector::length_vector(src, &self.allocations);
        let linked = self.is_linked(content_tp);
        let width = if linked {
            4
        } else {
            u32::from(self.size(content_tp))
        };
        // Resolve every element BEFORE inserting any of them.  An insert may grow the
        // destination store, and when source and destination share one store a half-walked
        // source is exactly the read that answers with another record's bytes.
        let mut records: Vec<DbRef> = Vec::with_capacity(length as usize);
        for i in 0..length {
            let slot = vector::get_vector(src, width, i64::from(i), &self.allocations);
            if slot.rec == 0 {
                continue;
            }
            if linked {
                let rec = keys::store(&slot, &self.allocations).get_u32_raw(slot.rec, slot.pos);
                // A null element has no key to be found under, so it stays in the vector and
                // out of the keyed collection — the same answer the group fill gives it.
                if rec == 0 {
                    continue;
                }
                records.push(DbRef {
                    store_nr: src.store_nr,
                    rec,
                    pos: RECORD_PAYLOAD,
                });
            } else {
                records.push(slot);
            }
        }
        for rec in &records {
            self.insert_keyed_copy(dest, rec, tp, parent, parent_tp, field);
        }
        if free_source
            && src.store_nr != dest.store_nr
            // Sentinel guard: the `allocations[..]` prechecks index by store_nr.
            && src.store_nr != u16::MAX
            && !self.is_stack_store(src.store_nr)
            && !self.allocations[src.store_nr as usize].free
            && !self.allocations[src.store_nr as usize].read_only
            && !self.allocations[src.store_nr as usize].is_free_protected()
        {
            self.free(src);
        }
    }

    /// @P306 — before linking a new keyed record, drop any EXISTING record
    /// that shares its key so the key stays unique (latest insert wins —
    /// `coll += [entry]` on a keyed collection dedups instead of stacking a
    /// shadowed duplicate).  For hash / index the records are SEPARATE store
    /// claims, so free the old one's nested heap, unlink it, and reclaim its
    /// slot.  (Sorted / ordered records are inline in the vector and the new
    /// one is already appended at the end when their `*_finish` runs, so they
    /// dedup by overwriting the found slot in place there, not here.)
    /// Replace any record already stored under `rec`'s key.
    ///
    /// `secondary` distinguishes a PRIMARY keyed collection (this index OWNS its
    /// records) from a SECONDARY index auto-maintained for a sibling field via
    /// `Field.other_indexes` (the "two views share records" pattern — a
    /// `vector<T>` + `hash<T[k]>` in one struct).  A secondary index must only
    /// UNLINK the stale key→record mapping; the displaced record is still held
    /// by the primary collection (the vector), so freeing it here corrupts that
    /// collection (read-back nulls / `Unknown record`).  This is the
    /// `other_indexes` analogue of the @P305 `keyed_field_is_linked` update-only
    /// path already used by `OpSetKeyed`.
    pub(crate) fn dedup_keyed(
        &mut self,
        data: &DbRef,
        rec: &DbRef,
        db: u16,
        content_tp: u16,
        secondary: bool,
    ) {
        let keys = self.types[db as usize].keys.clone();
        let key = keys::get_key(rec, &self.allocations, &keys);
        let existing = self.find(data, db, &key);
        // @PLN135 arc H — two entries of the same hash now share a chunk RECORD, so
        // "is this the same entry" is `(rec, pos)`, not `rec` alone.  Comparing only
        // the record number would read a neighbouring slot in the same chunk as the
        // entry being inserted and skip the dedup.
        if existing.rec != 0 && (existing.rec, existing.pos) != (rec.rec, rec.pos) {
            self.remove(data, &existing, db);
            if !secondary {
                self.remove_claims(&existing, content_tp);
                if matches!(self.types[db as usize].parts, Parts::Hash(_, _)) {
                    hash::free_entry(data, &existing, &mut self.allocations);
                } else {
                    self.store_mut(data).delete(existing.rec);
                }
            }
        }
    }

    /// Remove `rec` from the collection **and release what it owned** — the
    /// form a user-level removal (`c[key] = null`, `e#remove`) needs.
    ///
    /// [`Stores::remove`] deliberately only UNLINKS, because a SECONDARY index
    /// (a sibling field's `other_indexes`) shares its records with the primary
    /// collection and must never free them — the same split
    /// [`Stores::dedup_keyed`] makes when a new insert displaces an existing
    /// key. Nothing paired the unlink with a free on the removal side, so every
    /// removal leaked: with a constant population of 300 records, six
    /// insert-then-remove-all cycles grew claimed bytes 0.10 → 0.56 MB, and
    /// re-inserting after removing everything grew the store instead of reusing
    /// it. A long-lived store therefore grew without bound, and no compaction
    /// could have reclaimed it — the blocks were still marked LIVE.
    ///
    /// Order matters: the claims are read out of the record's own fields, so
    /// they must be released before the record is unlinked (a vector element is
    /// shifted over by its successor) and before its block is freed.
    /// Does the hash at `data` allocate its own entries, or only borrow records a
    /// sibling collection holds?  False for an uninitialised table, which has
    /// nothing to free either way.
    fn hash_owns_entries(&self, data: &DbRef) -> bool {
        let store = self.store(data);
        let claim = store.get_u32_raw(data.rec, data.pos);
        claim != 0 && hash::owns_entries(store, claim)
    }

    /// @FR-Col-Remove's by-RECORD form — "delete one element" AND release what it owned.
    /// [`Stores::remove_vector_at`] is the by-INDEX twin; [`Stores::remove`] below is the
    /// UNLINK half both of them share, and is deliberately not this.
    pub fn remove_owned(&mut self, data: &DbRef, rec: &DbRef, db: u16) {
        let parts = self.types[db as usize].parts.clone();
        let content = match &parts {
            Parts::Vector(c)
            | Parts::Array(c)
            | Parts::Sorted(c, _)
            | Parts::Ordered(c, _)
            | Parts::Hash(c, _)
            | Parts::Radix(c, _)
            | Parts::Trie(c, _)
            | Parts::Index(c, _, _) => *c,
            // Not a collection: `remove` panics on these, and it stays the one
            // place that decides so.
            _ => {
                self.remove(data, rec, db);
                return;
            }
        };
        // Whether the element has a store block of its OWN, or lives inline in
        // the container. Only the kinds `dedup_keyed` frees are freed here —
        // `Array` / `Ordered` also hold separate records, but their removal
        // path reads `rec` as an inline index and is wrong before this change
        // too, so this does not extend to them.
        // `Ordered` joins them (loft#719): its elements have their own store
        // records too, so the element must be DELETED after its claims are
        // released — the inline branch below only shifts the container and would
        // leak the record.  `Array` is left out deliberately: it is removed by
        // INDEX rather than by key, so `rec` reaches this function differently
        // and it has no test to move it on.
        let own_block = matches!(
            parts,
            Parts::Hash(..)
                | Parts::Index(..)
                | Parts::Radix(..)
                | Parts::Trie(..)
                | Parts::Ordered(..)
        );
        let rec_nr = rec.rec;
        if own_block {
            // Unlink BEFORE releasing the claims, which is the order
            // `dedup_keyed` uses and it is load-bearing: an `index`'s red-black
            // links live in a FIELD of the record, so to `remove_claims` they
            // look like owned children. Free first and it follows them into the
            // live siblings and takes the subtree with it — the whole
            // collection then reads back as "Item not found".
            self.remove(data, rec, db);
            // Walk the element's owned children from the RECORD's payload
            // start, not from the position the caller navigated with (loft#718).
            //
            // A field position in `content` is an offset within the element
            // record, so it is only meaningful against that record's payload —
            // byte 8, past the size header.  Most callers already hold such a
            // `DbRef` (a key lookup does), but an `index`'s LOOP cursor does
            // not: its red-black links live in a field of the record, so
            // `tree::next` navigates with `pos` set to that link's offset
            // (`Stores::fields`).  Handing that cursor to `remove_claims` walked
            // every field from there — for `Rec { id, n, label: text }` the
            // `text` landed at 8+20+16 = 44 in a 40-byte record, so `#remove`
            // in a filtered loop read past the record and corrupted the free
            // tree, which then recursed forever in `fl_insert_node` (SIGSEGV on
            // the interpreter, stack overflow on `--native`).
            //
            // `own_block` is exactly the set whose elements HAVE their own
            // record, so the payload start is the right answer for all of them
            // rather than a per-kind adjustment.
            //
            // @PLN135 arc H — a HASH entry is a slot in a chunked arena, not a record
            // of its own: its payload starts at the slot offset the caller already
            // holds, and normalising to byte 8 would walk the CHUNK's first slot
            // instead.  Its storage comes back to the arena rather than to
            // `Store::delete`, which would hand the whole chunk to the free tree.
            //
            // A hash in a LINKED GROUP has no arena — its entries are one record
            // each, so its siblings can name them by record id (loft#901) — and
            // `hash::free_entry` correctly declines to free a record it does not
            // own.  Declining is the whole story only while somebody else frees:
            // when the removal is spelled through this member it IS the free, so
            // taking the arena path leaked the record and everything it claimed.
            // `owns_entries` is the table's own answer to which case this is.
            if matches!(parts, Parts::Hash(..)) && self.hash_owns_entries(data) {
                self.remove_claims(rec, content);
                hash::free_entry(data, rec, &mut self.allocations);
                return;
            }
            let elem = DbRef {
                store_nr: rec.store_nr,
                rec: rec_nr,
                pos: RECORD_PAYLOAD,
            };
            self.remove_claims(&elem, content);
            self.store_mut(data).delete(rec_nr);
        } else {
            // Inline element: its successor is about to be shifted on top of
            // it, so the claims have to be read out while the fields are still
            // its own.
            self.remove_claims(rec, content);
            self.remove(data, rec, db);
        }
    }

    /// Remove the element at `index` from a vector-shaped container **and
    /// release what it owned** — the by-INDEX form, which is how a loop cursor
    /// (`e#remove`) and `v.remove(i)` both reach an element.
    ///
    /// [`Stores::remove_owned`] is the by-RECORD form a key lookup reaches; this
    /// is its twin for the containers that are addressed by position.
    ///
    /// `elem_tp` is the ELEMENT type, and its `linked` flag is the schema's own
    /// answer to which of the two layouts the container has:
    ///
    /// * not linked — a `vector`/`sorted` holds its elements INLINE, so a slot is
    ///   as wide as an element and there is no separate record to free.  What the
    ///   element OWNED is still its own — a `text`, a nested collection, a child
    ///   struct all live in records of their own — and the slot going away does
    ///   not release them, so the claims are walked here before the shift
    ///   overwrites the bytes that name them (loft#1402);
    /// * linked — an `array`/`ordered` (what a `vector`/`sorted` becomes as soon
    ///   as any keyed collection over the element type exists) holds 4-byte
    ///   record ids, so a slot is FOUR bytes and the record each one names is the
    ///   element's own.
    ///
    /// Handing the element's width to [`vector::remove_vector`] for the linked
    /// layout shifted a span several slots long, so removing one element removed
    /// its neighbour with it — and nothing freed the record (loft#903).
    ///
    /// @FR-Col-Remove — one of the two homes for "delete one element": this is the
    /// by-INDEX form (`v.remove(i)`, `e#remove`), [`Stores::remove_owned`] the by-RECORD
    /// one (`c[key] = null`).  Both owe the same two things, and both now do them: the
    /// container is UNLINKED (@FR-Col-RemoveDense renumbers it) and what the element OWNED
    /// is released, at either layout.
    ///
    /// The inline layout could not do the second until loft#1401: a `??`-discharged binding
    /// stayed a live view of the removed element, so releasing its children emptied a value
    /// the program was still reading (`445-generic-tree-walk` is the cell that showed it).
    /// Now such a binding materialises, and the release is safe.
    pub fn remove_vector_at(&mut self, data: &DbRef, elem_tp: u16, index: i64) -> bool {
        if !self.is_linked(elem_tp) {
            let size = u32::from(self.size(elem_tp));
            // Release before the shift: an inline element IS the slot, so once
            // `remove_vector` has moved its successors down there is nothing left to
            // read the claims out of.  The linked branch below can unlink first only
            // because the record it names survives the unlink.
            //
            // `get_vector` is the same index→element map `remove_vector` walks, and it
            // answers `rec == 0` for exactly the indices that one removes nothing for —
            // a null container, `i64::MIN`, and out of range after the same negative
            // normalisation — so the guard and the removal cannot disagree about which
            // indices name an element.
            let elem = vector::get_vector(data, size, index, &self.allocations);
            if elem.rec != 0 {
                self.remove_claims(&elem, elem_tp);
            }
            return vector::remove_vector(data, size, index, &mut self.allocations);
        }
        if data.is_null() || index < 0 {
            return false;
        }
        let vec_rec = self.store(data).get_u32_raw(data.rec, data.pos);
        if vec_rec == 0 {
            return false;
        }
        let len = self.store(data).get_u32_raw(vec_rec, 4);
        let Ok(slot) = u32::try_from(index) else {
            return false;
        };
        if slot >= len {
            return false;
        }
        let rec = self.store(data).get_u32_raw(vec_rec, 8 + slot * 4);
        // Unlink first, release the record's claims second, delete it last — the
        // order [`Stores::remove_owned`] uses, and for the same reason: the walk
        // reads the record's own fields, so nothing may have freed them yet.
        let shifted = vector::remove_vector(data, 4, index, &mut self.allocations);
        if rec != 0 {
            let elem = DbRef {
                store_nr: data.store_nr,
                rec,
                pos: RECORD_PAYLOAD,
            };
            self.remove_claims(&elem, elem_tp);
            self.store_mut(data).delete(rec);
        }
        shifted
    }

    /**
    Remove a specific record from a structure.

    UNLINK only — see [`Stores::remove_owned`] for the user-level form that also
    releases the record's heap.

    @FR-Col-RemoveKeyed for the five keyed kinds — removal is BY KEY and renumbers nothing,
    so every other key stays reachable; @FR-Col-RemoveDense for the two by-value ones, where
    the shift below is what keeps the container dense.
    # Panics
    When not in a structure.
    */
    pub fn remove(&mut self, data: &DbRef, rec: &DbRef, db: u16) {
        let parts = self.types[db as usize].parts.clone();
        // A NULL element is in no keyed view, so it leaves none — the LEAVE half of the
        // test `link_siblings` already applies on the ENTER half, through the same home
        // (`@FR-Col-Group`).  Only the keyed kinds ask: a by-value `vector<E?>` holds its
        // null slots as elements and removes them by position like any other.
        if matches!(
            parts,
            Parts::Hash(..) | Parts::Index(..) | Parts::Trie(..) | Parts::Radix(..)
        ) && self.absent_nullable_record(self.content(db), rec)
        {
            return;
        }
        match parts {
            // BY-VALUE: elements sit inline in the container, so the element's
            // byte position IS its index.
            Parts::Sorted(c, _) | Parts::Vector(c) => {
                let size = u32::from(self.types[c as usize].size);
                vector::remove_vector(
                    data,
                    size,
                    i64::from((rec.pos - 8) / size),
                    &mut self.allocations,
                );
            }
            // BY-REFERENCE: the container holds 4-byte rec-ids and the element
            // lives in its own record, so `rec` is that record at its payload
            // start — `(rec.pos - 8) / size` is 0 for EVERY element, and `size`
            // is the element's width where the container's slots are 4 bytes.
            // Removing anything therefore shifted the wrong span from the wrong
            // place (loft#719).  The slot has to be looked up by the record it
            // names.
            //
            // `Array` reaches this the same way and needed the same answer
            // (loft#900): it is the promoted form of a `vector<T>` sharing its
            // records with a keyed sibling, so removing one entry of a linked
            // group through the keyed member must unlink the vector's slot too,
            // and by-value arithmetic sent every such removal to slot 0.
            Parts::Ordered(_, _) | Parts::Array(_) => {
                let vec_rec = self.store(data).get_u32_raw(data.rec, data.pos);
                if vec_rec == 0 {
                    return;
                }
                let len = self.store(data).get_u32_raw(vec_rec, 4);
                let slot =
                    (0..len).find(|i| self.store(data).get_u32_raw(vec_rec, 8 + i * 4) == rec.rec);
                if let Some(i) = slot {
                    vector::remove_vector(data, 4, i64::from(i), &mut self.allocations);
                }
            }
            Parts::Hash(_, _) => {
                let keys = self.keys(db).to_vec();
                hash::remove(data, rec, &mut self.allocations, &keys);
            }
            Parts::Index(_, _, _) => {
                let left = self.fields(db);
                let keys = self.keys(db).to_vec();
                tree::remove(data, rec, left, &mut self.allocations, &keys);
            }
            Parts::Trie(_, _) => {
                let keys = self.keys(db).to_vec();
                crate::trie_db::remove(data, rec, &mut self.allocations, &keys);
            }
            Parts::Radix(_, _) => {
                let keys = self.keys(db).to_vec();
                crate::radix_db::remove(data, rec, &mut self.allocations, &keys);
            }
            Parts::Base
            | Parts::Struct(_)
            | Parts::Enum(_)
            | Parts::EnumValue(_, _)
            | Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::ShortRaw(_, _)
            | Parts::Int(_, _)
            | Parts::IntRaw(_, _)
            | Parts::DbRef
            | Parts::ChildRec(_) => panic!(
                "remove called on non-collection type: {} (db={})",
                self.types[db as usize].name, db
            ),
        }
    }

    // Output the hash content and validate its content.
    #[allow(dead_code)]
    pub(super) fn hash_dump(&mut self, hash_ref: &DbRef, db: u16, keys: &[u16]) {
        let claim = self.store(hash_ref).get_u32_raw(hash_ref.rec, hash_ref.pos);
        let length = self.store(hash_ref).get_u32_raw(claim, 4);
        let room = self.store(hash_ref).get_i32_raw(claim, 0) as u32;
        let elms = (room - 1) * 2;
        println!(
            "dump hash length:{length} elms:{elms} {:.2}%",
            100.0 * f64::from(length) / f64::from(elms)
        );
        let mut record = DbRef {
            store_nr: hash_ref.store_nr,
            rec: 0,
            pos: 0,
        };
        let mut l = 0;
        for i in 0..elms {
            let rec = self.store(hash_ref).get_u32_raw(claim, 8 + i * 4);
            if rec != 0 {
                let mut s = String::new();
                record.rec = rec;
                self.show(&mut s, &record, db, false);
                l += 1;
                println!("{i:4}:[{rec}]{s}");
                let mut k = Vec::new();
                for f in keys {
                    k.push(self.field_content(&record, db, *f));
                }
            }
        }
        assert_eq!(length, l, "Incorrect hash length");
    }

    #[allow(dead_code)]
    pub(super) fn compare_key(
        &self,
        rec: &DbRef,
        db: u16,
        keys: &[(u16, bool)],
        key: &[Content],
    ) -> Ordering {
        for (k_nr, k) in key.iter().enumerate() {
            let mut cmp = compare(k, &self.field_content(rec, db, keys[k_nr].0));
            if !keys[k_nr].1 {
                if cmp == Ordering::Less {
                    cmp = Ordering::Greater;
                } else if cmp == Ordering::Greater {
                    cmp = Ordering::Less;
                }
            }
            if cmp != Ordering::Equal {
                return cmp;
            }
        }
        Ordering::Equal
    }
}

#[cfg(test)]
mod tests {
    use crate::database::Stores;
    use crate::keys::{Content, DbRef};

    /// `find()` with a non-collection type must panic with a diagnostic message.
    #[test]
    #[should_panic(expected = "find called on non-collection type")]
    fn find_non_collection_panics() {
        let stores = Stores::new();
        let data = DbRef {
            store_nr: 0,
            rec: 0,
            pos: 0,
        };
        let _ = stores.find(&data, 0, &[Content::Long(0)]);
    }

    /// `remove()` with a non-collection type must panic with a diagnostic message.
    #[test]
    #[should_panic(expected = "remove called on non-collection type")]
    fn remove_non_collection_panics() {
        let mut stores = Stores::new();
        let data = DbRef {
            store_nr: 0,
            rec: 0,
            pos: 0,
        };
        let rec = DbRef {
            store_nr: 0,
            rec: 0,
            pos: 0,
        };
        stores.remove(&data, &rec, 0);
    }
}
