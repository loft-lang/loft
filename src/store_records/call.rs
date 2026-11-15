// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Calls and frees: a release operation's effect on the stores, a call's answer read from the
//! callee's summary (or its native answer), and what a callee writes back through a `&` parameter.

use super::{
    ABSENT, BTreeMap, BTreeSet, Data, ENDED, FREED_NAME, HANDED, LIVE, MEMBER_BASE, Origin, Pt,
    Pts, Run, SInfo, SKey, State, Summary, Val, Value, copy_alias, forget_holder,
    guarded_rest_reports, identity_test_no_use, info_of, join_arms, join_info, maybe_stale,
    normal_alts, set_dbref_value_use, slot_null, uncover_all,
};

impl Run<'_> {
    /// `OpFreeRecordIn` releases one record: a store of its own only where the record is
    /// that store's root (a placed record whose host was null); a record placed inside a host
    /// leaves the host's store live.
    pub(super) fn free_record_in(&mut self, args: &[Value], name: &str, st: &mut State) {
        if let Some(x) = Self::first_var(args, 0) {
            let placed: BTreeSet<SKey> = Self::stores_of(&Self::pts(st, x))
                .into_iter()
                .filter(|k| matches!(self.sites[k.site as usize].origin, Some(Origin::Place)))
                .collect();
            if !placed.is_empty() {
                self.end_only(st, x, name, true, Some(&placed));
            }
        }
    }

    /// A free op: its own argument is not a use.
    /// A capture SLOT freed unless the witness names its store (a closure build releasing the
    /// capture an earlier pass left in its record): the slot's stores the witness cannot name
    /// end, for certain where the slot names only that one.  `false` when the freed operand is
    /// no slot of a record.
    pub(super) fn free_slot_unless_witness(
        &mut self,
        args: &[Value],
        name: &str,
        st: &mut State,
    ) -> bool {
        let Some(Value::Call(g, gargs)) = args.first().map(Value::unspan) else {
            return false;
        };
        if *g != self.ops.get_dbref {
            return false;
        }
        let Some(rec) = Self::first_var(gargs, 0) else {
            return false;
        };
        if !Self::is_record(st, rec) {
            return false;
        }
        let held = Self::held_by_record(st, rec, Self::const_offset(gargs.get(1)));
        let w = args.get(1).and_then(|a| Self::named_var(st, a));
        let wp = w.map(|w| Self::pts(st, w));
        let label = format!("{name}({})", self.var_name(rec));
        let stores = Self::stores_of(&held);
        let single = stores.len() == 1 && !held.contains(&Pt::Unknown);
        for k in stores {
            if wp.as_ref().is_some_and(|wp| wp.contains(&Pt::Store(k))) {
                // The witness may name it: it ends where the witness does not, and where the
                // slot names only this store it stays live exactly where the witness names it.
                if let Some(w) = w
                    && single
                    && let Some(info) = st.stores.get_mut(&k)
                    && info.has(LIVE)
                    && !info.live.contains(&w)
                    && wp.as_ref().is_some_and(|wp| wp.len() > 1)
                {
                    self.sites[k.site as usize].ends.insert(label.clone());
                    info.status |= ENDED;
                    info.live.insert(w);
                    for a in &mut info.alts {
                        a.insert(w);
                    }
                }
                continue;
            }
            self.sites[k.site as usize].ends.insert(label.clone());
            if let Some(info) = st.stores.get_mut(&k)
                && info.has(LIVE)
            {
                if single {
                    info.status = (info.status & !LIVE) | ENDED;
                    info.live.clear();
                    info.alts.clear();
                } else {
                    info.status |= ENDED;
                }
            }
        }
        true
    }

    /// A capture SLOT as a guarded free's witness: the store is live exactly where the slot
    /// names it, so it joins the slot's correlation and goes where the record goes.
    pub(super) fn correlate_witness_slot(
        st: &mut State,
        get_dbref: u32,
        witness: Option<&Value>,
        k: SKey,
    ) {
        let Some(Value::Call(g, gargs)) = witness.map(Value::unspan) else {
            return;
        };
        if *g != get_dbref {
            return;
        }
        let (Some(r), Some(off)) = (Self::first_var(gargs, 0), Self::const_offset(gargs.get(1)))
        else {
            return;
        };
        for rec in Self::stores_of(&Self::pts(st, r)) {
            if let Some(slot) = st.slots.get_mut(&rec).and_then(|m| m.get_mut(&off))
                && slot.pts.contains(&Pt::Store(k))
            {
                slot.corr.insert(k);
            }
        }
    }

    /// A guarded free's stores the witness may name end only on the paths it does not: where
    /// `x` names one on every path it is live, it stays live exactly where the witness names
    /// it (`return if c { f } else { g }`).
    #[expect(clippy::too_many_arguments, reason = "the guarded free's own operands")]
    pub(super) fn end_where_witness_does_not(
        &mut self,
        st: &mut State,
        x: u16,
        xp: &Pts,
        w: Option<u16>,
        field_slot: bool,
        shared: &[SKey],
        args: &[Value],
        name: &str,
    ) {
        let label = format!("{name}({})", self.var_name(x));
        for k in shared {
            self.sites[k.site as usize].ends.insert(label.clone());
            let Some(info) = st.stores.get_mut(k) else {
                continue;
            };
            if !info.has(LIVE) {
                continue;
            }
            if field_slot {
                info.status = (info.status & !LIVE) | ENDED;
                info.live.clear();
                info.alts.clear();
                info.ended = [x].into();
                continue;
            }
            let strong = xp.len() == 1 || info.live.contains(&x);
            info.ended = if info.has(ENDED) {
                info.ended.intersection(&[x].into()).copied().collect()
            } else {
                [x].into()
            };
            info.status |= ENDED;
            if strong && let Some(w) = w {
                info.live.insert(w);
            }
            if strong && w.is_none() {
                Self::correlate_witness_slot(st, self.ops.get_dbref, args.get(1), *k);
            }
        }
    }

    pub(super) fn free_op(&mut self, d: u32, args: &[Value], st: State) -> State {
        let ops = self.ops;

        if ops.unconditional.contains(&d) || d == ops.text_free {
            let name = self.data.def(d).name().to_string();
            let mut st = st;
            if let Some(Value::Call(g, gargs)) = args.first().map(Value::unspan)
                && *g == ops.get_dbref
                && let Some(w) = Self::first_var(gargs, 0)
                && Self::is_record(&st, w)
            {
                self.end_slots(&mut st, w, &name);
                return st;
            }
            if d == ops.free_in {
                self.free_record_in(args, &name, &mut st);
                return st;
            }
            if let Some(x) = args.first().and_then(|a| Self::named_var(&st, a)) {
                // The code generator emits no `OpFreeRef` of a never-free variable
                // (`Codegen::generate_call`, S34), including one a move made never-free.
                let dropped = name == "OpFreeRef"
                    && x < MEMBER_BASE
                    && (self.data.def(self.d_nr).variables.is_skip_free(x)
                        || self.moved_away.contains(&x));
                // A text free of a tuple member releases the text in it; a nested member's
                // record stores (`u.2` of `(s, w, (h, w))`) are their work-refs' to free.
                let text_of_member = d == ops.text_free && x >= MEMBER_BASE;
                if !dropped && !text_of_member {
                    self.end(&mut st, x, &name, true);
                }
            }
            return st;
        }
        if ops.conditional.contains(&d) {
            // A free that is a no-op where the freed variable and its witness name one store.
            let name = self.data.def(d).name().to_string();
            let mut st = st;
            // A capture SLOT freed unless the witness names its store.
            if self.free_slot_unless_witness(args, &name, &mut st) {
                return st;
            }
            if let Some(x) = Self::first_var(args, 0) {
                let w = Self::first_var(args, 1);
                let xp = Self::pts(&st, x);
                let (wp, field_slot) = self.guard_witness(&st, w, args.get(1));
                let shared: Vec<SKey> = Self::stores_of(&xp)
                    .into_iter()
                    .filter(|k| wp.contains(&Pt::Store(*k)))
                    .collect();
                // Every store `x` may name is one the witness names — a caller's store (a
                // parameter's) as much as one of this frame's: the free declines on every path.
                let declines = xp.iter().any(|p| *p != Pt::Null)
                    && xp
                        .iter()
                        .all(|p| *p == Pt::Null || (!matches!(p, Pt::Unknown) && wp.contains(p)))
                    && xp.iter().any(|p| matches!(p, Pt::Param(_)));
                if declines {
                    // Nothing ends: the caller's store stays the caller's.
                } else if shared.is_empty() {
                    self.end(&mut st, x, &name, true);
                } else if xp.len() == 1 && wp.len() == 1 {
                    let label =
                        format!("{name}({}) skipped: its witness holds it", self.var_name(x));
                    self.sites[shared[0].site as usize].ends.insert(label);
                } else {
                    // Skipped on the paths where the witness names the store: a free of the
                    // rest, which is where the witness holds something else.
                    let witness_holds_it = shared
                        .iter()
                        .all(|k| w.is_some_and(|w| info_of(&st, *k).live.contains(&w)));
                    // The stores the witness cannot name end as any free ends them: the guard
                    // is distinct wherever `x` names one, so the free runs there — and frees
                    // again a store already freed, as a plain free would (`@FR-H-FreeTwice`).
                    let rest: BTreeSet<SKey> = Self::stores_of(&xp)
                        .into_iter()
                        .filter(|k| !shared.contains(k))
                        .collect();
                    if !rest.is_empty() {
                        self.end_only(&mut st, x, &name, guarded_rest_reports(), Some(&rest));
                    }
                    // A shared store already freed where `x` names it, on paths where the
                    // witness cannot point into it: the guard is distinct there, and the free
                    // runs a second time.
                    if guarded_rest_reports()
                        && let Some(w) = w
                    {
                        for k in &shared {
                            let info = info_of(&st, *k);
                            if info.has(ENDED) && info.ended.contains(&x) && !maybe_stale(&info, w)
                            {
                                let nm = self.var_name(x);
                                self.find(
                                    "re-free",
                                    k.site,
                                    format!("`{nm}` is freed by {name} after its store was already freed"),
                                );
                            }
                        }
                    }
                    if !witness_holds_it {
                        self.end_where_witness_does_not(
                            &mut st, x, &xp, w, field_slot, &shared, args, &name,
                        );
                    }
                }
            }
            return st;
        }
        st
    }

    /// `OpPlaceRecord(host, tp)`: a record inside the host's store, or the root of a fresh
    /// store where the host is null (`@FR-R-Place`).
    pub(super) fn place(&mut self, node: &Value, host: &Val, mut s: State) -> (State, Val) {
        let mut pts: Pts = host
            .pts
            .iter()
            .copied()
            .filter(|p| *p != Pt::Null)
            .collect();
        let mut corr = BTreeSet::new();
        if host.pts.contains(&Pt::Null) {
            let site = self.site(node, Origin::Place);
            let k = Self::fresh(&mut s, site);
            if !pts.is_empty()
                && let Some(info) = s.stores.get_mut(&k)
            {
                info.status = LIVE | ABSENT;
            }
            pts.insert(Pt::Store(k));
            corr.insert(k);
        }
        (s, Val { pts, corr })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one branch per op family the records give a meaning to"
    )]
    pub(super) fn call(
        &mut self,
        node: &Value,
        d: u32,
        args: &[Value],
        st: State,
    ) -> Option<(State, Val)> {
        let ops = self.ops;
        // `@FR-R-HostStore`: a result its function drops is released by record — its store whole
        // where that is not the answer's, as `OpFreeRefIfDistinct(rec, answer)` frees it; a record
        // of the answer's store goes and the store lives on.
        if self.data.def(d).name() == "OpReleaseRecIfDistinct"
            && let Some(free_if) = ops
                .conditional
                .iter()
                .copied()
                .find(|c| self.data.def(*c).name() == "OpFreeRefIfDistinct")
        {
            return Some((
                self.free_op(free_if, &args[..args.len().min(2)], st),
                Val::null(),
            ));
        }
        if ops.unconditional.contains(&d) || d == ops.text_free || ops.conditional.contains(&d) {
            let mut st = self.free_op(d, args, st);
            if self.resets.contains(&(std::ptr::from_ref(node) as usize))
                && let Some(x) = Self::first_var(args, 0)
            {
                // The free's variable is reset to null right after it.  Where it had (maybe)
                // ended a store, the store keeps that fact under a placeholder name, so a
                // later free of the same store by another name still reads as a second one.
                for info in st.stores.values_mut() {
                    forget_holder(info, x);
                    if info.ended.remove(&x) {
                        info.ended.insert(FREED_NAME);
                    }
                    if let Some(sn) = info.stale.as_mut()
                        && sn.remove(&x)
                    {
                        sn.insert(FREED_NAME);
                    }
                }
                st.vars.remove(&x);
            }
            // Reset where the guard let the free run: `x` names only what the witness may name
            // (the guard declined there), or null.
            if self
                .cond_resets
                .contains(&(std::ptr::from_ref(node) as usize))
                && let (Some(x), Some(w)) = (Self::first_var(args, 0), Self::first_var(args, 1))
            {
                let wp = Self::pts(&st, w);
                let kept: Pts = Self::pts(&st, x)
                    .into_iter()
                    .filter(|p| matches!(p, Pt::Store(_) | Pt::Param(_)) && wp.contains(p))
                    .chain(std::iter::once(Pt::Null))
                    .collect();
                for (k, info) in &mut st.stores {
                    if !kept.contains(&Pt::Store(*k)) {
                        forget_holder(info, x);
                        if info.ended.remove(&x) {
                            info.ended.insert(FREED_NAME);
                        }
                    }
                }
                if kept.len() == 1 {
                    st.vars.remove(&x);
                } else {
                    st.vars.insert(x, kept);
                }
            }
            return Some((st, Val::null()));
        }
        // `OpFnRefDetachShared(old, new)` (loft#1609): the closure half of `old` is nulled where
        // it shares a store with `new` — the rebind or link write handed that closure on, so
        // the release of `old` that follows finds nothing.  Followed where the share is
        // certain: `old` names exactly the one store `new` names.  Elsewhere `old` keeps its
        // stores, which can only keep a finding, never invent one.
        if self.data.def(d).name() == "OpFnRefDetachShared"
            && let (Some(old), Some(new)) = (Self::first_var(args, 0), Self::first_var(args, 1))
            && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_DETACH_SHARED").is_some())
        {
            let held = Self::pts(&st, old);
            let named = Self::pts(&st, new);
            let mut s = st;
            let one = matches!(Self::stores_of(&held)[..], [k]
                if held.iter().all(|p| *p == Pt::Store(k))
                    && named.iter().all(|p| *p == Pt::Store(k))
                    && !named.is_empty());
            // …or several, where both names must-hold each of them (a copy `k = h` of a
            // fn-ref built in branches): wherever one of those stores is live, both name it.
            let same = !held.is_empty()
                && held == named
                && held.iter().all(|p| matches!(p, Pt::Store(_)))
                && Self::stores_of(&held).iter().all(|k| {
                    let info = info_of(&s, *k);
                    info.has(LIVE) && info.live.contains(&old) && info.live.contains(&new)
                });
            // …or `new` is a plain copy of `old` neither has been bound away from since
            // (`k = h`): the two name one value on every path, so they share every store.
            let copy = copy_alias()
                && s.copies.contains(&(old.min(new), old.max(new)))
                && !held.is_empty()
                && held == named;
            if copy {
                // Wherever `old` held a store, its copy `new` holds it too: `new` takes over
                // `old`'s holder role before `old` lets go.
                for info in s.stores.values_mut() {
                    if info.live.contains(&old) {
                        info.live.insert(new);
                    }
                    for alt in &mut info.alts {
                        if alt.contains(&old) {
                            alt.insert(new);
                        }
                    }
                    // A name now in every alternative holds the store on every path.
                    if let Some(first) = info.alts.first() {
                        let every: BTreeSet<u16> = info
                            .alts
                            .iter()
                            .skip(1)
                            .fold(first.clone(), |m, a| m.intersection(a).copied().collect());
                        info.live.extend(every);
                        let live = info.live.clone();
                        info.alts = normal_alts(std::mem::take(&mut info.alts), &live);
                    }
                }
            }
            if one || same || copy {
                self.bind(&mut s, old, &Val::null());
            }
            return Some((s, Val::null()));
        }
        // `OpInitRefSentinel(v)`: `v` is reset to null and names no store from here.
        if self.data.def(d).name() == "OpInitRefSentinel"
            && let Some(x) = Self::first_var(args, 0)
        {
            let mut s = st;
            self.bind(&mut s, x, &Val::null());
            return Some((s, Val::null()));
        }
        // `OpPutRef(dst, src)`: a raw copy of the reference — `dst` names exactly the stores
        // `src` names.  A never-free `dst` is a witness (an entry snapshot `__ref_1 = v`,
        // `ensure_rebind_witness`) and owns none of them; any other takes them over (a buffer
        // rotation `__ref_1 = __ref_2; __ref_2 = l`).
        if self.data.def(d).name() == "OpPutRef"
            && let Some(dst) = Self::first_var(args, 0)
            && let Some(src) = args.get(1)
        {
            let (mut s, value) = self.eval(src, st)?;
            let witness = dst < self.data.def(self.d_nr).variables.count()
                && self.data.def(self.d_nr).variables.is_skip_free(dst);
            let bound = if witness {
                Val {
                    pts: value.pts,
                    corr: BTreeSet::new(),
                }
            } else {
                value
            };
            self.bind(&mut s, dst, &bound);
            if !witness && let Some(sv) = Self::first_var(args, 1) {
                Self::copy_alts(&mut s, dst, sv);
            }
            return Some((s, Val::null()));
        }
        if d == ops.database
            && let Some(x) = Self::first_var(args, 0)
        {
            // `OpDatabase` re-inits a LIVE store in place and mints only where the variable is
            // null (or names a freed store): a callee filling its hidden buffer answers the
            // caller's store, not one of its own (`state/io.rs::database`).
            let cur = Self::pts(&st, x);
            let kept: Pts = cur
                .iter()
                .copied()
                .filter(|p| match p {
                    Pt::Param(_) => true,
                    // A store this frame handed on (written back through a `&` parameter,
                    // into a field) still exists: the slot naming it is re-initialised in place.
                    Pt::Store(k) => {
                        let info = info_of(&st, *k);
                        (info.has(LIVE) || info.has(HANDED)) && !info.has(ENDED)
                    }
                    _ => false,
                })
                .collect();
            let mut st = st;
            // A loop pass that freed this site's store and reset `x` joins, at the head, a pass
            // that kept it: live where `x` names it, ended where `x` is null.  Re-initialised in
            // place on the first paths and minted on the others, `x` names a live store from
            // this site after either, and only `x` must-holds it.  The ended store a stale name
            // may still reach moves to the older generation.
            let here = self
                .site_of
                .get(&(std::ptr::from_ref(node) as usize))
                .copied();
            if let Some(site) = here
                && let [k] = Self::stores_of(&cur)[..]
                && k.site == site
                && !k.older
                && cur.iter().all(|p| matches!(p, Pt::Null | Pt::Store(_)))
                && !st.slots.contains_key(&k)
                && let info = info_of(&st, k)
                && info.has(LIVE)
                && info.has(ENDED)
                && !info.has(HANDED)
                && info.live.contains(&x)
                && info.captured_by.is_empty()
            {
                let old = SKey { site, older: true };
                let mut ended = info.ended.clone();
                ended.remove(&x);
                let part = SInfo {
                    status: info.status & !LIVE,
                    ended,
                    ..Default::default()
                };
                let merged = match st.stores.get(&old) {
                    Some(o) => join_info(o, &part),
                    None => part,
                };
                st.stores.insert(old, merged);
                for (&v, p) in &mut st.vars {
                    if v != x && p.contains(&Pt::Store(k)) {
                        p.insert(Pt::Store(old));
                    }
                }
                for p in st.points.values_mut() {
                    if p.contains(&Pt::Store(k)) {
                        p.insert(Pt::Store(old));
                    }
                }
                st.stores.insert(
                    k,
                    SInfo {
                        status: LIVE,
                        ..Default::default()
                    },
                );
                let val = Val {
                    pts: [Pt::Store(k)].into(),
                    corr: [k].into(),
                };
                self.bind(&mut st, x, &val);
                return Some((st, Val::null()));
            }
            // A HIDDEN buffer parameter is null where the caller handed the sentinel, and this
            // `OpDatabase` mints there (`Origin::MintInto`): read as re-initialised only, the
            // function answered the parameter alone (`Arg`), and a caller handing null lost the
            // store it made (`tail = if … { unspan(…) } else { NdNull }`).
            // `LOFT_RECORDS_NO_HIDDEN_PARAM_MINT=1` reads it as re-initialised.
            let hidden_param = !crate::env_once!(
                std::env::var_os("LOFT_RECORDS_NO_HIDDEN_PARAM_MINT").is_some()
            ) && cur.contains(&Pt::Param(x))
                // A RECORD return's buffer: a collection's is minted by the caller before the
                // call (`@FR-O-LazyBuffer`), so the callee never meets its null.
                && !crate::parser::vectors::is_collection(
                    self.data.def(self.d_nr).returned().base(),
                )
                && self.params.iter().position(|q| *q == x).is_some_and(|i| {
                    self.data
                        .def(self.d_nr)
                        .attributes()
                        .get(i)
                        .is_some_and(|a| a.hidden)
                });
            let val = if !kept.is_empty() && kept.len() == cur.len() && !hidden_param {
                Val {
                    corr: Self::stores_of(&kept).into_iter().collect(),
                    pts: kept,
                }
            } else if let Some(own) = {
                // A loop's head joins the null of the first pass with the store an earlier
                // pass made here: on the null path this `OpDatabase` makes that same store
                // (later passes find it live and re-init it in place), not another generation.
                let site = self
                    .site_of
                    .get(&(std::ptr::from_ref(node) as usize))
                    .copied();
                site.and_then(|site| {
                    kept.iter().find_map(|p| match p {
                        Pt::Store(k) if k.site == site && !k.older => Some(*k),
                        _ => None,
                    })
                })
            } {
                // A store of another site `x` must-holds is re-initialised in place where `x`
                // names it, so it stays correlated with `x` as well.
                let mut corr: BTreeSet<SKey> = Self::stores_of(&kept)
                    .into_iter()
                    .filter(|k| info_of(&st, *k).live.contains(&x))
                    .collect();
                corr.insert(own);
                Val { pts: kept, corr }
            } else {
                let origin = self
                    .params
                    .iter()
                    .position(|p| *p == x)
                    .map_or(Origin::Mint, |i| Origin::MintInto(i as u16));
                let site = self.site(node, origin);
                // A store that may also have ended (a loop that ran no pass joins the store an
                // earlier pass freed) is still `x`'s where it is live, when `x` names it wherever
                // it is live: re-initialised in place there, minted on the other paths.
                let mut kept = kept;
                for p in &cur {
                    if let Pt::Store(k) = p
                        && k.site != site
                        && let info = info_of(&st, *k)
                        && info.has(LIVE)
                        && info.live.contains(&x)
                    {
                        kept.insert(*p);
                    }
                }
                // A store `x` holds is re-initialised in place, so it stays live exactly where
                // `x` names it — as correlated with `x` as the store the null path makes.
                let mut corr: BTreeSet<SKey> = Self::stores_of(&kept)
                    .into_iter()
                    .filter(|k| info_of(&st, *k).live.contains(&x))
                    .collect();
                let k = Self::fresh(&mut st, site);
                corr.insert(k);
                let mut pts = kept;
                pts.insert(Pt::Store(k));
                Val { pts, corr }
            };
            // Compared with what `x` names NOW: a mint at the site of a store `x` named moved
            // that store to the older generation, so `x` names the older key while the new
            // store took the key `x` named before.
            if val.pts != Self::pts(&st, x) {
                self.bind(&mut st, x, &val);
            }
            return Some((st, Val::null()));
        }
        match self.data.def(d).name() {
            "n_protect_store_frees" => self.protected.extend(Self::first_var(args, 0)),
            "n_unprotect_store_frees" => {
                if let Some(v) = Self::first_var(args, 0)
                    && let Some(i) = self.protected.iter().rposition(|p| *p == v)
                {
                    self.protected.remove(i);
                }
            }
            _ => {}
        }
        let (mut s, vals) = if self.summaries.contains_key(&d) {
            self.user_call_args(args, st)?
        } else if matches!(
            self.data.def(d).name(),
            "OpEqRef" | "OpNeRef" | "OpStoreLive" | "OpRefIsNull" | "OpConvBoolFromRef"
        ) || (d == self.ops.distinct && identity_test_no_use())
        {
            // A reference COMPARISON reads the two references, never the stores they name:
            // comparing a freed store's reference with another is no use of it — the store
            // identity test (`OpDistinctStore`, `a.store_nr != b.store_nr`) too.  So are the
            // tests of a reference itself — null, naming a record — and the liveness test
            // (`@FR-L-CapOwn`'s guard), which exists to be asked of a store that may be freed.
            let mut s = st;
            let mut vals = Vec::with_capacity(args.len());
            for a in args {
                let (s2, value) = self.eval(a, s)?;
                vals.push(value);
                s = s2;
            }
            (s, vals)
        } else if d == self.ops.set_dbref && args.len() == 3 && !set_dbref_value_use() {
            // A reference WRITTEN into a slot is handed over, not read: the record (written)
            // and the offset are uses, the value's store is not touched.
            let (s, mut vals) = self.args(&args[..2], st)?;
            let (s, value) = self.eval(&args[2], s)?;
            vals.push(value);
            (s, vals)
        } else {
            self.args(args, st)?
        };
        // A copy that frees its source store (`COPY_FREE_SOURCE`, `OpSets::freed_source`):
        // the source's stores end here, except one the destination names or a bracket
        // protects, whose release the runtime declines.  A source that is no variable (a
        // join `if c { keep } else { mk() }`) frees what its value names.
        if let Some((src, dst)) = self.data.op_sets().freed_source(d, args) {
            let x = Self::first_var(args, src).or_else(|| args.get(src).and_then(Self::tail_var));
            let kept: BTreeSet<SKey> = vals
                .get(dst)
                .map(|v| v.pts.clone())
                .into_iter()
                .chain(self.protected.iter().map(|p| Self::pts(&s, *p)))
                .flat_map(|pts| Self::stores_of(&pts))
                .collect();
            let name = format!("{} (its source)", self.data.def(d).name());
            // A source that may be a PARAMETER's store — a call that answers its argument on
            // some path (`@FR-F-Ret`'s join) — is released on that path too: the source-free
            // has no runtime guard but the bracket and the destination.  The caller still
            // owns that store.  `LOFT_RECORDS_NO_PARAM_SOURCE_FREE=1` leaves it unreported.
            let kept_params: BTreeSet<Pt> = vals
                .get(dst)
                .map(|v| v.pts.clone())
                .into_iter()
                .chain(self.protected.iter().map(|p| Self::pts(&s, *p)))
                .flat_map(|pts| pts.into_iter().filter(|p| matches!(p, Pt::Param(_))))
                .collect();
            let src_pts = x.map_or_else(
                || vals.get(src).map(|v| v.pts.clone()).unwrap_or_default(),
                |x| Self::pts(&s, x),
            );
            if src_pts
                .iter()
                .any(|p| matches!(p, Pt::Param(_)) && !kept_params.contains(p))
                && !crate::env_once!(
                    std::env::var_os("LOFT_RECORDS_NO_PARAM_SOURCE_FREE").is_some()
                )
            {
                let what = x.map_or_else(
                    || "the value".to_string(),
                    |x| format!("`{}`", self.var_name(x)),
                );
                self.find(
                    "wrong-end",
                    u32::MAX,
                    format!("{what} may be the caller's store, and {name} releases it"),
                );
            }
            if let Some(x) = x {
                let freed: BTreeSet<SKey> = Self::stores_of(&Self::pts(&s, x))
                    .into_iter()
                    .filter(|k| !kept.contains(k))
                    .collect();
                if !freed.is_empty() {
                    self.end_only(&mut s, x, &name, true, Some(&freed));
                }
            } else if let Some(v) = vals.get(src) {
                // A store correlated with the value is live only where the value names it, so
                // the free ends it outright; any other only where the value may name it.
                for k in Self::stores_of(&v.pts) {
                    if kept.contains(&k) {
                        continue;
                    }
                    let pts = if v.corr.contains(&k) {
                        [Pt::Store(k)].into()
                    } else {
                        v.pts.clone()
                    };
                    s.vars.insert(FREED_NAME - 1, pts);
                    self.end_only(&mut s, FREED_NAME - 1, &name, true, Some(&[k].into()));
                }
                s.vars.remove(&(FREED_NAME - 1));
            }
        }
        let def = self.data.def(d);
        if def.name().ends_with("_OpDropAll")
            && let Some(w) = Self::first_var(args, 0)
            && Self::is_record(&s, w)
        {
            self.end_slots(&mut s, w, "its cascade");
        }
        // `OpDropFnRef(f)` runs the same cascade on the record the fn-ref names (@FR-L-CapOwn,
        // loft#1609): where the record's own release stood down (`___clos_N` reset because a
        // fn-ref still names it), this is where its captures end.
        // `LOFT_RECORDS_NO_DROP_FNREF_CASCADE=1` leaves them live.
        // A closure type whose cascade was retired (`Data::retire_def_name`: an earlier build
        // owns the capture) releases nothing, so only a record every one of whose possible
        // types keeps a live `_OpDropAll` is cascaded; a mix is left alone, which can only
        // keep a finding.
        if def.name() == "OpDropFnRef"
            && !self
                .standdown_drops
                .contains(&(std::ptr::from_ref(node) as usize))
            && let Some(f) = Self::first_var(args, 0)
            && Self::is_record(&s, f)
            && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_DROP_FNREF_CASCADE").is_some())
        {
            // The emitter's own split (`get_free_vars::fnref_stand_down`): while a local
            // closure-record variable the frame releases still names the record, that record's
            // release (its hook) is the cascade, and the fn-ref's drop is the one that stands
            // down.  A record the frame never releases itself (written into a fn-ref through a
            // link) ends by the fn-ref's drop alone.
            // And a record another name may still hold (`k = h`, then `return k`) is not this
            // drop's: the detach ahead of it (`OpFnRefDetachShared`) nulls `f` wherever the two
            // share, which the records may not have followed.  A link onto `f`'s variable is not
            // another holder, nor is a closure variable the frame never releases.
            let function = &self.data.def(self.d_nr).variables;
            let records: Vec<SKey> = Self::stores_of(&Self::pts(&s, f))
                .into_iter()
                .filter(|k| {
                    !s.vars.iter().any(|(&v, p)| {
                        v != f
                            && p.contains(&Pt::Store(*k))
                            && (v >= function.count()
                                || self.released_records.contains(&v)
                                || !(function.name(v).starts_with("___clos_")
                                    || matches!(
                                        function.tp(v).base(),
                                        crate::data::Type::RefVar(_)
                                    )))
                    })
                })
                .collect();
            let freed: Option<Vec<(SKey, BTreeSet<i32>)>> = records
                .iter()
                .map(|k| self.cascade_frees(*k).map(|offs| (*k, offs)))
                .collect();
            if let Some(freed) = freed {
                let name = self.var_name(f);
                for (k, offs) in freed {
                    self.end_record_slots(&mut s, k, &offs, &name);
                }
            }
        }
        // `OpCopyRecord(src, dst, tp)` copies the record's reference fields: a closure record
        // displaced into a copy (`__disp_N`, a captured type with a drop hook) leaves the stores
        // its slots name to the copy, whose cascade then releases them.
        if d == ops.copy_record
            && let (Some(src), Some(dst)) = (Self::first_var(args, 0), Self::first_var(args, 1))
        {
            let from: Vec<SKey> = Self::stores_of(&Self::pts(&s, src));
            let merged: Option<BTreeMap<i32, Val>> = from
                .iter()
                .filter_map(|k| s.slots.get(k))
                .fold(None, |acc: Option<BTreeMap<i32, Val>>, m| match acc {
                    None => Some(m.clone()),
                    Some(mut a) => {
                        for (o, v) in m {
                            let e = a.entry(*o).or_default();
                            e.pts.extend(v.pts.iter().copied());
                            e.corr = e.corr.intersection(&v.corr).copied().collect();
                        }
                        Some(a)
                    }
                });
            if let Some(merged) = merged {
                for k in Self::stores_of(&Self::pts(&s, dst)) {
                    s.slots.insert(k, merged.clone());
                }
            }
        }
        // A null written into a capture slot (`OpSetDbRef(w, off, OpNullRefSentinel())`, a
        // stand-down letting a record give up its capture): the slot holds null from here.
        if d == ops.set_dbref
            && slot_null()
            && let Some(value) = vals.get(2)
            && !value
                .pts
                .iter()
                .any(|p| matches!(p, Pt::Store(_) | Pt::Param(_) | Pt::Unknown))
            && let Some(w) = Self::first_var(args, 0)
            && let Some(off) = Self::const_offset(args.get(1))
        {
            let recs = Self::stores_of(&Self::pts(&s, w));
            let strong = recs.len() == 1;
            for k in recs {
                if let Some(v) = s.slots.get_mut(&k).and_then(|m| m.get_mut(&off)) {
                    if strong {
                        v.pts = [Pt::Null].into();
                        v.corr.clear();
                    } else {
                        v.pts.insert(Pt::Null);
                    }
                }
            }
        }
        if d == ops.set_dbref
            && let Some(value) = vals.get(2)
        {
            match Self::first_var(args, 0) {
                // A capture into a closure record that stays in this frame: the record holds
                // the store for as long as it lives, and the frame still releases it.
                Some(w)
                    if self
                        .data
                        .def(self.d_nr)
                        .variables
                        .name(w)
                        .starts_with("___clos_")
                        && !self.leaving.contains(&w) =>
                {
                    self.hold(
                        &mut s,
                        w,
                        Self::const_offset(args.get(1)),
                        value,
                        Self::first_var(args, 2),
                    );
                }
                // A reference written into another store's field: that store holds it now —
                // unless a variable the program named still holds it, or it is the host's own
                // store.  A stored reference then only points (`reference<T>`), and the named
                // owner still releases the store (`u`'s child linked into `k.to`).
                // A capture into a record built inside another store (a fn field): the
                // record takes it over, and its slot still answers a guard that reads it.
                Some(w)
                    if self
                        .data
                        .def(self.d_nr)
                        .variables
                        .name(w)
                        .starts_with("___clos_") =>
                {
                    s.field_captures
                        .entry(w)
                        .or_default()
                        .extend(value.pts.iter().copied());
                    for k in Self::stores_of(&value.pts) {
                        if let Some(info) = s.stores.get_mut(&k) {
                            info.captured_by.insert(w);
                        }
                    }
                    Self::hand_on_var(&mut s, Self::first_var(args, 2), value);
                }
                _ => {
                    let function = &self.data.def(self.d_nr).variables;
                    let host: BTreeSet<SKey> = vals
                        .first()
                        .map(|h| Self::stores_of(&h.pts).into_iter().collect())
                        .unwrap_or_default();
                    // …or a compiler temporary the function frees: a stored reference only points
                    // (`@FR-H-RefHome`), and the temporary is the record's home.
                    let freed_temps = &self.freed_temps;
                    let owned_elsewhere = |k: &SKey| {
                        host.contains(k)
                            || info_of(&s, *k).live.iter().any(|w| {
                                *w < function.count()
                                    && (!function.is_compiler_generated(*w)
                                        || freed_temps.contains(w))
                            })
                    };
                    let handed = Val {
                        pts: value
                            .pts
                            .iter()
                            .copied()
                            .filter(|p| !matches!(p, Pt::Store(k) if owned_elsewhere(k)))
                            .collect(),
                        corr: value
                            .corr
                            .iter()
                            .copied()
                            .filter(|k| !owned_elsewhere(k))
                            .collect(),
                    };
                    // What the frame still owns, the holder only points at.
                    let pointed: Pts = value
                        .pts
                        .iter()
                        .copied()
                        .filter(|p| matches!(p, Pt::Store(k) if owned_elsewhere(k)))
                        .collect();
                    Self::hand_on(&mut s, &handed);
                    if let Some(h) = Self::first_var(args, 0) {
                        uncover_all(&mut s, &pointed);
                        s.points.entry(h).or_default().extend(pointed);
                    }
                }
            }
        }
        // A fn-ref answer carries its closure record's store (`FnRef(_, w)` reads `w`), which
        // its type does not say: a callee handing its fn-typed parameter back (`fn pass(f) ->
        // fn(…) { f }`, summary `Arg`) answers the record its CALLER built, and the caller's
        // holder then names it beside the record's own local (loft#2512).  A closure the callee
        // builds and answers (`Fresh`) still reads as holding nothing: following it through a
        // rebind and a branch join is a step the records do not take yet.
        // `LOFT_RECORDS_NO_FN_RETURN=1` reads every fn-ref answer as holding nothing.
        let fn_answer = matches!(def.returned().base(), crate::data::Type::Function(..))
            && matches!(
                self.summaries.get(&d),
                Some(Summary::Arg(_) | Summary::Args(_))
            )
            && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_FN_RETURN").is_some());
        if def.returned().heap_dep().is_none() && !fn_answer {
            return Some((s, Val::null()));
        }
        let bodied = !matches!(def.code.unspan(), Value::Null) && !def.name().starts_with("Op");
        if !bodied {
            // An operator answering a reference answers one into its first argument's store;
            // a native's answer is not followed.
            if d == ops.place {
                let host = vals.into_iter().next().unwrap_or_else(Val::unknown);
                return Some(self.place(node, &host, s));
            }
            let val = if d == ops.get_dbref {
                // A stored reference read answers its REFERENT, which lives in another store
                // than the record holding it (`cur = cur.next`): what the records know that
                // slot holds, or nothing they follow.
                let held = match Self::first_var(args, 0) {
                    Some(w) => {
                        let mut h = Self::held_by_record(&s, w, Self::const_offset(args.get(1)));
                        // A reference this frame wrote into the holder (`a.next = b`).
                        h.extend(s.points.get(&w).into_iter().flatten().copied());
                        h
                    }
                    None => Pts::new(),
                };
                // loft#2504 — a slot of the lambda's own closure record (its hidden
                // `__closure` parameter) holds a store its caller's record owns: a capture
                // read out of it, and relayed into a record this frame builds, is the
                // caller's store, never this function's to end.  Asked of `__closure` alone:
                // a drop cascade's `self` is a parameter too, and ending what it holds is its
                // job.  `LOFT_RECORDS_NO_CLOSURE_SLOT_PARAM=1` leaves the read unknown.
                let relayed = Self::first_var(args, 0).filter(|&w| {
                    let frame = &self.data.def(self.d_nr).variables;
                    frame.is_argument(w) && frame.name(w) == "__closure"
                });
                if held.is_empty()
                    && let Some(w) = relayed
                    && !crate::env_once!(
                        std::env::var_os("LOFT_RECORDS_NO_CLOSURE_SLOT_PARAM").is_some()
                    )
                {
                    Val {
                        pts: [Pt::Param(w)].into(),
                        corr: BTreeSet::new(),
                    }
                } else if held.is_empty() {
                    Val::unknown()
                } else {
                    Val {
                        pts: held,
                        corr: BTreeSet::new(),
                    }
                }
            } else if def.name() == "OpStoreBufOnly" {
                // `@FR-R-HostStore`: the buffer where it is a store buffer, else null.
                let mut v = vals.into_iter().next().unwrap_or_else(Val::unknown);
                v.pts.insert(Pt::Null);
                v.corr.clear();
                v
            } else if def.name() == "OpNullRefSentinel" {
                // The null reference: no store.  Read as an operator answering into its
                // (absent) first argument it was unknown, and every `-> S?` callee whose null
                // arm spells it answered Unknown instead of Fresh.
                Val::null()
            } else if def.name() == "OpCopyClosure" {
                // A fn-ref copied out of a field is a copy (`(L-CapKeep)`): the closure record
                // is copied into a store of its own, which the copy's holder frees.
                let site = self.site(node, Origin::Copy);
                let k = Self::fresh(&mut s, site);
                Val {
                    pts: [Pt::Store(k)].into(),
                    corr: [k].into(),
                }
            } else if def.name().starts_with("Op")
                && def
                    .attributes
                    .first()
                    .is_some_and(|a| matches!(a.typedef.base(), crate::data::Type::Text(_)))
            {
                // …unless that argument holds no store to answer into: a parse
                // (`OpCastVectorFromText(text, tp)`) builds its answer in a store of its own.
                let site = self.site(node, Origin::Call(d));
                let k = Self::fresh(&mut s, site);
                Val {
                    pts: [Pt::Store(k)].into(),
                    corr: [k].into(),
                }
            } else if def.name().starts_with("Op") {
                vals.into_iter().next().unwrap_or_else(Val::unknown)
            } else {
                self.native_answer(node, d, &vals, &mut s)
            };
            return Some((s, val));
        }
        let val = match self.summaries.get(&d).copied().unwrap_or(Summary::Unknown) {
            Summary::Bottom => Val::null(),
            Summary::Unknown => Val::unknown(),
            Summary::Fresh => {
                let site = self.site(node, Origin::Call(d));
                let k = Self::fresh(&mut s, site);
                // A `-> T?` callee may answer null instead: the store is live only where the
                // answer names it.
                let mut pts: Pts = [Pt::Store(k)].into();
                if matches!(def.returned(), crate::data::Type::Optional(_)) {
                    pts.insert(Pt::Null);
                }
                Val {
                    pts,
                    corr: [k].into(),
                }
            }
            // The argument's store, BORROWED: the answer owns none of it, so a copy or a join
            // guard that consumes what its value owns leaves it with the argument's owner.
            Summary::Arg(i) => vals.get(i as usize).map_or_else(Val::unknown, |v| Val {
                pts: v.pts.clone(),
                corr: BTreeSet::new(),
            }),
            // A join over an argument that is NULL here is the callee's own mint: a callee
            // that answers its hidden buffer or a store it made, handed a null buffer, makes one
            // (`OpDatabase` on null).
            // One of several arguments' stores, borrowed: the answer owns none of them.
            Summary::Args(m) => {
                let mut pts = Pts::new();
                for (i, v) in vals.iter().enumerate() {
                    if i < 32 && m & (1u32 << i) != 0 {
                        pts.extend(v.pts.iter().copied());
                    }
                }
                Val {
                    pts,
                    corr: BTreeSet::new(),
                }
            }
            // `@FR-R-HostStore`: a callee handed a STORE buffer (`OpStoreBuf(host)`, never null)
            // claims every answer in that store, so it answers `host`'s store and mints none.
            Summary::Join(i) | Summary::Buf(i)
                if args.get(i as usize).is_some_and(|a| {
                    matches!(a.unspan(), Value::Call(sb, _)
                        if self.data.def(*sb).name() == "OpStoreBuf")
                }) && vals.get(i as usize).is_some_and(|v| !v.pts.is_empty()) =>
            {
                let pts: Pts = vals[i as usize]
                    .pts
                    .iter()
                    .copied()
                    .filter(|p| *p != Pt::Null)
                    .collect();
                Val {
                    pts,
                    corr: BTreeSet::new(),
                }
            }
            // A buffer-filler handed "null, or the store this same call made on an earlier
            // pass" (a loop's head): the null path makes that store's role again, so the call
            // answers that store — not one more generation of it.
            Summary::Buf(i)
                if vals.get(i as usize).is_some_and(|v| {
                    let site = self
                        .site_of
                        .get(&(std::ptr::from_ref(node) as usize))
                        .copied();
                    let stores = Self::stores_of(&v.pts);
                    !stores.is_empty()
                        && stores.iter().all(|k| Some(k.site) == site && !k.older)
                        && v.pts.iter().all(|p| matches!(p, Pt::Null | Pt::Store(_)))
                }) =>
            {
                let v = &vals[i as usize];
                let pts: Pts = v.pts.iter().copied().filter(|p| *p != Pt::Null).collect();
                Val {
                    corr: Self::stores_of(&pts).into_iter().collect(),
                    pts,
                }
            }
            Summary::Join(i) | Summary::Buf(i)
                if vals
                    .get(i as usize)
                    .is_some_and(|v| v.pts.iter().all(|p| *p == Pt::Null)) =>
            {
                let site = self.site(node, Origin::Call(d));
                let k = Self::fresh(&mut s, site);
                Val {
                    pts: [Pt::Store(k)].into(),
                    corr: [k].into(),
                }
            }
            // A buffer-filling callee handed a store answers that store, borrowed, as `Arg`.
            Summary::Buf(i)
                if vals.get(i as usize).is_some_and(|v| {
                    !v.pts.is_empty()
                        && v.pts
                            .iter()
                            .all(|p| matches!(p, Pt::Store(_) | Pt::Param(_)))
                }) =>
            {
                let v = &vals[i as usize];
                let mut pts = v.pts.clone();
                let mut corr = BTreeSet::new();
                // A PARAMETER buffer is null where this function's caller handed null: the
                // callee mints there, into the parameter, as `OpDatabase` on it does — this
                // function then answers `Buf` of that parameter, not `Arg` (`field_get` filling
                // its hidden buffer through `cl`).  Read as `Arg`, a caller handing a null
                // buffer saw a null answer and lost the store.
                // `LOFT_RECORDS_NO_PARAM_BUF_MINT=1` answers the parameter alone.
                if !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_PARAM_BUF_MINT").is_some())
                    // The parameter itself, not a place in it (`OpPlaceRecord(sc)`, never null).
                    && let Some(Value::Var(x)) = args.get(i as usize).map(Value::unspan)
                    && v.pts.contains(&Pt::Param(*x))
                    && let Some(param_at) = self.params.iter().position(|q| q == x)
                {
                    let site = self.site(node, Origin::MintInto(param_at as u16));
                    let minted = Self::fresh(&mut s, site);
                    if let Some(info) = s.stores.get_mut(&minted) {
                        info.status = LIVE | ABSENT;
                    }
                    pts.insert(Pt::Store(minted));
                    // The answer owns what the callee minted, as on the `OpDatabase` path.
                    corr.insert(minted);
                }
                Val { pts, corr }
            }
            Summary::Join(i) | Summary::Buf(i) => {
                let viewed = vals.get(i as usize).cloned().unwrap_or_else(Val::unknown);
                let site = self.site(node, Origin::Call(d));
                let k = Self::fresh(&mut s, site);
                if let Some(info) = s.stores.get_mut(&k) {
                    info.status = LIVE | ABSENT;
                }
                // A null argument is the path the callee mints on: it answers the fresh store
                // there, never null — unless its answer is `-> T?` itself.
                let mut pts = viewed.pts;
                pts.remove(&Pt::Null);
                if matches!(def.returned(), crate::data::Type::Optional(_)) {
                    pts.insert(Pt::Null);
                }
                pts.insert(Pt::Store(k));
                Val {
                    pts,
                    corr: [k].into(),
                }
            }
        };
        self.call_write_backs(&mut s, node, d, args);
        Some((s, val))
    }

    /// `@FR-F-ParamRef` at the call: a callee that writes a fresh value through its `&`
    /// parameter on every path ([`writes_back_fresh`]) hands that value to the variable the
    /// argument links (`f(OpCreateStack(w))`), and releases the store `w` held (the callee's
    /// `OpFreeRefIfDistinct`).  A store the call bracket protects across the call itself
    /// (`LOFT_NO_PROTECT_AFTER_CALL`) is not released: the runtime declines it, and nothing
    /// holds it once `w` names the new store.  A callee that writes on some paths only, or
    /// writes a borrowed value, is not followed: `w` keeps what it held, which can only keep a
    /// finding (`LOFT_RECORDS_NO_CALL_WRITE_BACK=1` leaves every write-back unfollowed).
    pub(super) fn call_write_backs(&mut self, s: &mut State, node: &Value, d: u32, args: &[Value]) {
        if crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_CALL_WRITE_BACK").is_some()) {
            return;
        }
        let bracket = self
            .call_bracket
            .as_ref()
            .filter(|(n, _)| *n == std::ptr::from_ref(node.unspan()) as usize)
            .map(|(_, p)| p.clone())
            .unwrap_or_default();
        for (i, a) in args.iter().enumerate() {
            let Value::Call(op, cargs) = a.unspan() else {
                continue;
            };
            if self.data.def(*op).name() != "OpCreateStack" {
                continue;
            }
            let Some(Value::Var(w)) = cargs.first().map(Value::unspan) else {
                continue;
            };
            let write = write_back_of(self.data, d, i);
            if write == WriteBack::None {
                continue;
            }
            // A write inside a branch happens on some runs: the caller's state after the call
            // is the join of the written one and the one that kept the old store (`join_arms`).
            let kept = (write == WriteBack::Maybe).then(|| s.clone());
            let gone: BTreeSet<SKey> = Self::stores_of(&Self::pts(s, *w))
                .into_iter()
                .filter(|k| !bracket.contains(k))
                .collect();
            if !gone.is_empty() {
                self.end_only(
                    s,
                    *w,
                    "the write-back through its `&` argument",
                    true,
                    Some(&gone),
                );
            }
            let site = self.site(a, Origin::Call(d));
            let k = Self::fresh(s, site);
            let value = Val {
                pts: [Pt::Store(k)].into(),
                corr: [k].into(),
            };
            self.bind(s, *w, &value);
            if let Some(kept) = kept {
                *s = join_arms(s, &kept);
            }
        }
    }
}

/// Does `d` write a FRESH value through its `i`-th parameter, a `&` link to a store, on every
/// path — a whole write (`p = P { … }`) at the top level of its body?  The code generator
/// releases the store the parameter displaced there (`@FR-F-ParamRef`).  A write inside a
/// branch or a loop, or of a value the function does not own, answers no: the caller then
/// keeps reading what its variable held, which can only keep a finding.
/// How a callee writes a fresh value through a `&` parameter: never, on every path (a
/// top-level whole write), or on some paths (a whole write inside a branch).
#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteBack {
    None,
    Always,
    Maybe,
}

/// [`writes_back_fresh`] for the top level, and the same write found inside an `if` arm for
/// `Maybe` (`LOFT_RECORDS_NO_BRANCH_WRITE_BACK=1` reads such a write as none).  A write inside a
/// loop body counts as `Maybe` too: the loop may run no pass.
fn write_back_of(data: &Data, d: u32, i: usize) -> WriteBack {
    if writes_back_fresh(data, d, i) {
        return WriteBack::Always;
    }
    if crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_BRANCH_WRITE_BACK").is_some()) {
        return WriteBack::None;
    }
    let def = data.def(d);
    let Some(p) = def.attributes.get(i).map(|_| i as u16) else {
        return WriteBack::None;
    };
    if !fresh_write_target(data, d, p) {
        return WriteBack::None;
    }
    if def.code.any_node(&mut |n| fresh_write_of(data, d, p, n)) {
        WriteBack::Maybe
    } else {
        WriteBack::None
    }
}

/// Is parameter `p` of `d` a `&` link to a store a whole write can replace?
fn fresh_write_target(data: &Data, d: u32, p: u16) -> bool {
    let vars = &data.def(d).variables;
    if !vars.is_argument(p) {
        return false;
    }
    let crate::data::Type::RefVar(inner) = vars.tp(p).base() else {
        return false;
    };
    inner.heap_def_nr().is_some()
        || matches!(
            inner.base(),
            crate::data::Type::Vector(..) | crate::data::Type::Enum(_, true, _)
        )
        || crate::data::is_keyed(inner.base())
}

/// Is `op` a whole write of a value `d` owns through parameter `p`?
fn fresh_write_of(data: &Data, d: u32, p: u16, op: &Value) -> bool {
    matches!(op.unspan(), Value::Set(v, rhs) if *v == p
    && !matches!(rhs.unspan(), Value::Call(c, _)
        if matches!(data.def(*c).name(), "OpCreateStack" | "OpVarRef"))
    && matches!(
        crate::use_analysis::ownership_of(data, d, rhs),
        crate::use_analysis::Own::Owned
    ))
}

fn writes_back_fresh(data: &Data, d: u32, i: usize) -> bool {
    let def = data.def(d);
    let vars = &def.variables;
    let Some(p) = def.attributes.get(i).map(|_| i as u16) else {
        return false;
    };
    if !vars.is_argument(p) {
        return false;
    }
    let crate::data::Type::RefVar(inner) = vars.tp(p).base() else {
        return false;
    };
    if !(inner.heap_def_nr().is_some()
        || matches!(
            inner.base(),
            crate::data::Type::Vector(..) | crate::data::Type::Enum(_, true, _)
        )
        || crate::data::is_keyed(inner.base()))
    {
        return false;
    }
    let ops = match def.code.unspan() {
        Value::Block(b) => &b.operators,
        _ => return false,
    };
    ops.iter().any(|op| {
        matches!(op.unspan(), Value::Set(v, rhs) if *v == p
        && !matches!(rhs.unspan(), Value::Call(c, _)
            if matches!(data.def(*c).name(), "OpCreateStack" | "OpVarRef"))
        && matches!(
            crate::use_analysis::ownership_of(data, d, rhs),
            crate::use_analysis::Own::Owned
        ))
    })
}
