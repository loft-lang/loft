// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! The value-record LEAVES: where each field of a returned record comes from (a fresh
//! record minted in the body, a view into an argument, a constant), the plan that reads a
//! view leaf, and the presence checks a nullable result needs at its uses.

use crate::data::{Block, Data, Type, Value};
use crate::database::Stores;
use crate::opt::body_writes::sub_record_of;
use crate::opt::loop_records::{IN_PLACE_SET_OPS, SCALAR_GETTERS};
use crate::opt::value_records::layout::{
    LeafSource, TupleParams, ViewPlan, elem_mint, fnref_value_on, is_nullable_layout, is_view_part,
    leaf_root, nullable_layout,
};
use crate::opt::value_records::sites::{
    ShapeCtx, VIEW_LEAF_READ_OPS, ValueLeaves, appends_into, buffer_uses_accounted, fnref_param,
    mv_return_source, namings_avoid_place, own_retbuf,
};
use crate::opt::writes::plain_record_type;
use std::collections::{BTreeSet, HashMap, HashSet};

/// `(R-FnRefValue)` — is a call through a value of `fn_type` a listed site?  Its signature
/// returns a plain record or takes one ([`plain_record_type`] — an optional one keeps the
/// buffer, whose null the tuple does not spell), no arm captures (a closure's environment
/// rides beside the records it builds) and no `&text` argument picks an arm's store instance
/// (the instance is another definition the group does not list).  `false` keeps the site on
/// the buffer road; the caller declines its arms outright.
pub(crate) fn fnref_site_listed(
    data: &Data,
    fn_type: &Type,
    args: &[Value],
    arms: &[crate::fnref::Arm],
) -> bool {
    if !fnref_value_on() || arms.is_empty() || arms.iter().any(|a| a.has_closure) {
        return false;
    }
    let Type::Function(params, ret, ..) = fn_type.base() else {
        return false;
    };
    data.store_text_mask(params, args) == 0
        && (plain_record_type(data, ret).is_some()
            || params.iter().any(|p| plain_record_type(data, p).is_some()))
}

/// @PLN164 C5 — the source a view-leaf field is delivered from at ONE `Object` exit.  ONE
/// home: the gate calls it once per exit and field and keeps the answer in the plan the
/// emitter reads.
///
/// The record form writes such a field with `OpAppendVector(OpGetField(buffer, off, _),
/// src, _)` — the deep copy the leaf removes.  The one resolvable source is the natural
/// form: a LOCAL whose own copy landed in an element appended to a parameter's container,
/// `p = mk_pts(n); sc.ops += [Op { opts: p }]; Mark { …, mpts: p }` — the leaf hands over
/// that element's field, because the same value already lives there in a store the frame
/// does not own.  Whether it does on EVERY path to this exit is [`fresh_leaf`]'s question.
///
/// A source that is itself a parameter-rooted place (`Mark { mpts: sc.first }`) is not
/// resolved: no shape of it reached admission under the per-exit test that stood here, and
/// the record form costs it nothing but the rewrite.
///
/// `None` declines the function.  It is the answer for every shape not named here: a source
/// that is not a local, a local with two destinations outside the frame, a field written by
/// something other than one append, a path on which the element is not the local's copy.
pub fn leaf_source(
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    bl: &Block,
    off: i64,
    exit_bufs: &HashSet<u16>,
    disturbed: Option<&crate::scopes::DisturbedParams>,
) -> Option<LeafSource> {
    let buf = *bl.result.depend().first()?;
    // The append and its source: the append NODE is what the value form drops whole (its
    // destination names the buffer's field, which is the place the leaf takes), and the
    // source is what the leaf is resolved from.
    let mut found: Option<(&Value, &Value)> = None;
    for op in &bl.operators {
        for pair in appends_into(data, op, buf, off) {
            if found.is_some() {
                return None;
            }
            found = Some(pair);
        }
    }
    // Every mention of the BUFFER in this exit must be one the value form accounts for, and
    // that is what makes both answers positive rather than a fallback: the value form drops
    // the whole block, so a mention it does not account for is work the tuple would LOSE.
    // Accounted: the allocate-or-reuse guard, a scalar `OpSet*`, the one recognised append
    // into the leaf's field, and the block's own yield of the buffer.
    //
    // The two spellings this test exists for both FILL the field without an append: a vector
    // LITERAL returned through the buffer pushes element by element
    // (`OpPushInt(OpGetField(buf, off, _), …)`), and a record literal's vector field appends
    // ELEMENTS through the record (`OpNewRecord(buf, <record>, <field nr>)`).  Read as
    // "nothing wrote the field", each delivered a NULL view for a vector the program had
    // filled — the first measured on the corpus (`723-ncc-loop-element-bind`, `len` 0 for 8),
    // the second on this unit's own `d6` cell.
    if !buffer_uses_accounted(data, bl, buf, found.map(|(node, _)| node)) {
        return None;
    }
    let Some((copy, src)) = found else {
        return Some(LeafSource::Null);
    };
    let Value::Var(local) = src.unspan() else {
        return None;
    };
    fresh_leaf(
        &FreshQuery {
            data,
            stores,
            def_nr,
            local: *local,
            exit: std::ptr::from_ref(bl) as usize,
            exit_copy: std::ptr::from_ref(copy.unspan()) as usize,
            exit_bufs,
            disturbed,
        },
        bl,
    )
}

/// The inputs of one [`fresh_leaf`] question.
pub(crate) struct FreshQuery<'a> {
    pub(crate) data: &'a Data,
    pub(crate) stores: &'a Stores,
    pub(crate) def_nr: u32,
    /// The local the exit copies into its view-leaf field.
    pub(crate) local: u16,
    /// The exit `Object` block, by address.
    pub(crate) exit: usize,
    /// The exit's own copy of the local, by address — the one the view replaces.
    pub(crate) exit_copy: usize,
    /// The return buffers of every exit of this record: a copy into one of them is an exit's
    /// own, never a destination the view could name.
    pub(crate) exit_bufs: &'a HashSet<u16>,
    pub(crate) disturbed: Option<&'a crate::scopes::DisturbedParams>,
}

/// @PLN164 E-1 (`@FR-O-ViewField`) — does the local's copy already live in a parameter's
/// container on EVERY path to this exit, and which expression names it there?
///
/// The record form answers the local's value AT THE EXIT.  A view answers the value of an
/// element appended earlier.  The two agree exactly when, on every path from the entry to
/// the exit, the last change to that container was the append of an element whose field
/// took a copy of the local, and neither the local, nor that element's field, nor the
/// container's order changed after the copy.  That is a per-PATH fact, and the walk
/// ([`FreshWalk`]) proves it over the structured IR: an `if` joins its arms, a loop runs to
/// a fixpoint, `break`/`continue`/`return` carry their state to where they go.
///
/// The candidates are fixed first, over the whole body: every copy of the local except the
/// exits' own must go into a field (the same offset each time) of an element minted into
/// ONE parameter place, or into a place the frame owns (a read of the local, and nothing
/// the view could name).  A second place outside the frame declines (`(O-ViewField)`: which
/// place the view names is not decided).
///
/// The local may view only stores the FRAME owns — a call's hidden buffer, the backing record
/// of a `[]` local — because a write to what it views changes the record form's answer and not
/// the element's: naming any of those stores counts as naming the local, and a parameter's
/// store is declined outright, since a caller may hand it in under a second name the walk
/// cannot see.  A variable whose type depends on the local is a view of it, and naming one
/// counts too.
///
/// The answer: one element temp on every path → [`LeafSource::Elem`]; several (one per arm)
/// → [`LeafSource::Last`]; a path where the fact does not hold → `None`, which declines the
/// function and costs the rewrite, never a value.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub(crate) fn fresh_leaf(q: &FreshQuery, exit_bl: &Block) -> Option<LeafSource> {
    let data = q.data;
    let def = data.def(q.def_nr);
    let vars = def.variables();
    let body = def.code();
    let why = |w: &str| {
        if std::env::var("LOFT_TRACE_VALUEREC").is_ok() {
            eprintln!("[viewleaf] {}: `{}` {w}", def.name(), vars.name(q.local));
        }
    };
    // The stores the local VIEWS — its deps, closed over theirs.  The local's value changes
    // whenever one of them is written, so a naming of any of them counts as a naming of the
    // local; and none may be a parameter, whose store the frame cannot watch (a caller may
    // hand the same store in twice, and a write through the other name would change the
    // local where this walk sees nothing).
    let own = crate::use_analysis::ownership_of(data, q.def_nr, &Value::Var(q.local));
    let mut aliases: HashSet<u16> = HashSet::new();
    let mut stack = vec![q.local];
    while let Some(v) = stack.pop() {
        for d in vars.tp(v).depend() {
            if d != q.local && d < vars.count() && aliases.insert(d) {
                stack.push(d);
            }
        }
    }
    let base_ok = match own {
        crate::use_analysis::Own::Owned => true,
        crate::use_analysis::Own::Borrowed { base } | crate::use_analysis::Own::Join { base } => {
            base == u16::MAX || !vars.is_argument(base)
        }
        crate::use_analysis::Own::Unknown => false,
    };
    if vars.is_argument(q.local) || !base_ok || aliases.iter().any(|a| vars.is_argument(*a)) {
        why(&format!("views a store the frame does not own ({own:?})"));
        return None;
    }
    // And the views OF any of those, closed: naming one names the local too.
    loop {
        let before = aliases.len();
        for w in 0..vars.count() {
            if w != q.local
                && !aliases.contains(&w)
                && vars
                    .tp(w)
                    .depend()
                    .iter()
                    .any(|d| *d == q.local || aliases.contains(d))
            {
                aliases.insert(w);
            }
        }
        if aliases.len() == before {
            break;
        }
    }
    // The candidates: every copy of the local that is not an exit's own.
    let mut cands: HashSet<u16> = HashSet::new();
    let mut copies: HashSet<usize> = HashSet::new();
    let mut shape: Option<(crate::scopes::ParamPlace, i32, i32, i64, i32)> = None;
    let mut ok = true;
    body.any_node(&mut |n| {
        let Value::Call(d, args) = n else {
            return false;
        };
        if (*d as usize) >= data.definitions.len()
            || data.def(*d).name() != "OpAppendVector"
            || std::ptr::from_ref(n) as usize == q.exit_copy
        {
            return false;
        }
        let [dst, src, ..] = &args[..] else {
            return false;
        };
        if !matches!(src.unspan(), Value::Var(w) if *w == q.local) {
            return false;
        }
        let Value::Call(g, gargs) = dst.unspan() else {
            ok &= leaf_root(data, q.stores, q.def_nr, dst, 0).is_none();
            return !ok;
        };
        if (*g as usize) < data.definitions.len()
            && data.def(*g).name() == "OpGetField"
            && let (Some(Value::Var(e)), Some(Value::Int(o)), Some(Value::Int(t))) = (
                gargs.first().map(Value::unspan),
                gargs.get(1).map(Value::unspan),
                gargs.get(2).map(Value::unspan),
            )
        {
            if q.exit_bufs.contains(e) {
                return false;
            }
            if let Some((place, ptp, fld)) = elem_mint(data, q.stores, q.def_nr, *e) {
                let this = (place, ptp, fld, i64::from(*o), *t);
                if shape.is_some_and(|s| s != this) {
                    ok = false;
                    return true;
                }
                shape = Some(this);
                cands.insert(*e);
                copies.insert(std::ptr::from_ref(n) as usize);
                return false;
            }
        }
        // A copy into anything else: a place the frame owns is a READ of the local; a second
        // place outside the frame leaves the view's place undecided.
        ok &= leaf_root(data, q.stores, q.def_nr, dst, 0).is_none();
        !ok
    });
    let Some((place, parent_tp, fld, off, tp)) = shape else {
        why("is copied into no element appended to a parameter");
        return None;
    };
    if !ok {
        why("is copied into a second place outside the frame");
        return None;
    }
    let mut walk = FreshWalk {
        q,
        aliases,
        cands,
        copies,
        place,
        fld,
        off,
        at_exit: None,
        loops: Vec::new(),
        ok: true,
    };
    // The exit's scalar writes into its buffer are the tuple's other parts, evaluated beside
    // the leaf: they may READ the local and nothing more.  Every other statement of the exit
    // is the build the tuple replaces, or a release that runs after the tuple is formed.
    for op in &exit_bl.operators {
        if let Value::Call(d, args) = op.unspan()
            && walk.op_name(*d).starts_with("OpSet")
            && matches!(args.first().map(Value::unspan), Some(Value::Var(b)) if q.exit_bufs.contains(b))
            && !args.iter().skip(1).all(|a| walk.only_reads(a))
        {
            why("is changed by the exit's own tuple");
            return None;
        }
    }
    walk.walk(
        body,
        Fresh {
            live: true,
            last: None,
            copied: BTreeSet::new(),
        },
    );
    let Some(at) = walk.at_exit.take() else {
        why("reaches an exit the walk never visits");
        return None;
    };
    if !walk.ok || !at.live {
        why("stands in control flow the walk does not model");
        return None;
    }
    let Some(last) = at.last else {
        why("is not the container's last element on every path to the exit");
        return None;
    };
    match last.len() {
        0 => None,
        1 => Some(LeafSource::Elem {
            elem: *last.iter().next()?,
            off,
            tp,
        }),
        _ => Some(LeafSource::Last {
            root: place,
            parent_tp,
            fld,
            off,
        }),
    }
}

/// One path state of [`FreshWalk`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Fresh {
    /// Some path reaches this point.  An unreachable point is the identity of the join.
    pub(crate) live: bool,
    /// On every path reaching here, the container's last element is one of these candidate
    /// elements, finished, with its viewed field still the local's copy; `None` where some
    /// path cannot say so.
    pub(crate) last: Option<BTreeSet<u16>>,
    /// The candidate elements whose viewed field holds the local's CURRENT value on every
    /// path: the copy ran, and neither the local nor that field changed since.
    pub(crate) copied: BTreeSet<u16>,
}

impl Fresh {
    pub(crate) fn dead() -> Self {
        Fresh {
            live: false,
            last: None,
            copied: BTreeSet::new(),
        }
    }

    /// Both paths: the last element is one of EITHER path's set, and a field is the local's
    /// copy only where both paths say so.
    pub(crate) fn join(self, o: Fresh) -> Fresh {
        if !self.live {
            return o;
        }
        if !o.live {
            return self;
        }
        Fresh {
            live: true,
            last: match (self.last, o.last) {
                (Some(a), Some(b)) => Some(a.union(&b).copied().collect()),
                _ => None,
            },
            copied: self.copied.intersection(&o.copied).copied().collect(),
        }
    }
}

/// The walk behind [`fresh_leaf`]: a forward must-analysis over loft's STRUCTURED IR.
///
/// An atomic statement is one event ([`FreshWalk::event`]).  Its answer is an UPPER bound
/// on change, in the direction a miss would cost meaning: a statement that names the local
/// anywhere but a read position kills the fact, and so does one that names the container
/// in a way [`namings_avoid_place`] cannot show moves nothing.  The only statements that
/// ESTABLISH the fact are the exact spellings of the candidate's copy and of its finish.
///
/// A straight-line node that carries control flow inside it (a jump, a loop, the exit) is
/// walked child by child, and it may not name anything the fact watches: its own effect
/// would stand after its operands', and this walk does not model that order — so such a node
/// declines rather than being guessed.
pub(crate) struct FreshWalk<'a> {
    pub(crate) q: &'a FreshQuery<'a>,
    pub(crate) aliases: HashSet<u16>,
    pub(crate) cands: HashSet<u16>,
    /// The candidates' copies of the local, by address.
    pub(crate) copies: HashSet<usize>,
    pub(crate) place: crate::scopes::ParamPlace,
    /// The container's field NUMBER, as the mint and the finish both name it.
    pub(crate) fld: i32,
    /// The viewed field's byte offset inside the element.
    pub(crate) off: i64,
    pub(crate) at_exit: Option<Fresh>,
    /// Per enclosing loop, innermost last: the states its `break`s and `continue`s carry.
    pub(crate) loops: Vec<(Fresh, Fresh)>,
    pub(crate) ok: bool,
}

impl FreshWalk<'_> {
    pub(crate) fn op_name(&self, d: u32) -> &str {
        if (d as usize) < self.q.data.definitions.len() {
            self.q.data.def(d).name()
        } else {
            ""
        }
    }

    pub(crate) fn is_local(&self, w: u16) -> bool {
        w == self.q.local || self.aliases.contains(&w)
    }

    pub(crate) fn names_local(&self, v: &Value) -> bool {
        v.any_node(&mut |n| {
            n.names_var_here(self.q.local) || self.aliases.iter().any(|a| n.names_var_here(*a))
        })
    }

    /// Does `n` name the local only where the value is READ?  Three positions are reads: the
    /// operand of an op that answers a value ([`VIEW_LEAF_READ_OPS`]), the SOURCE of an append
    /// whose destination is not the local, and a `const` argument of a call that answers no
    /// reference — a call that answers one could hand back a place inside the local, which
    /// the caller may then write.  Anything else is treated as a change.
    pub(crate) fn only_reads(&self, n: &Value) -> bool {
        match n.unspan() {
            Value::Call(d, args) => args.iter().enumerate().all(|(i, a)| {
                if matches!(a.unspan(), Value::Var(w) if self.is_local(*w)) {
                    self.read_position(*d, i, args)
                } else {
                    self.only_reads(a)
                }
            }),
            x if self.names_local_here(x) => false,
            x => {
                let mut ok = true;
                x.for_each_child(&mut |c| ok &= self.only_reads(c));
                ok
            }
        }
    }

    pub(crate) fn names_local_here(&self, n: &Value) -> bool {
        n.names_var_here(self.q.local) || self.aliases.iter().any(|a| n.names_var_here(*a))
    }

    pub(crate) fn read_position(&self, d: u32, i: usize, args: &[Value]) -> bool {
        let data = self.q.data;
        let name = self.op_name(d);
        if VIEW_LEAF_READ_OPS.contains(&name) {
            return i == 0;
        }
        if name == "OpAppendVector" {
            return i == 1 && !args.first().is_some_and(|a| self.names_local(a));
        }
        if name.is_empty() {
            return false;
        }
        let def = data.def(d);
        def.attributes().get(i).is_some_and(|a| a.value_const)
            && (crate::data::is_scalar(def.returned())
                || matches!(def.returned().base(), Type::Void | Type::Text(_)))
    }

    /// Does `n` touch element `e` only through fields other than the viewed one?  A
    /// projection (`OpGetField`) and a fixed-width scalar read or write
    /// ([`IN_PLACE_SET_OPS`], [`SCALAR_GETTERS`]) carry the field's byte offset as their
    /// second operand and reach nothing outside it.  Every other use of the temp — an element
    /// append through it, a hand-off — may reach the viewed field.
    pub(crate) fn elem_use_elsewhere(&self, n: &Value, e: u16) -> bool {
        match n.unspan() {
            Value::Call(d, args) if matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == e) =>
            {
                let name = self.op_name(*d);
                let field_op = name == "OpGetField"
                    || IN_PLACE_SET_OPS.contains(&name)
                    || SCALAR_GETTERS.contains(&name);
                field_op
                    && matches!(args.get(1).map(Value::unspan), Some(Value::Int(o)) if i64::from(*o) != self.off)
                    && args.iter().skip(1).all(|a| self.elem_use_elsewhere(a, e))
            }
            x if x.names_var_here(e) => false,
            x => {
                let mut ok = true;
                x.for_each_child(&mut |c| ok &= self.elem_use_elsewhere(c, e));
                ok
            }
        }
    }

    /// `OpFinishRecord(<root>, Var(e), _, fld)` into the viewed container: the element `e`
    /// becomes the container's last.
    pub(crate) fn finish_of(&self, n: &Value) -> Option<u16> {
        let Value::Call(d, args) = n else {
            return None;
        };
        if self.op_name(*d) != "OpFinishRecord" {
            return None;
        }
        match (
            args.first().map(Value::unspan),
            args.get(1).map(Value::unspan),
            args.get(3).map(Value::unspan),
        ) {
            (Some(Value::Var(root)), Some(Value::Var(elem)), Some(Value::Int(fld)))
                if *root == self.place.0 && *fld == self.fld && self.cands.contains(elem) =>
            {
                Some(*elem)
            }
            _ => None,
        }
    }

    /// One atomic statement.
    pub(crate) fn event(&mut self, v: &Value, mut st: Fresh) -> Fresh {
        let node = v.unspan();
        let root = self.place.0;
        if self.copies.contains(&(std::ptr::from_ref(node) as usize)) {
            if let Value::Call(_, args) = node
                && let Some(Value::Call(_, gargs)) = args.first().map(Value::unspan)
                && let Some(Value::Var(e)) = gargs.first().map(Value::unspan)
            {
                st.copied.insert(*e);
            }
            return st;
        }
        if self.names_local(node) && !self.only_reads(node) {
            st.last = None;
            st.copied.clear();
        }
        let finished = self.finish_of(node);
        let named: Vec<u16> = self
            .cands
            .iter()
            .copied()
            .filter(|e| node.reads_var(*e) && Some(*e) != finished)
            .collect();
        for e in named {
            if !self.elem_use_elsewhere(node, e) {
                st.copied.remove(&e);
                if st.last.as_ref().is_some_and(|l| l.contains(&e)) {
                    st.last = None;
                }
            }
        }
        if node.reads_var(root) {
            if let Some(e) = finished {
                st.last = st.copied.contains(&e).then(|| BTreeSet::from([e]));
            } else if !namings_avoid_place(
                self.q.data,
                self.q.stores,
                self.q.def_nr,
                node,
                self.place,
                self.q.disturbed,
            ) {
                st.last = None;
            }
        }
        st
    }

    /// A node that must be walked child by child: it carries a jump, a loop or the exit.
    pub(crate) fn structured(&self, v: &Value) -> bool {
        v.any_node(&mut |n| match n {
            Value::Return(_) | Value::Break(_) | Value::Continue(_) | Value::Loop(_) => true,
            Value::Block(bl) => std::ptr::from_ref(&**bl) as usize == self.q.exit,
            _ => false,
        })
    }

    /// Does `v` name anything the fact is about?
    pub(crate) fn watches(&self, v: &Value) -> bool {
        self.names_local(v)
            || v.reads_var(self.place.0)
            || self.cands.iter().any(|e| v.reads_var(*e))
    }

    pub(crate) fn seq(&mut self, ops: &[Value], mut st: Fresh) -> Fresh {
        for op in ops {
            if !st.live {
                break;
            }
            st = self.walk(op, st);
        }
        st
    }

    pub(crate) fn walk(&mut self, v: &Value, st: Fresh) -> Fresh {
        if !st.live || !self.ok {
            return st;
        }
        match v.unspan() {
            Value::Block(bl) => {
                if std::ptr::from_ref(&**bl) as usize == self.q.exit {
                    let prev = self.at_exit.take().unwrap_or_else(Fresh::dead);
                    self.at_exit = Some(prev.join(st));
                    return Fresh::dead();
                }
                self.seq(&bl.operators, st)
            }
            Value::Insert(ops) => self.seq(ops, st),
            Value::If(cond, then_v, else_v) => {
                let after_cond = self.walk(cond, st);
                let then_end = self.walk(then_v, after_cond.clone());
                let else_end = self.walk(else_v, after_cond);
                then_end.join(else_end)
            }
            Value::Loop(bl) => {
                let mut head = st.clone();
                // The lattice is finite and every round only widens the head, so this ends;
                // the bound is a guard, and running into it declines.
                for _ in 0..64 {
                    self.loops.push((Fresh::dead(), Fresh::dead()));
                    let end = self.seq(&bl.operators, head.clone());
                    let (brk, cont) = self.loops.pop().unwrap_or((Fresh::dead(), Fresh::dead()));
                    let next = st.clone().join(end).join(cont);
                    if next == head {
                        return brk;
                    }
                    head = next;
                }
                self.ok = false;
                Fresh::dead()
            }
            Value::Return(x) => {
                self.walk(x, st);
                Fresh::dead()
            }
            Value::Break(n) | Value::Continue(n) => {
                let brk = matches!(v.unspan(), Value::Break(_));
                let Some(i) = self.loops.len().checked_sub(1 + usize::from(*n)) else {
                    self.ok = false;
                    return Fresh::dead();
                };
                let slot = if brk {
                    &mut self.loops[i].0
                } else {
                    &mut self.loops[i].1
                };
                *slot = std::mem::replace(slot, Fresh::dead()).join(st);
                Fresh::dead()
            }
            Value::Drop(x) => self.walk(x, st),
            Value::Set(w, rhs)
                if self.structured(rhs)
                    || matches!(
                        rhs.unspan(),
                        Value::If(..) | Value::Block(_) | Value::Insert(_)
                    ) =>
            {
                let mut s = self.walk(rhs, st);
                if self.is_local(*w) {
                    s.last = None;
                    s.copied.clear();
                }
                if self.cands.contains(w) {
                    s.copied.remove(w);
                    if s.last.as_ref().is_some_and(|l| l.contains(w)) {
                        s.last = None;
                    }
                }
                s
            }
            other if self.structured(other) => {
                if self.watches(other) {
                    self.ok = false;
                    return Fresh::dead();
                }
                let mut s = st;
                other.for_each_child(&mut |c| {
                    let cur = std::mem::replace(&mut s, Fresh::dead());
                    s = self.walk(c, cur);
                });
                s
            }
            _ => self.event(v, st),
        }
    }
}

pub(crate) fn view_leaf_plan(
    data: &Data,
    stores: &Stores,
    d_nr: u32,
    fields: &[(i64, &'static str)],
    leaves: &ValueLeaves,
    record: u16,
    disturbed: Option<&crate::scopes::DisturbedParams>,
) -> Option<ViewPlan> {
    let mut plan = ViewPlan {
        roots: HashMap::new(),
        leaves: HashMap::new(),
    };
    let views: Vec<i64> = fields
        .iter()
        .filter(|(_, rt)| is_view_part(rt))
        .map(|(off, _)| *off)
        .collect();
    if views.is_empty() {
        return Some(plan);
    }
    let def = data.def(d_nr);
    let body = def.code();
    let trace = std::env::var("LOFT_TRACE_VALUEREC").is_ok();
    // The exits this plan is about: the `Object` builds of this record the value form turns
    // into tuples.
    let mut exits: Vec<&Block> = Vec::new();
    body.any_node(&mut |n| {
        let Value::Block(bl) = n else { return false };
        if bl.name != "Object" {
            return false;
        }
        let leaf = leaves
            .objects
            .contains(&(std::ptr::from_ref(&**bl) as usize));
        if !leaf || plain_record_type(data, &bl.result) != Some(record) {
            if trace {
                eprintln!(
                    "[viewleaf] {}: an Object skipped (leaf={leaf}, tp={:?} want {record})",
                    def.name(),
                    plain_record_type(data, &bl.result)
                );
            }
            return false;
        }
        exits.push(bl);
        false
    });
    let exit_bufs: HashSet<u16> = exits
        .iter()
        .filter_map(|bl| bl.result.depend().first().copied())
        .collect();
    for bl in &exits {
        for off in &views {
            let Some(src) = leaf_source(data, stores, d_nr, bl, *off, &exit_bufs, disturbed) else {
                if trace {
                    eprintln!(
                        "[viewleaf] {}: +{off} has no leaf proven on every path",
                        def.name()
                    );
                }
                return None;
            };
            let entry = plan.roots.entry(*off).or_default();
            if let Some(place) = src.root(data, stores, d_nr) {
                entry.insert(place);
            } else if src != LeafSource::Null {
                return None;
            }
            if trace {
                eprintln!("[viewleaf] {}: +{off} {src:?}", def.name());
            }
            plan.leaves
                .insert((std::ptr::from_ref(*bl) as usize, *off), src);
        }
    }
    if plan.roots.len() != views.len() {
        if trace {
            eprintln!(
                "[viewleaf] {}: {} exit(s), {} of {} view field(s) resolved",
                def.name(),
                exits.len(),
                plan.roots.len(),
                views.len()
            );
        }
        return None;
    }
    Some(plan)
}

/// An `Object` block that RETURNS the record it builds — `{ OpDatabase(p); OpSet*(p, …);
/// frees…; return p }`, `p` the buffer the block's result names.  The scope pass lowers an
/// explicit `return S{…}` this way when the frees the return owes (a live local of another
/// record type) go inside the block.  Answers `p`.  In the value form the block is `return
/// (tuple)`, with those frees between the tuple's evaluation and the return, so a statement
/// that releases a real store still runs, in its order.  ONE home: the gate reads the shape
/// here, the leaf walk records it, the emitter converts by it.
#[must_use]
pub fn object_own_return(bl: &Block) -> Option<u16> {
    if bl.name != "Object" {
        return None;
    }
    let [p] = bl.result.depend()[..] else {
        return None;
    };
    match bl.operators.last()?.unspan() {
        Value::Return(x) if matches!(x.unspan(), Value::Var(w) if *w == p) => Some(p),
        _ => None,
    }
}

/// Is `d` the op a `null` record exit lowers to — `OpNullRefSentinel`, the bare sentinel
/// that names no store?  (`OpConvRefFromNull` is not this: it claims a store.)
pub(crate) fn is_null_record(data: &Data, d: u32) -> bool {
    (d as usize) < data.definitions.len() && data.def(d).name() == "OpNullRefSentinel"
}

pub(crate) fn collect_leaves(
    body: &Value,
    locals: &HashMap<u16, u16>,
    own: bool,
    data: &Data,
) -> ValueLeaves {
    let mut out = ValueLeaves::default();
    fn leaves(v: &Value, locals: &HashMap<u16, u16>, out: &mut ValueLeaves, data: &Data) {
        match v.unspan() {
            Value::Var(w) => {
                out.reads.insert(std::ptr::from_ref(v) as usize);
                out.reads.insert(std::ptr::from_ref(v.unspan()) as usize);
                if !locals.contains_key(w) {
                    out.views.insert(std::ptr::from_ref(v) as usize);
                    out.views.insert(std::ptr::from_ref(v.unspan()) as usize);
                }
            }
            Value::Block(bl) if bl.name == "Object" => {
                out.objects.insert(std::ptr::from_ref(&**bl) as usize);
            }
            Value::Call(d, args) if args.is_empty() && is_null_record(data, *d) => {
                out.nulls.insert(std::ptr::from_ref(v) as usize);
                out.nulls.insert(std::ptr::from_ref(v.unspan()) as usize);
            }
            Value::Block(bl) if mv_return_source(bl, data).is_some() => {
                out.mv_returns.insert(std::ptr::from_ref(&**bl) as usize);
                if let Some(src) = mv_return_source(bl, data) {
                    leaves(src, locals, out, data);
                }
            }
            Value::Block(bl) => {
                if let Some(l) = bl.operators.last() {
                    leaves(l, locals, out, data);
                }
            }
            Value::If(_, a, b) => {
                leaves(a, locals, out, data);
                leaves(b, locals, out, data);
            }
            Value::Insert(ops) => {
                if let Some(l) = ops.last() {
                    leaves(l, locals, out, data);
                }
            }
            Value::Return(x) => leaves(x, locals, out, data),
            _ => {}
        }
    }
    // The walk meets a block before its statements, so an `Object` that returns the
    // record it builds ([`object_own_return`]) is recorded before its own `return` is
    // reached — that return reads the block's buffer, which is no leaf.
    let mut own_returns: HashSet<usize> = HashSet::new();
    body.any_node(&mut |n| {
        match n {
            Value::Block(bl) if own && object_own_return(bl).is_some() => {
                out.objects.insert(std::ptr::from_ref(&**bl) as usize);
                if let Some(last) = bl.operators.last() {
                    own_returns.insert(std::ptr::from_ref(last.unspan()) as usize);
                }
            }
            Value::Block(bl) if own && mv_return_source(bl, data).is_some() => {
                leaves(n, locals, &mut out, data);
                if let Some(last) = bl.operators.last() {
                    own_returns.insert(std::ptr::from_ref(last.unspan()) as usize);
                }
            }
            Value::Return(x) if own && !own_returns.contains(&(std::ptr::from_ref(n) as usize)) => {
                leaves(x, locals, &mut out, data);
            }
            Value::Set(w, rhs) if locals.contains_key(w) => leaves(rhs, locals, &mut out, data),
            _ => {}
        }
        false
    });
    if own {
        leaves(body, locals, &mut out, data);
    }
    out
}

/// The `Var` reads of `body` a tuple serves by itself: every read at a value position
/// ([`collect_leaves`]) and every read whose value is dropped ([`dropped_reads`]).
pub(crate) fn served_reads(
    body: &Value,
    locals: &HashMap<u16, u16>,
    own: bool,
    data: &Data,
) -> HashSet<usize> {
    let mut served = collect_leaves(body, locals, own, data).reads;
    served.extend(dropped_reads(body));
    served
}

/// The `Var` reads in `body` whose value is DROPPED — the tail of a statement block, of a
/// loop body, of a `Drop` — by both addresses.  Such a read of a tuple is served by doing
/// nothing, which is what the emitter's plain read of the local does.  The parser's
/// `return f(…)` chain ends in one when the scope pass leaves its `return` outside the
/// block.  Positional, so a read under a node this walk does not know counts as USED —
/// the conservative side, which only ever costs the optimisation.
pub(crate) fn dropped_reads(body: &Value) -> HashSet<usize> {
    fn walk(v: &Value, used: bool, out: &mut HashSet<usize>) {
        match v.unspan() {
            Value::Var(_) => {
                if !used {
                    out.insert(std::ptr::from_ref(v) as usize);
                    out.insert(std::ptr::from_ref(v.unspan()) as usize);
                }
            }
            Value::Block(bl) => {
                let n = bl.operators.len();
                let tail_used = used && !matches!(bl.result.base(), Type::Void);
                for (i, op) in bl.operators.iter().enumerate() {
                    walk(op, tail_used && i + 1 == n, out);
                }
            }
            Value::Loop(bl) => {
                for op in &bl.operators {
                    walk(op, false, out);
                }
            }
            Value::If(c, a, b) => {
                walk(c, true, out);
                walk(a, used, out);
                walk(b, used, out);
            }
            Value::Insert(ops) => {
                let n = ops.len();
                for (i, op) in ops.iter().enumerate() {
                    walk(op, used && i + 1 == n, out);
                }
            }
            Value::Drop(x) => walk(x, false, out),
            other => other.for_each_child(&mut |ch| walk(ch, true, out)),
        }
    }
    let mut out = HashSet::new();
    walk(body, true, &mut out);
    out
}

/// A presence test of a DENSE compiler temporary (the phantom buffer aside) is never the
/// constant "present" (`@FR-N-Store`): its `null` is a hoisted preamble the release machinery
/// tests where no bind ran — a work ref's displaced record before the call writes the buffer,
/// a `??` lift bound in one arm and released on every path — so such a temporary keeps the
/// record form.  Only the TEST: after its bind a temporary is present, so a copy from it, its
/// result and a join's widening keep [`local_surely_present`].
pub(crate) fn temp_presence_unserved(c: &ShapeCtx, local: u16) -> bool {
    c.data
        .def(c.def_nr)
        .variables()
        .is_compiler_generated(local)
        && own_retbuf(c.data, c.own) != Some(local)
}

/// `OpConvBoolFromRef(local)` / `OpRefIsNull(local)` — the presence test of `local` itself.
pub(crate) fn is_presence_test(name: &str, args: &[Value], local: u16) -> bool {
    matches!(name, "OpConvBoolFromRef" | "OpRefIsNull")
        && matches!(args, [a] if matches!(a.unspan(), Value::Var(w) if *w == local))
}

/// The uses of local `v` a release-side call accounts: its free, a copy from it, and either
/// side of a store-identity test — but not a test whose answer turns on the path
/// ([`identity_turns_on_path`]).  `settled`: the caller already decided the first operand's
/// use (a copy it declines, or a source it served as a leaf read).
pub(crate) fn release_uses<'h>(
    holds: &dyn Fn() -> &'h HoldGraph,
    name: &str,
    args: &[Value],
    v: u16,
    settled: bool,
    moot: &dyn Fn(u16) -> bool,
) -> u32 {
    let arg_is_v =
        |i: usize| matches!(args.get(i).map(Value::unspan), Some(Value::Var(w)) if *w == v);
    let identity = matches!(name, "OpFreeRefIfDistinct" | "OpDistinctStore");
    if identity && identity_turns_on_path(holds(), args) {
        // loft#2014 — unless every other operand is a value local or a converted build:
        // then nothing is released on any path and the constant "distinct" is right.
        let others: Vec<u16> = args
            .iter()
            .filter_map(|a| match a.unspan() {
                Value::Var(w) if *w != v => Some(*w),
                _ => None,
            })
            .collect();
        if others.is_empty() || !others.iter().all(|&w| moot(w)) {
            return 0;
        }
    }
    let first = arg_is_v(0)
        && !settled
        && matches!(
            name,
            "OpFreeRef" | "OpFreeRefIfDistinct" | "OpCopyRecord" | "OpDistinctStore"
        );
    u32::from(first) + u32::from(identity && arg_is_v(1))
}

/// Is local `w` the work-ref an `Object` build the value form converts (`objects`, by the
/// block's address) ends in — the build's own slot, which under the value form is a tuple
/// leaf in no store (loft#2014)?
pub(crate) fn converted_build_ref(body: &Value, objects: &HashSet<usize>, w: u16) -> bool {
    let mut found = false;
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n
            && bl.name == "Object"
            && objects.contains(&(std::ptr::from_ref(&**bl) as usize))
            && matches!(bl.operators.last().map(Value::unspan), Some(Value::Var(x)) if *x == w)
        {
            found = true;
        }
        found
    });
    found
}

/// A store-identity test answers "distinct" for every value local — it is in no store.  Between
/// two locals one of which may hold the other (a join's result and an arm it took) the answer
/// turns on the path, so the test is not served and both keep the record form: the constant
/// would release the record the result hands on.
pub(crate) fn identity_turns_on_path(holds: &HoldGraph, args: &[Value]) -> bool {
    match (
        args.first().map(Value::unspan),
        args.get(1).map(Value::unspan),
    ) {
        (Some(Value::Var(lhs)), Some(Value::Var(rhs))) => {
            holds.may_hold(*lhs, *rhs) || holds.may_hold(*rhs, *lhs)
        }
        _ => false,
    }
}

/// Which locals each local's binds may hand on: for every `Set(w, rhs)` in a body, the locals
/// at the leaves of `rhs` (an `if` arm, a value block's last operand, a return).  Built once
/// per body: asking the body itself per local per test was cubic in a straight-line function
/// of many temporaries (mesh3d's `cube` did not compile in five minutes).
pub(crate) struct HoldGraph(HashMap<u16, Vec<u16>>);

impl HoldGraph {
    pub(crate) fn of(body: &Value) -> Self {
        fn leaves(node: &Value, out: &mut Vec<u16>) {
            match node.unspan() {
                Value::Var(x) => out.push(*x),
                Value::If(_, a, b) => {
                    leaves(a, out);
                    leaves(b, out);
                }
                Value::Block(bl) if !matches!(bl.result.base(), Type::Void) => {
                    if let Some(l) = bl.operators.last() {
                        leaves(l, out);
                    }
                }
                Value::Insert(ops) => {
                    if let Some(l) = ops.last() {
                        leaves(l, out);
                    }
                }
                Value::Return(x) => leaves(x, out),
                _ => {}
            }
        }
        let mut edges: HashMap<u16, Vec<u16>> = HashMap::new();
        body.any_node(&mut |n| {
            if let Value::Set(w, rhs) = n {
                leaves(rhs, edges.entry(*w).or_default());
            }
            false
        });
        HoldGraph(edges)
    }

    /// Can local `holder` hold what local `held` holds — is `held` a leaf of one of
    /// `holder`'s binds, followed through the locals it names?
    pub(crate) fn may_hold(&self, holder: u16, held: u16) -> bool {
        if holder == held {
            return false;
        }
        let mut seen = HashSet::new();
        let mut todo = vec![holder];
        while let Some(v) = todo.pop() {
            if !seen.insert(v) {
                continue;
            }
            for &x in self.0.get(&v).into_iter().flatten() {
                if x == held {
                    return true;
                }
                todo.push(x);
            }
        }
        false
    }
}

/// The uses of local `v` one call accounts that turn on the local's LAYOUT `key`: a hand-off
/// to a `(R-ValueLocal)` TUPLE PARAMETER of callee `d` — the fields cross the call as the
/// scalars they are — and, for a nullable tuple only, a PRESENCE test (`if x`, `x == null`),
/// which reads the presence word (`@FR-R-ValueRecord`'s nullable clause, @C144).
///
/// A whole hand-off is served where the parameter takes the local's own layout, and where a
/// DENSE local meets a nullable parameter of its record and is surely present (`present`,
/// asked only then: the site widens it, `@FR-N-Store`).  A nullable tuple is never handed to
/// a dense parameter.  A SUB-RECORD of a dense local crosses to a dense parameter of the
/// sub-record's type.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handoffs_and_presence(
    data: &Data,
    params: &TupleParams,
    d: u32,
    name: &str,
    args: &[Value],
    v: u16,
    key: Option<u16>,
    present: &dyn Fn() -> bool,
) -> u32 {
    let arg_is_v =
        |i: usize| matches!(args.get(i).map(Value::unspan), Some(Value::Var(w)) if *w == v);
    let nullable = key.is_some_and(is_nullable_layout);
    // A presence test reads the presence word of a nullable tuple, and is the constant
    // "present" for a dense one that is surely so; any other dense tuple cannot answer it.
    let mut uses = u32::from(
        arg_is_v(0)
            && args.len() == 1
            && matches!(name, "OpConvBoolFromRef" | "OpRefIsNull")
            && (nullable || (key.is_some() && present())),
    );
    let Some(ps) = params.get(&d) else {
        return uses;
    };
    for (i, ptp) in ps {
        let whole = arg_is_v(*i)
            && match key {
                Some(k) => {
                    k == *ptp
                        || (!is_nullable_layout(k) && nullable_layout(k) == Some(*ptp) && present())
                }
                None => !is_nullable_layout(*ptp),
            };
        let sub = !nullable && sub_record_of(data, args.get(*i), v) == Some(*ptp);
        uses += u32::from(whole || sub);
    }
    uses
}

/// `(R-FnRefValue)` — how many of a fn-ref call's arguments hand local `v` whole to a
/// parameter every arm of the dispatch receives as a tuple.
pub(crate) fn fnref_handoffs(ctx: &ShapeCtx, local: u16, fref: u16, args: &[Value]) -> u32 {
    let hands = args
        .iter()
        .enumerate()
        .filter(|(idx, arg)| {
            matches!(arg.unspan(), Value::Var(w) if *w == local)
                && fnref_param(ctx.fnref, ctx.params, ctx.def_nr, fref, *idx).is_some()
        })
        .count();
    u32::try_from(hands).unwrap_or(u32::MAX)
}

/// The scalar setters a value local's field write may lower to (`(R-ValueLocal)`): the
/// ones `FusedElementWriteEmitter` is registered for, which carries the tuple arm.
pub const VALUE_RECORD_SETTERS: [&str; 3] = ["OpSetInt", "OpSetFloat", "OpSetSingle"];

pub const VALUE_RECORD_GETTERS: [&str; 6] = [
    "OpGetFloat",
    "OpGetInt",
    "OpGetSingle",
    "OpGetBoolean",
    "OpGetByte",
    "OpGetShort",
];
