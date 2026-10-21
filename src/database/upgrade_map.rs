// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @I70 — Database subsystem: the store upgrade map.
//
//! @PLN196 — the ONE home of the decision what happens to each stored value when a program's
//! definitions change: the upgrade map.
//!
//! For each (old type, new type) pair reached from a store's root it records where every new
//! field's value comes from — an old field of the same name, a conversion of it, or the new
//! definition's default — which old fields are dropped, how each collection is rebuilt, and
//! every change it refuses, with the reason.  It is derived once, from two descriptions
//! ([`LayoutDesc`]) and nothing else, so `loft layout check` (no data at all), the converter
//! and every later caller read the same answer; none of them re-decides a field's fate.
//!
//! `@FR-L-Sound`: an upgrade reads old bytes only through the OLD layout, writes only through
//! the NEW one, and every value either arrives under its own name, takes the new definition's
//! default, or the whole upgrade is refused before anything is written.

use crate::database::{BaseKind, Iterated, LayoutDesc, LayoutField, LayoutNode};
use std::collections::{BTreeMap, BTreeSet};

/// How a value of the old type becomes a value of the new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum How {
    /// The two types are laid out identically, closure included: the bytes are copied as
    /// they are, with their heap.
    Same,
    /// A record (or a struct-enum variant): each NEW field, in the new field order.
    Record(Vec<FieldFrom>),
    /// A number read through the old encoding and written through the new, whose range
    /// holds every value the old one can.
    Widen,
    /// A value enum: for each OLD variant, its NEW discriminant, or `None` when the variant
    /// is gone and the value becomes null.  `pairs` holds the pair of each surviving
    /// variant that carries fields.
    Variants {
        to: Vec<Option<u8>>,
        pairs: Vec<Option<usize>>,
    },
    /// A stored `reference<T>` into the store itself, re-pointed through the old-to-new record
    /// map once every record exists (`(L-Ref-Slot)`); null stays null.
    Reference,
    /// A collection rebuilt element by element in the new layout: each element converted
    /// through `elem` (an index into [`UpgradeMap::pairs`]) and inserted into the new kind,
    /// so a changed key, a new index or a changed stride is laid out by the new program's
    /// own insert.
    Elements { elem: usize },
}

/// Where one field of a new record takes its value from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldFrom {
    /// The old field of the same name (its index in the old field list), converted through
    /// pair `pair`.
    Old { field: usize, pair: usize },
    /// The new definition's default: the field did not exist.
    Default,
    /// A member of an index group (`@FR-Col-Group`) whose records arrive through the member
    /// at `source` (a new field number): filing a record in one member files it in all, so
    /// this one is never written, whether or not it existed before.
    Group { source: usize },
    /// Bookkeeping the layout writes itself (a struct-enum's discriminant, a `#` synthetic).
    Layout,
}

/// One (old type, new type) pair and how a value crosses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub old: u16,
    pub new: u16,
    pub how: How,
}

/// A change the upgrade cannot carry without losing or inventing data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// Where, as a field path from the store's root (`World.items[].score`).
    pub path: String,
    /// Why, in a sentence a programmer can act on.
    pub reason: String,
}

/// The upgrade from one layout to another, for one store root.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpgradeMap {
    /// Every pair reached from the root; the root's pair is first.
    pub pairs: Vec<Pair>,
    /// What the upgrade changes, one line each, for the report.
    pub changes: Vec<String>,
    /// What it refuses.  A map with a refusal is never executed.
    pub refusals: Vec<Refusal>,
    /// @PLN196 step 9 — per new record type with an `upgrade` routine that runs for this
    /// store, the pair that fills the routine's `old` struct from a stored record: each of
    /// its fields from the old field of that name, or its default when the store has none.
    pub projections: BTreeMap<u16, usize>,
}

impl UpgradeMap {
    /// Derive the map from the old layout `old` (rooted at `old_root`) to the new layout
    /// `new` (rooted at `new_root`).
    #[must_use]
    pub fn derive(old: &LayoutDesc, old_root: u16, new: &LayoutDesc, new_root: u16) -> Self {
        Self::derive_with(old, old_root, new, new_root, true, &BTreeMap::new())
    }

    /// [`UpgradeMap::derive`], choosing whether an unchanged record or collection may be
    /// copied whole.  With `whole_copies` off only a leaf (a number, a text, a plain enum)
    /// is copied as it is, so every record and collection is carried field by field and
    /// element by element through the old description — the identity conversion step 1
    /// measures the description-driven reader with.
    ///
    /// `routines` maps a new record type to the `old` struct of the `upgrade` routine that
    /// runs on its records for this store (@PLN196 step 9): such a record may lose and gain
    /// fields in one upgrade, because the routine is what carries a moved value.
    #[must_use]
    pub fn derive_with(
        old: &LayoutDesc,
        old_root: u16,
        new: &LayoutDesc,
        new_root: u16,
        whole_copies: bool,
        routines: &BTreeMap<u16, u16>,
    ) -> Self {
        let mut d = Deriver {
            routines,
            whole_copies,
            old,
            new,
            map: UpgradeMap::default(),
            memo: BTreeMap::new(),
            closures: BTreeMap::new(),
        };
        let root = d.root_label(old_root, new_root);
        d.pair(old_root, new_root, &root);
        d.map.changes.sort();
        d.map.changes.dedup();
        d.map
    }

    /// Does the upgrade change nothing — every type reached is laid out identically?
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.refusals.is_empty() && self.pairs.iter().all(|p| p.how == How::Same)
    }

    /// The report `loft layout check` prints: what changes, then what is refused.  Empty
    /// when nothing changes.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut s = String::new();
        for c in &self.changes {
            s.push_str("  ");
            s.push_str(c);
            s.push('\n');
        }
        for r in &self.refusals {
            s.push_str("  refused: ");
            s.push_str(&r.path);
            s.push_str(" — ");
            s.push_str(&r.reason);
            s.push('\n');
        }
        s
    }
}

struct Deriver<'a> {
    routines: &'a BTreeMap<u16, u16>,
    whole_copies: bool,
    old: &'a LayoutDesc,
    new: &'a LayoutDesc,
    map: UpgradeMap,
    memo: BTreeMap<(u16, u16), usize>,
    closures: BTreeMap<(bool, u16), String>,
}

/// The values a number's encoding can hold, as an inclusive range; `None` for a type that is
/// not a number.  `nullable` is whether it can also hold null.
fn number_range(node: &LayoutNode) -> Option<(i128, i128, bool)> {
    Some(match *node {
        LayoutNode::Base(BaseKind::Integer | BaseKind::Long) => {
            (i128::from(i64::MIN) + 1, i128::from(i64::MAX), true)
        }
        LayoutNode::Byte { start, nullable } => {
            let s = i128::from(start);
            (s, s + 255 - i128::from(nullable), nullable)
        }
        // The `+1` encoding spends raw code 0 on null whatever the declaration.
        LayoutNode::Short { start, nullable } => {
            let s = i128::from(start);
            (s, s + 65534, nullable)
        }
        LayoutNode::ShortRaw { start, nullable } => {
            let s = i128::from(start);
            (s, s + 65535 - i128::from(nullable), nullable)
        }
        LayoutNode::Int { nullable, .. } => {
            (i128::from(i32::MIN) + 1, i128::from(i32::MAX), nullable)
        }
        LayoutNode::IntRaw { nullable, .. } => {
            (0, i128::from(u32::MAX) - i128::from(nullable), nullable)
        }
        _ => return None,
    })
}

/// A short spelling of a node's kind for a refusal.
fn kind_word(node: &LayoutNode) -> &'static str {
    match node {
        LayoutNode::Base(BaseKind::Text) => "text",
        LayoutNode::Base(BaseKind::Boolean) => "boolean",
        LayoutNode::Base(BaseKind::Character) => "character",
        LayoutNode::Base(BaseKind::Single | BaseKind::Float) => "a float",
        LayoutNode::Base(_)
        | LayoutNode::Byte { .. }
        | LayoutNode::Short { .. }
        | LayoutNode::ShortRaw { .. }
        | LayoutNode::Int { .. }
        | LayoutNode::IntRaw { .. } => "an integer",
        LayoutNode::Record(_) => "a struct",
        LayoutNode::EnumValue(..) => "an enum variant",
        LayoutNode::Choices(_) => "an enum",
        LayoutNode::Vector(_) | LayoutNode::Array(_) | LayoutNode::FlatArray { .. } => "a vector",
        LayoutNode::Iterated(Iterated::Sorted { .. } | Iterated::Ordered { .. }) => "a sorted",
        LayoutNode::Iterated(Iterated::Hash { .. }) => "a hash",
        LayoutNode::Iterated(Iterated::Index { .. }) => "an index",
        LayoutNode::Iterated(Iterated::Radix { .. }) => "a spatial",
        LayoutNode::Iterated(Iterated::Trie { .. }) => "a trie",
        LayoutNode::Ref => "a reference",
        LayoutNode::ChildRec(_) => "a captured closure",
    }
}

impl Deriver<'_> {
    fn desc(&self, new: bool) -> &LayoutDesc {
        if new { self.new } else { self.old }
    }

    fn name(&self, new: bool, id: u16) -> String {
        self.desc(new)
            .names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| format!("#{id}"))
    }

    fn root_label(&self, old: u16, new: u16) -> String {
        let n = self.name(true, new);
        if self.name(false, old) == n {
            n
        } else {
            format!("{} (now {n})", self.name(false, old))
        }
    }

    /// The dump lines of `id`'s whole closure, sorted, plus the facts the dump does not
    /// render (a field's nullability, a reference's target): equal for two types iff a value
    /// of one can be copied as the other byte for byte.
    fn closure(&mut self, new: bool, id: u16) -> String {
        if let Some(c) = self.closures.get(&(new, id)) {
            return c.clone();
        }
        let d = self.desc(new);
        let mut seen = BTreeSet::new();
        let mut stack = vec![id];
        while let Some(t) = stack.pop() {
            if !seen.insert(t) {
                continue;
            }
            match d.nodes.get(&t) {
                Some(LayoutNode::Record(fs) | LayoutNode::EnumValue(_, fs)) => {
                    stack.extend(fs.iter().map(|f| f.content).filter(|c| *c != u16::MAX));
                }
                Some(LayoutNode::Choices(vs)) => {
                    stack.extend(vs.iter().map(|(v, _)| *v).filter(|v| *v != u16::MAX));
                }
                Some(LayoutNode::Vector(e) | LayoutNode::Array(e) | LayoutNode::ChildRec(e)) => {
                    stack.push(*e)
                }
                Some(LayoutNode::Iterated(it)) => stack.push(it.elem()),
                _ => {}
            }
        }
        let full = d.render_dump();
        let mut lines: Vec<String> = Vec::new();
        // A stored reference names a record by its number, which an upgrade renumbers: a
        // closure holding one is never copied whole, so no two closures with one compare equal.
        if seen
            .iter()
            .any(|t| matches!(d.nodes.get(t), Some(LayoutNode::Ref)))
        {
            lines.push(format!("@ref {}", if new { "new" } else { "old" }));
        }
        for t in &seen {
            let name = self.name(new, *t);
            if let Some(l) = full
                .lines()
                .find(|l| l.split('\t').next() == Some(name.as_str()))
            {
                lines.push(l.to_string());
            }
            if let Some(LayoutNode::Record(fs) | LayoutNode::EnumValue(_, fs)) = d.nodes.get(t) {
                for f in fs {
                    if f.nullable {
                        lines.push(format!("{name}.{}?", f.name));
                    }
                    if f.target != u16::MAX {
                        lines.push(format!("{name}.{}->{}", f.name, self.name(new, f.target)));
                    }
                }
            }
        }
        lines.sort();
        let c = lines.join("\n");
        self.closures.insert((new, id), c.clone());
        c
    }

    fn refuse(&mut self, path: &str, reason: String) {
        self.map.refusals.push(Refusal {
            path: path.to_string(),
            reason,
        });
    }

    /// Pair `old` with `new` at `path`; the pair's index, or `None` when refused.
    fn pair(&mut self, old: u16, new: u16, path: &str) -> Option<usize> {
        self.pair_as(old, new, path, true)
    }

    /// [`Self::pair`] that never copies the value whole when `whole` is off: the source member
    /// of an index group is filed record by record, because filing is what reaches a sibling
    /// (`@FR-Col-Group`) — a member added to the group would otherwise stay empty.
    fn pair_as(&mut self, old: u16, new: u16, path: &str, whole: bool) -> Option<usize> {
        if whole && let Some(i) = self.memo.get(&(old, new)) {
            return Some(*i);
        }
        let (Some(on), Some(nn)) = (
            self.old.nodes.get(&old).cloned(),
            self.new.nodes.get(&new).cloned(),
        ) else {
            self.refuse(path, "the layout does not describe this type".to_string());
            return None;
        };
        // A keyed collection this build lays out differently cannot even be walked.
        if let Some(token) = self.old.placements.get(&old) {
            self.refuse(
                path,
                format!(
                    "the store's {} was written by a build that lays it out differently \
                     (placement {token}), which this build cannot read",
                    kind_word(&on)
                ),
            );
            return None;
        }
        let same_name = self.name(false, old) == self.name(true, new);
        let leaf = match &on {
            LayoutNode::Base(_)
            | LayoutNode::Byte { .. }
            | LayoutNode::Short { .. }
            | LayoutNode::ShortRaw { .. }
            | LayoutNode::Int { .. }
            | LayoutNode::IntRaw { .. } => true,
            LayoutNode::Choices(vs) => vs.iter().all(|(t, _)| *t == u16::MAX),
            _ => false,
        };
        if same_name
            && ((self.whole_copies && whole) || leaf)
            && self.closure(false, old) == self.closure(true, new)
        {
            return Some(self.push(old, new, How::Same));
        }
        // Reserve the slot first, so a recursive type pairs with itself.
        let at = self.push(old, new, How::Same);
        let how = self.how(old, &on, new, &nn, path);
        match how {
            Some(h) => {
                self.map.pairs[at].how = h;
                Some(at)
            }
            None => None,
        }
    }

    fn push(&mut self, old: u16, new: u16, how: How) -> usize {
        self.map.pairs.push(Pair { old, new, how });
        let at = self.map.pairs.len() - 1;
        self.memo.insert((old, new), at);
        at
    }

    fn how(
        &mut self,
        old: u16,
        on: &LayoutNode,
        new: u16,
        nn: &LayoutNode,
        path: &str,
    ) -> Option<How> {
        let (old_name, new_name) = (self.name(false, old), self.name(true, new));
        match (on, nn) {
            (LayoutNode::Record(of), LayoutNode::Record(nf))
            | (LayoutNode::EnumValue(_, of), LayoutNode::EnumValue(_, nf)) => {
                if old_name != new_name {
                    self.refuse(
                        path,
                        format!("now holds a `{new_name}` where the store holds a `{old_name}`"),
                    );
                    return None;
                }
                let plan = self.record(&new_name, new, of, nf, path).map(How::Record)?;
                if let Some(&o) = self.routines.get(&new)
                    && !self.map.projections.contains_key(&new)
                {
                    let at = self.project(old, of, o, &format!("{path} (upgrade)"))?;
                    self.map.projections.insert(new, at);
                }
                Some(plan)
            }
            (LayoutNode::Choices(ov), LayoutNode::Choices(nv)) => {
                if old_name != new_name {
                    self.refuse(
                        path,
                        format!("now holds a `{new_name}` where the store holds a `{old_name}`"),
                    );
                    return None;
                }
                self.variants(ov, nv, &new_name, path)
            }
            (LayoutNode::Base(BaseKind::Single), LayoutNode::Base(BaseKind::Float)) => {
                self.map
                    .changes
                    .push(format!("{path}: widened from single to float"));
                Some(How::Widen)
            }
            (
                LayoutNode::Vector(oe) | LayoutNode::Array(oe),
                LayoutNode::Vector(ne) | LayoutNode::Array(ne),
            ) => {
                if matches!(on, LayoutNode::Vector(_)) != matches!(nn, LayoutNode::Vector(_)) {
                    let elem = self.name(true, *ne);
                    self.map.changes.push(if matches!(nn, LayoutNode::Array(_)) {
                        format!("{path}: each `{elem}` moves into a record of its own, because a keyed collection over `{elem}` now exists")
                    } else {
                        format!("{path}: each `{elem}` moves back inline, because no keyed collection over `{elem}` is left")
                    });
                }
                let elem = self.pair(*oe, *ne, &format!("{path}[]"))?;
                Some(How::Elements { elem })
            }
            (LayoutNode::Iterated(oi), LayoutNode::Iterated(ni)) => {
                self.keyed(oi, on, ni, nn, path)
            }
            // What the reference points at is checked on the FIELD (`record`), where the
            // target is recorded; the 12 bytes themselves are re-pointed.
            (LayoutNode::Ref, LayoutNode::Ref) => Some(How::Reference),
            _ => {
                if let (Some((olo, ohi, onull)), Some((nlo, nhi, nnull))) =
                    (number_range(on), number_range(nn))
                {
                    if nlo > olo || nhi < ohi {
                        self.refuse(
                            path,
                            format!("narrows from `{old_name}` to `{new_name}`: a stored value may not fit"),
                        );
                        return None;
                    }
                    if onull && !nnull {
                        self.refuse(
                            path,
                            format!(
                                "`{new_name}` cannot hold null, and the stored `{old_name}` can"
                            ),
                        );
                        return None;
                    }
                    self.map
                        .changes
                        .push(format!("{path}: widened from `{old_name}` to `{new_name}`"));
                    return Some(How::Widen);
                }
                self.refuse(
                    path,
                    format!("changes from {} to {}", kind_word(on), kind_word(nn)),
                );
                None
            }
        }
    }

    /// The plan of a keyed collection kept as the same kind: rebuilt element by element,
    /// under its new key when the key changed.
    fn keyed(
        &mut self,
        oi: &Iterated,
        on: &LayoutNode,
        ni: &Iterated,
        nn: &LayoutNode,
        path: &str,
    ) -> Option<How> {
        let same_kind = std::mem::discriminant(oi) == std::mem::discriminant(ni)
            || matches!(
                (oi, ni),
                (
                    Iterated::Sorted { .. } | Iterated::Ordered { .. },
                    Iterated::Sorted { .. } | Iterated::Ordered { .. }
                )
            );
        if !same_kind {
            self.refuse(
                path,
                format!("changes from {} to {}", kind_word(on), kind_word(nn)),
            );
            return None;
        }
        let (oe, ne) = (oi.elem(), ni.elem());
        let elem = self.pair(oe, ne, &format!("{path}[]"))?;
        let (ok, nk) = (self.key_names(false, oi), self.key_names(true, ni));
        if ok != nk {
            self.map.changes.push(format!(
                "{path}: rebuilt under the key [{}] (was [{}])",
                nk.join(", "),
                ok.join(", ")
            ));
        }
        Some(How::Elements { elem })
    }

    /// The plan of a value enum whose name did not change: each old variant to the new one of
    /// its name, or to null when the new program removed it.
    fn variants(
        &mut self,
        ov: &[(u16, String)],
        nv: &[(u16, String)],
        new_name: &str,
        path: &str,
    ) -> Option<How> {
        let mut to = Vec::new();
        let mut pairs = Vec::new();
        for (ot, oname) in ov {
            let Some(i) = nv.iter().position(|(_, n)| n == oname) else {
                to.push(None);
                pairs.push(None);
                self.map.changes.push(format!(
                    "{new_name}.{oname} removed: such a value becomes null"
                ));
                continue;
            };
            to.push(Some(i as u8));
            let nt = nv[i].0;
            pairs.push(if *ot != u16::MAX && nt != u16::MAX {
                Some(self.pair(*ot, nt, &format!("{path}.{oname}"))?)
            } else if *ot != nt {
                self.refuse(
                    &format!("{path}.{oname}"),
                    "a variant gains or loses its fields".to_string(),
                );
                return None;
            } else {
                None
            });
        }
        for (_, n) in nv {
            if !ov.iter().any(|(_, o)| o == n) {
                self.map.changes.push(format!("{new_name}.{n} added"));
            }
        }
        Some(How::Variants { to, pairs })
    }

    /// The key field names of a keyed collection, `-name` for a descending one.
    fn key_names(&self, new: bool, it: &Iterated) -> Vec<String> {
        let d = self.desc(new);
        let fields: Vec<LayoutField> = match d.nodes.get(&it.elem()) {
            Some(LayoutNode::Record(fs) | LayoutNode::EnumValue(_, fs)) => fs.clone(),
            _ => Vec::new(),
        };
        let name = |k: u16| {
            fields
                .get(k as usize)
                .map_or_else(|| format!("#{k}"), |f| f.name.clone())
        };
        match it {
            Iterated::Sorted { keys, .. }
            | Iterated::Ordered { keys, .. }
            | Iterated::Index { keys, .. } => keys
                .iter()
                .map(|(k, asc)| format!("{}{}", if *asc { "" } else { "-" }, name(*k)))
                .collect(),
            Iterated::Hash { keys, .. } | Iterated::Radix { keys, .. } => {
                keys.iter().map(|k| name(*k)).collect()
            }
            Iterated::Trie { key, .. } => vec![name(*key)],
        }
    }

    /// May the stored references of field `old` be carried into field `new`?  Only when the
    /// old layout records what they point at and it is the type the new field points at.
    fn reference_target_kept(
        &mut self,
        record: &str,
        old: &LayoutField,
        new: &LayoutField,
        path: &str,
    ) -> bool {
        let at = format!("{path}.{}", new.name);
        if old.target == u16::MAX {
            self.refuse(
                &at,
                format!(
                    "the store's recorded layout does not say what `{record}.{}` points at — \
                     it was written before stores recorded it; load and save it once with the \
                     program that wrote it",
                    old.name
                ),
            );
            return false;
        }
        let (was, now) = (self.name(false, old.target), self.name(true, new.target));
        if was != now {
            self.refuse(
                &at,
                format!("now points at a `{now}` where the stored references point at a `{was}`"),
            );
            return false;
        }
        true
    }

    /// `@FR-L-Upgrade-Old` — the pair that fills an `upgrade` routine's `old` struct (new
    /// type `o`) from a stored record of old type `old`: each field of `o` from the stored
    /// field of its name, through the same conversion a carried field takes, or the
    /// field's default when the store has none — a routine reads it only for the releases
    /// whose stores had it (`from`).  Nothing is refused for being dropped or added: the
    /// struct names the fields it reads and no others.
    fn project(&mut self, old: u16, of: &[LayoutField], o: u16, path: &str) -> Option<usize> {
        let Some(LayoutNode::Record(nf)) = self.new.nodes.get(&o).cloned() else {
            self.refuse(
                path,
                "an upgrade routine's `old` is not a struct".to_string(),
            );
            return None;
        };
        let at = self.push(old, o, How::Same);
        // Not memoised as (old, o): a stored record and the struct it projects onto are
        // never the same pair as the record and its own new type.
        self.memo.remove(&(old, o));
        let mut plan = Vec::with_capacity(nf.len());
        let oname = self.name(true, o);
        for f in &nf {
            if !f.is_data() {
                plan.push(FieldFrom::Layout);
                continue;
            }
            let fpath = format!("{path}.{}", f.name);
            if matches!(self.new.nodes.get(&f.content), Some(LayoutNode::Ref)) {
                self.refuse(
                    &fpath,
                    format!("`{oname}.{}` is a reference, which an upgrade routine cannot read from the old store", f.name),
                );
                return None;
            }
            match of.iter().position(|x| x.is_data() && x.name == f.name) {
                Some(i) => {
                    let pair = self.pair_as(of[i].content, f.content, &fpath, false)?;
                    plan.push(FieldFrom::Old { field: i, pair });
                }
                None => plan.push(FieldFrom::Default),
            }
        }
        self.map.pairs[at].how = How::Record(plan);
        Some(at)
    }

    /// The fields a record gains and loses, reported as changes; `false` when it does both in
    /// one upgrade with no `upgrade` routine to carry a moved value (`@FR-L-Upgrade-Routine`),
    /// which looks like a rename and is refused.
    fn added_and_dropped(
        &mut self,
        name: &str,
        new_id: u16,
        of: &[LayoutField],
        nf: &[LayoutField],
        path: &str,
    ) -> bool {
        let data = |f: &LayoutField| f.is_data();
        let added: Vec<&str> = nf
            .iter()
            .filter(|f| data(f) && !of.iter().any(|o| data(o) && o.name == f.name))
            .map(|f| f.name.as_str())
            .collect();
        let dropped: Vec<&str> = of
            .iter()
            .filter(|f| data(f) && !nf.iter().any(|n| data(n) && n.name == f.name))
            .map(|f| f.name.as_str())
            .collect();
        if !added.is_empty() && !dropped.is_empty() && self.routines.contains_key(&new_id) {
            self.map.changes.push(format!(
                "{name} loses {} and gains {}: its `upgrade` routine converts it",
                dropped.join(", "),
                added.join(", ")
            ));
        } else if !added.is_empty() && !dropped.is_empty() {
            self.refuse(
                path,
                format!(
                    "`{name}` loses {} and gains {} in one upgrade, which looks like a rename; \
                     keep the old field, give the new one a default that reads it \
                     (`new: T = $.old`), upgrade, then remove the old one; or write \
                     `fn upgrade(from: Version, old: <the old fields>, item: {name})`",
                    dropped.join(", "),
                    added.join(", ")
                ),
            );
            return false;
        }
        for a in &added {
            self.map
                .changes
                .push(format!("{name}.{a} added: filled with its default"));
        }
        for d in &dropped {
            self.map
                .changes
                .push(format!("{name}.{d} removed: its values are not kept"));
        }
        true
    }

    /// The per-field plan of a record whose name did not change.
    fn record(
        &mut self,
        name: &str,
        new_id: u16,
        of: &[LayoutField],
        nf: &[LayoutField],
        path: &str,
    ) -> Option<Vec<FieldFrom>> {
        if !self.added_and_dropped(name, new_id, of, nf, path) {
            return None;
        }
        let data = |f: &LayoutField| f.is_data();
        // In each index group the FIRST member that has stored data is converted; the others
        // receive the same records through it.  Writing a default into a later member would
        // overwrite what filing through the first one put there.
        let old_index = |f: &LayoutField| of.iter().position(|o| data(o) && o.name == f.name);
        let group_source = |at: usize| -> Option<usize> {
            let members: Vec<usize> = std::iter::once(at)
                .chain(
                    nf[at]
                        .group
                        .iter()
                        .filter(|g| **g != u16::MAX)
                        .map(|g| usize::from(*g)),
                )
                .collect();
            let first = members
                .iter()
                .copied()
                .filter(|m| nf.get(*m).is_some_and(|f| old_index(f).is_some()))
                .min()?;
            (first != at).then_some(first)
        };
        let mut plan = Vec::with_capacity(nf.len());
        let mut ok = true;
        for (at, f) in nf.iter().enumerate() {
            if !data(f) {
                plan.push(FieldFrom::Layout);
                continue;
            }
            if let Some(source) = group_source(at) {
                if old_index(f).is_none() {
                    // Reported here instead of as a default.
                    self.map.changes.retain(|c| {
                        c != &format!("{name}.{} added: filled with its default", f.name)
                    });
                    self.map.changes.push(format!(
                        "{name}.{} added: built from the records {name}.{} holds",
                        f.name, nf[source].name
                    ));
                }
                plan.push(FieldFrom::Group { source });
                continue;
            }
            match of.iter().position(|o| data(o) && o.name == f.name) {
                Some(i) => {
                    if matches!(self.old.nodes.get(&of[i].content), Some(LayoutNode::Ref))
                        && !self.reference_target_kept(name, &of[i], f, path)
                    {
                        ok = false;
                        continue;
                    }
                    let whole = f.group.iter().all(|g| *g == u16::MAX);
                    match self.pair_as(
                        of[i].content,
                        f.content,
                        &format!("{path}.{}", f.name),
                        whole,
                    ) {
                        Some(pair) => plan.push(FieldFrom::Old { field: i, pair }),
                        None => ok = false,
                    }
                }
                None => plan.push(FieldFrom::Default),
            }
        }
        ok.then_some(plan)
    }
}
