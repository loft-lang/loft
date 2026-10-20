// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @I70 — Database subsystem: the store upgrade converter.
//
//! @PLN196 — carry a store written under one layout into a NEW store under this program's
//! layout, store to store, by executing an [`UpgradeMap`].
//!
//! `@FR-L-Sound`: old bytes are read only through the OLD layout and written only through the
//! NEW one.  The old layout is a description ([`LayoutDesc`]); to read through it with the
//! walkers every collection kind already has, its types are appended to this program's type
//! table for the length of the upgrade — under their own names, never entered in the name
//! index — and removed again afterwards.  The appended rows are checked to re-render the old
//! layout exactly before a byte is read, so a row that came out different refuses the upgrade
//! instead of reading at the wrong place.
//!
//! The new store is built by the new program's own allocate-and-file pair (`record_new`,
//! `record_finish`), so a changed key, an added index, a group or a changed stride is laid out
//! exactly as the running program lays it out.  The old store is never written: a refusal
//! discards the new store and leaves the old one as it was.

use crate::database::{
    BaseKind, Field, FieldFrom, How, Iterated, LayoutDesc, LayoutNode, Parts, Stores, Type,
    UpgradeMap,
};
use crate::keys::DbRef;
use std::collections::BTreeMap;

/// The record ids the seven base types have in every type table (`Stores::new`).
fn base_id(k: BaseKind) -> u16 {
    match k {
        BaseKind::Integer => 0,
        BaseKind::Long => 1,
        BaseKind::Single => 2,
        BaseKind::Float => 3,
        BaseKind::Boolean => 4,
        BaseKind::Text => 5,
        BaseKind::Character => 6,
    }
}

/// One upgrade in progress: the map, and where each old description id sits in the table.
struct Run<'m> {
    map: &'m UpgradeMap,
    /// old description id → its appended (or base) row in `Stores::types`.
    old_ids: BTreeMap<u16, u16>,
    old: &'m LayoutDesc,
    /// The slot the old store's references name it by (its sidecar's `slot=`).
    self_slot: Option<u16>,
    /// Old record number → the new record converted from it, for re-pointing references.
    rec_map: std::collections::HashMap<u32, DbRef>,
    /// Every reference converted so far: where it goes in the new store, and the old
    /// `(store, rec, pos)` it held.  Written after the whole tree exists.
    fixups: Vec<(DbRef, (u16, u32, u32))>,
}

impl Stores {
    /// @PLN196 — upgrade the store in `slot`, written under `old` with its root at `old_root`
    /// (an id in `old`), into a fresh store rooted at this program's type `new_root`, and
    /// answer the new store's slot.  The caller owns the new slot; `slot` is left exactly as
    /// it was, whatever the answer.
    ///
    /// `whole_copies` lets an unchanged record or collection be copied as it is (the fast
    /// path); off, everything but a leaf is carried through the old description.
    ///
    /// # Errors
    /// The first refusal of the upgrade map, or the reason the conversion could not finish (a
    /// duplicate under a new key, an old layout this build cannot represent) — with nothing
    /// left allocated.
    pub fn upgrade_store(
        &mut self,
        slot: u16,
        old: &LayoutDesc,
        old_root: u16,
        new_root: u16,
        self_slot: Option<u16>,
        whole_copies: bool,
    ) -> Result<u16, String> {
        let new = self.layout_descriptor(&[new_root]);
        let map = UpgradeMap::derive_with(old, old_root, &new, new_root, whole_copies);
        if let Some(r) = map.refusals.first() {
            return Err(format!("{}: {}", r.path, r.reason));
        }
        let keep = self.types_len();
        let result = self.materialize_layout(old, old_root).and_then(|old_ids| {
            let mut run = Run {
                map: &map,
                old_ids,
                old,
                self_slot,
                rec_map: std::collections::HashMap::new(),
                fixups: Vec::new(),
            };
            self.upgrade_into_new_store(&mut run, slot, new_root)
        });
        // The appended rows go, whatever happened: nothing outside this call names them.
        self.rollback_types_to(keep);
        result
    }

    /// Append the types of `old` reachable from `root` to the type table, and answer where
    /// each landed.  The base types map onto the seed rows every table shares.
    fn materialize_layout(
        &mut self,
        old: &LayoutDesc,
        root: u16,
    ) -> Result<BTreeMap<u16, u16>, String> {
        let closure = old.closure_of(root);
        let mut ids: BTreeMap<u16, u16> = BTreeMap::new();
        let mut next = self.types_len();
        for (id, node) in &closure.nodes {
            if let LayoutNode::Base(k) = node {
                let b = base_id(*k);
                if self.types[b as usize].name != closure.names[id] {
                    return Err(format!(
                        "the base type `{}` is not where this build keeps it",
                        closure.names[id]
                    ));
                }
                ids.insert(*id, b);
            } else {
                ids.insert(*id, next);
                next += 1;
            }
        }
        let re = |k: u16| -> u16 {
            if k == u16::MAX {
                k
            } else {
                ids.get(&k).copied().unwrap_or(u16::MAX)
            }
        };
        let fields = |fs: &[crate::database::LayoutField]| -> Vec<Field> {
            fs.iter()
                .map(|f| {
                    Field::from_stored(
                        f.name.clone(),
                        re(f.content),
                        f.position,
                        Field::default_to_wire(None),
                        f.nullable,
                        re(f.target),
                        f.group.clone(),
                    )
                })
                .collect()
        };
        for (id, node) in &closure.nodes {
            let parts = match node {
                LayoutNode::Base(_) => continue,
                LayoutNode::Record(fs) => Parts::Struct(fields(fs)),
                LayoutNode::EnumValue(t, fs) => Parts::EnumValue(*t, fields(fs)),
                LayoutNode::Choices(vs) => {
                    Parts::Enum(vs.iter().map(|(t, n)| (re(*t), n.clone())).collect())
                }
                LayoutNode::Byte { start, nullable } => Parts::Byte(*start, *nullable),
                LayoutNode::Short { start, nullable } => Parts::Short(*start, *nullable),
                LayoutNode::Int { start, nullable } => Parts::Int(*start, *nullable),
                LayoutNode::ShortRaw { start, nullable } => Parts::ShortRaw(*start, *nullable),
                LayoutNode::IntRaw { start, nullable } => Parts::IntRaw(*start, *nullable),
                LayoutNode::Vector(e) => Parts::Vector(re(*e)),
                LayoutNode::Array(e) => Parts::Array(re(*e)),
                LayoutNode::ChildRec(e) => Parts::ChildRec(re(*e)),
                LayoutNode::Ref => Parts::DbRef,
                LayoutNode::Iterated(it) => match it {
                    Iterated::Sorted { elem, keys } => Parts::Sorted(re(*elem), keys.clone()),
                    Iterated::Ordered { elem, keys } => Parts::Ordered(re(*elem), keys.clone()),
                    Iterated::Hash { elem, keys } => Parts::Hash(re(*elem), keys.clone()),
                    Iterated::Index { elem, keys } => Parts::Index(re(*elem), keys.clone()),
                    Iterated::Radix { elem, keys } => Parts::Radix(re(*elem), keys.clone()),
                    Iterated::Trie { elem, key } => Parts::Trie(re(*elem), *key),
                },
                LayoutNode::FlatArray { .. } => {
                    return Err("a browser delivery node is not a store layout".to_string());
                }
            };
            let size = closure.sizes[id];
            let linked = closure.linked.contains(id)
                || closure.nodes.values().any(|n| {
                    matches!(n, LayoutNode::Array(e) | LayoutNode::Iterated(Iterated::Ordered { elem: e, .. }) if e == id)
                });
            let row = Type::from_stored(
                closure.names[id].clone(),
                parts,
                Vec::new(),
                false,
                linked,
                size,
                u8::try_from(size.clamp(1, 8)).unwrap_or(8),
                Vec::new(),
            );
            debug_assert_eq!(usize::from(ids[id]), self.types.len());
            self.types.push(row);
        }
        for id in ids.values() {
            if usize::from(*id) > 6 {
                self.determine_keys_for(usize::from(*id));
            }
        }
        // The rows must BE the old layout: re-rendered, they give back the old dump of the
        // root's closure, line for line.
        let written = closure.render_dump();
        let rebuilt = self.layout_dump(&[ids[&root]]);
        if written != rebuilt {
            let at = written
                .lines()
                .zip(rebuilt.lines())
                .find(|(a, b)| a != b)
                .map_or_else(String::new, |(a, b)| format!(": `{a}` came back as `{b}`"));
            return Err(format!(
                "this build cannot represent the store's old layout{at}"
            ));
        }
        Ok(ids)
    }

    /// Claim the new store, convert the root into it, and answer its slot; on a failure the
    /// new store is released.
    fn upgrade_into_new_store(
        &mut self,
        run: &mut Run,
        slot: u16,
        new_root: u16,
    ) -> Result<u16, String> {
        use crate::store::PRIMARY;
        let size = u32::from(self.size(new_root));
        let words = (8 + size).div_ceil(8).max(2);
        let mut dst = crate::store::Store::new_in_use(words + PRIMARY + 1);
        let root = dst.claim(words);
        if root != PRIMARY {
            return Err("a fresh store did not hand out its first record".to_string());
        }
        dst.zero_fill(root);
        let dst_slot = self.adopt_store(dst);
        let from = DbRef {
            store_nr: slot,
            rec: PRIMARY,
            pos: 8,
        };
        let to = DbRef {
            store_nr: dst_slot,
            rec: PRIMARY,
            pos: 8,
        };
        // The root record's back-pointer word travels with it.
        let back = self.allocations[slot as usize].get_u32_raw(PRIMARY, 4);
        self.allocations[dst_slot as usize].set_u32_raw(PRIMARY, 4, back);
        run.rec_map.insert(PRIMARY, to);
        let converted = self
            .convert(run, 0, &from, &to, Container::At(to, new_root))
            .and_then(|()| self.repoint_references(run, slot, dst_slot));
        match converted {
            Ok(()) => {
                self.allocations[dst_slot as usize].set_known_type(new_root);
                Ok(dst_slot)
            }
            Err(e) => {
                let gone = self.take_store(dst_slot);
                drop(gone);
                self.release_slot(dst_slot);
                Err(e)
            }
        }
    }

    /// `@FR-L-Ref-Slot` — write every converted reference, now that each record it can name
    /// exists in the new store.  A reference names the old store by its recorded slot and a
    /// record by its old number; it is written through `set_db_ref`, so the record it names
    /// keeps its place if it is ever removed (`@FR-H-Removed`).
    fn repoint_references(
        &mut self,
        run: &mut Run,
        old_slot: u16,
        dst_slot: u16,
    ) -> Result<(), String> {
        for (holder, (sn, rec, pos)) in std::mem::take(&mut run.fixups) {
            let target = if self.reference_removed(old_slot, rec) {
                DbRef {
                    store_nr: u16::MAX,
                    rec: 0,
                    pos: 0,
                }
            } else {
                if run.self_slot != Some(sn) {
                    return Err(if run.self_slot.is_none() {
                        "the store holds references and does not record which store they name"
                            .to_string()
                    } else {
                        "the store holds a reference into another store".to_string()
                    });
                }
                if pos != 8 {
                    return Err(
                        "a reference names a field inside a record, which an upgrade does not \
                         carry yet"
                            .to_string(),
                    );
                }
                let Some(new) = run.rec_map.get(&rec) else {
                    return Err("a reference names a record the store no longer holds".to_string());
                };
                DbRef {
                    store_nr: dst_slot,
                    rec: new.rec,
                    pos: new.pos,
                }
            };
            self.set_db_ref(&holder, 0, target);
        }
        Ok(())
    }

    /// The field list of old description node `old`.
    fn old_fields(run: &Run, old: u16) -> Vec<crate::database::LayoutField> {
        match run.old.nodes.get(&old) {
            Some(LayoutNode::Record(fs) | LayoutNode::EnumValue(_, fs)) => fs.clone(),
            _ => Vec::new(),
        }
    }

    /// Convert the value at `from` (old layout) through pair `pair` into `to` (new layout).
    /// `into` says where a collection's elements are filed: the record and field that hold
    /// it, or the store root.
    fn convert(
        &mut self,
        run: &mut Run,
        pair: usize,
        from: &DbRef,
        to: &DbRef,
        into: Container,
    ) -> Result<(), String> {
        let p = run.map.pairs[pair].clone();
        let old_tp = run.old_ids[&p.old];
        let new_tp = p.new;
        match &p.how {
            How::Same => {
                let size = u32::from(self.size(new_tp));
                self.copy_block(from, to, size);
                self.copy_claims(from, to, new_tp);
                Ok(())
            }
            How::Widen => {
                self.widen(from, old_tp, to, new_tp);
                Ok(())
            }
            How::Reference => {
                let st = &self.allocations[from.store_nr as usize];
                let sn = st.get_u32_raw(from.rec, from.pos);
                let (rec, pos) = (
                    st.get_u32_raw(from.rec, from.pos + 4),
                    st.get_u32_raw(from.rec, from.pos + 8),
                );
                if sn == u32::from(u16::MAX) && rec == 0 {
                    self.set_db_ref(
                        to,
                        0,
                        DbRef {
                            store_nr: u16::MAX,
                            rec: 0,
                            pos: 0,
                        },
                    );
                } else {
                    let sn = u16::try_from(sn).unwrap_or(u16::MAX - 1);
                    run.fixups.push((*to, (sn, rec, pos)));
                }
                Ok(())
            }
            How::Variants { to: map, pairs } => {
                // The discriminant byte holds `index + 1`; 0 is null.
                let raw = self.allocations[from.store_nr as usize].get_byte(from.rec, from.pos, 0);
                let at = usize::try_from(raw - 1).ok();
                let target = at.and_then(|i| map.get(i).copied().flatten());
                match (at, target) {
                    (Some(i), Some(j)) => {
                        if let Some(vp) = pairs.get(i).copied().flatten() {
                            // A struct-enum: the variant record writes its own tag.
                            self.convert(run, vp, from, to, into)
                        } else {
                            self.allocations[to.store_nr as usize].set_byte(
                                to.rec,
                                to.pos,
                                0,
                                i32::from(j) + 1,
                            );
                            Ok(())
                        }
                    }
                    // Null, or a variant the new program removed: null.
                    _ => {
                        self.allocations[to.store_nr as usize].set_byte(to.rec, to.pos, 0, 0);
                        Ok(())
                    }
                }
            }
            How::Record(plan) => {
                let of = Self::old_fields(run, p.old);
                for (i, from_field) in plan.iter().enumerate() {
                    let (npos, ncontent, nname) = match &self.types[new_tp as usize].parts {
                        Parts::Struct(fs) | Parts::EnumValue(_, fs) => {
                            (fs[i].position, fs[i].content, fs[i].name.clone())
                        }
                        _ => return Err("a record pair names a type with no fields".to_string()),
                    };
                    let field_to = DbRef {
                        store_nr: to.store_nr,
                        rec: to.rec,
                        pos: to.pos + u32::from(npos),
                    };
                    match from_field {
                        FieldFrom::Old { field, pair } => {
                            let field_from = DbRef {
                                store_nr: from.store_nr,
                                rec: from.rec,
                                pos: from.pos + u32::from(of[*field].position),
                            };
                            self.convert(
                                run,
                                *pair,
                                &field_from,
                                &field_to,
                                Container::Field(*to, new_tp, i as u16),
                            )?;
                        }
                        // Filled by filing through the group's source member.
                        FieldFrom::Group { .. } => {}
                        FieldFrom::Default => {
                            self.write_absent_value(ncontent, new_tp, i as u16, &field_to);
                        }
                        FieldFrom::Layout => {
                            if nname == "enum"
                                && let Parts::EnumValue(tag, _) = self.types[new_tp as usize].parts
                            {
                                self.allocations[to.store_nr as usize].set_byte(
                                    field_to.rec,
                                    field_to.pos,
                                    0,
                                    i32::from(tag),
                                );
                            }
                        }
                    }
                }
                Ok(())
            }
            How::Elements { elem } => self.convert_elements(run, *elem, from, old_tp, into),
        }
    }

    /// Read a number through the old encoding and write it through the new one.
    fn widen(&mut self, from: &DbRef, old_tp: u16, to: &DbRef, new_tp: u16) {
        let st = &self.allocations[from.store_nr as usize];
        if matches!(self.types[old_tp as usize].parts, Parts::Base) && old_tp == 2 {
            let v = st.get_single(from.rec, from.pos);
            self.allocations[to.store_nr as usize].set_float(to.rec, to.pos, f64::from(v));
            return;
        }
        let null = self
            .narrow_is_null(st, from.rec, from.pos, old_tp)
            .unwrap_or(false);
        let v: i64 = if null {
            i64::MIN
        } else {
            match self.types[old_tp as usize].parts {
                Parts::Byte(start, _) => i64::from(st.get_byte(from.rec, from.pos, start)),
                Parts::Short(start, _) => i64::from(st.get_short(from.rec, from.pos, start)),
                Parts::ShortRaw(start, _) => {
                    i64::from(st.get_short_full(from.rec, from.pos, start))
                }
                Parts::Int(..) => i64::from(st.get_i32_raw(from.rec, from.pos)),
                Parts::IntRaw(..) => i64::from(st.get_u32_raw(from.rec, from.pos)),
                _ => st.get_long(from.rec, from.pos),
            }
        };
        if !self.write_narrow_value(new_tp, v, to) {
            self.allocations[to.store_nr as usize].set_long(to.rec, to.pos, v);
        }
    }

    /// Does a value of `tp` hold a stored `reference<T>` anywhere in its closure?
    pub(super) fn holds_stored_refs(&self, tp: u16) -> bool {
        tp != u16::MAX
            && self
                .layout_closure(&[tp])
                .iter()
                .any(|t| matches!(self.types[*t as usize].parts, Parts::DbRef))
    }

    /// Every slot holding a stored `reference<T>` in the value of type `tp` at `at`, each
    /// once.  A record a group's members share is reached through each member; its slots are
    /// listed once, because rewriting one twice would read the first rewrite as a reference
    /// into another store.
    pub(super) fn stored_ref_slots(&self, at: &DbRef, tp: u16) -> Vec<DbRef> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        self.collect_ref_slots(at, tp, &mut out, &mut seen);
        out
    }

    fn collect_ref_slots(
        &self,
        at: &DbRef,
        tp: u16,
        out: &mut Vec<DbRef>,
        seen: &mut std::collections::HashSet<(u32, u32, u16)>,
    ) {
        if tp == u16::MAX || !seen.insert((at.rec, at.pos, tp)) {
            return;
        }
        let st = &self.allocations[at.store_nr as usize];
        match &self.types[tp as usize].parts {
            Parts::DbRef => out.push(*at),
            Parts::Struct(fs) | Parts::EnumValue(_, fs) => {
                for f in fs {
                    if f.position != u16::MAX && f.content != u16::MAX {
                        let fat = DbRef {
                            store_nr: at.store_nr,
                            rec: at.rec,
                            pos: at.pos + u32::from(f.position),
                        };
                        self.collect_ref_slots(&fat, f.content, out, seen);
                    }
                }
            }
            Parts::Enum(vs) => {
                let raw = st.get_byte(at.rec, at.pos, 0);
                if let Some((v, _)) = usize::try_from(raw - 1).ok().and_then(|i| vs.get(i)) {
                    self.collect_ref_slots(at, *v, out, seen);
                }
            }
            Parts::ChildRec(c) => {
                let cur = st.get_u32_raw(at.rec, at.pos);
                if cur != 0 && cur != DbRef::ABSENT_REC {
                    let child = DbRef {
                        store_nr: at.store_nr,
                        rec: cur,
                        pos: 8,
                    };
                    self.collect_ref_slots(&child, *c, out, seen);
                }
            }
            _ => {
                let elem = self.content(tp);
                if elem != u16::MAX
                    && let Some(es) = self.old_elements(at, tp)
                {
                    for e in es {
                        self.collect_ref_slots(&e, elem, out, seen);
                    }
                }
            }
        }
    }

    /// `@FR-L-Ref-Slot`, loft#2099 — after a whole store image was put in `slot`, make its stored references
    /// name it there.  A reference stores its target's slot number, and the image keeps the
    /// one it had where it was written (`recorded`, from its sidecar): those become `slot`.
    ///
    /// # Errors
    /// A reference into any OTHER store — that store is not part of the image, so nothing
    /// here can name it — and a store holding references whose sidecar did not record its
    /// slot, whose references cannot be told apart.  Nothing is rewritten then.
    pub(crate) fn rebase_stored_refs(
        &mut self,
        slot: u16,
        recorded: Option<u16>,
    ) -> Result<(), String> {
        let tp = self.allocations[slot as usize].known_type;
        if !self.holds_stored_refs(tp) {
            return Ok(());
        }
        let root = DbRef {
            store_nr: slot,
            rec: crate::store::PRIMARY,
            pos: 8,
        };
        let slots = self.stored_ref_slots(&root, tp);
        let st = &self.allocations[slot as usize];
        let read = |r: &DbRef| u16::try_from(st.get_u32_raw(r.rec, r.pos)).unwrap_or(u16::MAX);
        let live: Vec<DbRef> = slots
            .into_iter()
            .filter(|r| read(r) != u16::MAX || st.get_u32_raw(r.rec, r.pos + 4) != 0)
            .collect();
        if live.is_empty() {
            return Ok(());
        }
        let Some(from) = recorded else {
            return Err(
                "it holds references, and it was written by a version of loft that did not \
                 record which of them point into the store itself"
                    .to_string(),
            );
        };
        if live.iter().any(|r| read(r) != from) {
            return Err(
                "it holds a reference into another store, which is not part of the file"
                    .to_string(),
            );
        }
        let st = &mut self.allocations[slot as usize];
        for r in live {
            st.set_u32_raw(r.rec, r.pos, u32::from(slot));
        }
        Ok(())
    }

    /// The elements of the old collection at `from`, as references to their payloads, in the
    /// order the new collection should receive them.
    fn old_elements(&self, from: &DbRef, old_tp: u16) -> Option<Vec<DbRef>> {
        let st = &self.allocations[from.store_nr as usize];
        let cur = st.get_u32_raw(from.rec, from.pos);
        if cur == DbRef::ABSENT_REC {
            return None;
        }
        if cur == 0 {
            return Some(Vec::new());
        }
        let at = |rec: u32, pos: u32| DbRef {
            store_nr: from.store_nr,
            rec,
            pos,
        };
        Some(match &self.types[old_tp as usize].parts {
            Parts::Vector(e) | Parts::Sorted(e, _) => {
                let size = u32::from(self.size(*e));
                let len = st.get_u32_raw(cur, 4);
                (0..len).map(|i| at(cur, 8 + size * i)).collect()
            }
            Parts::Array(_) | Parts::Ordered(..) => {
                let len = st.get_u32_raw(cur, 4);
                (0..len)
                    .map(|i| at(st.get_u32_raw(cur, 8 + 4 * i), 8))
                    .collect()
            }
            Parts::Hash(..) => {
                // In the source's own record order, as `copy_claims_hash_body` keeps it.
                let mut e: Vec<DbRef> = crate::hash::entries(from, &self.allocations)
                    .into_iter()
                    .map(|(d, _)| d)
                    .collect();
                e.sort_by_key(|d| (d.rec, d.pos));
                e
            }
            Parts::Index(..) => crate::btree::records(from, &self.allocations)
                .into_iter()
                .map(|r| at(r, 8))
                .collect(),
            Parts::Radix(..) => crate::radix_db::records(from, &self.allocations)
                .into_iter()
                .map(|r| at(r, 8))
                .collect(),
            Parts::Trie(..) => crate::trie_db::records(from, &self.allocations)
                .into_iter()
                .map(|r| at(r, 8))
                .collect(),
            _ => return Some(Vec::new()),
        })
    }

    /// How many elements the collection at `slot` of type `tp` holds.
    fn collection_count(&self, slot: &DbRef, tp: u16) -> u32 {
        match self.types[tp as usize].parts {
            Parts::Vector(_) | Parts::Sorted(..) | Parts::Array(_) | Parts::Ordered(..) => {
                crate::vector::length_vector(slot, &self.allocations)
            }
            Parts::Hash(..) => crate::hash::count(slot, &self.allocations),
            Parts::Index(..) => crate::btree::count(slot, &self.allocations),
            Parts::Radix(..) => crate::radix_db::count(slot, &self.allocations),
            Parts::Trie(..) => crate::trie_db::count(slot, &self.allocations),
            _ => 0,
        }
    }

    /// Rebuild the collection at `from` element by element into the new store, filing each
    /// element through the new program's own allocate-and-file pair.
    fn convert_elements(
        &mut self,
        run: &mut Run,
        elem: usize,
        from: &DbRef,
        old_tp: u16,
        into: Container,
    ) -> Result<(), String> {
        // `parent`/`parent_tp`/`field` are what `record_new` takes; `slot` is the collection's
        // own word in the new store, which the count reads.
        let (parent, parent_tp, field, slot) = match into {
            Container::Field(rec, tp, f) => (rec, tp, f, self.field_ref(&rec, tp, f)),
            Container::At(slot, tp) => (slot, tp, u16::MAX, slot),
        };
        let coll_tp = self.sub_record_type(parent_tp, field);
        let Some(elements) = self.old_elements(from, old_tp) else {
            self.allocations[slot.store_nr as usize].set_u32_raw(
                slot.rec,
                slot.pos,
                DbRef::ABSENT_REC,
            );
            return Ok(());
        };
        for src in elements {
            let before = self.collection_count(&slot, coll_tp);
            let rec = self.record_new(&parent, parent_tp, field);
            self.zero_element(&rec, coll_tp);
            if src.pos == 8 {
                run.rec_map.insert(src.rec, rec);
            }
            let elem_tp = run.map.pairs[elem].new;
            self.convert(run, elem, &src, &rec, Container::At(rec, elem_tp))?;
            self.record_finish(&parent, &rec, parent_tp, field);
            if self.collection_count(&slot, coll_tp) == before {
                let key = crate::keys::get_key(
                    &rec,
                    &self.allocations,
                    &self.types[coll_tp as usize].keys,
                )
                .iter()
                .map(|c| match c {
                    crate::keys::Content::Long(v) => v.to_string(),
                    crate::keys::Content::Float(v) => v.to_string(),
                    crate::keys::Content::Single(v) => v.to_string(),
                    crate::keys::Content::Str(t) => format!("\"{}\"", t.str()),
                })
                .collect::<Vec<_>>()
                .join(", ");
                let place = match into {
                    Container::Field(_, tp, f) => match &self.types[tp as usize].parts {
                        Parts::Struct(fs) | Parts::EnumValue(_, fs) => {
                            format!("{}.{}", self.types[tp as usize].name, fs[f as usize].name)
                        }
                        _ => self.types[coll_tp as usize].name.clone(),
                    },
                    Container::At(..) => self.types[coll_tp as usize].name.clone(),
                };
                return Err(format!(
                    "{place}: two stored records share the key [{key}] under the new \
                     definition, so one of them would be lost"
                ));
            }
        }
        Ok(())
    }

    /// Clear a freshly allocated element's payload, so a field the plan leaves to the layout
    /// reads as zero rather than as whatever the block held.
    fn zero_element(&mut self, rec: &DbRef, coll_tp: u16) {
        let elem = self.content(coll_tp);
        if elem == u16::MAX {
            return;
        }
        let size = self.size(elem);
        let st = &mut self.allocations[rec.store_nr as usize];
        for b in 0..u32::from(size) {
            st.set_byte(rec.rec, rec.pos + b, 0, 0);
        }
    }
}

/// What [`Stores::upgrade_file`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileUpgrade {
    /// The store already has this program's layout; nothing was written.
    Current,
    /// The store was upgraded; the old file was kept at `kept`.
    Upgraded {
        kept: std::path::PathBuf,
        changes: Vec<String>,
    },
}

#[cfg(feature = "mmap")]
impl Stores {
    /// @PLN196 step 7 — upgrade the store FILE at `path` to this program's type `new_root`
    /// (or, when `None`, the type of the stored root's name): the new image and its sidecar
    /// are written beside the store, then swapped in by rename, and the old pair is kept as
    /// `<path>.old`.  A refusal, or a failure anywhere before the swap, leaves the store and
    /// its sidecar byte for byte as they were.
    ///
    /// # Errors
    /// Why the store cannot be upgraded: no recorded layout, a layout this build cannot read,
    /// a refusal of the upgrade map, or an I/O failure.
    pub fn upgrade_file(
        &mut self,
        path: &std::path::Path,
        new_root: Option<u16>,
    ) -> Result<FileUpgrade, String> {
        use crate::schema_sidecar::{LayoutIdentity, dschema_path};
        let side = dschema_path(path);
        let text = crate::file_access::read_to_string(&side).map_err(|_| {
            format!(
                "{} has no recorded layout ({} is missing), so what it holds cannot be known",
                path.display(),
                side.display()
            )
        })?;
        let id = LayoutIdentity::from_sidecar(&text)
            .ok_or_else(|| format!("{} is not a layout file", side.display()))?;
        let mut old = LayoutDesc::from_dump(&id.dump)?;
        old.apply_facts(&id.facts)?;
        // Version 2 names the root; version 1 leaves it to be read from the closure, where
        // it is the one type nothing else refers to.
        let named = old
            .names
            .iter()
            .find(|(_, n)| !id.root.is_empty() && **n == id.root)
            .map(|(k, _)| *k);
        let roots = named.map_or_else(|| old.unreferenced(), |r| vec![r]);
        let [old_root] = roots[..] else {
            return Err(format!(
                "{} does not say which of its types is the store's root",
                side.display()
            ));
        };
        let old_name = old.names[&old_root].clone();
        let new_root = match new_root {
            Some(r) => r,
            None => *self.names.get(&old_name).ok_or_else(|| {
                format!("the store holds a `{old_name}`, which this program does not define")
            })?,
        };
        let current = LayoutIdentity::of(self, &[new_root]);
        if current.layout_hash == id.layout_hash && current.dump == id.dump {
            return Ok(FileUpgrade::Current);
        }
        let new = self.layout_descriptor(&[new_root]);
        let changes = UpgradeMap::derive(&old, old_root, &new, new_root).changes;
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let store = crate::store::Store::from_bytes(&bytes)
            .ok_or_else(|| format!("{} is not a store image", path.display()))?;
        let old_slot = self.adopt_store(store);
        let result = self.upgrade_store(old_slot, &old, old_root, new_root, id.slot, true);
        let gone = self.take_store(old_slot);
        drop(gone);
        self.release_slot(old_slot);
        let new_slot = result?;
        // The new image's references name the slot it was built in.
        let mut current = current;
        current.slot = Some(new_slot);
        let image = self.store_image(new_slot);
        let gone = self.take_store(new_slot);
        drop(gone);
        self.release_slot(new_slot);
        let image = image.ok_or_else(|| "the upgraded store did not form an image".to_string())?;
        // Beside, then swapped: the old pair is renamed away first, so at every moment one
        // complete store with its own sidecar sits at `path`, or the old one does.
        let with = |suffix: &str| {
            let mut p = path.as_os_str().to_owned();
            p.push(suffix);
            std::path::PathBuf::from(p)
        };
        let (next, kept) = (with(".upgrade"), with(".old"));
        let io = |e: std::io::Error| e.to_string();
        super::allocation::write_image_atomic(&next, &image).map_err(io)?;
        current.write_beside(&next).map_err(io)?;
        std::fs::rename(path, &kept).map_err(io)?;
        std::fs::rename(&side, dschema_path(&kept)).map_err(io)?;
        std::fs::rename(&next, path).map_err(io)?;
        std::fs::rename(dschema_path(&next), &side).map_err(io)?;
        Ok(FileUpgrade::Upgraded { kept, changes })
    }

    /// The byte image of the store in `slot`, ending at its last live record.
    fn store_image(&self, slot: u16) -> Option<Vec<u8>> {
        let s = &self.allocations[slot as usize];
        let words = s.capacity_words();
        let raw = unsafe { std::slice::from_raw_parts(s.base_ptr(), (words as usize) * 8) };
        let live_end = super::allocation::store_image_live_end(raw, words).unwrap_or(words);
        super::allocation::build_padded_store_image(
            raw,
            words,
            live_end.max(crate::store::MIN_BOUND_WORDS),
        )
    }
}

/// Where a value's collection elements are filed, in the terms `record_new` takes.
#[derive(Clone, Copy)]
enum Container {
    /// The collection IS the value at `.0`, of type `.1`: the store's root, or an element of
    /// an outer collection that is itself a collection.
    At(DbRef, u16),
    /// Field `.2` of the record `.0`, whose type is `.1`.
    Field(DbRef, u16, u16),
}
