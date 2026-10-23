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
    /// The fill function of each (new type, field name) whose default is not a constant.
    fills: &'m Fills,
    /// Every record that still owes one of those fills, and every routine call, in
    /// conversion order.
    pending: PendingFills,
    /// Owe every added field the converter could not fill itself to the running program's
    /// drain ([`Owed`]), instead of to `fills`.
    owe: bool,
    /// @PLN196 step 9 — the `upgrade` routine of each record type that runs for this store;
    /// empty when none does (the store is not older than this program, or none is written).
    routines: &'m Routines,
    /// The release the store records, for the routine's `from`.
    from: &'m str,
    /// The store the routines' `old` records and their `Release` live in, made on first use
    /// and released once every routine ran; its first record is the `Release`.
    scratch: Option<u16>,
    /// Filling a routine's `old` record: its elements are copies, never the record a stored
    /// reference names.
    projecting: bool,
}

/// @PLN196 step 9 — the `upgrade(from: Release, old: O, item: T)` routine the programmer
/// wrote for record type `T` (the key of [`Routines`]): it runs on every record of `T` that a
/// store older than this program's release is converted with, after the fields carried by
/// name and the added fields' defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routine {
    /// The type of `old`: the stored fields the routine reads, filled by name.
    pub old: u16,
    /// The type of `from`, the stdlib `Release`.
    pub version: u16,
    /// The function a caller without the drain calls it through (`__upgrade_<T>`).
    pub name: String,
}

/// Per record type, its `upgrade` routine.
pub type Routines = BTreeMap<u16, Routine>;

/// The key the drain matches an owed routine call by: a field position no record reaches.
#[must_use]
pub fn routine_key(tp: u16) -> i64 {
    owed_key(tp, u16::MAX)
}

/// What a conversion left for the program to run on the new store, in this order.
#[derive(Debug, Clone, PartialEq)]
pub enum Pending {
    /// Fill one added field of `rec` with its default through function `name`.
    Fill { rec: DbRef, name: String },
    /// Run the `upgrade` routine `name` on `item`, with its `old` record and `from`.
    Routine {
        item: DbRef,
        old: DbRef,
        from: DbRef,
        name: String,
    },
}

/// @PLN196 step 5 — per (new type, field name), the function that writes that field's
/// default into a record (`Parser::synth_upgrade_fills`): a default that is not a constant
/// is evaluated by the program, not by the store layer.
pub type Fills = BTreeMap<(u16, String), String>;

/// What an upgrade left for the program to run, in order: each record owing an expression
/// default with the function that fills it, and each routine call (`host::Program`).
pub type PendingFills = Vec<Pending>;

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
        let (new_slot, pending) = self.upgrade_store_with(
            slot,
            old,
            old_root,
            new_root,
            self_slot,
            whole_copies,
            &Fills::new(),
        )?;
        debug_assert!(pending.is_empty(), "no fills were offered");
        Ok(new_slot)
    }

    /// [`Stores::upgrade_store`] with the program's fill functions: an added field whose
    /// default is not a constant takes its direct write here and is listed in the answer,
    /// with the function that fills it, for the caller to run.
    ///
    /// # Errors
    /// As [`Stores::upgrade_store`].
    #[allow(clippy::too_many_arguments)]
    pub fn upgrade_store_with(
        &mut self,
        slot: u16,
        old: &LayoutDesc,
        old_root: u16,
        new_root: u16,
        self_slot: Option<u16>,
        whole_copies: bool,
        fills: &Fills,
    ) -> Result<(u16, PendingFills), String> {
        let (new_slot, pending, scratch) = self.upgrade_store_run(
            slot,
            old,
            old_root,
            new_root,
            self_slot,
            whole_copies,
            fills,
            false,
            (&Routines::new(), ""),
        )?;
        debug_assert!(scratch.is_none(), "no routine was offered");
        Ok((new_slot, pending))
    }

    /// [`Stores::upgrade_store_with`], owing the added fields to [`Owed`] when `owe` is set,
    /// and running `routines` with the release `from` the store records.  Also answers the
    /// scratch store the routines' records are in, which the caller releases after them.
    #[allow(clippy::too_many_arguments)]
    fn upgrade_store_run(
        &mut self,
        slot: u16,
        old: &LayoutDesc,
        old_root: u16,
        new_root: u16,
        self_slot: Option<u16>,
        whole_copies: bool,
        fills: &Fills,
        owe: bool,
        (routines, from): (&Routines, &str),
    ) -> Result<(u16, PendingFills, Option<u16>), String> {
        let map = self.upgrade_map(old, old_root, new_root, whole_copies, routines);
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
                fills,
                pending: Vec::new(),
                owe,
                routines,
                from,
                scratch: None,
                projecting: false,
            };
            let built = self.upgrade_into_new_store(&mut run, slot, new_root);
            match built {
                Ok(new_slot) => Ok((new_slot, run.pending, run.scratch)),
                Err(e) => {
                    if let Some(sc) = run.scratch {
                        self.release_scratch(sc);
                    }
                    Err(e)
                }
            }
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
        for (id, node) in &closure.nodes {
            let Some(parts) = layout_parts(node, &ids)? else {
                continue;
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
        // DEFINED: the upgraded store is written back as an image (`Store::defined`).
        let mut dst = crate::store::Store::new_in_use_defined(words + PRIMARY + 1);
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
                self.convert_reference(run, from, to);
                Ok(())
            }
            How::Variants { to: map, pairs } => {
                self.convert_variant(run, map, pairs, from, to, into)
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
                            if let Some(f) = run.fills.get(&(new_tp, nname.clone())) {
                                run.pending.push(Pending::Fill {
                                    rec: *to,
                                    name: f.clone(),
                                });
                            } else if run.owe {
                                self.owed.fills.push((
                                    *to,
                                    owed_key(new_tp, i as u16),
                                    DbRef::NULL,
                                ));
                            }
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
                if let Some(&proj) = run.map.projections.get(&new_tp) {
                    self.owe_routine(run, proj, from, to, new_tp)?;
                }
                Ok(())
            }
            How::Elements { elem } => self.convert_elements(run, *elem, from, old_tp, into),
        }
    }

    /// An enum value through its variant map: the discriminant byte holds `index + 1`, and 0
    /// is null.  A struct-enum's variant record converts through its own pair and writes its
    /// own tag; a variant the new program removed becomes null.
    fn convert_variant(
        &mut self,
        run: &mut Run,
        map: &[Option<u8>],
        pairs: &[Option<usize>],
        from: &DbRef,
        to: &DbRef,
        into: Container,
    ) -> Result<(), String> {
        let raw = self.allocations[from.store_nr as usize].get_byte(from.rec, from.pos, 0);
        let at = usize::try_from(raw - 1).ok();
        if let (Some(i), Some(j)) = (at, at.and_then(|i| map.get(i).copied().flatten())) {
            if let Some(vp) = pairs.get(i).copied().flatten() {
                return self.convert(run, vp, from, to, into);
            }
            self.allocations[to.store_nr as usize].set_byte(to.rec, to.pos, 0, i32::from(j) + 1);
        } else {
            self.allocations[to.store_nr as usize].set_byte(to.rec, to.pos, 0, 0);
        }
        Ok(())
    }

    /// A stored reference: null stays null; any other is set aside until every record it can
    /// name exists (`repoint_references`).
    fn convert_reference(&mut self, run: &mut Run, from: &DbRef, to: &DbRef) {
        let st = &self.allocations[from.store_nr as usize];
        let sn = st.get_u32_raw(from.rec, from.pos);
        let rec = st.get_u32_raw(from.rec, from.pos + 4);
        let pos = st.get_u32_raw(from.rec, from.pos + 8);
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
                // An inline table lists its entries in insertion order already; its positions
                // are its buckets, which a seed orders (`(Col-Inline)`).
                if !crate::hash::is_inline(st, cur) {
                    e.sort_by_key(|d| (d.rec, d.pos));
                }
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
        // `(Col-Inline)` — an inline table files an entry by copying it out of its stage, and
        // moves every entry when it grows.  Sized for the whole collection first it never
        // grows here, so an entry stays where it was filed and the references taken to it
        // below — a pending fill, a mapped record — stay true once moved off the stage.
        if matches!(self.types[coll_tp as usize].parts, Parts::Hash(..)) {
            self.reserve_hash(&slot, elements.len() as i64, coll_tp);
        }
        for src in elements {
            let before = self.collection_count(&slot, coll_tp);
            let rec = self.record_new(&parent, parent_tp, field);
            self.zero_element(&rec, coll_tp);
            if src.pos == 8 && !run.projecting {
                run.rec_map.insert(src.rec, rec);
            }
            let elem_tp = run.map.pairs[elem].new;
            let pending_from = run.pending.len();
            let owed_from = self.owed.fills.len();
            self.convert(run, elem, &src, &rec, Container::At(rec, elem_tp))?;
            self.record_finish(&parent, &rec, parent_tp, field);
            let stage = rec;
            let rec = self.filed(&rec);
            if rec != stage {
                // The element was built in a stage and copied into its table: what was taken
                // to the stage now names the entry, at the same offset into it.
                let moved = |d: &mut DbRef| {
                    if d.store_nr == stage.store_nr && d.rec == stage.rec && d.pos >= stage.pos {
                        *d = DbRef {
                            store_nr: rec.store_nr,
                            rec: rec.rec,
                            pos: rec.pos + (d.pos - stage.pos),
                        };
                    }
                };
                for p in &mut run.pending[pending_from..] {
                    match p {
                        Pending::Fill { rec: d, .. } => moved(d),
                        Pending::Routine {
                            item, old, from, ..
                        } => {
                            moved(item);
                            moved(old);
                            moved(from);
                        }
                    }
                }
                // The fills a load OWES its program (`run.owe`) name their records the same way.
                for (d, _, old) in &mut self.owed.fills[owed_from..] {
                    moved(d);
                    moved(old);
                }
                if src.pos == 8
                    && let Some(d) = run.rec_map.get_mut(&src.rec)
                {
                    moved(d);
                }
            }
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

    /// `@FR-L-Upgrade-Old` — fill the `old` record of `item`'s routine from the stored record
    /// at `from` through projection pair `proj`, and owe the routine call: to the drain, or
    /// to the caller's list.  The routine runs after `item`'s fields and its owed defaults.
    fn owe_routine(
        &mut self,
        run: &mut Run,
        proj: usize,
        from: &DbRef,
        item: &DbRef,
        tp: u16,
    ) -> Result<(), String> {
        let Some(r) = run.routines.get(&tp).cloned() else {
            return Ok(());
        };
        let scratch = self.upgrade_scratch(run, r.version);
        let words = (8 + u32::from(self.size(r.old))).div_ceil(8).max(2);
        let rec = self.allocations[scratch as usize].claim(words);
        self.allocations[scratch as usize].zero_fill(rec);
        let old = DbRef {
            store_nr: scratch,
            rec,
            pos: 8,
        };
        let was = std::mem::replace(&mut run.projecting, true);
        let done = self.convert(run, proj, from, &old, Container::At(old, r.old));
        run.projecting = was;
        done?;
        let version = DbRef {
            store_nr: scratch,
            rec: crate::store::PRIMARY,
            pos: 8,
        };
        if run.owe {
            self.owed.fills.push((*item, routine_key(tp), old));
            self.owed.from = Some(version);
        } else {
            run.pending.push(Pending::Routine {
                item: *item,
                old,
                from: version,
                name: r.name,
            });
        }
        Ok(())
    }

    /// The scratch store of `run`'s routines, made on first use with the `Release` of the
    /// release the store records as its first record.
    fn upgrade_scratch(&mut self, run: &mut Run, version_tp: u16) -> u16 {
        if let Some(sc) = run.scratch {
            return sc;
        }
        use crate::store::PRIMARY;
        let words = (8 + u32::from(self.size(version_tp))).div_ceil(8).max(2);
        let mut st = crate::store::Store::new_in_use(words + PRIMARY + 1);
        let root = st.claim(words);
        st.zero_fill(root);
        let (major, minor, patch) = crate::database::version_triple(run.from).unwrap_or((0, 0, 0));
        if let Parts::Struct(fs) = &self.types[version_tp as usize].parts {
            for (f, v) in fs.iter().zip([major, minor, patch]) {
                let pos = 8 + u32::from(f.position);
                if self.size(f.content) == 4 {
                    st.set_int(root, pos, i64::from(v));
                } else {
                    st.set_long(root, pos, i64::from(v));
                }
            }
        }
        st.set_known_type(version_tp);
        let sc = self.adopt_store(st);
        run.scratch = Some(sc);
        sc
    }

    /// Release the scratch store of a conversion's routines.
    pub(crate) fn release_scratch(&mut self, sc: u16) {
        drop(self.take_store(sc));
        self.release_slot(sc);
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

#[cfg_attr(not(feature = "mmap"), allow(dead_code))]
/// An upgrade begun by [`Stores::upgrade_file_begin`]: the new store is built in `new_slot`
/// and nothing is on disk yet.
#[derive(Debug)]
pub struct FileUpgradeJob {
    path: std::path::PathBuf,
    current: crate::schema_sidecar::LayoutIdentity,
    changes: Vec<String>,
    new_slot: u16,
    /// The routines' scratch store, released with the job.
    scratch: Option<u16>,
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

/// The `.dschema` text and the image of the store file at `path`.
fn read_store_file(path: &std::path::Path) -> Result<(String, Vec<u8>), String> {
    let side = crate::schema_sidecar::dschema_path(path);
    let text = crate::file_access::read_to_string(&side).map_err(|_| {
        format!(
            "{} has no recorded layout ({} is missing), so what it holds cannot be known",
            path.display(),
            side.display()
        )
    })?;
    let bytes = crate::file_access::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((text, bytes))
}

/// What a build of this program would do with a world whose layout a sidecar records
/// ([`Stores::world_upgrade_plan`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldPlan {
    /// The layout is this program's: the world is read as it is.
    Same,
    /// The world converts; the changes, one per line of `UpgradeMap::changes`.
    Converts(Vec<String>),
}

/// The running world against this program, every refusal kept ([`Stores::world_upgrade`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldUpgrade {
    /// The release of the build that runs the world.
    pub version: String,
    /// The layout is this program's: nothing converts.
    pub same: bool,
    /// What converts, one line per change.
    pub changes: Vec<String>,
    /// What stops the world from crossing, each with its path from the world's type.
    pub refusals: Vec<crate::database::Refusal>,
}

/// The layout a `.dschema` text records, read back into its description, with the id of the
/// store's root in it.  `what` names the store in an error.
fn recorded_layout(
    sidecar: &str,
    what: &str,
) -> Result<(crate::schema_sidecar::LayoutIdentity, LayoutDesc, u16), String> {
    let id = crate::schema_sidecar::LayoutIdentity::from_sidecar(sidecar)
        .ok_or_else(|| format!("{what}.dschema is not a layout file"))?;
    let mut old = LayoutDesc::from_dump(&id.dump)?;
    old.apply_facts(&id.facts)?;
    // Version 2 names the root; version 1 leaves it to be read from the closure, where it is
    // the one type nothing else refers to.
    let named = old
        .names
        .iter()
        .find(|(_, n)| !id.root.is_empty() && **n == id.root)
        .map(|(k, _)| *k);
    let roots = named.map_or_else(|| old.unreferenced(), |r| vec![r]);
    let [old_root] = roots[..] else {
        return Err(format!(
            "{what}.dschema does not say which of its types is the store's root"
        ));
    };
    Ok((id, old, old_root))
}

/// A store file converted into a new slot by [`Stores::upgrade_image`].
struct Converted {
    /// The new layout, recording the slot the new store's references name.
    current: crate::schema_sidecar::LayoutIdentity,
    changes: Vec<String>,
    new_slot: u16,
    pending: PendingFills,
    /// The routines' scratch store, released after them.
    scratch: Option<u16>,
}

/// The key a drain matches an owed fill by: the new type and the field's position in it.
/// `Parser::synth_upgrade_drain` computes the same key for each fill function it calls.
#[must_use]
pub fn owed_key(tp: u16, field: u16) -> i64 {
    (i64::from(tp) << 16) | i64::from(field)
}

/// @PLN196 step 8 — what a converting load still owes the running program.  The converter
/// fills every added field it can (a constant default, or absence); a default the program
/// must compute is listed here, and the drain the parser puts after every `store_load` and
/// `store_persist_bind` (`Parser::synth_upgrade_drain`) walks the list with
/// [`Stores::owed_next`] and [`Stores::owed_record`], calls each field's fill function, and
/// ends with [`Stores::owed_finish`].
#[derive(Default)]
pub(crate) struct Owed {
    /// Each record owing a fill, with the key of the field it owes; for a routine call
    /// ([`routine_key`]), also its `old` record.
    fills: Vec<(DbRef, i64, DbRef)>,
    /// The `Release` the routine calls are told the store's release with.
    from: Option<DbRef>,
    /// The store the routines' records are in, released by [`Stores::owed_finish`].
    scratch: Option<u16>,
    /// How many of `fills` the drain has taken.
    at: usize,
}

impl Stores {
    /// Convert the store FILE at `path` (and its sidecar) into a new slot under this
    /// program's type `new_root` (or the type of the stored root's name), and answer it;
    /// `None` when the store already has this layout.  Nothing on disk changes.
    ///
    /// # Errors
    /// As [`Stores::upgrade_file`].
    fn upgrade_image(
        &mut self,
        path: &std::path::Path,
        new_root: Option<u16>,
        fills: &Fills,
        owe: bool,
        routines: &Routines,
    ) -> Result<Option<Converted>, String> {
        let (text, bytes) = read_store_file(path)?;
        let what = path.display().to_string();
        self.upgrade_image_from(&text, &bytes, &what, new_root, fills, owe, routines)
    }

    /// [`Stores::upgrade_image`] for a store image already in memory, `sidecar` being the
    /// text of its `.dschema`; `what` names it in a refusal.  `Store::from_bytes` validates
    /// the image's block structure before anything reads it, as the untrusted loaders do.
    ///
    /// `routines` run on a store older than this program's release (@PLN196 step 9); a store
    /// older than the oldest release it reads is refused before anything is read.
    ///
    /// # Errors
    /// As [`Stores::upgrade_file`].
    #[allow(clippy::too_many_arguments)]
    fn upgrade_image_from(
        &mut self,
        sidecar: &str,
        bytes: &[u8],
        what: &str,
        new_root: Option<u16>,
        fills: &Fills,
        owe: bool,
        routines: &Routines,
    ) -> Result<Option<Converted>, String> {
        use crate::schema_sidecar::LayoutIdentity;
        let (id, old, old_root) = recorded_layout(sidecar, what)?;
        self.release().admits(&id.version)?;
        let old_name = old.names[&old_root].clone();
        let new_root = match new_root {
            Some(r) => r,
            None => *self.names.get(&old_name).ok_or_else(|| {
                format!("the store holds a `{old_name}`, which this program does not define")
            })?,
        };
        let current = LayoutIdentity::of(self, &[new_root]);
        if current.layout_hash == id.layout_hash && current.dump == id.dump {
            return Ok(None);
        }
        // A store older than this program goes through the routines; one the program wrote,
        // or a later release wrote, is converted by name alone.
        let routines = &self.routines_for(&id.version, routines);
        let changes = self
            .upgrade_map(&old, old_root, new_root, true, routines)
            .changes;
        let store = crate::store::Store::from_bytes(bytes)
            .ok_or_else(|| format!("{what} is not a store image"))?;
        let old_slot = self.adopt_store(store);
        let result = self.upgrade_store_run(
            old_slot,
            &old,
            old_root,
            new_root,
            id.slot,
            true,
            fills,
            owe,
            (routines, &id.version),
        );
        drop(self.take_store(old_slot));
        self.release_slot(old_slot);
        let (new_slot, pending, scratch) = result.map_err(|e| self.refusal_for(&id.version, e))?;
        // The new image's references name the slot it was built in, and it is now this
        // release's store.
        let mut current = current;
        current.slot = Some(new_slot);
        current.version.clone_from(&self.release().version);
        Ok(Some(Converted {
            current,
            changes,
            new_slot,
            pending,
            scratch,
        }))
    }

    /// @PLN196 step 8 — what a build of THIS program would do with the running world whose
    /// layout `sidecar` records: read it as it is, convert it, or refuse it.  A live rebuild
    /// asks this when it compiles the next instance (`LOFT_CHECK_WORLD`), so a refused change
    /// fails the BUILD and the swap is never offered; the swap then only carries out a plan
    /// already known to hold.  The world's type is found by name.
    ///
    /// # Errors
    /// Why the world cannot be carried into this program: a layout this build cannot read, a
    /// world type this program does not define, or the upgrade map's first refusal.
    pub fn world_upgrade_plan(
        &self,
        sidecar: &str,
        routines: &Routines,
    ) -> Result<WorldPlan, String> {
        let up = self.world_upgrade(sidecar, routines)?;
        if let Some(r) = up.refusals.first() {
            return Err(self.refusal_for(&up.version, format!("{}: {}", r.path, r.reason)));
        }
        Ok(if up.same {
            WorldPlan::Same
        } else {
            WorldPlan::Converts(up.changes)
        })
    }

    /// [`Self::world_upgrade_plan`] with every refusal kept, each with its field path from the
    /// world's type: what an editor shows at the type while the old build keeps running
    /// (`layout-upgrade-refused`, the parser's `check_running_world`).
    ///
    /// # Errors
    /// Why the world cannot even be compared: a layout this build cannot read, a release this
    /// build does not admit, or a world type this program does not define.
    pub fn world_upgrade(
        &self,
        sidecar: &str,
        routines: &Routines,
    ) -> Result<WorldUpgrade, String> {
        let (id, old, old_root) = recorded_layout(sidecar, "the running world")?;
        self.release().admits(&id.version)?;
        let name = old.names[&old_root].clone();
        let new_root = *self.names.get(&name).ok_or_else(|| {
            format!("the running world is a `{name}`, which this build does not define")
        })?;
        let current = crate::schema_sidecar::LayoutIdentity::of(self, &[new_root]);
        if current.layout_hash == id.layout_hash && current.dump == id.dump {
            return Ok(WorldUpgrade {
                version: id.version,
                same: true,
                changes: Vec::new(),
                refusals: Vec::new(),
            });
        }
        // The world records the release of the instance that runs it: the same decision as a
        // load of a store that release wrote (@FR-L-Upgrade-Routine).
        let routines = self.routines_for(&id.version, routines);
        let map = self.upgrade_map(&old, old_root, new_root, true, &routines);
        Ok(WorldUpgrade {
            version: id.version,
            same: false,
            changes: map.changes,
            refusals: map.refusals,
        })
    }

    /// What a store written with the project's baseline layout `old` (accepted as release
    /// `version`) does on its way into this program's `roots`: the changes it goes through and
    /// the refusals that stop it, each refusal's path starting at its root type's name.  The
    /// same decision a load takes, routines included, so `loft layout check` and the parser's
    /// `layout-upgrade-refused` warning cannot disagree with the converter.  A root the
    /// baseline does not hold has no stored data to carry, and is skipped.
    #[must_use]
    pub fn baseline_upgrade(
        &self,
        old: &LayoutDesc,
        version: &str,
        roots: &[u16],
        routines: &Routines,
    ) -> (Vec<String>, Vec<crate::database::Refusal>) {
        let routines = self.routines_for(version, routines);
        let mut changes = Vec::new();
        let mut refusals = Vec::new();
        for &root in roots {
            let name = &self.types[root as usize].name;
            let Some((&old_root, _)) = old.names.iter().find(|(_, n)| *n == name) else {
                continue;
            };
            let map = self.upgrade_map(old, old_root, root, true, &routines);
            changes.extend(map.changes);
            refusals.extend(map.refusals);
        }
        (changes, refusals)
    }

    /// `@FR-L-Upgrade-Routine` — the routines that run on a store whose recorded release is
    /// `written`: every one for a store older than this program (or one that records no
    /// release), none for a store this release or a later one wrote.  The ONE home of that
    /// decision, which the converter, the loaders and a live rebuild's check all ask.
    fn routines_for(&self, written: &str, routines: &Routines) -> Routines {
        if self.release().is_newer_than(written) {
            routines.clone()
        } else {
            Routines::new()
        }
    }

    /// The upgrade map from the layout `old` (rooted at `old_root`) to this program's
    /// `new_root`, with each of `routines` admitting its record type's lost-and-gained fields
    /// and filling its `old` struct.
    fn upgrade_map(
        &self,
        old: &LayoutDesc,
        old_root: u16,
        new_root: u16,
        whole_copies: bool,
        routines: &Routines,
    ) -> UpgradeMap {
        let roots: Vec<u16> = std::iter::once(new_root)
            .chain(routines.values().map(|r| r.old))
            .collect();
        let new = self.layout_descriptor(&roots);
        let lift: BTreeMap<u16, u16> = routines.iter().map(|(t, r)| (*t, r.old)).collect();
        UpgradeMap::derive_with(old, old_root, &new, new_root, whole_copies, &lift)
    }

    /// A conversion's refusal `e` for a store written by release `written`: one older than
    /// this program names that release and the routine that could carry it across.
    fn refusal_for(&self, written: &str, e: String) -> String {
        if written.is_empty() || !self.release().is_newer_than(written) {
            e
        } else {
            format!(
                "the store was written by {written}: {e}; a routine \
                 `fn upgrade(from: Release, old: <the old fields>, item: <the type>)` can carry \
                 it across"
            )
        }
    }

    /// `@FR-L-Sound` — `store_load` of a file written under an older layout (@PLN196, open
    /// question 3): convert it in memory into `slot`.  The file is not touched; the records
    /// that owe a computed default are listed in [`Owed`] for the program's drain.
    ///
    /// # Errors
    /// Why the store cannot be converted; `slot` keeps what it held.
    pub(crate) fn load_upgraded(
        &mut self,
        slot: u16,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let (text, bytes) = read_store_file(path)?;
        self.load_upgraded_from(slot, &text, &bytes, &path.display().to_string())
    }

    /// [`Stores::load_upgraded`] for an image already in memory, with the text of its
    /// `.dschema`: the URL loaders, after their own checks on the fetched bytes.
    ///
    /// # Errors
    /// As [`Stores::load_upgraded`].
    pub(crate) fn load_upgraded_from(
        &mut self,
        slot: u16,
        sidecar: &str,
        bytes: &[u8],
        what: &str,
    ) -> Result<(), String> {
        self.reset_owed();
        let root = self.allocations[slot as usize].known_type;
        let routines = self.upgrade_routines.clone();
        let converted = self
            .upgrade_image_from(
                sidecar,
                bytes,
                what,
                Some(root),
                &Fills::new(),
                true,
                &routines,
            )
            .and_then(|c| c.ok_or_else(|| "its recorded layout could not be compared".to_string()));
        // A conversion that stopped part way may already have listed records it owes; they
        // are in the store it released, and the drain after this call must not reach them.
        let c = converted.inspect_err(|_| self.reset_owed())?;
        self.owed.scratch = c.scratch;
        let before = self.place_converted(slot, c.new_slot)?;
        drop(before);
        Ok(())
    }

    /// Move the converted store in `new_slot` into `slot`, keeping `slot`'s bookkeeping, and
    /// make every reference and owed record that named `new_slot` name `slot`; answer what
    /// `slot` held.
    fn place_converted(&mut self, slot: u16, new_slot: u16) -> Result<crate::store::Store, String> {
        let mut store = self.take_store(new_slot);
        self.release_slot(new_slot);
        let s = &self.allocations[slot as usize];
        store.set_known_type(s.known_type);
        store.free = s.free;
        store.set_created_at(s.created_at);
        store.last_op_at = s.last_op_at;
        store.pinned = s.pinned;
        let before = std::mem::replace(&mut self.allocations[slot as usize], store);
        if let Err(why) = self.rebase_stored_refs(slot, Some(new_slot)) {
            let converted = std::mem::replace(&mut self.allocations[slot as usize], before);
            drop(converted);
            self.reset_owed();
            return Err(why);
        }
        for (r, _, _) in &mut self.owed.fills {
            if r.store_nr == new_slot {
                r.store_nr = slot;
            }
        }
        Ok(before)
    }

    /// The key of the next owed fill ([`owed_key`]), or -1 when the drain has taken them all;
    /// its record is [`Stores::owed_record`].
    pub fn owed_next(&mut self) -> i64 {
        match self.owed.fills.get(self.owed.at) {
            Some(&(_, key, _)) => {
                self.owed.at += 1;
                key
            }
            None => -1,
        }
    }

    /// The record of the owed fill [`Stores::owed_next`] last answered.
    #[must_use]
    pub fn owed_record(&self) -> DbRef {
        self.owed
            .at
            .checked_sub(1)
            .and_then(|i| self.owed.fills.get(i))
            .map_or(
                DbRef {
                    store_nr: u16::MAX,
                    rec: 0,
                    pos: 0,
                },
                |&(r, _, _)| r,
            )
    }

    /// The `old` record of the owed routine call [`Stores::owed_next`] last answered.
    #[must_use]
    pub fn owed_old(&self) -> DbRef {
        self.owed
            .at
            .checked_sub(1)
            .and_then(|i| self.owed.fills.get(i))
            .map_or(DbRef::NULL, |&(_, _, o)| o)
    }

    /// The `Release` an owed routine call is told the store's release with.
    #[must_use]
    pub fn owed_from(&self) -> DbRef {
        self.owed.from.unwrap_or(DbRef::NULL)
    }

    /// @PLN196 step 9 — register the `upgrade` routine of record type `tp`: its `old` struct
    /// `old` and the `Release` type `version`.  The drain's wrapper registers every routine
    /// before the load it follows, so the converter knows which records owe one.
    pub fn upgrade_routine(&mut self, tp: u16, old: u16, version: u16) {
        // A program can call this itself: numbers that name no type register nothing.
        let n = self.types_len();
        if tp >= n || old >= n || version >= n {
            return;
        }
        self.upgrade_routines.insert(
            tp,
            Routine {
                old,
                version,
                name: String::new(),
            },
        );
    }

    /// Forget what a drain still owed, releasing the routines' scratch store.
    fn reset_owed(&mut self) {
        let scratch = std::mem::take(&mut self.owed).scratch;
        if let Some(sc) = scratch {
            self.release_scratch(sc);
        }
    }

    /// End a drain: forget the owed fills and release the routines' scratch store.
    /// Answers whether the call the drain follows still succeeds, which a load always does
    /// once its fills ran.
    pub fn owed_finish(&mut self) -> bool {
        let owed = std::mem::take(&mut self.owed);
        if let Some(sc) = owed.scratch {
            self.release_scratch(sc);
        }
        true
    }
}

/// Write `image` and its sidecar beside the store at `path`, then swap them in; the old pair
/// is kept as `<path>.old`, replacing an earlier `.old`, so one upgrade back stays on disk.
/// At every moment one complete store with its own sidecar sits at `path`.
#[cfg(feature = "mmap")]
fn swap_in_upgrade(
    path: &std::path::Path,
    image: &[u8],
    current: &crate::schema_sidecar::LayoutIdentity,
) -> Result<std::path::PathBuf, String> {
    use crate::schema_sidecar::dschema_path;
    let side = dschema_path(path);
    let with = |suffix: &str| {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        std::path::PathBuf::from(p)
    };
    let (next, kept) = (with(".upgrade"), with(".old"));
    let io = |e: std::io::Error| e.to_string();
    super::allocation::write_image_atomic(&next, image).map_err(io)?;
    current.write_beside(&next).map_err(io)?;
    crate::file_access::rename(path, &kept).map_err(io)?;
    crate::file_access::rename(&side, dschema_path(&kept)).map_err(io)?;
    crate::file_access::rename(&next, path).map_err(io)?;
    crate::file_access::rename(dschema_path(&next), &side).map_err(io)?;
    Ok(kept)
}

#[cfg(feature = "mmap")]
impl Stores {
    /// @PLN196 step 7 — upgrade the store FILE at `path` to this program's type `new_root`
    /// (or, when `None`, the type of the stored root's name): the new image and its sidecar
    /// are written beside the store, then swapped in by rename, and the old pair is kept as
    /// `<path>.old`.  A refusal, or a failure anywhere before the swap, leaves the store and
    /// its sidecar byte for byte as they were.
    ///
    /// A field whose default is not a constant needs the program to run it; this entry has
    /// no program, so such an upgrade goes through [`crate::host::Program::upgrade_file`].
    ///
    /// # Errors
    /// Why the store cannot be upgraded: no recorded layout, a layout this build cannot read,
    /// a refusal of the upgrade map, or an I/O failure.
    pub fn upgrade_file(
        &mut self,
        path: &std::path::Path,
        new_root: Option<u16>,
    ) -> Result<FileUpgrade, String> {
        match self.upgrade_file_begin(path, new_root, &Fills::new(), &Routines::new())? {
            None => Ok(FileUpgrade::Current),
            Some((job, _)) => self.upgrade_file_finish(job),
        }
    }

    /// The first half of [`Stores::upgrade_file`]: read the store and its recorded layout,
    /// convert it into a new store in this table, and answer the job and the records that
    /// still owe an expression default (`fills`).  `None` when the store is already current.
    /// Nothing is written to disk; [`Stores::upgrade_file_finish`] does that, and
    /// [`Stores::upgrade_file_abandon`] drops the job instead.
    ///
    /// # Errors
    /// As [`Stores::upgrade_file`].
    pub fn upgrade_file_begin(
        &mut self,
        path: &std::path::Path,
        new_root: Option<u16>,
        fills: &Fills,
        routines: &Routines,
    ) -> Result<Option<(FileUpgradeJob, PendingFills)>, String> {
        Ok(self
            .upgrade_image(path, new_root, fills, false, routines)?
            .map(|c| {
                (
                    FileUpgradeJob {
                        path: path.to_path_buf(),
                        current: c.current,
                        changes: c.changes,
                        new_slot: c.new_slot,
                        scratch: c.scratch,
                    },
                    c.pending,
                )
            }))
    }

    /// Write a begun upgrade's new store beside the old one and swap it in; the old pair is
    /// kept as `<path>.old`.
    ///
    /// # Errors
    /// An I/O failure; the store at `path` is the old one until the first rename succeeds.
    pub fn upgrade_file_finish(&mut self, job: FileUpgradeJob) -> Result<FileUpgrade, String> {
        let image = self.store_image(job.new_slot);
        self.upgrade_file_abandon(&job);
        let image = image.ok_or_else(|| "the upgraded store did not form an image".to_string())?;
        let kept = swap_in_upgrade(&job.path, &image, &job.current)?;
        Ok(FileUpgrade::Upgraded {
            kept,
            changes: job.changes,
        })
    }

    /// Release a begun upgrade's new store without writing anything.
    pub fn upgrade_file_abandon(&mut self, job: &FileUpgradeJob) {
        drop(self.take_store(job.new_slot));
        self.release_slot(job.new_slot);
        if let Some(sc) = job.scratch {
            self.release_scratch(sc);
        }
    }

    /// The new store a begun upgrade built, for running its fills on.
    #[must_use]
    pub fn upgrade_file_store(job: &FileUpgradeJob) -> u16 {
        job.new_slot
    }

    /// @PLN196 step 8 — write the store in `slot` to `path` exactly as it is, with its layout
    /// sidecar beside it: the live build swap's snapshot.  Nothing is rebuilt, so every record
    /// keeps its place and a stored reference keeps naming it; the sidecar records `slot`, so
    /// the loader re-points those references at the slot it loads into (`@FR-L-Ref-Slot`).
    ///
    /// # Errors
    /// An untyped store, or an I/O failure.
    pub fn write_store_snapshot(
        &mut self,
        slot: u16,
        path: &std::path::Path,
    ) -> Result<(), String> {
        let tp = self.allocations[slot as usize].known_type;
        if tp == u16::MAX {
            return Err("the store records no type".to_string());
        }
        let image = self
            .store_image(slot)
            .ok_or_else(|| "the store did not form an image".to_string())?;
        crate::file_access::write(path, &image).map_err(|e| e.to_string())?;
        crate::schema_sidecar::LayoutIdentity::of_store(self, slot, tp)
            .write_beside(path)
            .map_err(|e| e.to_string())
    }
}

#[cfg_attr(not(any(feature = "mmap", feature = "wasm")), allow(dead_code))]
impl Stores {
    /// The byte image of the store in `slot`, ending at its last live record.
    pub(crate) fn store_image(&mut self, slot: u16) -> Option<Vec<u8>> {
        if self.allocations[slot as usize].defined {
            return defined_image(&self.allocations[slot as usize]);
        }
        // Not DEFINED (`Store::defined`): its unwritten bytes must not reach an image, so it
        // is rebuilt into a defined scratch store first, as `store_persist_copy` writes.  The
        // root keeps its place; the rest is laid out anew.
        let (tp, root_words) = {
            let s = &self.allocations[slot as usize];
            let header = s.read::<i32>(crate::store::PRIMARY, 0);
            if s.known_type == u16::MAX || header <= 0 {
                return None;
            }
            (s.known_type, header as u32)
        };
        if !self.type_is_compactable(tp, &mut Vec::new()) {
            return None;
        }
        let dst = self.rebuild_into_scratch(slot, tp, root_words).ok()?;
        let rebuilt = self.take_store(dst);
        self.release_slot(dst);
        defined_image(&rebuilt)
    }

    /// @PLN18 08-S6 — the world record `w` of type `tp` as the browser swap hands it over: its
    /// store's layout sidecar and image, when `w` is the root of a store of its own; `None`
    /// for a world that shares its store, which goes over as JSON.
    pub(crate) fn world_image(&mut self, w: &DbRef, tp: u16) -> Option<(String, Vec<u8>)> {
        let store = &self.allocations[w.store_nr as usize];
        if store.known_type != tp || w.rec != crate::store::PRIMARY {
            return None;
        }
        let image = self.store_image(w.store_nr)?;
        let sidecar =
            crate::schema_sidecar::LayoutIdentity::of_store(self, w.store_nr, tp).to_sidecar();
        Some((sidecar, image))
    }
}

/// The `Parts` an old description node becomes in the appended table, its type references
/// moved through `ids`; `None` for a base type, which maps onto its seed row instead.
fn layout_parts(node: &LayoutNode, ids: &BTreeMap<u16, u16>) -> Result<Option<Parts>, String> {
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
    Ok(Some(match node {
        LayoutNode::Base(_) => return Ok(None),
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
    }))
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

/// The byte image of a DEFINED store, ending at its last live record.
fn defined_image(s: &crate::store::Store) -> Option<Vec<u8>> {
    let words = s.capacity_words();
    let raw = s.raw_bytes();
    let live_end = super::allocation::store_image_live_end(raw, words).unwrap_or(words);
    super::allocation::build_padded_store_image(
        raw,
        words,
        live_end.max(crate::store::MIN_BOUND_WORDS),
    )
}
