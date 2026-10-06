// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-L-CapKeep`: the closure records and fn-ref locals whose store must outlive the pass or
//! the binding that built it, and the statements that keep or release it at a build, a rebind and
//! the end of a loop pass.

use super::drops::drop_hook;
use super::phase::var_mentions_in;
use super::{Scopes, call};
use crate::data::{Block, Data, Type, Value, v_if, v_set};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;

impl Scopes<'_> {
    /// The holders a store-identity test may read at this point: those with a DOMINATING
    /// bind (`fnref_bound`), so none is read out of a slot nothing has written (SLOTS.md § the
    /// reserve does not initialise).  A record is pre-initialised at the function's head.
    pub(super) fn closure_keep_live(&self, except: &[u16]) -> Vec<u16> {
        self.closure_keep
            .holders
            .iter()
            .copied()
            .filter(|h| !except.contains(h) && self.fnref_bound.iter().any(|b| b.contains(h)))
            .collect()
    }

    /// The caller's fn-refs behind the frame's `&fn(…)` parameters, copied into temps the
    /// store-identity tests can read: `(before, temps, after)`.  The copies own nothing, so
    /// `after` nulls them before any sweep of the temps could release the caller's record.
    pub(super) fn closure_keep_links(
        &mut self,
        function: &mut Function,
    ) -> (Vec<Value>, Vec<u16>, Vec<Value>) {
        let mut pre = Vec::new();
        let mut temps = Vec::new();
        let mut post = Vec::new();
        for p in self.closure_keep.links.clone() {
            let Type::RefVar(inner) = function.tp(p).base().clone() else {
                continue;
            };
            self.lift_counter += 1;
            let tmp = function.add_temp_var(&format!("__fklink_{}", self.lift_counter), &inner);
            self.var_scope.insert(tmp, self.scope);
            pre.push(v_set(tmp, Value::Var(p)));
            post.push(v_set(tmp, Value::Null));
            temps.push(tmp);
        }
        (pre, temps, post)
    }

    /// `v_if` chain: `then` when `closure` names a store none of `holders` and none of
    /// `records` names, `shared` otherwise.
    pub(super) fn closure_keep_unless_shared(
        closure: &Value,
        holders: &[u16],
        records: &[u16],
        then: Value,
        shared: &Value,
        data: &Data,
    ) -> Value {
        let distinct = data.def_nr("OpDistinctStore");
        let fn_closure = data.def_nr("OpFnRefClosure");
        let mut out = then;
        let others = holders
            .iter()
            .map(|&h| Value::Call(fn_closure, vec![Value::Var(h)]))
            .chain(records.iter().map(|&r| Value::Var(r)));
        for other in others.collect::<Vec<_>>().into_iter().rev() {
            out = v_if(
                Value::Call(distinct, vec![closure.clone(), other]),
                out,
                shared.clone(),
            );
        }
        out
    }

    /// `@FR-L-CapKeep` at a BUILD in a loop: the statements that hand the record the build is
    /// about to rebuild to the fn-ref that still names it (loft#1636).
    ///
    /// A record is rebuilt in place on every pass, which is right while nothing but the build's
    /// own target names it.  Where another fn-ref of the frame still holds it (a pass that was
    /// kept), the record local lets go of it instead — the record is that holder's from here on
    /// — and the build mints a new one.  So do the literal backings the record's captures
    /// reach: the kept record still reads them, and the next pass's literal would refill them.
    pub(super) fn closure_keep_detach(
        &self,
        stmt: &Value,
        function: &Function,
        data: &Data,
        links: &[u16],
    ) -> Vec<Value> {
        let k = &self.closure_keep;
        if !k.gated || self.loops.is_empty() {
            return Vec::new();
        }
        // Asked at the statement that BINDS the build (`f = fn() {…}`): the build block opens
        // with the release of what the previous record adopted, which must already see the
        // record handed over.  Its own target is rebound by the statement, so the record it
        // names is not being kept.  A build not bound directly (an argument, `keep(fn() {…})`)
        // is asked at its `OpDatabase` statement instead.
        let database = data.def_nr("OpDatabase");
        let is_build = |op: &Value| {
            matches!(op.unspan(), Value::Call(d, args) if *d == database
                && matches!(args.first().map(Value::unspan), Some(Value::Var(r)) if k.records.contains(r)))
        };
        let (target, built): (Option<u16>, Vec<u16>) = match stmt.unspan() {
            Value::Set(t, rhs) => {
                let Value::Block(b) = rhs.unspan() else {
                    return Vec::new();
                };
                if b.name != "fn_ref_with_closure" {
                    return Vec::new();
                }
                let built = b
                    .operators
                    .iter()
                    .filter(|op| is_build(op))
                    .filter_map(|op| match op.unspan() {
                        Value::Call(_, args) => match args.first().map(Value::unspan) {
                            Some(Value::Var(r)) => Some(*r),
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect();
                (Some(*self.var_mapping.get(t).unwrap_or(t)), built)
            }
            Value::Call(_, args) if is_build(stmt) && self.keep_build_target.is_none() => {
                match args.first().map(Value::unspan) {
                    Some(Value::Var(r)) => (None, vec![*r]),
                    _ => return Vec::new(),
                }
            }
            _ => return Vec::new(),
        };
        if built.is_empty() {
            return Vec::new();
        }
        let backings: Vec<(u16, Value, u16)> = k
            .captures
            .iter()
            .filter_map(|(rec, off, x)| {
                let b = *self.capture_build_backing.backing.get(x)?;
                (function.name(b).starts_with("__vdb_")).then(|| (*rec, off.clone(), b))
            })
            .collect();
        let except: Vec<u16> = target.into_iter().collect();
        let mut holders = self.closure_keep_live(&except);
        holders.extend_from_slice(links);
        if holders.is_empty() && self.closure_keep.links.is_empty() {
            return Vec::new();
        }
        let sentinel = || Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new());
        let mut out = Vec::new();
        for r in built {
            let mut detach: Vec<Value> = Vec::new();
            for (_, off, b) in backings.iter().filter(|(rec, _, _)| *rec == r) {
                let captured =
                    Value::Call(data.def_nr("OpGetDbRef"), vec![Value::Var(r), off.clone()]);
                detach.push(v_if(
                    Value::Call(
                        data.def_nr("OpDistinctStore"),
                        vec![Value::Var(*b), captured],
                    ),
                    Value::Null,
                    v_set(*b, sentinel()),
                ));
            }
            detach.push(v_set(r, sentinel()));
            let detach = Value::Insert(detach);
            // A holder naming the record is the whole condition; nobody naming it keeps it.
            out.push(v_if(
                Value::Call(data.def_nr("OpConvBoolFromRef"), vec![Value::Var(r)]),
                Self::closure_keep_unless_shared(
                    &Value::Var(r),
                    &holders,
                    &[],
                    Value::Null,
                    &detach,
                    data,
                ),
                Value::Null,
            ));
        }
        out
    }

    /// `@FR-L-CapKeep` at the END OF A PASS: the statements that release what a loop body's
    /// fn-ref holds where no other name kept it (loft#1636).
    ///
    /// A fn-ref every mention of which lies in this block is dead at its `}`, so the closure it
    /// holds is released there, hooks included, exactly as a record local of the block would
    /// be — unless another name of the frame still holds the same store, which then owns it.
    /// A record this block built for such a fn-ref is released first, through its own cascade,
    /// and the fn-ref nulled so it does not release the store a second time.  The block's end
    /// is not reached by a `break` or a `continue`: those passes release at the next rebuild or
    /// at the function's end, once either way.
    pub(super) fn closure_keep_pass_end(
        &mut self,
        bl: &Block,
        function: &mut Function,
        data: &Data,
    ) -> Vec<Value> {
        if !self.closure_keep.gated || self.loops.is_empty() || bl.result != Type::Void {
            return Vec::new();
        }
        let mut here: HashMap<u16, usize> = HashMap::default();
        for op in &bl.operators {
            for (v, n) in var_mentions_in(op) {
                *here.entry(v).or_insert(0) += n;
            }
        }
        let dead: Vec<u16> = self
            .closure_keep
            .owning
            .iter()
            .copied()
            .filter(|t| {
                here.get(t)
                    .is_some_and(|&n| n > 0 && Some(&n) == self.mentions.get(t))
                    && self.var_scope.contains_key(t)
                    && !function.is_captured(*t)
                    && !function.is_skip_free(*t)
            })
            .collect();
        // A record built INLINE (`run(fn() {…})`) has no fn-ref of its own: its only name is
        // the value the build hands to the expression around it, so it is dead at the end of
        // the block its build lies in.  (Its local is also named by the function's head
        // pre-init, so the mention count cannot say this.)
        let database = data.def_nr("OpDatabase");
        let mut built_here: HashSet<u16> = HashSet::default();
        for op in &bl.operators {
            op.walk(&mut |n| {
                if let Value::Call(d, args) = n.unspan()
                    && *d == database
                    && let Some(Value::Var(r)) = args.first().map(Value::unspan)
                {
                    built_here.insert(*r);
                }
            });
        }
        // …unless a capture it reads is reassigned after the build (`s = build(|i| s.a + i)`):
        // `(O-Latest)` hands that store back to the frame's release
        // (`reassigned_after_build`), and releasing the record here would release it twice.
        let reassigned = &self.capture_build_backing.reassigned_after_build;
        let inline: Vec<u16> = self
            .closure_keep
            .records
            .iter()
            .copied()
            .filter(|r| {
                !self.closure_keep.targets.contains_key(r)
                    && built_here.contains(r)
                    && !self
                        .closure_keep
                        .captures
                        .iter()
                        .any(|(rec, _, x)| rec == r && reassigned.contains(x))
            })
            .collect();
        if dead.is_empty() && inline.is_empty() {
            return Vec::new();
        }
        let (link_pre, links, link_post) = self.closure_keep_links(function);
        let mut out = link_pre;
        let sentinel = || Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new());
        for r in self.closure_keep.records.clone() {
            let built_for: Vec<u16> = self
                .closure_keep
                .targets
                .get(&r)
                .map(|ts| ts.iter().copied().filter(|t| dead.contains(t)).collect())
                .unwrap_or_default();
            if built_for.is_empty() && !inline.contains(&r) {
                continue;
            }
            let mut others = self.closure_keep_live(&built_for);
            others.extend(links.iter().copied());
            let mut release = Vec::new();
            if let Some(hook) = drop_hook(function, r, data) {
                release.push(hook);
            }
            release.push(call("OpFreeRef", r, data));
            release.push(v_set(r, sentinel()));
            for &t in &built_for {
                release.push(v_set(t, Value::Null));
            }
            out.push(v_if(
                Value::Call(data.def_nr("OpConvBoolFromRef"), vec![Value::Var(r)]),
                Self::closure_keep_unless_shared(
                    &Value::Var(r),
                    &others,
                    &[],
                    Value::Insert(release),
                    &Value::Null,
                    data,
                ),
                Value::Null,
            ));
        }
        for &t in &dead {
            out.extend(self.closure_keep_stand_down(t, function, data));
            if data.any_closure_drop() {
                out.push(call("OpDropFnRef", t, data));
            }
            out.push(call("OpFreeRef", t, data));
            out.push(v_set(t, Value::Null));
        }
        out.extend(link_post);
        out
    }

    /// `@FR-L-CapKeep` at the scope-end release of the fn-ref `v`: null its closure half where
    /// a closure record or another live fn-ref names the same store, so the release that
    /// follows finds nothing and that name releases the store instead.  Each stand-down nulls,
    /// so of several names released in one sweep exactly the last releases.
    pub(super) fn closure_keep_stand_down(
        &mut self,
        v: u16,
        function: &mut Function,
        data: &Data,
    ) -> Vec<Value> {
        // The caller's fn-refs behind the `&fn(…)` parameters are names too: a closure written
        // out through one is the caller's (loft#1443's shape, which a sweep that asked only the
        // frame's own names released under the caller).
        let (link_pre, links, link_post) = self.closure_keep_links(function);
        let mut out = link_pre;
        out.extend(
            self.closure_keep_live(&[v])
                .into_iter()
                .chain(links)
                .map(|y| {
                    Value::Call(
                        data.def_nr("OpFnRefDetachShared"),
                        vec![Value::Var(v), Value::Var(y)],
                    )
                }),
        );
        let closure = Value::Call(data.def_nr("OpFnRefClosure"), vec![Value::Var(v)]);
        for &r in &self.closure_keep.records {
            out.push(v_if(
                Value::Call(
                    data.def_nr("OpDistinctStore"),
                    vec![closure.clone(), Value::Var(r)],
                ),
                Value::Null,
                v_set(v, Value::Null),
            ));
        }
        out.extend(link_post);
        out
    }

    /// `@FR-L-CapKeep` at the re-mint of a literal BACKING a record captures (loft#1636): the
    /// statements that let the frame's backing go where the record reading it was kept.
    ///
    /// The literal runs ahead of the build in the pass, so by the build the backing would
    /// already hold the new pass's elements.  Where a fn-ref other than the build's own target
    /// still names the record whose capture is this backing, that record is kept, and the
    /// backing is its from here on: the literal mints a fresh store.
    pub(super) fn closure_keep_backing(
        &self,
        stmt: &Value,
        function: &Function,
        data: &Data,
        links: &[u16],
    ) -> Vec<Value> {
        let k = &self.closure_keep;
        if !k.gated || self.loops.is_empty() {
            return Vec::new();
        }
        let Value::Call(d, args) = stmt.unspan() else {
            return Vec::new();
        };
        if *d != data.def_nr("OpDatabase") {
            return Vec::new();
        }
        let Some(Value::Var(b)) = args.first().map(Value::unspan) else {
            return Vec::new();
        };
        if !function.name(*b).starts_with("__vdb_") {
            return Vec::new();
        }
        let sentinel = || Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new());
        let distinct = data.def_nr("OpDistinctStore");
        let mut out = Vec::new();
        for (r, off, x) in &k.captures {
            if self.capture_build_backing.backing.get(x) != Some(b) {
                continue;
            }
            let except = k.targets.get(r).cloned().unwrap_or_default();
            let mut holders = self.closure_keep_live(&except);
            holders.extend_from_slice(links);
            if holders.is_empty() && k.links.is_empty() {
                continue;
            }
            let captured =
                Value::Call(data.def_nr("OpGetDbRef"), vec![Value::Var(*r), off.clone()]);
            let kept = Self::closure_keep_unless_shared(
                &Value::Var(*r),
                &holders,
                &[],
                Value::Null,
                &v_set(*b, sentinel()),
                data,
            );
            out.push(v_if(
                Value::Call(data.def_nr("OpConvBoolFromRef"), vec![Value::Var(*r)]),
                v_if(
                    Value::Call(distinct, vec![Value::Var(*b), captured]),
                    Value::Null,
                    kept,
                ),
                Value::Null,
            ));
        }
        out
    }

    /// `@FR-L-CapKeep` at a REBIND of an owning fn-ref local: `(before, after)` the statement
    /// (loft#1636).  The displaced value is copied aside before, and released after only where
    /// no other name of the frame — the local's new value, another holder, a closure record —
    /// holds the same store.  Released, it runs the record's cascade (`OpDropFnRef`) and frees
    /// the store; the copy is then nulled so the sweep of the temp releases nothing.
    pub(super) fn closure_keep_rebind(
        &mut self,
        stmt: &Value,
        function: &mut Function,
        data: &Data,
    ) -> Option<(Vec<Value>, Vec<Value>)> {
        if !self.closure_keep.gated {
            return None;
        }
        let Value::Set(ov, _) = stmt.unspan() else {
            return None;
        };
        let v = *self.var_mapping.get(ov).unwrap_or(ov);
        if !self.closure_keep.owning.contains(&v)
            || !self.var_scope.contains_key(&v)
            || !self.fnref_bound.iter().any(|b| b.contains(&v))
            || function.is_captured(v)
            || function.is_skip_free(v)
        {
            return None;
        }
        let mut holders = self.closure_keep_live(&[]);
        let records = self.closure_keep.records.clone();
        let (link_pre, links, link_post) = self.closure_keep_links(function);
        holders.extend(links);
        let tp = function.tp(v).clone();
        self.lift_counter += 1;
        let tmp = function.add_temp_var(&format!("__fkeep_{}", self.lift_counter), &tp);
        self.var_scope.insert(tmp, self.scope);
        let pre = vec![v_set(tmp, Value::Var(v))];
        let mut release = Vec::new();
        if data.any_closure_drop() {
            release.push(call("OpDropFnRef", tmp, data));
        }
        release.push(call("OpFreeRef", tmp, data));
        let closure = Value::Call(data.def_nr("OpFnRefClosure"), vec![Value::Var(tmp)]);
        let mut post = link_pre;
        post.push(Self::closure_keep_unless_shared(
            &closure,
            &holders,
            &records,
            Value::Insert(release),
            &Value::Null,
            data,
        ));
        post.extend(link_post);
        post.push(v_set(tmp, Value::Null));
        Some((pre, post))
    }

    /// `@FR-L-CapOwn` at a REBIND of a fn-ref local that holds a CALL's closure: `(before,
    /// after)` the statement, or `None` where it is not such a rebind (loft#1609).
    ///
    /// A closure record that left its frame is released by the fn-ref holding it, and once the
    /// local names the new value nothing names the old one.  Before the statement the old
    /// value is copied into a displaced temp; after it, the temp gives up a closure it shares
    /// with the new value (`OpFnRefDetachShared` — `f = keep(f)` hands the same record back),
    /// runs the record's cascade and frees its store, and is set to null so the scope-end
    /// sweep of the temp releases nothing.  Declined where the old value may be a record this
    /// frame built (the record releases through itself) or another local shares the fn-ref
    /// (`g = f`: releasing it here would leave `g` a freed closure).
    pub(super) fn fnref_call_rebind(
        &mut self,
        stmt: &Value,
        function: &mut Function,
        data: &Data,
    ) -> Option<(Vec<Value>, Vec<Value>)> {
        let Value::Set(ov, rhs) = stmt.unspan() else {
            return None;
        };
        // Only a CALL's closure is displaced here: a lambda built in this frame is released
        // through its own record, and the first assignment of a local declared ahead of a
        // branch (`h` bound in each arm) displaces only its null pre-init.
        if !matches!(rhs.unspan(), Value::Call(d, _) if !data.def(*d).name().starts_with("Op")) {
            return None;
        }
        let v = *self.var_mapping.get(ov).unwrap_or(ov);
        if !matches!(function.tp(v).base(), Type::Function(..))
            || !self.fnref_bound.iter().any(|b| b.contains(&v))
            || !self.var_scope.contains_key(&v)
            || function.is_argument(v)
            || function.is_captured(v)
            || function.is_compiler_generated(v)
            || function.is_skip_free(v)
        {
            return None;
        }
        let count = function.count();
        // Every entry a callee's tagged note (a call result), never a local of this frame.
        if function.tp(v).depend().iter().any(|&r| r < count) {
            return None;
        }
        if (0..count).any(|u| {
            u != v
                && matches!(function.tp(u).base(), Type::Function(..))
                && function.tp(u).depend().contains(&v)
        }) {
            return None;
        }
        let tp = function.tp(v).clone();
        self.lift_counter += 1;
        let tmp = function.add_temp_var(&format!("__fdisp_{}", self.lift_counter), &tp);
        self.var_scope.insert(tmp, self.scope);
        let pre = vec![v_set(tmp, Value::Var(v))];
        let mut post = vec![Value::Call(
            data.def_nr("OpFnRefDetachShared"),
            vec![Value::Var(tmp), Value::Var(v)],
        )];
        if data.any_closure_drop() {
            post.push(call("OpDropFnRef", tmp, data));
        }
        post.push(call("OpFreeRef", tmp, data));
        post.push(v_set(tmp, Value::Null));
        Some((pre, post))
    }
}

/// loft#1636, `@FR-L-CapKeep` — the closure records and the fn-ref locals of one frame whose
/// releases are decided by STORE IDENTITY at run time.
///
/// `(L-CapScalar)` makes every build of a closure its own value, and `(L-CapOwn)` frees each
/// record once.  A record built in a loop is rebuilt in place on the next pass, which is right
/// while nothing else names it, and a fn-ref local kept from an earlier pass (`if i == 0 { g =
/// f }`) is such a name.  Which pass was kept is a per-run fact, so where more than one name can
/// reach one record the frame asks, at the moment it would let a record go, whether any other
/// of its names still holds that store:
/// - a build that would rebuild a record another fn-ref still names builds a new one instead
///   (`Scopes::closure_keep_detach`);
/// - a rebind of a fn-ref releases the record it displaced only where no other name holds it
///   (`Scopes::closure_keep_rebind`);
/// - the scope-end sweep releases a record, or a fn-ref's closure, only where no name released
///   after it holds the same store (`Scopes::closure_keep_stand_down`).
///
/// `holders` are the frame's own fn-ref locals, `records` its `___clos_N` locals not confined
/// to one pass.  `owning` are the holders every assignment of which yields a closure this
/// frame owns (a build, another holder, a call's result): only a displaced value of those is
/// ever released at a rebind, so a fn-ref that may view a caller's closure never is.
/// `LOFT_NO_CLOSURE_KEEP=1` leaves every frame ungated — the bisect step for a wrong value, a
/// leak or a double release out of a closure kept from one loop pass.
#[derive(Default)]
pub(crate) struct ClosureKeep {
    pub(super) gated: bool,
    pub(super) records: Vec<u16>,
    pub(super) holders: Vec<u16>,
    owning: HashSet<u16>,
    /// Every capture a build writes: `(record, offset, captured local)`.
    captures: Vec<(u16, Value, u16)>,
    /// The fn-ref locals each record's build is bound to directly (`f = fn() {…}`).
    targets: HashMap<u16, Vec<u16>>,
    /// The `&fn(…)` parameters: a record written through one is the CALLER's name for it.
    pub(super) links: Vec<u16>,
}

impl ClosureKeep {
    /// Whether the frame decides at run time if the record `r` was delivered through a
    /// `&fn(…)` parameter, instead of taking every link write as a delivery.
    pub(crate) fn decides_link(&self, r: u16) -> bool {
        self.gated && !self.links.is_empty() && self.records.contains(&r)
    }
}

#[expect(clippy::too_many_lines, reason = "inherited")]
pub(crate) fn closure_keep_set(data: &Data, function: &Function, code: &Value) -> ClosureKeep {
    if crate::env_once!(std::env::var_os("LOFT_NO_CLOSURE_KEEP").is_some()) {
        return ClosureKeep::default();
    }
    let confined = pass_confined_records(data, function, code);
    let mut out = ClosureKeep::default();
    for v in 0..function.count() {
        if function.is_argument(v) {
            if let Type::RefVar(inner) = function.tp(v).base()
                && matches!(inner.base(), Type::Function(..))
            {
                out.links.push(v);
            }
            continue;
        }
        match function.tp(v).base() {
            Type::Function(..) if !function.is_compiler_generated(v) => out.holders.push(v),
            Type::Reference(r, _)
                if function.name(v).starts_with("___clos_") && !confined.contains(r) =>
            {
                out.records.push(v);
            }
            _ => {}
        }
    }
    let database = data.def_nr("OpDatabase");
    let mut built_in_loop = false;
    let mut shared = false;
    let mut foreign: HashSet<u16> = HashSet::default();
    // The three `&mut` out-params are one answer taken in one pass; bundling them into a
    // struct would name a type that exists only to satisfy the count.  Same reading as the
    // other 27 sites that carry this allow.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        node: &Value,
        in_loop: bool,
        keep: &ClosureKeep,
        data: &Data,
        database: u32,
        built_in_loop: &mut bool,
        shared: &mut bool,
        foreign: &mut HashSet<u16>,
    ) {
        let inner = in_loop || matches!(node.unspan(), Value::Loop(_));
        match node.unspan() {
            Value::Call(d, args)
                if in_loop
                    && *d == database
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(r)) if keep.records.contains(r)) =>
            {
                *built_in_loop = true;
            }
            Value::Set(p, rhs)
                if keep.links.contains(p)
                    && matches!(rhs.unspan(), Value::Var(x) if keep.holders.contains(x)) =>
            {
                *shared = true;
            }
            Value::Set(h, rhs) if keep.holders.contains(h) => match rhs.unspan() {
                Value::Var(x) if keep.holders.contains(x) => *shared = true,
                Value::Call(d, args) if !data.def(*d).name().starts_with("Op") => {
                    if args
                        .iter()
                        .any(|a| matches!(a.unspan(), Value::Var(x) if keep.holders.contains(x)))
                    {
                        *shared = true;
                    }
                }
                Value::Block(b) if b.name == "fn_ref_with_closure" => {}
                Value::Null | Value::Int(_) => {}
                _ => {
                    foreign.insert(*h);
                }
            },
            _ => {}
        }
        node.unspan().for_each_child(&mut |ch| {
            walk(
                ch,
                inner,
                keep,
                data,
                database,
                built_in_loop,
                shared,
                foreign,
            );
        });
    }
    walk(
        code,
        false,
        &out,
        data,
        database,
        &mut built_in_loop,
        &mut shared,
        &mut foreign,
    );
    out.gated = (built_in_loop && !out.records.is_empty()) || (shared && !out.holders.is_empty());
    if !out.gated {
        return ClosureKeep::default();
    }
    let set_dbref = data.def_nr("OpSetDbRef");
    let mut captures = Vec::new();
    code.walk(&mut |n| {
        if let Value::Call(d, args) = n.unspan()
            && *d == set_dbref
            && let (Some(Value::Var(r)), Some(off), Some(Value::Var(x))) = (
                args.first().map(Value::unspan),
                args.get(1),
                args.get(2).map(Value::unspan),
            )
            && out.records.contains(r)
        {
            captures.push((*r, off.clone(), *x));
        }
    });
    out.captures = captures;
    let mut targets: HashMap<u16, Vec<u16>> = HashMap::default();
    code.walk(&mut |n| {
        if let Value::Set(h, rhs) = n.unspan()
            && let Value::Block(b) = rhs.unspan()
            && b.name == "fn_ref_with_closure"
        {
            for op in &b.operators {
                if let Value::Call(d, args) = op.unspan()
                    && *d == database
                    && let Some(Value::Var(r)) = args.first().map(Value::unspan)
                    && out.records.contains(r)
                {
                    targets.entry(*r).or_default().push(*h);
                }
            }
        }
    });
    out.targets = targets;
    out.owning = out
        .holders
        .iter()
        .copied()
        .filter(|h| !foreign.contains(h))
        .collect();
    if crate::env_once!(std::env::var_os("LOFT_TRACE_CLOSURE_KEEP").is_some()) {
        eprintln!(
            "[closure-keep] {}: records {:?} holders {:?} owning {:?}",
            function.name,
            out.records
                .iter()
                .map(|&v| function.name(v).to_string())
                .collect::<Vec<_>>(),
            out.holders
                .iter()
                .map(|&v| function.name(v).to_string())
                .collect::<Vec<_>>(),
            out.owning
                .iter()
                .map(|&v| function.name(v).to_string())
                .collect::<Vec<_>>(),
        );
    }
    out
}

/// loft#1610, `@FR-L-CapOwn` — the closure record types whose every value is confined to one
/// pass of a loop.
///
/// `(L-CapOwn)` frees a captured store once, "by whichever of the two outlives the other".  A
/// record built in a loop body is rebuilt every pass, and while it ADOPTED its captures each
/// pass's record released what it adopted: a loop-body capture through a backing the next pass
/// had already refilled (the next pass's element, released early and then twice more), a
/// function-scope capture once per pass.  Where the fn-ref local that holds the record is itself
/// a loop-body local used only as a callee, the record dies with the pass, so it cannot outlive
/// anything it captured, and the frame's own release is the one that stands.
///
/// Confined: a `Set(f, …)` inside a loop whose value builds a closure (`FnRef` over a
/// `___clos_N`), where every `Set` of `f` lies in that loop and `f` is never read as a VALUE
/// anywhere.  A call through it is `CallRef(f, …)`, whose callee is an index and not a `Var`.
/// The operands of a release (`OpFreeRef`, `OpConvBoolFromRef`, `OpRefIsNull`, …) are not
/// escapes: this is asked of the body before the scope pass and after it, and the two readings
/// must agree.
pub(super) fn pass_confined_records(
    data: &Data,
    function: &Function,
    code: &Value,
) -> HashSet<u32> {
    let ops = |names: &[&str]| -> HashSet<u32> {
        names
            .iter()
            .map(|n| data.def_nr(n))
            .filter(|&d| d != u32::MAX)
            .collect()
    };
    let mut walk = ConfinementWalk {
        release_ops: ops(&[
            "OpFreeRef",
            "OpFreeRefIfDistinct",
            "OpConvBoolFromRef",
            "OpRefIsNull",
            // The `(L-CapKeep)` store-identity tests and a fn-ref's own release (loft#1869):
            // they compare or release what a local names and hand it nowhere.
            "OpDistinctStore",
            "OpFnRefDetachShared",
            "OpDropFnRef",
        ]),
        fn_closure: ops(&["OpFnRefClosure"]),
        // The `(L-CapKeep)` holders the scope pass itself binds (`__fkeep_N = f`,
        // `__fklink_N`) only compare stores: their bind is no escape, or the reading after the
        // scope pass would disagree with the one before it.
        keep_temps: (0..function.count())
            .filter(|&v| {
                let n = function.name(v);
                n.starts_with("__fkeep_") || n.starts_with("__fklink_")
            })
            .collect(),
        next_id: 0,
        builds: Vec::new(),
        sets: HashMap::default(),
        read_as_value: HashSet::default(),
        held_in: Vec::new(),
    };
    walk.walk(code, usize::MAX);
    // A fn-ref held in a record that is not confined escapes with it; repeat until nothing
    // more escapes (each round only grows `read_as_value`).
    loop {
        let out = walk.confined(data, function);
        let mut grew = false;
        for &(f, r) in &walk.held_in {
            let holder_confined =
                matches!(function.tp(r).base(), Type::Reference(rec, _) if out.contains(rec));
            if !holder_confined && walk.read_as_value.insert(f) {
                grew = true;
            }
        }
        if !grew {
            return out;
        }
    }
}

/// The one walk [`pass_confined_records`] reads its facts from.
struct ConfinementWalk {
    release_ops: HashSet<u32>,
    fn_closure: HashSet<u32>,
    keep_temps: HashSet<u16>,
    next_id: usize,
    /// (fn-ref local, record local, loop id) for every closure build inside a loop.
    builds: Vec<(u16, u16, usize)>,
    /// Per local: the loop ids of every `Set` of it (usize::MAX outside any loop).
    sets: HashMap<u16, HashSet<usize>>,
    /// Locals read as a VALUE somewhere that is not a release operand.
    read_as_value: HashSet<u16>,
    /// `(fn-ref, record local)`: a build that captured the fn-ref's closure (loft#1869).
    held_in: Vec<(u16, u16)>,
}

impl ConfinementWalk {
    fn built_record(v: &Value) -> Option<u16> {
        match v.unspan() {
            Value::FnRef(_, rec, _) if *rec != u16::MAX => Some(*rec),
            Value::Block(bl) => bl.operators.last().and_then(Self::built_record),
            Value::Insert(ops) => ops.last().and_then(Self::built_record),
            _ => None,
        }
    }

    fn walk(&mut self, node: &Value, loop_id: usize) {
        // loft#1869 — a closure build that captures fn-ref `f` (`OpSetDbRef(r, _,
        // OpFnRefClosure(f))`) hands `f`'s record to record `r`: an escape only where `r`'s own
        // record escapes, settled once every build is known.
        if let Value::Call(_, args) = node.unspan()
            && let (Some(Value::Var(r)), Some(Value::Call(c, inner))) = (
                args.first().map(Value::unspan),
                args.get(2).map(Value::unspan),
            )
            && self.fn_closure.contains(c)
            && let Some(Value::Var(f)) = inner.first().map(Value::unspan)
        {
            self.held_in.push((*f, *r));
            return;
        }
        let mut loop_id = loop_id;
        match node.unspan() {
            Value::Set(v, rhs)
                if self.keep_temps.contains(v) && matches!(rhs.unspan(), Value::Var(_)) =>
            {
                return;
            }
            Value::Loop(_) => {
                loop_id = self.next_id;
                self.next_id += 1;
            }
            // A null write stores no record — the scope pass writes them at a head and after a
            // loop, and the reading after it must agree with the one before (loft#1869).
            Value::Set(_, rhs) if matches!(rhs.unspan(), Value::Null) => return,
            Value::Set(v, rhs) => {
                self.sets.entry(*v).or_default().insert(loop_id);
                if loop_id != usize::MAX
                    && let Some(rec) = Self::built_record(rhs)
                {
                    self.builds.push((*v, rec, loop_id));
                }
            }
            Value::Var(v) => {
                self.read_as_value.insert(*v);
            }
            Value::Call(d, _) if self.release_ops.contains(d) => return,
            _ => {}
        }
        node.unspan().for_each_child(&mut |c| self.walk(c, loop_id));
    }

    /// The record types every build of which is confined, given what is read as a value.
    fn confined(&self, data: &Data, function: &Function) -> HashSet<u32> {
        let mut out = HashSet::default();
        for &(f, rec, loop_id) in &self.builds {
            let only_this_loop = self
                .sets
                .get(&f)
                .is_some_and(|ids| ids.len() == 1 && ids.contains(&loop_id));
            if only_this_loop
                && !self.read_as_value.contains(&f)
                && let Type::Reference(record, _) = function.tp(rec).base()
                && data.def(*record).name.starts_with("__closure_")
            {
                out.insert(*record);
            }
        }
        out
    }
}
