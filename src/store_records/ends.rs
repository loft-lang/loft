// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! How a store's responsibility ends along a path: a free, the cascade a closure record's
//! release runs over its capture slots, what a variable hands on to a caller or a store, and what
//! the function's exit leaves live.

use super::{
    ABSENT, BTreeSet, ENDED, FREED_NAME, Finding, HANDED, LIVE, MEMBER_BASE, Pt, Pts, Run, SKey,
    State, Val, Value, cover_on, field_record_holds, forget_holder, guarded_rest_reports, info_of,
    maybe_stale, normal_alts, slot_holders_off, slot_sole_holder, stale_on_end, uncover_all,
};

impl Run<'_> {
    /// End the stores `v` names.  `report` is off for a guarded free, whose skip the
    /// analysis may not have seen.
    pub(super) fn end(&mut self, st: &mut State, v: u16, how: &str, report: bool) {
        self.end_only(st, v, how, report, None);
    }

    /// [`Self::end`], limited to the stores in `only` when it is given.
    #[expect(
        clippy::too_many_lines,
        reason = "one re-free condition per path shape the records tell apart"
    )]
    pub(super) fn end_only(
        &mut self,
        st: &mut State,
        v: u16,
        how: &str,
        report: bool,
        only: Option<&BTreeSet<SKey>>,
    ) {
        // A record variable freed (and reset) says nothing more about a store it did not
        // capture: drop it from that store's `live_some`.
        for info in st.stores.values_mut() {
            if info.captured_by.is_empty() && info.live_some.contains(&v) {
                info.live_some.clear();
            }
        }
        let pts = Self::pts(st, v);
        let label = format!("{how}({})", self.var_name(v));
        for k in Self::stores_of(&pts) {
            if only.is_some_and(|o| !o.contains(&k)) {
                continue;
            }
            self.sites[k.site as usize].ends.insert(label.clone());
            let info = info_of(st, k);
            // A variable naming only this store or null, where the store is ended on every
            // path: freed again wherever the variable is not null.
            // A guarded free's stores its witness cannot name (`only`): the guard is distinct
            // wherever `v` names one, so the free reaches each as if `v` named it alone.
            let only_this = pts.iter().all(|p| *p == Pt::Null || *p == Pt::Store(k))
                || (only.is_some() && guarded_rest_reports());
            // …and a store a JOIN's answer may be, beside stores that exist only on the other
            // paths (`Summary::Join` mints its fresh store LIVE | ABSENT): where those are absent
            // the variable names this one, so ended here, it is freed again on that path
            // (`__ref_N = f(x, __ref_N)` answering `x`, freed with `x`'s owner — loft#2474's
            // elaborator, `arm_delivery`).  A null beside them is a path that frees nothing, as
            // for `only_this`.  `LOFT_RECORDS_NO_JOIN_REFREE=1` leaves it unreported.
            let join_path = !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_JOIN_REFREE").is_some())
                    && pts.len() > 1
                    // Not a capture: a store a closure record holds ends by the closure rules.
                    && info.captured_by.is_empty()
                    && pts.iter().all(|p| match p {
                        Pt::Store(o) if *o == k => true,
                        Pt::Store(o) => st.stores.get(o).is_some_and(|i| i.status & ABSENT != 0),
                        Pt::Null => true,
                        _ => false,
                    });
            // …and a DEFINITE name of a store a weak end already released on some path (a
            // maybe-null holder freed it first): freed again on that path.
            let weakly_ended = pts.len() == 1
                && info.has(LIVE)
                && !info.ended.is_empty()
                && !info.ended.contains(&v);
            // …and a variable naming the store wherever it exists (`live`; its other answer is
            // null, where the store was never made), after another holder ended it on some
            // path: freed again on that path (`__ncc_N` after `chain`'s displaced free released
            // the store `chain` was bound to — loft#2474's elaborator, `enum_field`).
            // `LOFT_RECORDS_NO_LIVE_NAME_REFREE=1` leaves it unreported.
            let named_where_live = !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_LIVE_NAME_REFREE").is_some())
                    && info.has(LIVE)
                    && info.live.contains(&v)
                    && !info.ended.contains(&v)
                    // Known to name it where it ended (`stale`): a store ended where the names
                    // are not followed (`None`, a loop's earlier pass) says nothing about `v`.
                    && info.stale.as_ref().is_some_and(|s| s.contains(&v))
                    // A record variable: a closure's fn-ref is released through the guards
                    // the closure rules follow, not this one.
                    && v < MEMBER_BASE
                    && v < self.data.def(self.d_nr).variables.count()
                    && self.data.def(self.d_nr).variables.tp(v).base().heap_def_nr().is_some()
                    // A null beside it is where the store was never made (ABSENT), not where a
                    // rebind (a link write-back) replaced it.
                    && pts.iter().all(|p| {
                        (*p == Pt::Null && info.has(ABSENT)) || *p == Pt::Store(k)
                    });
            if report
                && info.has(ENDED)
                && (info.ended.contains(&v)
                    || (((only_this && !info.has(LIVE)) || weakly_ended || named_where_live)
                        && maybe_stale(&info, v))
                    || (join_path && !info.has(LIVE)))
            {
                // Every end here is a free, which the runtime refuses on a free slot
                // (`@FR-H-FreeTwice`) and runs on whatever store took the slot since.
                let name = self.var_name(v);
                self.find(
                    "re-free",
                    k.site,
                    format!(
                        "`{name}` is freed by {how} after its store was already freed{}",
                        self.ended_note(k)
                    ),
                );
            }
            if report && info.has(HANDED) && pts.len() == 1 && !info.has(LIVE) {
                let name = self.var_name(v);
                self.find(
                    "wrong-end",
                    k.site,
                    format!("`{name}` is freed by {how} after its store was handed on"),
                );
            }
            if !info.has(LIVE) {
                continue;
            }
            let at_line = self.line;
            self.end_lines
                .entry(k)
                .or_insert_with(|| (at_line, label.clone()));
            // The alternatives naming `v` are the paths this free ends it on.
            let rest_alts: Vec<BTreeSet<u16>> = info
                .alts
                .iter()
                .filter(|a| !a.contains(&v))
                .cloned()
                .collect();
            let strong = pts.len() == 1
                || info.live.contains(&v)
                || (!info.alts.is_empty() && rest_alts.is_empty());
            let mut next = info.clone();
            if strong {
                next.ended = if info.has(ENDED) {
                    info.ended.intersection(&info.live).copied().collect()
                } else {
                    info.live.clone()
                };
                next.stale = stale_on_end(st, k, &info);
                next.status = (info.status & !LIVE) | ENDED;
                next.live.clear();
                next.alts.clear();
            } else {
                next.ended = if info.has(ENDED) {
                    info.ended.intersection(&[v].into()).copied().collect()
                } else {
                    [v].into()
                };
                // A store a reset name (maybe) freed stays marked: another maybe-free does not
                // make that one not have happened.
                if info.ended.contains(&FREED_NAME) {
                    next.ended.insert(FREED_NAME);
                }
                next.stale = stale_on_end(st, k, &info);
                next.status |= ENDED;
                if rest_alts.len() < info.alts.len() {
                    next.live = rest_alts.iter().skip(1).fold(rest_alts[0].clone(), |m, a| {
                        m.intersection(a).copied().collect()
                    });
                    let live = next.live.clone();
                    next.alts = normal_alts(rest_alts, &live);
                }
            }
            st.stores.insert(k, next);
        }
        self.report_param_ends(v, how, report, only, &pts);
        if report && only.is_none() {
            Self::uncover_freed(st, v, &pts);
        }
    }

    /// The wrong ends of an end of `v`: a free of the caller's store (a parameter's), named
    /// alone or beside null, or beside stores of the frame's own where nothing guards the free.
    pub(super) fn report_param_ends(
        &mut self,
        v: u16,
        how: &str,
        report: bool,
        only: Option<&BTreeSet<SKey>>,
        pts: &Pts,
    ) {
        // A free of a variable that names the caller's store — alone, or that or null.
        // The "or null" reading only for a plain free: a guarded one (`OpFreeRefIfDistinct`)
        // of a buffer that may be the caller's declines where its witness holds it.
        let params: Vec<&Pt> = if how == "OpFreeRef" {
            pts.iter().filter(|p| **p != Pt::Null).collect()
        } else {
            pts.iter().collect()
        };
        // A plain free of a variable that MAY name the caller's store beside stores of its own:
        // nothing at run time tells the paths apart, so on the one where it names the caller's
        // store, that store is released under the caller (the source-free's twin).  A guarded
        // end (`only`) asked its witness already.  `LOFT_RECORDS_NO_MAYBE_PARAM_END=1` leaves it
        // unreported.
        if report
            && only.is_none()
            && how == "OpFreeRef"
            && params.len() > 1
            && params.iter().any(|p| matches!(p, Pt::Param(_)))
            && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_MAYBE_PARAM_END").is_some())
        {
            let name = self.var_name(v);
            self.find(
                "wrong-end",
                u32::MAX,
                format!("`{name}` may be the caller's store, and {how} releases it"),
            );
        }
        if report && params.len() == 1 && how != "OpFreeText" && matches!(params[0], Pt::Param(_)) {
            let name = self.var_name(v);
            self.find(
                "wrong-end",
                u32::MAX,
                format!("`{name}` is freed by {how}, but its store is the caller's"),
            );
        }
    }

    /// A certain free through `v`: `v` leaves the cover of every key it names, and a key whose
    /// cover that empties is not live — every name that held it is freed (`SInfo::cover`).
    pub(super) fn uncover_freed(st: &mut State, v: u16, pts: &Pts) {
        if !cover_on() {
            return;
        }
        for k in Self::stores_of(pts) {
            if let Some(info) = st.stores.get_mut(&k)
                && let Some(c) = info.cover.as_mut()
                && c.remove(&v)
                && c.is_empty()
                && info.has(LIVE)
            {
                info.status &= !LIVE;
                info.live.clear();
                info.alts.clear();
            }
        }
    }

    /// Does `w` name a closure record that holds a capture?
    pub(super) fn is_record(st: &State, w: u16) -> bool {
        Self::stores_of(&Self::pts(st, w))
            .iter()
            .any(|k| st.slots.contains_key(k))
    }

    /// What the capture slots of the record(s) `w` names hold: the slot at `off`, or every
    /// slot.  Stores only.
    pub(super) fn held_by_record(st: &State, w: u16, off: Option<i32>) -> Pts {
        let mut out: Pts = st
            .field_captures
            .get(&w)
            .map(|p| {
                p.iter()
                    .copied()
                    .filter(|q| matches!(q, Pt::Store(_)))
                    .collect()
            })
            .unwrap_or_default();
        for k in Self::stores_of(&Self::pts(st, w)) {
            if let Some(m) = st.slots.get(&k) {
                for (o, v) in m {
                    if off.is_none_or(|x| x == *o) {
                        out.extend(v.pts.iter().copied().filter(|q| matches!(q, Pt::Store(_))));
                    }
                }
            }
        }
        out
    }

    /// What a guarded free's witness names, and whether it is a capture slot of a record
    /// built inside another store (a fn field): under such a slot a shared store is never
    /// left live — where the slot names it the record took it over, everywhere else the
    /// free ends it.  A slot of any closure record answers as the witness.
    pub(super) fn guard_witness(
        &self,
        st: &State,
        w: Option<u16>,
        arg: Option<&Value>,
    ) -> (Pts, bool) {
        match (w, arg.map(Value::unspan)) {
            (Some(w), _) => (Self::pts(st, w), false),
            (None, Some(Value::Call(g, gargs))) if *g == self.ops.get_dbref => {
                let r = Self::first_var(gargs, 0);
                let field = r.is_some_and(|r| st.field_captures.contains_key(&r));
                let pts = r
                    .map(|r| Self::held_by_record(st, r, Self::const_offset(gargs.get(1))))
                    .unwrap_or_default();
                (pts, field)
            }
            _ => (Pts::default(), false),
        }
    }

    /// A constant slot offset (`OpGetDbRef(w, 8i32)`).
    pub(super) fn const_offset(v: Option<&Value>) -> Option<i32> {
        match v.map(Value::unspan) {
            Some(Value::Int(n)) => Some(*n),
            _ => None,
        }
    }

    /// The capture `OpSetDbRef(w, off, value)`: the slot of each record `w` names holds the
    /// value's stores from here on — replacing what it held where `w` names one record and
    /// the offset is known (a null empties it: the code generator's give-up of a capture
    /// before a return, `@FR-L-CapOne`).
    pub(super) fn hold(
        &self,
        st: &mut State,
        w: u16,
        off: Option<i32>,
        value: &Val,
        src: Option<u16>,
    ) {
        uncover_all(st, &value.pts);
        let recs = Self::stores_of(&Self::pts(st, w));
        let strong = recs.len() == 1 && off.is_some();
        // A closure slot keeps the caller's store (`Pt::Param`) beside this frame's own, so the
        // record's cascade can be asked whether it ends one (loft#2504).
        let pts: Pts = value
            .pts
            .iter()
            .copied()
            .filter(|p| {
                matches!(p, Pt::Store(_))
                    || (matches!(p, Pt::Param(_)) && self.is_closure_record(w))
            })
            .collect();
        let held = Val {
            corr: value
                .corr
                .iter()
                .copied()
                .filter(|k| pts.contains(&Pt::Store(*k)))
                .collect(),
            pts,
        };
        for k in &held.pts {
            if let Pt::Store(k) = k
                && let Some(info) = st.stores.get_mut(k)
            {
                info.captured_by.insert(w);
            }
        }
        for k in recs {
            let at = off.unwrap_or(i32::MIN);
            match src.filter(|_| strong) {
                Some(x) => {
                    st.slot_src.entry(k).or_default().insert(at, x);
                }
                None => {
                    if let Some(m) = st.slot_src.get_mut(&k) {
                        m.remove(&at);
                    }
                }
            }
            if strong {
                // The slot holds another value from here: `w` is not a holder of the one it held.
                let before: Vec<SKey> = st
                    .slots
                    .get(&k)
                    .and_then(|m| m.get(&at))
                    .map(|v| Self::stores_of(&v.pts))
                    .unwrap_or_default();
                for old in before {
                    if !held.pts.contains(&Pt::Store(old))
                        && let Some(info) = st.stores.get_mut(&old)
                    {
                        forget_holder(info, w);
                    }
                }
                // A store the slot of a closure record now holds is held by that record wherever
                // it is live: a path on which `w` names no record does not hold it there.
                // Only an OWNER slot (`OpSetDbRef(w, off, __own_x)`, the capture hand-over)
                // takes its store over; a view capture beside the store's backing does not.
                let owner_src = src.is_some_and(|x| {
                    self.data
                        .def(self.d_nr)
                        .variables
                        .name(x)
                        .starts_with("__own_")
                });
                if self.is_closure_record(w)
                    && owner_src
                    && let [only] = Self::stores_of(&held.pts)[..]
                    && let Some(info) = st.stores.get_mut(&only)
                    && info.has(LIVE)
                    && !slot_holders_off()
                {
                    info.live.insert(w);
                    for a in &mut info.alts {
                        a.insert(w);
                    }
                }
            }
            let slot = st.slots.entry(k).or_default().entry(at).or_default();
            if strong {
                slot.clone_from(&held);
            } else {
                slot.pts.extend(held.pts.iter().copied());
                slot.corr = slot.corr.intersection(&held.corr).copied().collect();
            }
        }
    }

    /// The capture slots the closure record minted at `k`'s site releases when a fn-ref's drop
    /// runs its cascade: the offsets its type's live `_OpDropAll` frees
    /// (`OpFreeRef(OpGetDbRef(self, off))`).  `None` where the site minted no closure variable
    /// or its types disagree; an empty set for a retired cascade (`Data::retire_def_name`: an
    /// earlier build owns the capture), which releases nothing.
    pub(super) fn cascade_frees(&self, k: SKey) -> Option<BTreeSet<i32>> {
        let function = &self.data.def(self.d_nr).variables;
        let mut out: Option<BTreeSet<i32>> = None;
        for &v in &self.sites[k.site as usize].holders {
            if v >= function.count() {
                continue;
            }
            // The frame's closure variable, or a copy of its record (`__disp_N`): by type.
            let crate::data::Type::Reference(d, _) = function.tp(v).base() else {
                continue;
            };
            let name = self.data.def(*d).name();
            if !name.starts_with("__closure_") {
                continue;
            }
            let drop = self
                .data
                .def_nr(&format!("t_{}{name}_OpDropAll", name.len()));
            let mut offs = BTreeSet::new();
            if drop != u32::MAX {
                let (free, get) = (
                    self.data.def_nr("OpFreeRef"),
                    self.data.def_nr("OpGetDbRef"),
                );
                self.data.def(drop).code.walk(&mut |n| {
                    if let Value::Call(op, a) = n.unspan()
                        && *op == free
                        && let [inner] = a.as_slice()
                        && let Value::Call(g, b) = inner.unspan()
                        && *g == get
                        && let [_, off] = b.as_slice()
                        && let Value::Int(o) = off.unspan()
                    {
                        offs.insert(*o);
                    }
                });
            }
            match &out {
                Some(o) if *o != offs => return None,
                _ => out = Some(offs),
            }
        }
        out
    }

    /// End the stores the slots `offs` of closure record `k` hold — a fn-ref's drop cascade
    /// (`OpDropFnRef`), named after the fn-ref `name`.  A slot store already ended on every
    /// path is ended twice.
    pub(super) fn end_record_slots(
        &mut self,
        st: &mut State,
        k: SKey,
        offs: &BTreeSet<i32>,
        name: &str,
    ) {
        let held: Vec<SKey> = st
            .slots
            .get(&k)
            .map(|m| {
                m.iter()
                    .filter(|(o, _)| offs.contains(o))
                    .flat_map(|(_, v)| Self::stores_of(&v.pts))
                    .collect()
            })
            .unwrap_or_default();
        let label = format!("its cascade({name})");
        for h in held {
            self.sites[h.site as usize].ends.insert(label.clone());
            let info = info_of(st, h);
            if info.has(ENDED) && !info.has(LIVE) {
                self.find(
                    "double-end",
                    h.site,
                    format!("the record `{name}` releases a capture by its cascade after its store was already freed"),
                );
            }
            if !info.has(LIVE) {
                continue;
            }
            let mut next = info.clone();
            next.ended.clone_from(&info.live);
            next.stale = stale_on_end(st, h, &info);
            next.status = (info.status & !LIVE) | ENDED;
            next.live.clear();
            next.alts.clear();
            st.stores.insert(h, next);
        }
    }

    /// loft#2504, `@FR-L-CapOwn` — a closure record's cascade that frees a slot holding the
    /// CALLER's store (`Pt::Param`, a capture relayed out of this lambda's own `__closure`):
    /// the record adopted a store that belongs further out, and its release frees it under
    /// the record that owns it.
    pub(super) fn report_cascade_param_ends(&mut self, st: &State, w: u16) {
        for r in Self::stores_of(&Self::pts(st, w)) {
            let Some(m) = st.slots.get(&r) else {
                continue;
            };
            let offs = self.cascade_frees(r);
            let relayed = m.iter().any(|(o, v)| {
                offs.as_ref().is_none_or(|f| f.contains(o))
                    && v.pts.iter().any(|p| matches!(p, Pt::Param(_)))
            });
            if relayed {
                let name = self.var_name(w);
                self.find(
                    "wrong-end",
                    r.site,
                    format!(
                        "the record `{name}` releases by its cascade a capture its frame relayed \
                         from its own closure: that store is the caller's"
                    ),
                );
            }
        }
    }

    /// Release the stores closure record `w`'s capture slots hold — its cascade
    /// (`<record>_OpDropAll(w)`), or a free of a slot read (`OpFreeRef(OpGetDbRef(w, off))`,
    /// `scopes::rebound_captures`).  A slot store already ended on every path is ended twice.
    pub(super) fn end_slots(&mut self, st: &mut State, w: u16, how: &str) {
        let mut held: Vec<SKey> = if how == "its cascade" {
            // The cascade frees the slots its record type's `_OpDropAll` frees (an owner slot,
            // not a view capture beside it), and drops a record such a slot holds with what
            // ITS cascade frees.
            let mut out: Vec<SKey> = st
                .field_captures
                .get(&w)
                .map(Self::stores_of)
                .unwrap_or_default();
            let mut recs: Vec<SKey> = Self::stores_of(&Self::pts(st, w));
            let mut i = 0;
            while i < recs.len() {
                let r = recs[i];
                i += 1;
                let Some(m) = st.slots.get(&r) else {
                    continue;
                };
                let offs = self.cascade_frees(r);
                // `@FR-L-CapOwn` — a store is released ONCE.  The cascade frees each slot it
                // names, so a store two of those slots hold — a capture that is a view of
                // another capture's store, taken as an owner (loft#2523) — is freed once per
                // slot.  Counted only where a slot names that one store: a slot that MAY hold
                // several (a join) says nothing definite.
                let mut sole: Vec<(SKey, Vec<i32>)> = Vec::new();
                for (o, v) in m {
                    if offs.as_ref().is_some_and(|f| !f.contains(o)) {
                        continue;
                    }
                    if let [only] = Self::stores_of(&v.pts).as_slice() {
                        match sole.iter_mut().find(|(k, _)| k == only) {
                            Some((_, slots)) => slots.push(*o),
                            None => sole.push((*only, vec![*o])),
                        }
                    }
                }
                for (k, slots) in &sole {
                    if slots.len() > 1 {
                        let name = self.var_name(w);
                        self.find(
                            "double-end",
                            k.site,
                            format!(
                                "the record `{name}` releases one store through {} of its capture \
                                 slots (offsets {slots:?}): a capture that views another \
                                 capture's store is not an owner of it",
                                slots.len()
                            ),
                        );
                    }
                }
                for (o, v) in m {
                    if offs.as_ref().is_some_and(|f| !f.contains(o)) {
                        continue;
                    }
                    for k in Self::stores_of(&v.pts) {
                        if !out.contains(&k) {
                            out.push(k);
                        }
                        if st.slots.contains_key(&k) && !recs.contains(&k) {
                            recs.push(k);
                        }
                    }
                }
            }
            out
        } else {
            Self::stores_of(&Self::held_by_record(st, w, None))
        };
        held.dedup();
        if how == "its cascade" {
            self.report_cascade_param_ends(st, w);
        }
        let label = format!("{how}({})", self.var_name(w));
        for k in held {
            self.sites[k.site as usize].ends.insert(label.clone());
            let info = info_of(st, k);
            if info.has(ENDED) && !info.has(LIVE) {
                let name = self.var_name(w);
                self.find(
                    if how == "its cascade" { "double-end" } else { "re-free" },
                    k.site,
                    format!("the record `{name}` releases a capture by {how} after its store was already freed"),
                );
            }
            if !info.has(LIVE) {
                continue;
            }
            let mut next = info.clone();
            next.ended.clone_from(&info.live);
            next.stale = stale_on_end(st, k, &info);
            next.status = (info.status & !LIVE) | ENDED;
            next.live.clear();
            next.alts.clear();
            st.stores.insert(k, next);
        }
    }

    /// The value leaves this function's hands: returned, or stored where another owner
    /// holds it.
    /// [`Self::hand_on`] of the value variable `x` holds: the stores it names also record
    /// that `x`'s value went on (`handed_by`).
    pub(super) fn hand_on_var(st: &mut State, x: Option<u16>, val: &Val) {
        Self::hand_on(st, val);
        let Some(x) = x else {
            return;
        };
        for k in Self::stores_of(&Self::pts(st, x)) {
            if let Some(info) = st.stores.get_mut(&k)
                && info.has(LIVE)
            {
                info.handed_by.insert(x);
            }
        }
    }

    pub(super) fn hand_on(st: &mut State, val: &Val) {
        uncover_all(st, &val.pts);
        let single = val.pts.len() == 1;
        let mut records = Vec::new();
        for k in Self::stores_of(&val.pts) {
            if st.slots.contains_key(&k) {
                records.push(k);
            }
            if let Some(info) = st.stores.get_mut(&k)
                && info.status & LIVE != 0
            {
                if single || val.corr.contains(&k) {
                    info.status = (info.status & !LIVE) | HANDED;
                    info.live.clear();
                    info.alts.clear();
                } else {
                    info.status |= HANDED;
                }
            }
        }
        // A fn field's record lives in its host's store: where that store leaves, the record
        // leaves with it, and so do the stores it holds.
        if field_record_holds() {
            // …and a closure record handed on itself leaves with what its variable holds.
            let mut gone: Vec<u16> = st
                .field_captures
                .keys()
                .copied()
                .filter(|w| {
                    Self::stores_of(&Self::pts(st, *w))
                        .iter()
                        .any(|k| val.pts.contains(&Pt::Store(*k)))
                })
                .collect();
            // Only a record the value certainly names: a returned fn-ref that may be a plain
            // lambda on some run hands no record on there.
            let certain: Vec<SKey> = records
                .iter()
                .copied()
                .filter(|k| single || val.corr.contains(k))
                .collect();
            for (&v, p) in &st.vars {
                if certain.iter().any(|k| p.contains(&Pt::Store(*k))) && !gone.contains(&v) {
                    gone.push(v);
                }
            }
            for info in st.stores.values_mut() {
                for w in &gone {
                    if info.has(LIVE) && info.live.contains(w) {
                        info.handed_by.insert(*w);
                    }
                }
            }
        }
        // A store no variable names, held only in the slots of records this value certainly
        // hands on, leaves with them on every path where it is live: the records' model knows
        // no other holder of it.
        let sole: BTreeSet<SKey> = if slot_sole_holder() {
            let certain: Vec<SKey> = records
                .iter()
                .copied()
                .filter(|k| single || val.corr.contains(k))
                .collect();
            let held: BTreeSet<SKey> = certain
                .iter()
                .filter_map(|r| st.slots.get(r))
                .flat_map(|m| m.values().flat_map(|v| Self::stores_of(&v.pts)))
                .collect();
            held.into_iter()
                .filter(|k| {
                    let info = info_of(st, *k);
                    info.live.is_empty()
                        && info.alts.is_empty()
                        && !st.vars.values().any(|p| p.contains(&Pt::Store(*k)))
                        && !st
                            .field_captures
                            .values()
                            .any(|p| p.contains(&Pt::Store(*k)))
                        && st.slots.iter().all(|(r, m)| {
                            certain.contains(r)
                                || !m.values().any(|v| v.pts.contains(&Pt::Store(*k)))
                        })
                })
                .collect()
        } else {
            BTreeSet::new()
        };
        // A closure record that leaves takes what its slots hold with it (`@FR-L-CapOne`).
        for k in records {
            if let Some(m) = st.slots.remove(&k) {
                let src = st.slot_src.remove(&k).unwrap_or_default();
                for (off, mut v) in m {
                    v.corr.extend(
                        Self::stores_of(&v.pts)
                            .into_iter()
                            .filter(|c| sole.contains(c)),
                    );
                    Self::hand_on_var(st, src.get(&off).copied(), &v);
                }
            }
        }
    }

    /// A closure record the function hands back takes what its slots hold to the caller, which
    /// calls it there: a capture whose store this frame already released — ended on every path
    /// where it exists — is read after its end by the caller (`@FR-L-CapOwn`; loft#2210's
    /// owner-slot hand-off under `LOFT_NO_WRITE_OUT_NCC_ARM`).  Records inside records are
    /// followed through their own slots.
    pub(super) fn leaving_captures(&mut self, s: &State, value: &Val) {
        let mut todo: Vec<SKey> = Self::stores_of(&value.pts);
        let mut seen: BTreeSet<SKey> = BTreeSet::new();
        while let Some(rec) = todo.pop() {
            if !seen.insert(rec) {
                continue;
            }
            let Some(m) = s.slots.get(&rec) else {
                continue;
            };
            for slot in m.values() {
                for k in Self::stores_of(&slot.pts) {
                    let info = info_of(s, k);
                    if info.has(ENDED) && !info.has(LIVE) && !info.has(HANDED) {
                        self.find(
                            "use-after-end",
                            k.site,
                            "a closure record handed back captures a store this frame already \
                             released: the caller reads it through the closure"
                                .to_string(),
                        );
                    }
                    if s.slots.contains_key(&k) {
                        todo.push(k);
                    }
                }
            }
        }
    }

    /// Every store that may still be live at an exit is a leak.
    pub(super) fn exit(&mut self, st: &State, how: &str) {
        if self.traced {
            self.trace(&Value::Text(format!("exit: {how}")), st);
        }
        // A store a HIDDEN buffer parameter holds (`OpDatabase(__retbuf)`, `Origin::MintInto`)
        // goes back to the caller through that buffer, which the caller releases: no leak of
        // this frame's.  Without this, minting there read as a leak at every exit where the
        // answer was not followed into the return (`LOFT_RECORDS_NO_HIDDEN_PARAM_MINT`'s mint;
        // `pc(x) -> Node?` answering a literal arm's copy).  `LOFT_RECORDS_NO_BUFFER_HANDS_BACK=1`
        // reports it.
        let buffer_params: Vec<u16> =
            if crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_BUFFER_HANDS_BACK").is_some()) {
                Vec::new()
            } else {
                let attrs = self.data.def(self.d_nr).attributes();
                self.params
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| attrs.get(*i).is_some_and(|a| a.hidden))
                    .map(|(_, p)| *p)
                    .collect()
            };
        for (k, info) in &st.stores {
            if !info.has(LIVE) || !info.live.is_disjoint(&info.handed_by) {
                continue;
            }
            if buffer_params
                .iter()
                .any(|b| st.vars.get(b).is_some_and(|p| p.contains(&Pt::Store(*k))))
            {
                continue;
            }
            let holders: Vec<String> = st
                .vars
                .iter()
                .filter(|(_, p)| p.contains(&Pt::Store(*k)))
                .map(|(v, _)| format!("`{}`", self.var_name(*v)))
                .collect();
            let held = if holders.is_empty() {
                "nothing".to_string()
            } else {
                holders.join(", ")
            };
            let f = Finding {
                kind: "leak",
                site: k.site,
                detail: format!("a path reaches {how} with the store live, held by {held}"),
            };
            let line = self.line;
            self.findings.entry(f).or_insert(line);
        }
    }

    /// Drop what no later step can observe: a store that cannot be live any more once no
    /// variable names it.
    pub(super) fn normalize(st: &mut State) {
        let named: BTreeSet<SKey> = st.vars.values().flat_map(Self::stores_of).collect();
        st.stores
            .retain(|k, s| s.status & LIVE != 0 || named.contains(k));
    }
}
