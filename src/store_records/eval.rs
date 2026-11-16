// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Evaluating a statement: an expression's value and the stores it names, an assignment's binds
//! and the stores it displaces, the brackets that protect a call's arguments, statement sequences
//! and loops, and a call's arguments.

use super::*;

impl Run<'_> {
    /// Run `v` from `st`: the state it ends in and its value; `None` when no path falls
    /// through (a `break`, `continue` or `return`).
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per IR shape the records follow"
    )]
    #[expect(
        clippy::many_single_char_names,
        reason = "one arm per Value variant, each binding its fields by the IR's own short names"
    )]
    pub(super) fn eval(&mut self, v: &Value, st: State) -> Option<(State, Val)> {
        self.steps += 1;
        if self.steps > STEP_BUDGET {
            self.imprecise = true;
            return None;
        }
        if let Value::Line(n) = v {
            self.line = *n;
        }
        match v {
            Value::Span(b) => self.eval(&b.1, st),
            Value::Block(b) if matches!(b.name, "fn_ref_field_reset" | "fn_ref_field_set") => {
                self.in_field_reset += 1;
                let out = self.seq(&b.operators, st);
                self.in_field_reset -= 1;
                out
            }
            Value::Block(b) => self.seq(&b.operators, st),
            Value::Insert(ops) => self.seq(ops, st),
            Value::Loop(b) => self.run_loop(&b.operators, st),
            Value::Var(x) => {
                let mut val = Self::read(&st, *x);
                // A tuple carries its members: handed on, copied or returned, they go with it.
                for (_, p) in Self::members(&st, *x) {
                    val.pts.extend(p.iter().copied().filter(|q| *q != Pt::Null));
                }
                Some((st, val))
            }
            Value::Set(x, val) => {
                let mut st = st;
                if let Value::Boolean(c) = val.unspan() {
                    st.flags.insert(*x, *c);
                    return Some((st, Val::null()));
                }
                st.flags.remove(x);
                if self.in_field_reset > 0
                    && matches!(val.unspan(), Value::Call(n, _)
                        if self.data.def(*n).name() == "OpNullRefSentinel")
                {
                    Self::give_back_field_captures(&mut st, *x);
                }
                // The bases `x` will be a view pair of: a tail of the value projecting `w`, where
                // `w` is null before the value runs and no other tail's path sets it — so where
                // the value is not that projection, `w` is still null.
                let view_bases: Vec<u16> = if views_enabled() {
                    let mut tails = Vec::new();
                    value_tails(self.data, val, &mut tails);
                    let mut out = Vec::new();
                    for (i, (_, t)) in tails.iter().enumerate() {
                        if let Value::Call(op, pargs) = t.unspan()
                            && crate::use_analysis::is_projection_op(self.data, *op)
                            && let Some(Value::Var(w)) =
                                pargs.first().map(|a| Self::projection_root(self.data, a))
                            && Self::pts(&st, *w).iter().all(|p| *p == Pt::Null)
                            && !tails
                                .iter()
                                .enumerate()
                                .any(|(j, (arm, _))| j != i && sets_var(arm, *w))
                        {
                            out.push(*w);
                        }
                    }
                    out
                } else {
                    Vec::new()
                };
                let copied = self.copied_from(val);
                // `x = f(…, x)` answering `x`'s own store (a value operator writing its result
                // into the buffer `x` hands it): `x` still names, and must-holds, what it did.
                let kept: Vec<SKey> = if copied == Some(*x) {
                    st.stores
                        .iter()
                        .filter(|(_, i)| i.has(LIVE) && i.live.contains(x))
                        .map(|(k, _)| *k)
                        .collect()
                } else {
                    Vec::new()
                };
                let child_rec = matches!(val.unspan(), Value::Call(n, _)
                    if self.data.def(*n).name() == "OpChildRec");
                let (mut st, out) = self.assign(v, *x, val, st)?;
                if child_rec {
                    st.nonnull.insert(*x);
                }
                if let Value::Var(src) = val.unspan()
                    && src != x
                    && copy_alias()
                {
                    st.copies.insert(((*src).min(*x), (*src).max(*x)));
                }
                if views_enabled() {
                    for w in view_bases {
                        if w != *x {
                            st.views.insert((*x, w));
                        }
                    }
                }
                if let Some(src) = copied
                    && src != *x
                {
                    Self::copy_alts(&mut st, *x, src);
                    // …and a store `src` must-holds that `x` now names, `x` must-holds too.
                    let named = Self::pts(&st, *x);
                    for (k, info) in &mut st.stores {
                        if info.has(LIVE)
                            && info.live.contains(&src)
                            && named.contains(&Pt::Store(*k))
                        {
                            info.live.insert(*x);
                        }
                    }
                }
                for k in kept {
                    if Self::pts(&st, *x).contains(&Pt::Store(k))
                        && let Some(info) = st.stores.get_mut(&k)
                        && info.has(LIVE)
                    {
                        info.live.insert(*x);
                        for a in &mut info.alts {
                            a.insert(*x);
                        }
                    }
                }
                Some((st, out))
            }
            Value::If(cond, then_v, else_v) => {
                let (s, _) = self.eval(cond, st)?;
                let (then_st, else_st) = self.decide(cond, s);
                let then_out = then_st.and_then(|s| self.eval(then_v, s));
                let else_out = else_st.and_then(|s| self.eval(else_v, s));
                let mut out = join_out(then_out, else_out)?;
                Self::normalize(&mut out.0);
                Some(out)
            }
            Value::Return(val) => {
                let (mut s, value) = self.eval(val, st)?;
                if let Value::Var(x) = val.unspan() {
                    self.use_var(&s, *x, "the return");
                }
                self.answer = self.answer.join(self.summary_of(&value.pts));
                let x = match val.unspan() {
                    Value::Var(x) => Some(*x),
                    _ => None,
                };
                self.leaving_captures(&s, &value);
                Self::hand_on_var(&mut s, x, &value);
                self.exit(&s, "a return");
                None
            }
            Value::Break(n) => {
                match self.loops.len().checked_sub(1 + *n as usize) {
                    Some(i) => {
                        let prev = self.loops[i].0.take();
                        self.loops[i].0 = join_opt(prev, Some(st));
                    }
                    None => self.imprecise = true,
                }
                None
            }
            Value::Continue(n) => {
                match self.loops.len().checked_sub(1 + *n as usize) {
                    Some(i) => {
                        let prev = self.loops[i].1.take();
                        self.loops[i].1 = join_opt(prev, Some(st));
                    }
                    None => self.imprecise = true,
                }
                None
            }
            Value::Drop(x) => self.eval(x, st).map(|(s, _)| (s, Val::null())),
            Value::Call(d, args) => self.call(v, *d, args, st),
            Value::CallRef(g, args) => {
                let (s, _) = self.args(args, st)?;
                // A call through a closure reads what its record captured.
                if let Some(w) = self.fn_record.get(g).copied() {
                    let held: Vec<SKey> = Self::stores_of(&Self::held_by_record(&s, w, None));
                    for k in held {
                        self.sites[k.site as usize]
                            .uses
                            .insert("a closure call".to_string());
                        let info = info_of(&s, k);
                        if info.has(ENDED) && !info.has(LIVE) {
                            let name = self.var_name(*g);
                            self.find(
                                "use-after-end",
                                k.site,
                                format!(
                                    "a call through `{name}` reads a capture whose store was freed"
                                ),
                            );
                        }
                    }
                }
                Some((s, Val::unknown()))
            }
            // A fn-ref holds its closure record.
            Value::FnRef(_, w, _) if *w != u16::MAX => {
                let val = Self::read(&st, *w);
                Some((st, val))
            }
            Value::TuplePut(t, i, x) => {
                let (mut s, value) = self.eval(x, st)?;
                // A put into a local tuple's member assigns that member, followed or not: the
                // member's earlier store, where it was the only name, is lost by the put.
                let local =
                    !self.params.contains(t) && *t < self.data.def(self.d_nr).variables.count();
                match member_key(*t, *i).filter(|k| local || s.vars.contains_key(k)) {
                    Some(k) => self.bind(&mut s, k, &value),
                    None => Self::hand_on(&mut s, &value),
                }
                Some((s, Val::null()))
            }
            Value::Tuple(xs) if std::mem::take(&mut self.member_tuple) => {
                let (s, parts) = self.tuple_elems(xs, st)?;
                let mut whole = Val::null();
                for p in parts {
                    whole.pts.extend(p.pts);
                    whole.corr.extend(p.corr);
                }
                if whole.pts.len() > 1 {
                    whole.pts.remove(&Pt::Null);
                }
                Some((s, whole))
            }
            Value::Tuple(xs) => {
                let target = self.tuple_target;
                let (mut s, vals) = if target.is_some() {
                    self.tuple_elems(xs, st)?
                } else {
                    self.args(xs, st)?
                };
                match target {
                    // A branch arm's tail literal for a tuple local: its members.
                    Some(t) => {
                        for (i, value) in vals.iter().enumerate() {
                            if let Some(k) = u16::try_from(i).ok().and_then(|i| member_key(t, i)) {
                                self.bind(&mut s, k, value);
                            }
                        }
                    }
                    None => {
                        for value in &vals {
                            Self::hand_on(&mut s, value);
                        }
                    }
                }
                Some((s, Val::null()))
            }
            Value::TupleGet(t, i) => match member_key(*t, *i).filter(|k| st.vars.contains_key(k)) {
                Some(k) => {
                    let val = Self::read(&st, k);
                    Some((st, val))
                }
                // A heap member of a tuple PARAMETER lies in a store the caller holds, as the
                // parameter's own would: a callee that answers it borrows argument `t`
                // (`fn first(p: (R, integer)) -> R { p.0 }`).
                None if self.params.contains(t) && self.member_is_heap(*t, *i) => Some((
                    st,
                    Val {
                        pts: [Pt::Param(*t)].into(),
                        corr: BTreeSet::new(),
                    },
                )),
                None => Some((st, Val::unknown())),
            },
            // `(G-Own)` — a yield whose type hands over gives the consumer the stores its value
            // names, and the generator forgets the temps that held them
            // (`coroutine_layout::yield_handed_temps`, the one home both code generators read).
            Value::Yield(y)
                if matches!(self.data.def(self.d_nr).returned().base(),
                    crate::data::Type::Iterator(inner, _)
                        if crate::coroutine_layout::yield_handed_over(inner)) =>
            {
                let (mut s, value) = self.eval(y, st)?;
                Self::hand_on(&mut s, &value);
                for w in crate::coroutine_layout::yield_handed_temps(self.data, self.d_nr, y) {
                    s.vars.remove(&w);
                    for info in s.stores.values_mut() {
                        forget_holder(info, w);
                        info.ended.remove(&w);
                    }
                }
                Some((s, Val::null()))
            }
            Value::Yield(_) | Value::Iter(..) => {
                self.imprecise = true;
                Some((st, Val::unknown()))
            }
            _ => Some((st, Val::null())),
        }
    }

    /// A fresh store at this assignment, the variable's own: a copy the code generator makes.
    pub(super) fn copied(&mut self, st: &mut State, node: &Value) -> Val {
        let site = self.site(node, Origin::Copy);
        let k = Self::fresh(st, site);
        Val {
            pts: [Pt::Store(k)].into(),
            corr: [k].into(),
        }
    }

    /// The stores a value owns outright (correlated fresh stores): consumed by a copy or an
    /// adopt into the destination's own store.
    pub(super) fn consume(st: &mut State, value: &Val) {
        for k in &value.corr {
            if let Some(info) = st.stores.get_mut(k)
                && info.status & LIVE != 0
            {
                info.status = (info.status & !LIVE) | HANDED;
                info.live.clear();
                info.alts.clear();
            }
        }
    }

    /// `x = val`, as the code generator lowers it: the arm `bind_lowering` decides, with its
    /// store meaning — a bind, a copy into a store of `x`'s own, a copy-or-adopt, a move —
    /// and, for a reassignment, the free of the store `x` displaces.
    pub(super) fn assign(
        &mut self,
        node: &Value,
        x: u16,
        val: &Value,
        st: State,
    ) -> Option<(State, Val)> {
        use crate::bind_lowering::FirstArm;
        let data = self.data;
        let function = &data.def(self.d_nr).variables;
        // A tuple literal into a tuple local: each member holds what its element names, and
        // the tuple's own release of a member (`OpFreeRef(t.i)`) ends it.
        // The literal may be the tail of a value block (a call's record unboxed into a stack
        // tuple: `{ __ref_2 = mp(k); (OpGetField(__ref_2, …), …) }`); its leading statements
        // run first.
        let (prefix, lit): (&[Value], &Value) = match val.unspan() {
            Value::Block(b) => match b.operators.split_last() {
                Some((last, init)) => (init, last),
                None => (&[], val),
            },
            Value::Insert(ops) => match ops.split_last() {
                Some((last, init)) => (init, last),
                None => (&[], val),
            },
            _ => (&[], val),
        };
        // A value BRANCH (an `if`, or a `match`'s block ending in its `if` chain) whose tails are
        // tuple literals binds the members on each arm.
        if !matches!(lit.unspan(), Value::Tuple(_))
            && matches!(function.tp(x).base(), crate::data::Type::Tuple(_))
            && member_key(x, 15).is_some()
            && branch_ends_in_tuples(val)
        {
            let prev = self.tuple_target.replace(x);
            let out = self.eval(val, st);
            self.tuple_target = prev;
            return out.map(|(s, _)| (s, Val::null()));
        }
        if let Value::Tuple(xs) = lit.unspan()
            && matches!(function.tp(x).base(), crate::data::Type::Tuple(_))
            && u16::try_from(xs.len())
                .ok()
                .and_then(|n| member_key(x, n.saturating_sub(1)))
                .is_some()
        {
            let st = if prefix.is_empty() {
                st
            } else {
                self.seq(prefix, st)?.0
            };
            let (mut s, vals) = self.tuple_elems(xs, st)?;
            for (i, value) in vals.iter().enumerate() {
                let k = member_key(x, i as u16).expect("checked above");
                self.bind(&mut s, k, value);
            }
            return Some((s, Val::null()));
        }
        if self
            .first_sets
            .contains(&(std::ptr::from_ref(node) as usize))
        {
            let arm = crate::bind_lowering::first_arm(data, self.d_nr, function, x, val);
            // @FR-O-Proxy asks oracle — the records' own reading; it drives no emission.
            if let FirstArm::RecordVarCopy { src, .. } = arm
                && function.uses(src) == 1
                && function.proxy_says_owned(src)
                && !function.is_argument(src)
                && !function.is_captured(src)
            {
                // The move: `x` takes `src`'s store, and `src`'s free is dropped.
                self.moved_away.insert(src);
                let value = Self::read(&st, src);
                let mut s = st;
                self.bind(&mut s, x, &value);
                return Some((s, Val::null()));
            }
            let pre_protected = self.first_bind_bracket(&st, val, arm);
            let (mut s, value) = self.eval_bracketed(val, st, pre_protected.as_ref())?;
            let bound = match arm {
                // A fn-ref holds the closure record its build names (`FnRef(_, w)` reads `w`),
                // so its first bind holds that store as a reassignment does.  One read out of a
                // record this function was handed (a cascade's `__held_fn`) is that record's
                // to release, not a store of this frame's.
                FirstArm::FnRef if value.pts.iter().any(|p| matches!(p, Pt::Store(_))) => value,
                FirstArm::FnRef => Val::null(),
                FirstArm::Text
                | FirstArm::RefVar
                | FirstArm::RefNull
                | FirstArm::IterNull
                | FirstArm::NullableVectorNull
                | FirstArm::TupleNull => Val::null(),
                // A keyed local's null init gives it a keyed store of its own, as a vector
                // local's does (`gen_set_first_keyed_null`).
                // The code generator's own answer (`bind_lowering::vector_null_mints`,
                // `keyed_null_mints`): a lift temporary, a lazy buffer or a borrowed view gets no
                // store at its null init.
                FirstArm::VectorNull | FirstArm::KeyedNull if null_init_mints(arm, function, x) => {
                    self.copied(&mut s, node)
                }
                FirstArm::VectorNull | FirstArm::KeyedNull => Val::null(),
                FirstArm::RecordCopyCall { .. }
                | FirstArm::RecordVarCopy { .. }
                | FirstArm::ElementCopy { .. }
                | FirstArm::TupleElementCopy { .. } => self.copied(&mut s, node),
                FirstArm::CallCopy { .. } | FirstArm::NullableCallCopy { .. } => {
                    if crate::use_analysis::call_return_frees_source(data, self.d_nr, val) {
                        Self::consume(&mut s, &value);
                        self.unprotected_source_free(&mut s, val, &value, pre_protected.as_ref());
                    }
                    self.copied(&mut s, node)
                }
                FirstArm::CallRefJoin { .. }
                | FirstArm::NullableCallJoin { .. }
                | FirstArm::CallJoin { .. }
                | FirstArm::OpaqueCallRef { .. } => self.join_bound(&mut s, arm, x, node, &value),
                FirstArm::CallAdopt | FirstArm::Plain => value,
            };
            self.bind(&mut s, x, &bound);
            return Some((s, Val::null()));
        }
        self.reassign(node, x, val, st)
    }

    /// What a runtime-join bind leaves in `x`: its own copy, and beside it every store the
    /// join adopted while another variable held it ([`Self::join_adopts_held`]).
    pub(super) fn join_bound(
        &mut self,
        s: &mut State,
        arm: crate::bind_lowering::FirstArm,
        x: u16,
        node: &Value,
        value: &Val,
    ) -> Val {
        self.tuple_witness(s, arm, value);
        let held = Self::join_adopts_held(s, arm, x, value);
        let mut owned = value.clone();
        owned.corr.retain(|k| !held.contains(k));
        Self::consume(s, &owned);
        let mut bound = self.copied(s, node);
        // Adopted on the paths where the value names them, so they stay correlated with the
        // bound value as they were with the source.
        bound
            .corr
            .extend(held.iter().copied().filter(|k| value.corr.contains(k)));
        bound.pts.extend(held.into_iter().map(Pt::Store));
        bound
    }

    /// A runtime join copies only what lies in its witness's store and ADOPTS everything
    /// else.  A store the value names that ANOTHER variable holds (a branch arm's call that
    /// answered into the frame's own buffer) is therefore `x`'s as well from here on, not a
    /// store consumed into `x`'s copy: the two holders' frees are one store's, which is the
    /// fault to report.  Answers those stores.
    /// `LOFT_RECORDS_NO_JOIN_ADOPT_HELD=1` reads every join as a copy.
    pub(super) fn join_adopts_held(
        s: &State,
        arm: crate::bind_lowering::FirstArm,
        x: u16,
        value: &Val,
    ) -> Vec<SKey> {
        use crate::bind_lowering::FirstArm;
        let (FirstArm::CallRefJoin { base, .. }
        | FirstArm::NullableCallJoin { base, .. }
        | FirstArm::CallJoin { base, .. }) = arm
        else {
            return Vec::new();
        };
        if crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_JOIN_ADOPT_HELD").is_some()) {
            return Vec::new();
        }
        let witness = if base == u16::MAX {
            Pts::new()
        } else {
            Self::pts(s, base)
        };
        Self::stores_of(&value.pts)
            .into_iter()
            .filter(|k| {
                !witness.contains(&Pt::Store(*k))
                    && s.vars
                        .iter()
                        .any(|(&w, p)| w != x && w != base && p.contains(&Pt::Store(*k)))
            })
            .collect()
    }

    /// A native's answer, as its DECLARATION says it — the one thing known of a function with
    /// no body, and the author's statement rather than a fact the code generator derived:
    /// `-> T[p]` answers a view of parameter `p` (`fn field(self: const JsonValue, …) ->
    /// JsonValue[self]`), so it names what that argument names and owns nothing; a heap answer
    /// of a native whose parameters hold no store at all is a store of its own (`json_parse`).  Read as unknown,
    /// a local bound from a view of a lifted native answer was freed beside the lift and no
    /// finding could say so.  `LOFT_RECORDS_NO_NATIVE_ANSWERS=1` reads every native as unknown.
    pub(super) fn native_answer(
        &mut self,
        node: &Value,
        d: u32,
        vals: &[Val],
        s: &mut State,
    ) -> Val {
        if crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_NATIVE_ANSWERS").is_some()) {
            return Val::unknown();
        }
        let def = self.data.def(d);
        let deps = def.returned().depend();
        let viewed: Vec<usize> = deps
            .iter()
            .map(|&p| usize::from(p))
            .filter(|&i| i < def.attributes.len() && !def.attributes[i].name.starts_with("__"))
            .collect();
        if viewed.is_empty() {
            // Only a PUBLIC declaration none of whose parameters holds a store: a native handed a
            // record or a collection may answer into it without saying so (the keyed runtime's
            // own helpers do), and a private one may answer a buffer the runtime keeps.
            let holds_no_store = def.pub_visible
                && def.attributes.iter().all(|a| {
                    crate::data::is_scalar(a.typedef.base())
                        || matches!(a.typedef.base(), crate::data::Type::Text(_))
                });
            if !deps.is_empty() || !holds_no_store {
                return Val::unknown();
            }
            let site = self.site(node, Origin::Call(d));
            let k = Self::fresh(s, site);
            return Val {
                pts: [Pt::Store(k)].into(),
                corr: [k].into(),
            };
        }
        let mut pts = Pts::new();
        for i in viewed {
            match vals.get(i) {
                Some(v) => pts.extend(v.pts.iter().copied()),
                None => return Val::unknown(),
            }
        }
        Val {
            pts,
            corr: BTreeSet::new(),
        }
    }

    /// A join whose witness is a TUPLE slot (`use_analysis::join_witness_members` names no
    /// member): the slot holds no store, so the comparison cannot tell a member the callee
    /// handed back from one it minted — the member is adopted beside its holder and released
    /// twice (loft#2471).
    pub(super) fn tuple_witness(
        &mut self,
        s: &State,
        arm: crate::bind_lowering::FirstArm,
        value: &Val,
    ) {
        use crate::bind_lowering::FirstArm;
        let (FirstArm::CallRefJoin { base, .. }
        | FirstArm::NullableCallJoin { base, .. }
        | FirstArm::CallJoin { base, .. }) = arm
        else {
            return;
        };
        let function = &self.data.def(self.d_nr).variables;
        if base == u16::MAX
            || !matches!(function.tp(base).base(), crate::data::Type::Tuple(_))
            || crate::use_analysis::join_witness_members(function, base)
                .is_some_and(|m| !m.is_empty())
        {
            return;
        }
        let name = self.var_name(base);
        for k in Self::stores_of(&value.pts) {
            if info_of(s, k).has(LIVE) {
                self.find(
                    "re-free",
                    k.site,
                    format!(
                        "a join's witness `{name}` is a tuple: no store to compare, so a member \
                         handed back is adopted beside its holder and released twice"
                    ),
                );
            }
        }
    }

    /// `x = val` where `x` already holds a slot: the displaced store's free, then the arm.
    /// The variable the `&` link `x` was installed on (`x = OpCreateStack(t)`), when every
    /// install of `x` names that one variable; `None` for a link to a field or an element, or
    /// one re-pointed between variables.
    pub(super) fn link_install(&self, x: u16) -> Option<u16> {
        let data = self.data;
        // A link to a store-backed kind: a scalar or text link holds no store to release.  A
        // fn-ref is one: a capturing closure's value names its record's store, so a write
        // through `pd: &fn() = &g` lands that record in `g` (B-Ref-Intro/1454).
        // `LOFT_RECORDS_NO_FN_LINK_INSTALL=1` leaves a fn link unfollowed.
        let crate::data::Type::RefVar(inner) = data.def(self.d_nr).variables.tp(x).base() else {
            return None;
        };
        let fn_link = matches!(inner.base(), crate::data::Type::Function(..))
            && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_FN_LINK_INSTALL").is_some());
        if !(fn_link
            || matches!(
                inner.base(),
                crate::data::Type::Vector(..)
                    | crate::data::Type::Reference(..)
                    | crate::data::Type::Enum(_, true, _)
            )
            || crate::data::is_keyed(inner.base()))
        {
            return None;
        }
        let mut target: Option<u16> = None;
        let mut other = false;
        fn walk(node: &Value, link: u16, data: &Data, target: &mut Option<u16>, other: &mut bool) {
            if let Value::Set(set, rhs) = node
                && *set == link
            {
                match rhs.unspan() {
                    Value::Call(op, args) if data.def(*op).name() == "OpCreateStack" => {
                        match args.first().map(Value::unspan) {
                            Some(Value::Var(var)) if target.is_none_or(|p| p == *var) => {
                                *target = Some(*var);
                            }
                            _ => *other = true,
                        }
                    }
                    // A re-point to another place (`p = &v[i]`, `p = &o.f`, `p = q`).
                    Value::Call(op, _)
                        if matches!(
                            data.def(*op).name(),
                            "OpVarRef" | "OpGetField" | "OpGetVector" | "OpVectorRef"
                        ) =>
                    {
                        *other = true;
                    }
                    _ => {}
                }
            }
            node.for_each_child(&mut |c| walk(c, link, data, target, other));
        }
        walk(&data.def(self.d_nr).code, x, data, &mut target, &mut other);
        if other { None } else { target }
    }

    /// A whole write `x = val` THROUGH the link `x` to the variable `t` (`@FR-B-Ref-Write`):
    /// the value lands in `t`, and the code generator releases the store `t` displaced unless
    /// the new value lives in it — the slot move's write-back (`OpMoveSlotRef`) or the
    /// stash / install / `OpFreeRefIfDistinct` — but never through a link to a plain parameter
    /// (`Function::link_write_releases`, `@FR-F-ParamRebind`).  A read of `x` in `val` reads
    /// `t`.  A child read (`x = &x.next`) stays in `t`'s store when the child is present and
    /// leaves `t` null when it is absent, so the release is the absent path's alone.
    pub(super) fn link_write(
        &mut self,
        x: u16,
        t: u16,
        val: &Value,
        st: State,
    ) -> Option<(State, Val)> {
        let data = self.data;
        let function = &data.def(self.d_nr).variables;
        let crate::data::Type::RefVar(inner) = function.tp(x).base() else {
            return None;
        };
        let owned = matches!(
            crate::use_analysis::ownership_of(data, self.d_nr, val),
            crate::use_analysis::Own::Owned
        );
        // The code generator's own test (`state/codegen.rs`'s `amp_displaces`): an owned value
        // of a store-backed kind, or any record.  What a `&` parameter displaces is the
        // caller's to give up (`@FR-F-ParamRef`), which no store here records; a plain
        // parameter is not released at all, unless the cure is switched off.
        let store_kind = matches!(
            inner.base(),
            crate::data::Type::Vector(..)
                | crate::data::Type::Reference(..)
                | crate::data::Type::Enum(_, true, _)
        ) || crate::data::is_keyed(inner.base());
        let releases = ((store_kind && owned) || inner.heap_def_nr().is_some())
            && function.link_write_releases(x)
            && !(function.is_argument(t)
                && matches!(function.tp(t).base(), crate::data::Type::RefVar(_)));
        let mut st = st;
        let old = Self::pts(&st, t);
        st.vars.insert(x, old.clone());
        for k in Self::stores_of(&old) {
            if let Some(info) = st.stores.get_mut(&k)
                && let Some(c) = info.cover.as_mut()
            {
                c.insert(x);
            }
        }
        // The link reads its target: where a store `t` names is ended, the link may name it.
        for k in Self::stores_of(&old) {
            if let Some(info) = st.stores.get_mut(&k)
                && info.has(ENDED)
                && let Some(sn) = info.stale.as_mut()
            {
                sn.insert(x);
            }
        }
        let (mut s, value) = self.eval(val, st)?;
        let child_read = matches!(val.unspan(),
            Value::Call(d, _) if data.def(*d).name() == "OpRefFromChildRec");
        // The link reads what its target holds, so both are bound to the written value.
        let s = if !releases {
            self.bind(&mut s, t, &value);
            self.bind(&mut s, x, &value);
            s
        } else if child_read {
            let mut present = s.clone();
            self.bind(&mut present, t, &value);
            self.bind(&mut present, x, &value);
            let mut absent = s;
            self.end(&mut absent, t, "the link write-back", true);
            self.bind(&mut absent, t, &Val::null());
            self.bind(&mut absent, x, &Val::null());
            join_arms(&present, &absent)
        } else {
            // Released only where the new value is not in it: the stores the value names stay.
            let gone: BTreeSet<SKey> = Self::stores_of(&old)
                .into_iter()
                .filter(|k| !value.pts.contains(&Pt::Store(*k)))
                .collect();
            if !old.iter().all(|p| value.pts.contains(p)) {
                self.end_only(&mut s, t, "the link write-back", true, Some(&gone));
            }
            self.bind(&mut s, t, &value);
            self.bind(&mut s, x, &value);
            s
        };
        Some((s, Val::null()))
    }

    /// The reassignments that are not `bind_lowering`'s arms: a write through a local link, the
    /// write-back of a `&` parameter, a lazy buffer's mint.  `Err` hands the state back.
    pub(super) fn reassign_special(
        &mut self,
        node: &Value,
        x: u16,
        val: &Value,
        st: State,
    ) -> Result<Option<(State, Val)>, Box<State>> {
        let data = self.data;
        let function = &data.def(self.d_nr).variables;
        let installs = matches!(val.unspan(), Value::Call(d, _)
            if matches!(data.def(*d).name(), "OpCreateStack" | "OpVarRef"));
        // `@FR-F-ParamRef` — an OWNED value written through a slot link (`OpSetSlotRef` with
        // `COPY_FREE_SOURCE`): copied into the field the cell names and its store released
        // there, or moved into the caller's variable.  Either way the frame gives it up.
        if !installs
            && data.slot_link_def(function.tp(x)).is_some()
            && matches!(
                crate::use_analysis::ownership_of(data, self.d_nr, val),
                crate::use_analysis::Own::Owned
            )
        {
            let Some((mut s, value)) = self.eval(val, st) else {
                return Ok(None);
            };
            Self::hand_on(&mut s, &value);
            return Ok(Some((s, Val::null())));
        }
        if let Some(t) = self.link_install(x)
            && !installs
        {
            return Ok(self.link_write(x, t, val, st));
        }
        // `@FR-F-ParamRef` — a whole write through a `&` PARAMETER is the write-back: the
        // value is the caller's from here (handed on), and the store it displaced is the
        // caller's to release (the generator's stash / install / `OpFreeRefIfDistinct`).
        if function.is_argument(x)
            && matches!(function.tp(x).base(), crate::data::Type::RefVar(_))
            && !matches!(val.unspan(), Value::Call(d, _)
                if matches!(data.def(*d).name(), "OpCreateStack" | "OpVarRef"))
        {
            let shown = Self::pts(&st, x);
            let Some((mut s, value)) = self.eval(val, st) else {
                return Ok(None);
            };
            // `@FR-B-Copy`, `@FR-O-Owner` (loft#2528) — a RECORD written whole through a `&` is
            // the caller's binding from here, so it must be a store no other owner holds.  A
            // value that is ANOTHER argument's store (`out = args[1]!`, a view of `x`) leaves
            // the caller's local naming that argument's record: it aliases it, and its release
            // frees the store under its owner.  The generator copies such a place
            // (`Parser::amp_rhs_is_place`); a value still naming a parameter here is the defect.
            if matches!(function.tp(x).base(), crate::data::Type::RefVar(inner)
                    if inner.heap_def_nr().is_some())
                && let Some(Pt::Param(p)) = value
                    .pts
                    .iter()
                    .find(|pt| matches!(pt, Pt::Param(p) if *p != x))
                && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_AMP_PARAM_STORE").is_some())
            {
                let (name, owner) = (self.var_name(x), self.var_name(*p));
                self.find(
                    "wrong-end",
                    u32::MAX,
                    format!(
                        "`{name}` is written a store of the argument `{owner}`: the caller's \
                         local then names a record it does not own, and its release frees it"
                    ),
                );
            }
            // `@FR-L-CapOwn` — a closure this frame wrote through the same `&fn` link before
            // never reached the caller: the write displaces it, and it is this frame's again,
            // held by whatever still names it (its `___clos_N`, which the frame releases).
            // Only a store this frame made; the caller's own value (`Param`) stays the caller's.
            // `LOFT_RECORDS_NO_LINK_TAKE_BACK=1` leaves it handed on.
            if matches!(function.tp(x).base(), crate::data::Type::RefVar(inner)
                    if matches!(inner.base(), crate::data::Type::Function(..)))
                && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_LINK_TAKE_BACK").is_some())
            {
                for k in Self::stores_of(&shown) {
                    if value.pts.contains(&Pt::Store(k)) {
                        continue;
                    }
                    let holders: Vec<u16> = s
                        .vars
                        .iter()
                        .filter(|&(&w, p)| w != x && p.len() == 1 && p.contains(&Pt::Store(k)))
                        .map(|(&w, _)| w)
                        .collect();
                    if let Some(info) = s.stores.get_mut(&k)
                        && info.has(HANDED)
                        && !info.has(ENDED)
                        && !holders.is_empty()
                    {
                        info.status = (info.status & !HANDED) | LIVE;
                        info.live.extend(holders);
                    }
                }
            }
            Self::hand_on(&mut s, &value);
            // …and a read of the link from here answers what was written through it: the
            // caller's variable now holds it (`__fklink_N = out` after `out = h` names `h`'s
            // closure record).  Handed on, the store takes no live holder from this.
            // `LOFT_RECORDS_NO_LINK_READS_WRITE=1` leaves the link reading the parameter.
            // Inside a loop the link keeps reading the parameter: a write on some passes only
            // (`if i == 0 { p = f }`) joins into "the caller's value or this pass's closure",
            // and nothing here ties that choice back to the pass, so the read would split the
            // releases that guard on it both ways (L-CapKeep/1636, three findings more).
            if self.loops.is_empty()
                && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_LINK_READS_WRITE").is_some())
            {
                self.bind(&mut s, x, &value);
            }
            return Ok(Some((s, Val::null())));
        }
        // A lazy buffer's guarded `Set(v, Null)` is where its store is made (`set_var`).
        if crate::bind_lowering::lazy_buffer_mints_at(function, x, val) {
            let mut st = st;
            let site = self.site(node, Origin::Mint);
            let k = Self::fresh(&mut st, site);
            let minted = Val {
                pts: [Pt::Store(k)].into(),
                corr: [k].into(),
            };
            self.bind(&mut st, x, &minted);
            return Ok(Some((st, Val::null())));
        }
        Err(Box::new(st))
    }

    /// Does a displaced free's witness `w` keep the store `x` displaces?  The runtime keeps it
    /// when the witness names it.  A null `x` frees nothing, so the null `x` may also hold (a
    /// loop's first pass) is no reason to free: the two are compared without it.
    pub(super) fn witness_keeps(st: &State, x: u16, w: u16) -> bool {
        let held: Pts = Self::pts(st, x)
            .into_iter()
            .filter(|p| *p != Pt::Null)
            .collect();
        let named: Pts = Self::pts(st, w)
            .into_iter()
            .filter(|p| *p != Pt::Null)
            .collect();
        if held.is_empty() {
            return false;
        }
        // …or every store `x` holds is one `w` names wherever it is live (`live`): a buffer
        // refilled in place across a loop's passes names this pass's store or the one an earlier
        // pass left, and `x` bound from it last pass names that one — the same store on every
        // path where it exists (tuple_match's `mc`).  `LOFT_RECORDS_NO_LIVE_WITNESS=1` asks for
        // the two sets to be equal.
        held == named
            || (!crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_LIVE_WITNESS").is_some())
                && held.iter().all(|p| match p {
                    Pt::Store(k) => named.contains(p) && info_of(st, *k).live.contains(&w),
                    _ => false,
                }))
    }

    pub(super) fn reassign(
        &mut self,
        node: &Value,
        x: u16,
        val: &Value,
        st: State,
    ) -> Option<(State, Val)> {
        use crate::bind_lowering::{DisplacedFree, ReArm};
        let data = self.data;
        let function = &data.def(self.d_nr).variables;
        let st = match self.reassign_special(node, x, val, st) {
            Ok(done) => return done,
            Err(st) => *st,
        };
        let rb = match self.rebinds.get(&(std::ptr::from_ref(node) as usize)) {
            Some(rb) => *rb,
            None => crate::bind_lowering::re_bind(data, self.d_nr, function, x, val),
        };
        let mut st = st;
        // A keyed local assigned null is cleared IN PLACE (`B0`): the code generator emits no
        // displaced free for it (`Codegen::generate_set_inner` returns at that arm).
        if let DisplacedFree::Pre { witness } = rb.free
            && rb.arm != ReArm::KeyedNull
        {
            let keep = witness
                .into_iter()
                .chain(rb.entry_w)
                .any(|w| Self::witness_keeps(&st, x, w));
            if !keep {
                self.end(&mut st, x, "the displaced free", true);
            }
        }
        // Only the copy arm (R2b) brackets the call at run time.
        let pre_protected = self
            .pre_call_protected(&st, val)
            .filter(|_| matches!(rb.arm, ReArm::CallCopy { .. }) || bracket_every_bind());
        let (mut s, value) = self.eval_bracketed(val, st, pre_protected.as_ref())?;
        // What `x` held, read AFTER the value: a call that makes a store at a site `x`'s old
        // store came from moves that store to the older generation (`fresh`), and a key read
        // before it would name the new store instead (a loop's `fixed = step(i)`).
        let old = Self::pts(&s, x);
        // Parked in the state while the arm runs: a copy that mints at the site `x`'s old store
        // came from moves it to the older generation, and a key kept outside the state would
        // then name the new store (a loop's `class = pick(class, c)`).
        s.vars.insert(OLD_NAME, old.clone());
        let bound = match rb.arm {
            ReArm::KeyedNull => Self::read(&s, x),
            ReArm::Join { .. } | ReArm::OpaqueCallRef { .. } => {
                Self::consume(&mut s, &value);
                self.copied(&mut s, node)
            }
            ReArm::CallCopy { .. } => {
                if crate::use_analysis::call_return_frees_source(data, self.d_nr, val) {
                    Self::consume(&mut s, &value);
                    if rb.free != DisplacedFree::Stash {
                        self.unprotected_source_free(&mut s, val, &value, pre_protected.as_ref());
                    }
                    // The copy's source-free releases what the call answered, and a call that
                    // reads `x` may answer `x`'s own store.  Unprotected (`ReBind::protects_old`),
                    // that store is then released by the copy AND by the stashed free after it.
                    let x_arg = crate::use_analysis::protectable_ref_args(data, self.d_nr, val)
                        .0
                        .contains(&x);
                    if rb.free == DisplacedFree::Stash
                        && rb.copies_into_fresh()
                        && rb.rhs_reads_v
                        && x_arg
                        && !rb.protects_old(x_arg)
                        && let Some(k) = Self::stores_of(&old).first()
                    {
                        let name = self.var_name(x);
                        self.find(
                            "re-free",
                            k.site,
                            format!(
                                "`{name}`'s old store is released by the copy's source-free and \
                                 again by the stashed free"
                            ),
                        );
                    }
                }
                // A stashed local re-initialises its own store in place after the call unless
                // the copy lands in a fresh store (`ReBind::copies_into_fresh`, the code
                // generator's own answer); anything else starts from a fresh one.
                let in_place = rb.free == DisplacedFree::Stash && !rb.copies_into_fresh();
                if in_place && Self::stores_of(&old).len() == 1 && old.len() == 1 {
                    Val {
                        pts: old.clone(),
                        corr: Self::stores_of(&old).into_iter().collect(),
                    }
                } else {
                    self.copied(&mut s, node)
                }
            }
            ReArm::VarCopy { .. } | ReArm::ElementCopy { .. } => self.copied(&mut s, node),
            ReArm::Plain => value,
        };
        let old = s.vars.remove(&OLD_NAME).unwrap_or(old);
        if rb.free == DisplacedFree::Stash && rb.arm != ReArm::KeyedNull && bound.pts != old {
            self.stashed_free(&mut s, x, &old, &bound);
            return Some((s, Val::null()));
        }
        self.bind(&mut s, x, &bound);
        Some((s, Val::null()))
    }

    /// The stashed old reference of a reassignment, freed unless the new value names the
    /// same store, with `x` bound to the new value.
    pub(super) fn stashed_free(&mut self, s: &mut State, x: u16, old: &Pts, bound: &Val) {
        // `@FR-F-ParamRebind` — a visible parameter that still names the caller's store, rebound
        // to a value that cannot be that store: the guard (`OpFreeRefIfDistinct(old, new)`) is
        // distinct on every path, so the stashed free releases the caller's store.  The rebind's
        // own detach (`rebind_local_heap_param`) nulls the slot first; this is the rebind that
        // lost it.  `LOFT_RECORDS_NO_STASHED_PARAM_END=1` leaves it unreported.
        if old.len() == 1
            && let Some(Pt::Param(p)) = old.iter().next()
            && !bound.pts.contains(&Pt::Param(*p))
            && !bound.pts.iter().any(|q| matches!(q, Pt::Unknown))
            && self.params.iter().position(|q| *q == *p).is_some_and(|i| {
                self.data
                    .def(self.d_nr)
                    .attributes()
                    .get(i)
                    .is_some_and(|a| {
                        !a.hidden && !matches!(a.typedef, crate::data::Type::RefVar(_))
                    })
            })
            && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_STASHED_PARAM_END").is_some())
        {
            let name = self.var_name(x);
            self.find(
                "wrong-end",
                u32::MAX,
                format!("`{name}` is rebound, and the stashed free releases the caller's store"),
            );
        }
        let tmp = u16::MAX;
        s.vars.insert(tmp, old.clone());
        for k in Self::stores_of(old) {
            if let Some(info) = s.stores.get_mut(&k)
                && info.live.contains(&x)
            {
                info.live.insert(tmp);
            }
        }
        self.bind(s, x, bound);
        let shared: Vec<SKey> = Self::stores_of(old)
            .into_iter()
            .filter(|k| bound.pts.contains(&Pt::Store(*k)))
            .collect();
        if shared.is_empty() {
            self.end(s, tmp, "the stashed free", false);
        } else {
            // A store the new value may name too is freed on the paths where it does not:
            // where `x` named it on every path it was live, it stays live exactly where
            // `x` names it now (`none = rec(k - 1, none)`, answering `none` or a fresh store).
            let rest: BTreeSet<SKey> = Self::stores_of(old)
                .into_iter()
                .filter(|k| !shared.contains(k))
                .collect();
            if !rest.is_empty() {
                self.end_only(s, tmp, "the stashed free", false, Some(&rest));
            }
            for k in &shared {
                let Some(info) = s.stores.get_mut(k) else {
                    continue;
                };
                if info.has(LIVE) && (old.len() == 1 || info.live.contains(&tmp)) {
                    info.status |= ENDED;
                    info.live.insert(x);
                }
            }
        }
        s.vars.remove(&tmp);
        for info in s.stores.values_mut() {
            forget_holder(info, tmp);
            info.ended.remove(&tmp);
        }
    }

    /// The stores the copy bracket protects when it is set around the call itself
    /// (`LOFT_NO_PROTECT_AFTER_CALL`): what the protectable arguments name BEFORE the call.
    /// `None` when the bracket is set after the call, as the code generators set it by default
    /// (`keys::protect_after_call`), so the source-free reads the state after the call.
    /// The stores a first bind's copy bracket protects: the copy arm's only (F9c, F7-copy).  A
    /// plain bind or a join sets none at run time, so the records read the call unbracketed.
    pub(super) fn first_bind_bracket(
        &self,
        st: &State,
        val: &Value,
        arm: crate::bind_lowering::FirstArm,
    ) -> Option<BTreeSet<SKey>> {
        let bracketed = matches!(
            arm,
            crate::bind_lowering::FirstArm::CallCopy { .. }
                | crate::bind_lowering::FirstArm::NullableCallCopy { .. }
        );
        self.pre_call_protected(st, val)
            .filter(|_| bracketed || bracket_every_bind())
    }

    /// Evaluate a bind's value with the call bracket `pre` (its protected stores) set for it.
    pub(super) fn eval_bracketed(
        &mut self,
        val: &Value,
        st: State,
        pre: Option<&BTreeSet<SKey>>,
    ) -> Option<(State, Val)> {
        self.call_bracket = pre.map(|p| (std::ptr::from_ref(val.unspan()) as usize, p.clone()));
        let out = self.eval(val, st);
        self.call_bracket = None;
        out
    }

    pub(super) fn pre_call_protected(&self, st: &State, call: &Value) -> Option<BTreeSet<SKey>> {
        if crate::keys::protect_after_call() {
            return None;
        }
        Some(
            crate::use_analysis::protectable_ref_args(self.data, self.d_nr, call)
                .0
                .into_iter()
                .flat_map(|a| Self::stores_of(&Self::pts(st, a)))
                .collect(),
        )
    }

    /// A copy's source-free (`COPY_FREE_SOURCE`) releases what the call answered unless the
    /// copy bracket protects it: a store of the answer that no protectable argument names
    /// (`use_analysis::protectable_ref_args`, the code generator's own list) ends here.  The
    /// arguments are read after the call (`@FR-O-Move`: a `&` argument the callee re-points
    /// holds another store than the one it answers), or before it where `pre` says the bracket is
    /// set around the call.
    pub(super) fn unprotected_source_free(
        &mut self,
        st: &mut State,
        call: &Value,
        value: &Val,
        pre: Option<&BTreeSet<SKey>>,
    ) {
        let protected: BTreeSet<SKey> = match pre {
            Some(p) => p.clone(),
            None => crate::use_analysis::protectable_ref_args(self.data, self.d_nr, call)
                .0
                .into_iter()
                .flat_map(|a| Self::stores_of(&Self::pts(st, a)))
                .collect(),
        };
        let freed: BTreeSet<SKey> = Self::stores_of(&value.pts)
            .into_iter()
            .filter(|k| !protected.contains(k) && !value.corr.contains(k))
            .filter(|k| info_of(st, *k).has(LIVE))
            .collect();
        if freed.is_empty() {
            return;
        }
        st.vars.insert(FREED_NAME - 1, value.pts.clone());
        self.end_only(
            st,
            FREED_NAME - 1,
            "the copy's source-free",
            true,
            Some(&freed),
        );
        st.vars.remove(&(FREED_NAME - 1));
    }

    pub(super) fn seq(&mut self, ops: &[Value], st: State) -> Option<(State, Val)> {
        let mut cur = (st, Val::null());
        for op in ops {
            let value = cur.1;
            cur = self.eval(op, cur.0)?;
            if self.traced {
                self.trace(op, &cur.0);
            }
            // A block's value is its last value-producing statement's: the cleanup frees the
            // scope pass places after it (`{ if … ; OpFreeRefIfDistinct(…) }`) run while the
            // value waits, and answer nothing.
            if self.is_void_call(op) || self.is_void_release(op) {
                cur.1 = value;
            }
        }
        Some(cur)
    }

    /// `LOFT_STORE_TRACE=<fn substring>`: each statement of a matching function, with what
    /// every variable names, every store's statuses and holders, and every closure record's
    /// capture slots (with the stores live only through them) after it.
    pub(super) fn trace(&self, op: &Value, st: &State) {
        let names = |s: &BTreeSet<u16>| s.iter().map(|v| self.var_name(*v)).collect::<Vec<_>>();
        let key = |k: &SKey| format!("#{}{}", k.site, if k.older { "'" } else { "" });
        let mut o: String = format!("{op:?}").chars().take(110).collect();
        o.push_str("\n    vars:");
        for (v, p) in &st.vars {
            let pts: Vec<String> = p
                .iter()
                .map(|p| match p {
                    Pt::Store(k) => key(k),
                    other => format!("{other:?}"),
                })
                .collect();
            let _ = write!(o, " {}={{{}}}", self.var_name(*v), pts.join(","));
        }
        if !st.views.is_empty() {
            o.push_str("\n    views:");
            for (x, w) in &st.views {
                let _ = write!(o, " {}~{}", self.var_name(*x), self.var_name(*w));
            }
        }
        o.push_str("\n    stores:");
        for (k, i) in &st.stores {
            let alts: Vec<Vec<String>> = i.alts.iter().map(names).collect();
            let _ = write!(
                o,
                " {}:st{} live{:?} alts{alts:?} end{:?}",
                key(k),
                i.status,
                names(&i.live),
                names(&i.ended)
            );
            if let Some(c) = &i.cover {
                let _ = write!(o, " cover{:?}", names(c));
            }
            // The record correlations: captured by, and non-null / null wherever it is live.
            if !i.captured_by.is_empty() || !i.live_some.is_empty() || !i.live_null.is_empty() {
                let _ = write!(
                    o,
                    " cap{:?} some{:?} null{:?}",
                    names(&i.captured_by),
                    names(&i.live_some),
                    names(&i.live_null)
                );
            }
        }
        if !st.slots.is_empty() {
            o.push_str("\n    slots:");
            for (k, m) in &st.slots {
                for (off, v) in m {
                    let pts: Vec<String> = v
                        .pts
                        .iter()
                        .map(|p| match p {
                            Pt::Store(k) => key(k),
                            other => format!("{other:?}"),
                        })
                        .collect();
                    let corr: Vec<String> = v.corr.iter().map(key).collect();
                    let _ = write!(
                        o,
                        " {}@{off}={{{}}}corr{{{}}}",
                        key(k),
                        pts.join(","),
                        corr.join(",")
                    );
                }
            }
        }
        eprintln!("[store-trace] {o}");
    }

    /// A call to an operator or function that answers nothing (a free, a set).
    /// A release statement after a block's value (`op_sets::is_release_statement`, the one
    /// reading the C and rustc writers share): the value waits past it.
    pub(super) fn is_void_release(&self, op: &Value) -> bool {
        crate::op_sets::is_release_statement(self.data, op)
    }

    pub(super) fn is_void_call(&self, op: &Value) -> bool {
        let Value::Call(d, _) = op.unspan() else {
            return false;
        };
        (*d as usize) < self.data.definitions.len()
            && matches!(self.data.def(*d).returned().base(), crate::data::Type::Void)
    }

    pub(super) fn run_loop(&mut self, body: &[Value], st: State) -> Option<(State, Val)> {
        // The first passes on their own (`peeled_passes`), each from the state the one before
        // left: the null a variable has before its first bind, and the store it held before
        // the loop, never meet the state of a later pass at the head (a lazy buffer's mint, a
        // store rebound away, a closure built per pass).  Their exits — the zero-pass one
        // included — join the others.
        let mut exits: Option<State> = None;
        let mut from = st;
        let mut head = None;
        for _ in 0..peeled_passes() {
            self.loops.push((None, None));
            let out = self.seq(body, from);
            let (brk, cont) = self.loops.pop().unwrap_or_default();
            exits = join_opt(exits, brk);
            head = join_opt(out.map(|(s, _)| s), cont);
            let Some(h) = &head else {
                break;
            };
            from = h.clone();
        }
        let Some(mut head) = head else {
            return exits.map(|s| (s, Val::null()));
        };
        Self::normalize(&mut head);
        for pass in 0..LOOP_PASSES {
            self.loops.push((None, None));
            let out = self.seq(body, head.clone());
            let (brk, cont) = self.loops.pop().unwrap_or_default();
            exits = join_opt(exits, brk);
            let again = join_opt(join_opt(Some(head.clone()), out.map(|(s, _)| s)), cont)
                .expect("the head state is a path");
            let mut again = again;
            Self::normalize(&mut again);
            if again == head || self.imprecise {
                break;
            }
            if pass + 1 == LOOP_PASSES {
                self.imprecise = true;
            }
            head = again;
        }
        exits.map(|s| (s, Val::null()))
    }

    /// The elements of a tuple literal bound into a tuple local's members: a nested tuple
    /// literal is part of its member, so it answers the stores it holds rather than handing
    /// them on (`n = ((S { … }, 22), 8)`).
    pub(super) fn tuple_elems(&mut self, xs: &[Value], st: State) -> Option<(State, Vec<Val>)> {
        let mut s = st;
        let mut vals = Vec::with_capacity(xs.len());
        for x in xs {
            if let Value::Tuple(inner) = x.unspan() {
                let (s2, parts) = self.tuple_elems(inner, s)?;
                s = s2;
                let mut whole = Val::null();
                for p in parts {
                    whole.pts.extend(p.pts);
                    whole.corr.extend(p.corr);
                }
                if whole.pts.len() > 1 {
                    whole.pts.remove(&Pt::Null);
                }
                vals.push(whole);
            } else if Self::tail_is_tuple(x) {
                // A member moved out of another tuple (`u = t`): a block ending in the
                // member's own tuple literal.  Its tail is joined like a bare nested literal.
                let target = self.tuple_target.take();
                let out = self.eval_member_block(x, s);
                self.tuple_target = target;
                let (s2, v) = out?;
                s = s2;
                vals.push(v);
            } else {
                let (s2, vs) = self.args(std::slice::from_ref(x), s)?;
                s = s2;
                vals.extend(vs);
            }
        }
        Some((s, vals))
    }

    /// Is `v` a block or insert whose value is a tuple literal?
    pub(super) fn tail_is_tuple(v: &Value) -> bool {
        match v.unspan() {
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|t| matches!(t.unspan(), Value::Tuple(_))),
            Value::Insert(ops) => ops
                .last()
                .is_some_and(|t| matches!(t.unspan(), Value::Tuple(_))),
            _ => false,
        }
    }

    /// Evaluate a [`Self::tail_is_tuple`] block with its tail read as a member.
    pub(super) fn eval_member_block(&mut self, v: &Value, st: State) -> Option<(State, Val)> {
        let ops = match v.unspan() {
            Value::Block(bl) => &bl.operators,
            Value::Insert(ops) => ops,
            _ => return self.eval(v, st),
        };
        let (last, init) = ops.split_last()?;
        let mut s = st;
        for op in init {
            let (s2, _) = self.eval(op, s)?;
            s = s2;
        }
        self.member_tuple = true;
        let out = self.eval(last, s);
        self.member_tuple = false;
        out
    }

    /// Is member `i` of tuple variable `t` a store reference?
    pub(super) fn member_is_heap(&self, t: u16, i: u16) -> bool {
        let vars = &self.data.def(self.d_nr).variables;
        t < vars.count()
            && matches!(vars.tp(t).base(), crate::data::Type::Tuple(elems)
                if elems.get(i as usize).is_some_and(|e| crate::data::is_dbref(e.base())))
    }

    /// A user function's arguments: a tuple literal argument is the callee's to READ, so its
    /// members' stores stay with the caller, who releases them (a call-minted member is lifted
    /// into a temp the scope pass frees) — where a tuple literal elsewhere hands them on.
    pub(super) fn user_call_args(
        &mut self,
        args: &[Value],
        st: State,
    ) -> Option<(State, Vec<Val>)> {
        let mut s = st;
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            // A block ending in the literal (`{ __ref_8 = tp; (OpGetField(__ref_8, …), …) }`, a
            // boxed tuple unboxed into the argument) runs its leading statements first.
            let (prefix, lit): (&[Value], &Value) = match a.unspan() {
                Value::Block(b) => match b.operators.split_last() {
                    Some((last, init)) if matches!(last.unspan(), Value::Tuple(_)) => (init, last),
                    _ => (&[], a),
                },
                Value::Insert(ops) => match ops.split_last() {
                    Some((last, init)) if matches!(last.unspan(), Value::Tuple(_)) => (init, last),
                    _ => (&[], a),
                },
                _ => (&[], a),
            };
            if let Value::Tuple(xs) = lit.unspan() {
                if !prefix.is_empty() {
                    s = self.seq(prefix, s)?.0;
                }
                let target = self.tuple_target.take();
                let parts = self.tuple_elems(xs, s);
                self.tuple_target = target;
                let (s2, parts) = parts?;
                s = s2;
                let mut whole = Val::null();
                for p in parts {
                    whole.pts.extend(p.pts);
                }
                if whole.pts.len() > 1 {
                    whole.pts.remove(&Pt::Null);
                }
                vals.push(whole);
            } else {
                let (s2, vs) = self.args(std::slice::from_ref(a), s)?;
                s = s2;
                vals.extend(vs);
            }
        }
        Some((s, vals))
    }

    /// Evaluate call arguments left to right.  A bare variable argument is a use of its
    /// store.
    pub(super) fn args(&mut self, args: &[Value], st: State) -> Option<(State, Vec<Val>)> {
        let target = self.tuple_target.take();
        let member = std::mem::take(&mut self.member_tuple);
        let out = self.args_inner(args, st);
        self.tuple_target = target;
        self.member_tuple = member;
        out
    }

    pub(super) fn args_inner(&mut self, args: &[Value], st: State) -> Option<(State, Vec<Val>)> {
        let mut s = st;
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            let (s2, value) = self.eval(a, s)?;
            if let Value::Var(x) = a.unspan() {
                self.use_var(&s2, *x, "a call");
            }
            if let Some(h) = Self::tail_var(a) {
                self.use_points(&s2, h);
            }
            vals.push(value);
            s = s2;
        }
        Some((s, vals))
    }

    pub(super) fn summary_of(&self, pts: &Pts) -> Summary {
        let mut out = Summary::Bottom;
        for p in pts {
            let one = match p {
                Pt::Null => Summary::Bottom,
                Pt::Store(k) => match self.sites[k.site as usize].origin {
                    Some(Origin::MintInto(i)) => Summary::Buf(i),
                    _ => Summary::Fresh,
                },
                Pt::Param(v) => self
                    .params
                    .iter()
                    .position(|x| x == v)
                    .map_or(Summary::Unknown, |i| Summary::Arg(i as u16)),
                Pt::Unknown => Summary::Unknown,
            };
            out = out.join(one);
        }
        out
    }

    pub(super) fn first_var(args: &[Value], i: usize) -> Option<u16> {
        args.get(i).and_then(|a| {
            if let Value::Var(x) = a.unspan() {
                Some(*x)
            } else {
                None
            }
        })
    }

    /// The variable a value block hands back as its tail (`{#Object __ref_p2_N = …;
    /// __ref_p2_N}`): a copy that frees such a source releases that variable's store.
    pub(super) fn tail_var(v: &Value) -> Option<u16> {
        match v.unspan() {
            Value::Var(x) => Some(*x),
            Value::Block(bl) => bl.operators.last().and_then(Self::tail_var),
            Value::Insert(ops) => ops.last().and_then(Self::tail_var),
            _ => None,
        }
    }
}
