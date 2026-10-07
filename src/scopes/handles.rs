// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Generator HANDLES: which locals and tuple members own a handle, which only view a member's
//! handle, and the retain that keeps a handle alive while a second holder shares it.

use super::Scopes;
use crate::data::{Data, Type, Value, v_set};
use crate::fxhash::FxHashSet as HashSet;
use crate::variables::Function;
use std::collections::{BTreeMap, BTreeSet};

/// Is `rhs` a generator handle read out of a field or an element — a VIEW of a member's handle
/// (loft#1585), which the parser marks never-free?
fn is_handle_projection(rhs: &Value, data: &Data) -> bool {
    matches!(rhs.unspan(), Value::Call(d, _) if data.def(*d).name() == "OpGetDbRef")
}

/// The values a generator-handle right-hand side can deliver: the tails of a value `if` (a
/// lowered `match` among them) and of a block, each arm on its own path.
fn handle_leaves<'v>(rhs: &'v Value, out: &mut Vec<&'v Value>) {
    match rhs.unspan() {
        Value::If(_, t, e) => {
            handle_leaves(t, out);
            handle_leaves(e, out);
        }
        Value::Block(bl) => {
            if let Some(tail) = bl.operators.last() {
                handle_leaves(tail, out);
            }
        }
        Value::Insert(ops) => {
            if let Some(tail) = ops.last() {
                handle_leaves(tail, out);
            }
        }
        Value::Null => {}
        other => out.push(other),
    }
}

/// Does a generator-handle right-hand side deliver a member's handle on some path, and one of
/// its own on some path?  `h = t.g` answers `(true, false)`, `h = steps()` `(false, true)`, and
/// `h = if c { t.g } else { steps() }` both (loft#1585).
pub(crate) fn handle_rhs_kinds(rhs: &Value, data: &Data) -> (bool, bool) {
    let mut leaves = Vec::new();
    handle_leaves(rhs, &mut leaves);
    let views = leaves
        .iter()
        .filter(|l| is_handle_projection(l, data))
        .count();
    (views > 0, views < leaves.len())
}

/// `rhs` with the per-path flag of the handle it is bound to written in every arm that delivers
/// one — `true` where the arm delivers a member's handle, `false` where it delivers one of its
/// own — ahead of the arm's value, so the flag says what the local holds on the path that ran.
pub(super) fn tag_handle_leaves(rhs: &Value, flag: u16, data: &Data) -> Value {
    let tag = |leaf: &Value| v_set(flag, Value::Boolean(is_handle_projection(leaf, data)));
    match rhs {
        Value::Span(b) => Value::Span(Box::new((b.0, tag_handle_leaves(&b.1, flag, data)))),
        Value::If(c, t, e) => Value::If(
            c.clone(),
            Box::new(tag_handle_leaves(t, flag, data)),
            Box::new(tag_handle_leaves(e, flag, data)),
        ),
        Value::Block(bl) if !bl.operators.is_empty() => {
            let mut bl = bl.clone();
            let last = bl.operators.len() - 1;
            let tail = bl.operators[last].clone();
            if matches!(
                tail.unspan(),
                Value::If(..) | Value::Block(..) | Value::Insert(..)
            ) {
                bl.operators[last] = tag_handle_leaves(&tail, flag, data);
            } else if !matches!(tail.unspan(), Value::Null) {
                bl.operators.insert(last, tag(&tail));
            }
            Value::Block(bl)
        }
        Value::Insert(ops) if !ops.is_empty() => {
            let mut ops = ops.clone();
            let last = ops.len() - 1;
            let tail = ops[last].clone();
            ops[last] = tag_handle_leaves(&tail, flag, data);
            Value::Insert(ops)
        }
        Value::Null => Value::Null,
        leaf => Value::Insert(vec![tag(leaf), leaf.clone()]),
    }
}

/// [`Scopes::owned_handle_members`], read off every assignment of a tuple local and of its
/// members.  An assignment that is not a fresh handle makes the member a view for good — a
/// leak where one path did own it, never a release of a frame somebody else holds.
pub(super) fn owned_handle_members(
    code: &Value,
    function: &Function,
    data: &Data,
) -> HashSet<(u16, u16)> {
    fn fresh(rhs: &Value, idx: usize, data: &Data) -> bool {
        match rhs.unspan() {
            // A tuple a call answers (a generator's advance included) is the caller's whole.
            Value::Call(d, _) => {
                let name = data.def(*d).name();
                !name.starts_with("Op") || name == "OpCoroutineNext"
            }
            Value::Tuple(items) => items.get(idx).is_some_and(
                |m| matches!(m.unspan(), Value::Call(d, _) if data.def(*d).name() != "OpGetDbRef"),
            ),
            Value::Block(bl) => bl.operators.last().is_some_and(|t| fresh(t, idx, data)),
            Value::Insert(ops) => ops.last().is_some_and(|t| fresh(t, idx, data)),
            _ => false,
        }
    }
    let mut verdict: BTreeMap<(u16, u16), bool> = BTreeMap::new();
    code.walk(&mut |n| {
        let (v, rhs, only) = match n.unspan() {
            Value::Set(v, rhs) => (*v, rhs.as_ref(), None),
            Value::TuplePut(v, idx, rhs) => (*v, rhs.as_ref(), Some(*idx as usize)),
            _ => return,
        };
        let Type::Tuple(elems) = function.tp(v).base() else {
            return;
        };
        for (i, t) in elems.iter().enumerate() {
            if !matches!(t.base(), Type::Iterator(_, _)) || only.is_some_and(|o| o != i) {
                continue;
            }
            let ok = match only {
                Some(_) => matches!(rhs.unspan(), Value::Call(d, _)
                    if data.def(*d).name() != "OpGetDbRef"),
                None => fresh(rhs, i, data),
            };
            let e = verdict.entry((v, i as u16)).or_insert(true);
            *e &= ok;
        }
    });
    verdict
        .into_iter()
        .filter_map(|(k, owned)| owned.then_some(k))
        .collect()
}

/// The generator locals assigned both a member's handle and a value of their own
/// ([`Scopes::handle_views`]), on two assignments or on two arms of one.  Sorted, so their
/// flags are minted in one order on every compile.
pub(super) fn mixed_handle_views(code: &Value, function: &Function, data: &Data) -> Vec<u16> {
    let mut views: BTreeSet<u16> = BTreeSet::new();
    let mut owned: BTreeSet<u16> = BTreeSet::new();
    code.walk(&mut |n| {
        if let Value::Set(v, rhs) = n.unspan()
            && matches!(function.tp(*v).base(), Type::Iterator(_, _))
            && function.is_skip_free(*v)
        {
            let (view, own) = handle_rhs_kinds(rhs, data);
            if view {
                views.insert(*v);
            }
            if own {
                owned.insert(*v);
            }
        }
    });
    views.intersection(&owned).copied().collect()
}

/// `(G-Hold)`, loft#1708 — a generator handle that lands in a SECOND holder while its source
/// keeps its own hold takes a hold on the frame (`OpCoroutineRetain`), so each holder releases
/// once and the frame dies with the last of them.
///
/// The holders that release are a field or an element (their cascade), a tuple member and a
/// local (the scope sweep), and a generator frame (its end, or its abandonment).  A source
/// KEEPS its hold when it is a parameter (the caller's, or a generator frame's), a view
/// (`skip_free`), or a member read.  A fresh call hands its one hold over, and an owned local
/// placed in a field MOVES there (`(H-Move)`, `handle_handoff`): neither is retained.  Before
/// this, the source released the frame while the new holder still named it, and the holder
/// answered null — `g = gen(); wrap(g)` returned from a function, `h = g` of a parameter.
pub(super) fn retain_shared_handles(
    d_nr: u32,
    code: &mut Value,
    vars: &Function,
    data: &Data,
    database: &crate::database::Stores,
) {
    let retain = data.def_nr("OpCoroutineRetain");
    if retain == u32::MAX {
        return;
    }
    let cx = RetainCx {
        retain,
        set_dbref: data.def_nr("OpSetDbRef"),
        get_dbref: data.def_nr("OpGetDbRef"),
        vars,
        data,
        database,
    };
    cx.rewrite(code);
    if matches!(data.def(d_nr).returned.base(), Type::Tuple(_)) {
        cx.rewrite_tuple_members(code);
    }
}

struct RetainCx<'a> {
    retain: u32,
    set_dbref: u32,
    get_dbref: u32,
    vars: &'a Function,
    data: &'a Data,
    database: &'a crate::database::Stores,
}

impl RetainCx<'_> {
    fn is_handle_var(&self, x: u16) -> bool {
        matches!(self.vars.tp(x).base(), Type::Iterator(_, _))
    }

    /// Is `val` a handle whose source keeps its own hold?
    fn kept(&self, val: &Value) -> bool {
        match val.unspan() {
            Value::Var(x) => {
                self.is_handle_var(*x) && (self.vars.is_argument(*x) || self.vars.is_skip_free(*x))
            }
            Value::TupleGet(x, i) => matches!(
                self.vars.tp(*x).base(),
                Type::Tuple(m) if m.get(*i as usize).is_some_and(|t| matches!(t.base(), Type::Iterator(_, _)))
            ),
            Value::Call(d, args) if *d == self.get_dbref => self.handle_field(args),
            _ => false,
        }
    }

    /// Does `OpGetDbRef(rec, off)` read a generator-handle field?
    fn handle_field(&self, args: &[Value]) -> bool {
        let (Some(Value::Var(r)), Some(Value::Int(off))) = (
            args.first().map(Value::unspan),
            args.get(1).map(Value::unspan),
        ) else {
            return false;
        };
        let Some(d_nr) = self.vars.tp(*r).base().heap_def_nr() else {
            return false;
        };
        let kt = self.data.def(d_nr).known_type();
        self.data.def(d_nr).attributes().iter().any(|a| {
            matches!(a.typedef.base(), Type::Iterator(_, _))
                && i64::from(self.database.position(kt, &a.name)) == i64::from(*off)
        })
    }

    fn wrap(&self, v: &mut Value) {
        let inner = std::mem::replace(v, Value::Null);
        *v = Value::Call(self.retain, vec![inner]);
    }

    fn is_generator(&self, d: u32) -> bool {
        let def = self.data.def(d);
        def.is_loft_defined()
            && def.code != Value::Null
            && matches!(def.returned.base(), Type::Iterator(_, _))
    }

    fn rewrite_tuple_members(&self, v: &mut Value) {
        match v {
            Value::Span(b) => self.rewrite_tuple_members(&mut b.1),
            // A block's value is its last operator — a function body's tail is its return.
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last_mut() {
                    self.rewrite_tuple_members(last);
                }
            }
            Value::Tuple(members) => {
                for m in members.iter_mut() {
                    if self.kept(m) {
                        self.wrap(m);
                    }
                }
            }
            _ => {}
        }
    }

    fn rewrite(&self, v: &mut Value) {
        v.for_each_child_mut(&mut |c| self.rewrite(c));
        match v {
            Value::Call(d, args) if *d == self.set_dbref && args.len() == 3 => {
                if self.kept(&args[2]) {
                    self.wrap(&mut args[2]);
                }
            }
            // A generator HOLDS every handle it is handed, to its end or its abandonment.
            Value::Call(d, args) if self.is_generator(*d) => {
                for a in args.iter_mut() {
                    let handle = match a.unspan() {
                        Value::Var(x) => self.is_handle_var(*x),
                        _ => self.kept(a),
                    };
                    if handle {
                        self.wrap(a);
                    }
                }
            }
            // A LOCAL that will release (not a view, `skip_free`) and is bound from a parameter or
            // a view VARIABLE.  A member read into such a local is the parser's move out of the
            // member — a destructuring binder of a temp — and the local's own flag already says
            // it owns; retaining there leaked the frame.
            Value::Set(x, val) => {
                if self.is_handle_var(*x)
                    && !self.vars.is_argument(*x)
                    && !self.vars.is_skip_free(*x)
                    && matches!(val.unspan(), Value::Var(_))
                    && self.kept(val)
                {
                    self.wrap(val);
                } else if matches!(self.vars.tp(*x).base(), Type::Tuple(_)) {
                    self.rewrite_tuple_members(val);
                }
            }
            Value::Return(val) => self.rewrite_tuple_members(val),
            _ => {}
        }
    }
}

impl Scopes<'_> {
    /// The frees of the generator-handle members tuple `v` owns ([`Self::owned_handle_members`]),
    /// in reverse index order as its other members'.
    pub(super) fn tuple_handle_frees(
        &self,
        v: u16,
        function: &Function,
        data: &Data,
    ) -> Vec<Value> {
        let Type::Tuple(elems) = function.tp(v).base() else {
            return Vec::new();
        };
        if function.is_skip_free(v) {
            return Vec::new();
        }
        (0..elems.len())
            .rev()
            .filter(|&i| self.owned_handle_members.contains(&(v, i as u16)))
            .map(|i| Value::Call(data.def_nr("OpFreeRef"), vec![Value::TupleGet(v, i as u16)]))
            .collect()
    }
}
