// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Branch decisions: what a condition proves about the stores a path holds — live, null, or two
//! names for distinct stores (`OpDistinctStore`) — and the narrowing of each arm's state by it.

use super::{
    ABSENT, BTreeSet, Data, ENDED, HANDED, LIVE, Origin, Pt, Pts, Run, SKey, State, Summary, Value,
    field_record_holds, info_of, join_opt_arms, rule_out_everywhere, rule_out_holder,
    slot_corr_narrow, slot_dead_arm, slot_holders_off, slot_null, views_enabled,
};

impl Run<'_> {
    /// The states the two arms of an `if` on `cond` start from, as (then, else); `None` is
    /// an arm the test rules out.  A test the records cannot read keeps both arms with the
    /// state unchanged, which is sound: narrowing only ever removes paths a test excludes.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per test shape the records can read; the split is release work (M-file-split)"
    )]
    pub(super) fn decide(&self, cond: &Value, st: State) -> (Option<State>, Option<State>) {
        let both = |st: State| (Some(st.clone()), Some(st));
        match cond.unspan() {
            Value::Boolean(true) => (Some(st), None),
            Value::Var(b) if st.flags.contains_key(b) => {
                if st.flags[b] {
                    (Some(st), None)
                } else {
                    (None, Some(st))
                }
            }
            Value::Boolean(false) => (None, Some(st)),
            Value::Call(op, a) if *op == self.ops.not && a.len() == 1 => {
                let (t, f) = self.decide(&a[0], st);
                (f, t)
            }
            // A condition that is itself a choice: `a && b` and `a || b` lower to this.
            Value::If(c, t, e) => {
                let (ct, cf) = self.decide(c, st);
                let (tt, tf) = ct.map_or((None, None), |s| self.decide(t, s));
                let (et, ef) = cf.map_or((None, None), |s| self.decide(e, s));
                (join_opt_arms(tt, et), join_opt_arms(tf, ef))
            }
            // `OpEqInt(OpGetInt4(x, 0), 0)`: a nullable struct-enum's absent tag, the second half
            // of its null test (`x == null` is `OpRefIsNull(x) || tag(x) == 0`).  No local OWNS a
            // tag-0 COPY — copying an absent field answers null — so the absent side drops `x`'s copies.  Without it
            // the `x == null` arm kept `x`'s copy and a rebind there read as a leak (`cond =
            // mc.cond` under `if cond == null`).  OPT-IN (`LOFT_RECORDS_TAG_NULL_TEST=1`): a MINTED record
            // can carry a non-absent variant the records do not know, so the absent side keeps it,
            // and suite B-Copy/a-branch-handing-back-… then reads one false finding where the
            // default reads none — the precise answer needs the variant a mint writes.
            Value::Call(op, a)
                if a.len() == 2
                    && self.data.def(*op).name() == "OpEqInt"
                    && matches!(a[1].unspan(), Value::Int(0))
                    && crate::env_once!(
                        std::env::var_os("LOFT_RECORDS_TAG_NULL_TEST").is_some()
                    )
                    && let Value::Call(g, ga) = a[0].unspan()
                    && self.data.def(*g).name() == "OpGetInt4"
                    && matches!(ga.get(1).map(Value::unspan), Some(Value::Int(0)))
                    && let Some(Value::Var(x)) = ga.first().map(Value::unspan)
                    && *x < self.data.def(self.d_nr).variables.count()
                    && matches!(
                        self.data.def(self.d_nr).variables.tp(*x),
                        crate::data::Type::Optional(inner) if matches!(inner.base(), crate::data::Type::Enum(_, true, _))
                    ) =>
            {
                // The absent side: `x` is null, or a record with the absent tag — never a COPY
                // (copying an absent field answers null), but a call's answer through a buffer can
                // be one (`fold(n) -> Own?` answering its buffer marked absent).  So only the copies
                // leave `x` there.  The present side names a store.
                let (_, present) = Self::null_test(*x, st.clone());
                let mut absent = st;
                let keep: Pts = Self::pts(&absent, *x)
                    .into_iter()
                    .filter(|p| match p {
                        Pt::Store(k) => {
                            !matches!(self.sites[k.site as usize].origin, Some(Origin::Copy))
                        }
                        _ => true,
                    })
                    .collect();
                if keep.is_empty() {
                    (None, present)
                } else {
                    Self::narrow(&mut absent, *x, keep);
                    (Some(absent), present)
                }
            }
            Value::Call(op, a) if *op == self.ops.distinct => {
                let (distinct, mut same) = self.decide_distinct(a, st);
                // Where the two are the SAME, every name set holding one holds the other.
                if let (Some(x), Some(w)) =
                    (self.operand_var(a.first()), self.operand_var(a.get(1)))
                    && let Some(s) = &mut same
                {
                    Self::equate(s, x, w);
                }
                // `OpDistinctStore(x, OpGetDbRef(w, off))` with `w` a fn field's record: where
                // they are the SAME, the record's slot names `x`'s store, so the record holds it.
                if let (Some(x), Some(Value::Call(g, ga))) =
                    (self.operand_var(a.first()), a.get(1).map(Value::unspan))
                    && *g == self.ops.get_dbref
                    && let Some(Value::Var(w)) = ga.first().map(Value::unspan)
                    && let Some(s) = &mut same
                    && (s.field_captures.contains_key(w) || self.is_closure_record(*w))
                    && field_record_holds()
                {
                    // Every name of the record holds it: `w`, and a fn-ref holding the same
                    // record (`___clos_1` is nulled where `g` takes it over).
                    let rec = Self::stores_of(&Self::pts(s, *w));
                    let names: Vec<u16> = s
                        .vars
                        .iter()
                        .filter(|(v, p)| {
                            **v == *w || rec.iter().any(|k| p.contains(&Pt::Store(*k)))
                        })
                        .map(|(v, _)| *v)
                        .collect();
                    for k in Self::stores_of(&Self::pts(s, x)) {
                        if let Some(info) = s.stores.get_mut(&k)
                            && info.has(LIVE)
                        {
                            info.live.extend(names.iter().copied());
                            for alt in &mut info.alts {
                                alt.extend(names.iter().copied());
                            }
                        }
                    }
                }
                (distinct, same)
            }
            Value::Call(op, a) if a.len() == 1 => {
                if *op == self.ops.store_live
                    && let Some(arms) = self.slot_live_test(&a[0], &st)
                {
                    return arms;
                }
                if (*op == self.ops.ref_present || *op == self.ops.ref_is_null)
                    && let Some((present, null)) = self.slot_present_test(&a[0], &st)
                {
                    return if *op == self.ops.ref_present {
                        (present, null)
                    } else {
                        (null, present)
                    };
                }
                let Value::Var(x) = a[0].unspan() else {
                    return both(st);
                };
                if *op == self.ops.ref_is_null {
                    Self::null_test(*x, st)
                } else if *op == self.ops.vector_is_null && self.only_call_answers(&st, *x) {
                    // A collection a call answered is null only as the null sentinel: a
                    // `-> hash<…>?` hands back no absent-marked store (loft#1150).  A local's
                    // null may be such a store, so for it the test decides nothing.
                    Self::null_test(*x, st)
                } else if *op == self.ops.ref_present {
                    // `OpConvBoolFromRef(x)`: true when `x` names a record.  A freed store
                    // still names one, so this is the null test turned round, never a
                    // liveness test.
                    let (null, present) = Self::null_test(*x, st);
                    (present, null)
                } else if *op == self.ops.store_live {
                    let (live, dead) = Self::live_test(*x, st);
                    (live, self.no_record_holds(*x, dead))
                } else {
                    both(st)
                }
            }
            _ => both(st),
        }
    }

    /// A fn field overwritten while it held the frame's own closure record (a field reset or
    /// set nulls `___clos_N` where the field's old record is it): the record leaves the field, and
    /// the stores it took over at the capture are the frame's again, held by whatever still
    /// names them (`h.f = fn() { v.a }; h.f = id` — the frame then frees `v`).
    pub(super) fn give_back_field_captures(st: &mut State, w: u16) {
        let Some(taken) = st.field_captures.get(&w).cloned() else {
            return;
        };
        for k in Self::stores_of(&taken) {
            let holders: BTreeSet<u16> = st
                .vars
                .iter()
                .filter(|(_, p)| p.len() == 1 && p.contains(&Pt::Store(k)))
                .map(|(v, _)| *v)
                .collect();
            if let Some(info) = st.stores.get_mut(&k)
                && info.has(HANDED)
                && info.captured_by.contains(&w)
            {
                info.status = (info.status & !HANDED) | LIVE;
                info.live = holders;
                info.handed_by.clear();
            }
        }
    }

    /// A closure record variable (`___clos_N`): its capture slots hold stores.
    pub(super) fn is_closure_record(&self, w: u16) -> bool {
        self.data
            .def(self.d_nr)
            .variables
            .name(w)
            .starts_with("___clos_")
    }

    /// On a path where the record closure record `x` names is not live, its slots hold nothing:
    /// the holder sets naming it do not hold there.  (A NULL `x` says less: the record may have
    /// been handed to a fn-ref and live on.)
    pub(super) fn no_record_holds(&self, x: u16, st: Option<State>) -> Option<State> {
        let mut st = st?;
        if self.is_closure_record(x) && !slot_holders_off() {
            rule_out_everywhere(&mut st, x);
        }
        Some(st)
    }

    /// Does every store `x` may name come from a call's answer?
    pub(super) fn only_call_answers(&self, st: &State, x: u16) -> bool {
        let stores = Self::stores_of(&Self::pts(st, x));
        !stores.is_empty()
            && stores
                .iter()
                .all(|k| matches!(self.sites[k.site as usize].origin, Some(Origin::Call(_))))
    }

    /// `OpRefIsNull(x)`, as (null arm, store arm).  Where `x` is null, a store `x` points
    /// into on every path where that store is live is not live, and one it points into on
    /// every path where it is ended is not ended.
    pub(super) fn null_test(x: u16, st: State) -> (Option<State>, Option<State>) {
        let pts = Self::pts(&st, x);
        let may_null = !st.nonnull.contains(&x)
            && pts
                .iter()
                .any(|p| matches!(p, Pt::Null | Pt::Param(_) | Pt::Unknown));
        let may_store = pts.iter().any(|p| !matches!(p, Pt::Null));
        let null_arm = may_null
            .then(|| {
                let mut s = st.clone();
                for k in Self::stores_of(&pts) {
                    if let Some(info) = s.stores.get_mut(&k) {
                        rule_out_holder(info, x);
                        // Null on this arm: `x` names none of its stores here.
                        if let Some(sn) = info.stale.as_mut() {
                            sn.remove(&x);
                        }
                        if info.ended.contains(&x) {
                            info.status &= !ENDED;
                            info.ended.clear();
                        }
                        if info.status == 0 {
                            info.status = ABSENT;
                        }
                    }
                }
                s.vars.remove(&x);
                Self::record_null_possible(&mut s, x).then_some(s)
            })
            .flatten();
        let store_arm = may_store
            .then(|| {
                let mut s = st;
                if let Some(p) = s.vars.get_mut(&x) {
                    p.remove(&Pt::Null);
                }
                Self::record_present_possible(&mut s, x).then_some(s)
            })
            .flatten();
        (null_arm, store_arm)
    }

    /// On a path where the closure-record variable `x` names no record: a store live only
    /// where one of its `live_some` records does is not live here once `x` was the last.  `false` when that leaves a store no status — the
    /// path does not happen.
    pub(super) fn record_null_possible(st: &mut State, x: u16) -> bool {
        for info in st.stores.values_mut() {
            if info.has(LIVE) && info.live_some.remove(&x) && info.live_some.is_empty() {
                info.status &= !LIVE;
                info.live.clear();
                info.alts.clear();
                if info.status == 0 {
                    return false;
                }
            }
        }
        true
    }

    /// On a path where the closure-record variable `x` names a record: a store live only
    /// where it is null is not live here.  `false` when that leaves a store no status.
    pub(super) fn record_present_possible(st: &mut State, x: u16) -> bool {
        for info in st.stores.values_mut() {
            if info.has(LIVE) && info.live_null.contains(&x) {
                info.status &= !LIVE;
                info.live.clear();
                info.alts.clear();
                info.live_null.clear();
                if info.status == 0 {
                    return false;
                }
            }
        }
        true
    }

    /// `OpStoreLive(x)` (`@FR-L-CapOwn`'s guard): is the store `x` names still allocated?
    /// A store handed on keeps both answers: its new holder may or may not have freed it.
    pub(super) fn live_test(x: u16, st: State) -> (Option<State>, Option<State>) {
        let pts = Self::pts(&st, x);
        let followed = !pts.iter().any(|p| matches!(p, Pt::Param(_) | Pt::Unknown));
        let stores: Vec<SKey> = Self::stores_of(&pts).into_iter().collect();
        // Live: `x` names a store, and that store is live or handed on.
        let live_arm = {
            let mut s = st.clone();
            if let Some(p) = s.vars.get_mut(&x) {
                p.remove(&Pt::Null);
            }
            let mut possible = !followed || !stores.is_empty();
            for k in &stores {
                if let Some(info) = s.stores.get_mut(k) {
                    let only = followed && stores.len() == 1;
                    if only || info.ended.contains(&x) {
                        info.status &= !(ENDED | ABSENT);
                        info.ended.clear();
                    }
                    if only && info.status == 0 {
                        possible = false;
                    }
                }
            }
            if !Self::record_present_possible(&mut s, x) {
                possible = false;
            }
            possible.then_some(s)
        };
        // Not live: `x` is null, or the store it names has ended (or was never made).
        let dead_arm = {
            let mut s = st;
            let mut possible = !followed || pts.contains(&Pt::Null) || stores.is_empty();
            for k in &stores {
                if let Some(info) = s.stores.get_mut(k) {
                    rule_out_holder(info, x);
                    if info.status & !LIVE != 0 {
                        possible = true;
                    }
                    if info.status == 0 {
                        info.status = ABSENT;
                    }
                } else {
                    possible = true;
                }
            }
            // Where no store `x` names can have ended, the dead arm is `x` null.
            let as_null = followed && stores.iter().all(|k| info_of(&s, *k).status & !ABSENT == 0);
            if as_null && !Self::record_null_possible(&mut s, x) {
                possible = false;
            }
            possible.then_some(s)
        };
        (live_arm, dead_arm)
    }

    /// The variable whose value `val` is on every path where that variable names a store: a
    /// plain read or an alias (`OpRefAlias`), or a call answering an argument's store as it
    /// is (`Arg`, or `Buf` where the buffer it is handed names one).
    pub(super) fn copied_from(&self, val: &Value) -> Option<u16> {
        match val.unspan() {
            Value::Var(y) => Some(*y),
            Value::Call(d, args)
                if (*d as usize) < self.data.definitions.len()
                    && self.data.def(*d).name() == "OpRefAlias" =>
            {
                Self::first_var(args, 0)
            }
            Value::Call(d, args) => match self.summaries.get(d) {
                Some(Summary::Arg(i) | Summary::Buf(i)) => Self::first_var(args, *i as usize),
                // `@FR-R-HostStore`: a callee handed a STORE buffer answers in that store.
                Some(Summary::Join(i)) => match args.get(*i as usize).map(Value::unspan) {
                    Some(Value::Call(sb, a)) if self.data.def(*sb).name() == "OpStoreBuf" => {
                        Self::first_var(a, 0)
                    }
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    /// `dst` now names what `src` names wherever `src` names a store: an alternative holding
    /// `src` holds `dst` as well.
    pub(super) fn copy_alts(st: &mut State, dst: u16, src: u16) {
        for info in st.stores.values_mut() {
            for a in &mut info.alts {
                if a.contains(&src) {
                    a.insert(dst);
                }
            }
        }
    }

    /// On a path where `x` and `w` name one store: a must-holder set or an alternative
    /// holding either holds both.
    pub(super) fn equate(st: &mut State, x: u16, w: u16) {
        for info in st.stores.values_mut() {
            if !info.has(LIVE) {
                continue;
            }
            if info.live.contains(&x) || info.live.contains(&w) {
                info.live.insert(x);
                info.live.insert(w);
            }
            for a in &mut info.alts {
                if a.contains(&x) || a.contains(&w) {
                    a.insert(x);
                    a.insert(w);
                }
            }
        }
    }

    /// The variable a projection chain starts from (`a.b[0].c` → `a`), through spans.
    pub(super) fn projection_root<'v>(data: &Data, v: &'v Value) -> &'v Value {
        match v.unspan() {
            Value::Call(op, a)
                if crate::use_analysis::is_projection_op(data, *op) && !a.is_empty() =>
            {
                Self::projection_root(data, &a[0])
            }
            other => other,
        }
    }

    /// The variable an `OpDistinctStore` operand names a store through: the variable itself,
    /// or a fn-ref whose closure record it is (`OpFnRefClosure(g)`).
    pub(super) fn operand_var(&self, v: Option<&Value>) -> Option<u16> {
        match v?.unspan() {
            Value::Var(x) => Some(*x),
            Value::Call(op, a) if *op == self.ops.fnref_closure => Self::first_var(a, 0),
            _ => None,
        }
    }

    /// `OpDistinctStore(a, b)`: where both operands name one single store (or both are
    /// null), the two are that store on every path and the DISTINCT arm cannot run; where
    /// they name two different single stores, the SAME arm cannot.  Anything wider keeps
    /// both arms.  An operand may be a variable, a closure's record (`OpFnRefClosure`) or a
    /// capture slot read out of a record that stays in this frame (`OpGetDbRef`).
    pub(super) fn decide_distinct(
        &self,
        args: &[Value],
        st: State,
    ) -> (Option<State>, Option<State>) {
        // A view pair narrows `x` further on both sides of what the rest decides (`view_pair_arms`).
        if let Some((x, w)) = Self::view_pair(args, &st) {
            let wp = Self::pts(&st, w);
            let (distinct, same) = self.decide_distinct_plain(args, st);
            let distinct = distinct.and_then(|mut d| {
                let xp: Pts = Self::pts(&d, x)
                    .into_iter()
                    .filter(|p| !matches!(p, Pt::Store(k) if wp.contains(&Pt::Store(*k))))
                    .collect();
                if xp.is_empty() {
                    return None;
                }
                Self::narrow(&mut d, x, xp);
                Some(d)
            });
            let same = same.and_then(|mut sm| {
                let xp = Self::pts(&sm, x);
                let keep: Pts = xp.iter().copied().filter(|p| wp.contains(p)).collect();
                if keep.is_empty() {
                    return None;
                }
                if keep != xp {
                    Self::narrow(&mut sm, x, keep);
                }
                Some(sm)
            });
            return (distinct, same);
        }
        self.decide_distinct_plain(args, st)
    }

    /// The view pair (`State::views`) an identity test of two variables reads, as `(view, base)`.
    pub(super) fn view_pair(args: &[Value], st: &State) -> Option<(u16, u16)> {
        if !views_enabled() {
            return None;
        }
        let (Some(Value::Var(x)), Some(Value::Var(w))) = (
            args.first().map(Value::unspan),
            args.get(1).map(Value::unspan),
        ) else {
            return None;
        };
        if st.views.contains(&(*x, *w)) {
            Some((*x, *w))
        } else if st.views.contains(&(*w, *x)) {
            Some((*w, *x))
        } else {
            None
        }
    }

    /// `OpDistinctStore` without the view pairs: the identity test as the stores alone decide it.
    pub(super) fn decide_distinct_plain(
        &self,
        args: &[Value],
        st: State,
    ) -> (Option<State>, Option<State>) {
        let operand = |v: &Value| -> Option<Pts> {
            match v.unspan() {
                Value::Var(x) => Some(Self::pts(&st, *x)),
                Value::TupleGet(..) => Self::named_var(&st, v).map(|k| Self::pts(&st, k)),
                Value::Call(op, a) if *op == self.ops.fnref_closure => {
                    match a.first().map(Value::unspan) {
                        Some(Value::Var(g)) => Some(Self::pts(&st, *g)),
                        _ => None,
                    }
                }
                Value::Call(op, a) if *op == self.ops.get_dbref => {
                    match a.first().map(Value::unspan) {
                        Some(Value::Var(w)) => {
                            Some(Self::held_by_record(&st, *w, Self::const_offset(a.get(1))))
                                .filter(|p| !p.is_empty())
                        }
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        let (Some(a), Some(b)) = (
            args.first().and_then(operand),
            args.get(1).and_then(operand),
        ) else {
            return (Some(st.clone()), Some(st));
        };
        let single = |p: &Pts| match p.iter().next() {
            Some(q @ (Pt::Store(_) | Pt::Null | Pt::Param(_))) if p.len() == 1 => Some(*q),
            _ => None,
        };
        if let Some(arms) = self.slot_arms(args, &a, &b, &st) {
            return arms;
        }
        match (single(&a), single(&b)) {
            (Some(x), Some(y)) if x == y => (None, Some(st)),
            (Some(Pt::Store(_) | Pt::Null), Some(Pt::Store(_) | Pt::Null)) => (Some(st), None),
            // A store this frame made is never the caller's (`if OpDistinctStore(f, __rbw_f)`
            // after `f` was rebound to a fresh store).
            (Some(Pt::Store(_)), Some(Pt::Param(_))) | (Some(Pt::Param(_)), Some(Pt::Store(_))) => {
                (Some(st), None)
            }
            // The witness names one store: where the first operand is DISTINCT from it, it
            // does not name that store (`if OpDistinctStore(__ref_1, __rbw___ref_1)` — the
            // caller's buffer is never the one freed under it).
            (_, Some(y @ (Pt::Store(_) | Pt::Param(_))))
                if let Some(x) = self.operand_var(args.first())
                    && a.len() > 1
                    && a.contains(&y) =>
            {
                let mut distinct = st.clone();
                let rest: Pts = a.iter().copied().filter(|p| *p != y).collect();
                Self::narrow(&mut distinct, x, rest);
                let mut same = st;
                Self::narrow(&mut same, x, [y].into());
                (Some(distinct), Some(same))
            }
            // The witness is null: where the first operand is the SAME, it is null too.
            (_, Some(Pt::Null))
                if let Some(x) = self.operand_var(args.first())
                    && a.len() > 1
                    && a.contains(&Pt::Null)
                    && !a.contains(&Pt::Unknown) =>
            {
                let mut same = st.clone();
                Self::narrow(&mut same, x, [Pt::Null].into());
                let mut distinct = st;
                let rest: Pts = a.iter().copied().filter(|p| *p != Pt::Null).collect();
                Self::narrow(&mut distinct, x, rest);
                (Some(distinct), Some(same))
            }
            // Where the two are the SAME, the first names only what the witness may name too:
            // a store both may name, or null or a caller's store where the witness may be null
            // or a caller's store (two parameters may name one store, or both be null).
            _ if let Some(x) = self.operand_var(args.first())
                && !a.contains(&Pt::Unknown)
                && !b.contains(&Pt::Unknown) =>
            {
                let b_null = b.iter().any(|p| matches!(p, Pt::Null | Pt::Param(_)));
                let same_pts: Pts = a
                    .iter()
                    .copied()
                    .filter(|p| {
                        b.contains(p)
                            || (*p == Pt::Null && b_null)
                            // A caller's store may be another parameter's, or null.
                            || (matches!(p, Pt::Param(_)) && b_null)
                    })
                    .collect();
                if same_pts.is_empty() {
                    return (Some(st), None);
                }
                let mut same = st.clone();
                if same_pts != a {
                    Self::narrow(&mut same, x, same_pts);
                }
                (self.distinct_narrowed(args, x, &a, st), Some(same))
            }
            _ => (Some(st.clone()), Some(st)),
        }
    }

    /// The DISTINCT arm of `OpDistinctStore(x, w)`: `x` names no store the witness holds
    /// wherever it is live, as `x` does, since there the two are one store (`__ref_p2_1` and
    /// the tuple member `b.0` it built, after a loop that ran no pass).  Where the witness
    /// alone holds such a store, `x` names it only where it is live, so there too the two are
    /// one store.  Where that rules out every store `x` names, the arm cannot run.
    pub(super) fn distinct_narrowed(
        &self,
        args: &[Value],
        x: u16,
        a: &Pts,
        st: State,
    ) -> Option<State> {
        let w = args
            .get(1)
            .and_then(|v| Self::named_var(&st, v).or_else(|| self.operand_var(Some(v))));
        let distinct_pts: Pts = a
            .iter()
            .copied()
            .filter(|p| match (p, w) {
                (Pt::Store(k), Some(w)) => {
                    let info = info_of(&st, *k);
                    !(info.has(LIVE)
                        && info.live.contains(&w)
                        && (info.live.contains(&x) || !info.has(ENDED)))
                }
                _ => true,
            })
            .collect();
        if distinct_pts.is_empty() {
            return None;
        }
        // The same for the witness: a store both name on every path where it is live is named
        // by neither where they are distinct (`if OpDistinctStore(x, __lift) null else __lift =
        // null`, loft#2477: the lift does not hold `x`'s store on the arm that keeps it).  Read
        // before `x` is narrowed, which leaves that store not live here.
        // `LOFT_RECORDS_NO_DISTINCT_WITNESS_NARROW=1` narrows `x` alone.
        let w_keep: Option<(u16, Pts)> = w
            .filter(|&w| {
                w != x
                    && !crate::env_once!(
                        std::env::var_os("LOFT_RECORDS_NO_DISTINCT_WITNESS_NARROW").is_some()
                    )
                    && matches!(args.get(1).map(Value::unspan), Some(Value::Var(v)) if *v == w)
            })
            .and_then(|w| {
                let wp = Self::pts(&st, w);
                let keep: Pts = wp
                    .iter()
                    .copied()
                    .filter(|p| match p {
                        Pt::Store(k) => {
                            let info = info_of(&st, *k);
                            !(info.has(LIVE) && info.live.contains(&w) && info.live.contains(&x))
                        }
                        _ => true,
                    })
                    .collect();
                (keep != wp).then_some((w, keep))
            });
        let mut distinct = st;
        if distinct_pts != *a {
            Self::narrow(&mut distinct, x, distinct_pts);
        }
        if let Some((w, keep)) = w_keep {
            if keep.is_empty() {
                return None;
            }
            Self::narrow(&mut distinct, w, keep);
        }
        Some(distinct)
    }

    /// On a path where `x` is known to name only `to`: a store it does not name there, and that
    /// is live only where `x` names it, is not live on this path.
    /// Keep, in the slot `off` of every record `w` names, only the stores `keep` admits — a
    /// path-sensitive refinement of what the slot holds on one arm of an identity test.  Its
    /// `corr` follows: a store dropped from the slot is not live through it.
    /// `OpDistinctStore(OpGetDbRef(w, off), v)` where `v` names one store `y` and the slot may
    /// hold `y` or others (a loop rebuild's guarded free of the record's old capture): where
    /// DISTINCT, the slot does not hold `y`; where SAME, only `y` — as a variable first operand
    /// is narrowed.  `LOFT_RECORDS_NO_SLOT_NARROW=1` keeps both arms unnarrowed.
    pub(super) fn slot_arms(
        &self,
        args: &[Value],
        a: &Pts,
        b: &Pts,
        st: &State,
    ) -> Option<(Option<State>, Option<State>)> {
        let Some(Value::Call(op, ga)) = args.first().map(Value::unspan) else {
            return None;
        };
        let (Some(Value::Var(w)), Some(off)) =
            (ga.first().map(Value::unspan), Self::const_offset(ga.get(1)))
        else {
            return None;
        };
        if *op != self.ops.get_dbref
            || crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_SLOT_NARROW").is_some())
        {
            return None;
        }
        let y = match b.iter().next() {
            Some(y @ Pt::Store(_)) if b.len() == 1 => Some(*y),
            _ => None,
        };
        let Some(y) = y.filter(|y| a.len() >= 2 && a.contains(y)) else {
            // The other operand names several stores (or null): where SAME, the slot names only
            // a store both may name; none at all, and the arm cannot run.
            if !slot_corr_narrow()
                || a.contains(&Pt::Unknown)
                || b.contains(&Pt::Unknown)
                || a.iter().any(|p| matches!(p, Pt::Param(_)))
                || b.iter().any(|p| matches!(p, Pt::Param(_)))
                || !a.iter().any(|p| matches!(p, Pt::Store(_)))
            {
                return None;
            }
            let both: Pts = a.intersection(b).copied().collect();
            if both.len() == a.len() {
                return None;
            }
            let same = if both.iter().any(|p| matches!(p, Pt::Store(_))) {
                let mut same = st.clone();
                Self::narrow_slot(&mut same, *w, off, |p| both.contains(p)).then_some(same)
            } else {
                None
            };
            return Some((Some(st.clone()), same));
        };
        let mut distinct = st.clone();
        let d_ok = Self::narrow_slot(&mut distinct, *w, off, |p| *p != y);
        let mut same = st.clone();
        let s_ok = Self::narrow_slot(&mut same, *w, off, |p| *p == y);
        Some((d_ok.then_some(distinct), s_ok.then_some(same)))
    }

    /// `OpStoreLive(OpGetDbRef(w, off))` (the frame end's let-go of a capture, `@FR-L-CapOwn`):
    /// where it holds, the slot names none of its stores that is freed on every path or was
    /// never made — the free it guards cannot reach one (`LOFT_RECORDS_NO_SLOT_LIVE_NARROW=1`).
    /// The other arm keeps the slot whole.
    /// `OpConvBoolFromRef(OpGetDbRef(w, off))`: the (present, null) arms, decided by what the
    /// slot may hold on this path — a store for the first, null for the second.  `None` where a
    /// record `w` may name has no such slot followed.
    pub(super) fn slot_present_test(
        &self,
        arg: &Value,
        st: &State,
    ) -> Option<(Option<State>, Option<State>)> {
        if !slot_null() {
            return None;
        }
        let Value::Call(g, ga) = arg.unspan() else {
            return None;
        };
        let (Some(Value::Var(w)), Some(off)) =
            (ga.first().map(Value::unspan), Self::const_offset(ga.get(1)))
        else {
            return None;
        };
        if *g != self.ops.get_dbref {
            return None;
        }
        let recs = Self::stores_of(&Self::pts(st, *w));
        let mut held: Vec<&Pts> = Vec::new();
        for k in &recs {
            held.push(&st.slots.get(k)?.get(&off)?.pts);
        }
        if held.is_empty() {
            return None;
        }
        let may_store = held
            .iter()
            .any(|p| p.iter().any(|q| matches!(q, Pt::Store(_) | Pt::Param(_))));
        let may_null = held.iter().any(|p| p.is_empty() || p.contains(&Pt::Null));
        Some((may_store.then(|| st.clone()), may_null.then(|| st.clone())))
    }

    pub(super) fn slot_live_test(
        &self,
        arg: &Value,
        st: &State,
    ) -> Option<(Option<State>, Option<State>)> {
        let Value::Call(g, ga) = arg.unspan() else {
            return None;
        };
        let (Some(Value::Var(w)), Some(off)) =
            (ga.first().map(Value::unspan), Self::const_offset(ga.get(1)))
        else {
            return None;
        };
        if *g != self.ops.get_dbref
            || crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_SLOT_LIVE_NARROW").is_some())
        {
            return None;
        }
        let held_pts = Self::held_by_record(st, *w, Some(off));
        let held = Self::stores_of(&held_pts);
        let dead: BTreeSet<SKey> = held
            .iter()
            .copied()
            .filter(|k| info_of(st, *k).status & (LIVE | HANDED) == 0)
            .collect();
        // The dead arm of a slot that names ONE store: that store is not live there.  Where the
        // records have it ended or absent on some path, the arm keeps those; where they have it
        // live on every path, the arm cannot run.
        let single = slot_dead_arm()
            && held.len() == 1
            && !held_pts
                .iter()
                .any(|p| matches!(p, Pt::Unknown | Pt::Param(_)));
        // …and a store correlated with the slot of the one record `w` names: live only where
        // the slot names it, and where it does, it is the store found dead.
        let recs = Self::stores_of(&Self::pts(st, *w));
        let corr: Vec<SKey> = if slot_dead_arm() && recs.len() == 1 {
            st.slots
                .get(&recs[0])
                .and_then(|m| m.get(&off))
                .map(|v| {
                    v.corr
                        .iter()
                        .copied()
                        .filter(|c| Self::sole_slot(st, recs[0], off, *c))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if dead.is_empty() && !single && corr.is_empty() {
            return None;
        }
        let mut live = st.clone();
        if !dead.is_empty() {
            let _ = Self::narrow_slot(
                &mut live,
                *w,
                off,
                |p| !matches!(p, Pt::Store(k) if dead.contains(k)),
            );
        }
        let mut not_live: Vec<SKey> = corr;
        if single {
            not_live.push(held[0]);
        }
        let mut d = st.clone();
        let mut feasible = true;
        for k in not_live {
            if let Some(i) = d.stores.get_mut(&k)
                && i.has(LIVE)
            {
                if i.status & (ENDED | ABSENT | HANDED) == 0 {
                    feasible = false;
                    break;
                }
                i.status &= !LIVE;
                i.live.clear();
                i.alts.clear();
            }
        }
        let dead_arm = feasible.then_some(d);
        Some((Some(live), dead_arm))
    }

    /// Is slot `off` of record `rec` the only holder of `k` — no variable, fn field or other
    /// slot names it?  A slot's correlation ("live only where the slot names it") is read only
    /// then: a store an older record's slot also holds is live through that one too.
    pub(super) fn sole_slot(st: &State, rec: SKey, off: i32, k: SKey) -> bool {
        let p = Pt::Store(k);
        !st.vars.values().any(|v| v.contains(&p))
            && !st.field_captures.values().any(|v| v.contains(&p))
            && st.slots.iter().all(|(r, m)| {
                m.iter()
                    .all(|(o, v)| (*r == rec && *o == off) || !v.pts.contains(&p))
            })
    }

    /// `false` where the narrowing leaves no path: a store live only where the slot names it,
    /// and live on every path, is dropped from the only record that could hold it.
    pub(super) fn narrow_slot(
        st: &mut State,
        w: u16,
        off: i32,
        keep: impl Fn(&Pt) -> bool,
    ) -> bool {
        let recs = Self::stores_of(&Self::pts(st, w));
        let mut gone: Vec<SKey> = Vec::new();
        if recs.len() == 1
            && let Some(v) = st.slots.get(&recs[0]).and_then(|m| m.get(&off))
        {
            gone = v
                .corr
                .iter()
                .copied()
                .filter(|c| !keep(&Pt::Store(*c)) && Self::sole_slot(st, recs[0], off, *c))
                .collect();
        }
        for k in &recs {
            if let Some(v) = st.slots.get_mut(k).and_then(|m| m.get_mut(&off)) {
                v.pts.retain(|p| !matches!(p, Pt::Store(_)) || keep(p));
                v.corr.retain(|c| keep(&Pt::Store(*c)));
            }
        }
        // A correlated store is live only where the slot names it: dropped from the slot of the
        // one record `w` names, it is not live on this path.
        if slot_corr_narrow() && recs.len() == 1 {
            for c in gone {
                if let Some(i) = st.stores.get_mut(&c)
                    && i.has(LIVE)
                {
                    if i.status & (ABSENT | ENDED | HANDED) == 0 {
                        return false;
                    }
                    i.status &= !LIVE;
                    i.live.clear();
                    i.alts.clear();
                }
            }
        }
        true
    }

    pub(super) fn narrow(st: &mut State, x: u16, to: Pts) {
        let from = Self::pts(st, x);
        // A plain copy of `x` still naming exactly what `x` did is the same value: narrowed too.
        let twins: Vec<u16> = st
            .copies
            .iter()
            .filter_map(|&(a, b)| match (a == x, b == x) {
                (true, _) => Some(b),
                (_, true) => Some(a),
                _ => None,
            })
            .filter(|&u| Self::pts(st, u) == from)
            .collect();
        for u in twins {
            Self::narrow_one(st, u, &from, to.clone());
        }
        Self::narrow_one(st, x, &from, to);
    }

    pub(super) fn narrow_one(st: &mut State, x: u16, from: &Pts, to: Pts) {
        for k in Self::stores_of(from) {
            if to.contains(&Pt::Store(k)) {
                continue;
            }
            if let Some(info) = st.stores.get_mut(&k)
                && info.has(LIVE)
            {
                rule_out_holder(info, x);
                if info.status == 0 {
                    st.stores.remove(&k);
                }
            }
        }
        if to.len() == 1 && to.contains(&Pt::Null) {
            st.vars.remove(&x);
        } else {
            st.vars.insert(x, to);
        }
    }
}
