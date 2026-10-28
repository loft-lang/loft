// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-DeferredCopy` (C): a record local bound by a COPY that the function only reads
//! takes the copy where it could first be observed instead of at the bind.
//!
//! `prog = callee_prog` copies the record under `(B-Copy)`: a later write to the source must
//! not reach `prog`.  A copy can only be told from a view once something writes the store the
//! source lives in, and every such write is a statement or call the compiler sees.  So the bind
//! takes the source's reference and raises the local's flag (`loft_dcv<x>`), and before each
//! statement that may write a store not proven apart from the source's
//! (`StoreFacts::distinct`) the copy is taken if the flag is still up — the values are the
//! copy's, since nothing changed the source between the bind and that statement.  Where every
//! deferred bind's source is an element view `V[e]` and the statement writes a path rooted at
//! `V[k]`, the guard also asks whether `V[k]` is the element the local views
//! (`loft_dc_is_elem`); a write rooted at another element changes nothing the local reads.  An
//! interpreter's `prog = callee_prog` then costs one copy per lowering, not one per call.
//! Design: `bench/portal/analysis/deferred-copy.md`.
//!
//! The local must hold an owner witness (`__own_<x>`, `@FR-O-Witness`): it marks whether the
//! local owns a copy, so a deferred view is never released.  Every mention of the local, and of
//! each reference variable derived from it (`op = prog.ops[pc]`), must be a read; a derived
//! view is moved into the copy when it is taken (`loft_dc_remap`), as is any address held for
//! one of them (`@FR-R-RecPtr`) — only the views read after the point (`lease::Frame::read_after`),
//! and no point is emitted where the local and its views are not read again.  A text borrowed
//! from the local declines it.
//! `LOFT_NO_DEFERRED_COPY=1` copies at every bind again; `LOFT_TRACE_DEFERRED_COPY=1` names each
//! admission, decline and copy point, and why each view is or is not moved.
use super::FnWriter;
use crate::data::{Type, Value};
use std::collections::{HashMap, HashSet};

/// One deferred local.
#[derive(Clone, Debug)]
pub(crate) struct DcVar {
    /// Its owner witness.
    pub own: u16,
    /// The reference variables derived from it (their deps reach it).
    pub derived: Vec<u16>,
    /// Its record type.
    pub tp: u16,
    /// The variables its deferred binds' sources' stores end at (`StoreFacts::terminal`).
    pub sources: Vec<u16>,
    /// The deferred binds: their `Set` nodes, by address.
    pub binds: HashSet<usize>,
    /// The statements after which the local and its views are dead (`dead_after`): a write
    /// there needs no copy, since nothing reads what it would have disturbed.
    pub dead: HashSet<usize>,
    /// `(V, element size)` when every deferred bind's source is a whole element of the vector
    /// variable `V` (`elem_source`): the local's reference is then that element's, and a write
    /// rooted at `V[k]` is guarded by comparing the two elements.
    pub elem: Option<(u16, i64)>,
}

/// What a statement asks of a deferred local before it runs.
enum Point {
    /// It writes nothing the local's source could be in.
    Clear,
    /// Its only unproven writes are rooted at `V[k]` for these `k` (`DcVar::elem`'s `V`).
    Index(Vec<Value>),
    /// It may write the source some other way.
    Flag,
}

/// The function's deferred locals.
#[derive(Clone, Debug, Default)]
pub(crate) struct DeferredCopies {
    pub vars: HashMap<u16, DcVar>,
    /// `call_writes_store`'s memo, shared by every copy-point test of the function.
    pub writes: std::rc::Rc<std::cell::RefCell<HashMap<u32, bool>>>,
    /// The reference variables that hold the null reference at every mention (`null_only`):
    /// a callee handed one names no store that exists, so whatever it writes through it is new.
    pub null_only: HashSet<u16>,
}

/// An operator that only READS through its first operand.
fn read_op(name: &str) -> bool {
    crate::opt::loop_records::SCALAR_GETTERS.contains(&name)
        || matches!(
            name,
            "OpGetField"
                | "OpGetVector"
                | "OpGetVectorNullable"
                | "OpLengthVector"
                | "OpVectorIsNull"
                | "OpGetText"
                | "OpConvBoolFromRef"
                | "OpRefIsNull"
        )
}

/// `rhs` names `x` itself — bare, or through `OpRefAlias`, as the owner witness takes it.
fn names_var(data: &crate::data::Data, rhs: &Value, x: u16) -> bool {
    match rhs.unspan() {
        Value::Var(w) => *w == x,
        Value::Call(op, args) if (*op as usize) < data.definitions.len() => {
            data.def(*op).name() == "OpRefAlias"
                && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == x)
        }
        _ => false,
    }
}

fn node_addr(v: &Value) -> usize {
    std::ptr::from_ref(v.unspan()) as usize
}

impl FnWriter<'_> {
    pub(super) fn dc_trace() -> bool {
        crate::env_once!(std::env::var("LOFT_TRACE_DEFERRED_COPY").is_ok_and(|v| v != "0"))
    }

    /// The deferred locals of this function (`@FR-R-DeferredCopy`).
    pub(super) fn deferred_plan(&self) -> DeferredCopies {
        let mut out = DeferredCopies::default();
        if crate::env_once!(std::env::var("LOFT_NO_DEFERRED_COPY").is_ok_and(|v| v != "0"))
            || self.gen_elem.is_some()
        {
            return out;
        }
        let vars = self.vars;
        let code = self.code;
        let Some(facts) = self.store_facts() else {
            return out;
        };
        // A suspended or forked body runs the reads where no copy point stands.
        if code.any_node(&mut |n| {
            matches!(n, Value::Yield(_) | Value::Parallel(_) | Value::CallRef(..))
        }) {
            return out;
        }
        for x in 0..vars.count() {
            if let Some(dv) = self.dc_candidate(x, &facts) {
                out.vars.insert(x, dv);
            }
        }
        if !out.vars.is_empty() {
            out.null_only = (0..vars.count())
                .filter(|v| {
                    self.shapes.get(*v as usize) == Some(&super::CType::Ref)
                        && !vars.is_argument(*v)
                        && !vars.is_captured(*v)
                        && !vars.is_lazy_buffer(*v)
                        && !facts.placed.contains(v)
                        && facts.adopted.is_none_or(|(a, w)| *v != a && *v != w)
                        && facts
                            .owned
                            .is_none_or(|o| !o.buffers.contains(v) && !o.records.contains(v))
                        && null_only(code, self.cx.data, *v)
                })
                .collect();
        }
        out
    }

    /// Local `x` as a deferred copy (`deferred_plan`), or `None` with the trace naming why.
    fn dc_candidate(&self, x: u16, facts: &crate::opt::hoist::StoreFacts<'_>) -> Option<DcVar> {
        let data = self.cx.data;
        let vars = self.vars;
        let n = vars.count();
        if self.shapes.get(x as usize) != Some(&super::CType::Ref)
            || vars.is_argument(x)
            || vars.is_captured(x)
        {
            return None;
        }
        let own_name = format!("__own_{}", vars.name(x));
        let own = (0..n).find(|w| vars.name(*w) == own_name)?;
        let tp = data.def(vars.tp(x).base().heap_def_nr()?).known_type();
        let why = |what: &str| {
            if Self::dc_trace() {
                eprintln!(
                    "deferred-copy: {} {} declined: {what}",
                    data.def(self.d).name(),
                    vars.name(x)
                );
            }
        };
        if !remappable(self.cx.stores, tp, &mut HashSet::new()) {
            why("its type reaches a keyed collection, which the remap does not follow");
            return None;
        }
        let (binds, sources, srcs) = match self.dc_binds(x, facts) {
            Ok(Some(b)) => b,
            Ok(None) => return None,
            Err(()) => {
                why("a copy bind from an owner or an untraceable source");
                return None;
            }
        };
        let Some(mut derived) = self.dc_derived(x) else {
            why("a text is borrowed from it");
            return None;
        };
        let refs: HashSet<u16> = derived.iter().copied().chain([x]).collect();
        if refs.iter().any(|v| vars.is_captured(*v)) {
            why("it or a view of it is captured");
            return None;
        }
        if !reads_only(self.code, data, &refs, x, own, &|y, rhs| {
            self.whole_value_bind(y, rhs).is_some()
        }) {
            why("it or a view of it is used other than by reading");
            return None;
        }
        derived.sort_unstable();
        if Self::dc_trace() {
            let ends: Vec<String> = sources
                .iter()
                .map(|t| {
                    let fresh = if facts.fresh(*t) { " fresh" } else { "" };
                    format!("{}{fresh}", vars.name(*t))
                })
                .collect();
            eprintln!(
                "deferred-copy: {} {} deferred ({} bind(s), {} derived view(s), source store {ends:?})",
                data.def(self.d).name(),
                vars.name(x),
                binds.len(),
                derived.len()
            );
        }
        let dead = dead_after(self.code, &refs, &binds);
        let elem = self.dc_elem(&srcs);
        if Self::dc_trace() {
            match elem {
                Some((v, _)) => eprintln!("deferred-copy:   index guard on {}", vars.name(v)),
                None => eprintln!(
                    "deferred-copy:   flag only: a source is not a whole element of a vector the function owns or is given"
                ),
            }
        }
        Some(DcVar {
            own,
            derived,
            tp,
            sources,
            binds,
            dead,
            elem,
        })
    }

    /// `x`'s deferrable binds — whole-value copies from a VIEW — and the variables their
    /// sources' stores end at; `Ok(None)` without one, `Err` where a copy bind's source is an
    /// owner or untraceable (that bind must copy, so the local is not deferred at all).
    #[allow(clippy::type_complexity)]
    fn dc_binds(
        &self,
        x: u16,
        facts: &crate::opt::hoist::StoreFacts<'_>,
    ) -> Result<Option<(HashSet<usize>, Vec<u16>, Vec<u16>)>, ()> {
        let vars = self.vars;
        let mut binds = HashSet::new();
        let mut sources = Vec::new();
        let mut srcs = Vec::new();
        let mut bad_source = false;
        self.code.any_node(&mut |node| {
            if let Value::Set(v, rhs) = node
                && *v == x
                && let Some((src, _)) = self.whole_value_bind(x, rhs)
            {
                let view = !vars.tp(src).depend().is_empty() || vars.is_argument(src);
                match facts.terminal(src) {
                    Some(t) if view => {
                        binds.insert(node_addr(node));
                        if !sources.contains(&t) {
                            sources.push(t);
                        }
                        if !srcs.contains(&src) {
                            srcs.push(src);
                        }
                    }
                    _ => bad_source = true,
                }
            }
            false
        });
        if bad_source {
            return Err(());
        }
        Ok((!binds.is_empty()).then_some((binds, sources, srcs)))
    }

    /// `(V, element size)` when every one of `srcs` is only ever bound to a whole element of
    /// the one vector variable `V` — `s = V[e]`, through a `??` discharge's temp — and `V` is
    /// an argument or a local that owns its store, so it cannot be re-pointed at a vector
    /// inside an element (`@FR-R-DeferredCopy`'s index guard).
    fn dc_elem(&self, srcs: &[u16]) -> Option<(u16, i64)> {
        let data = self.cx.data;
        let vars = self.vars;
        let mut found: Option<(u16, i64)> = None;
        for &s in srcs {
            let e = elem_source(self.code, data, vars, s, 0)?;
            if found.is_some_and(|f| f != e) {
                return None;
            }
            found = Some(e);
        }
        let (v, _) = found?;
        // An owner depends on nothing but its own store witness (`__vdb_N`, a local vector's);
        // a dependency on any other variable makes `V` a view, which could name a vector
        // inside an element.
        let owner = vars.is_argument(v)
            || vars
                .tp(v)
                .depend()
                .iter()
                .all(|d| *d == v || vars.name(*d).starts_with("__vdb"));
        (owner
            && !vars.is_captured(v)
            && self.shapes.get(v as usize) == Some(&super::CType::Ref)
            && !matches!(vars.tp(v).base(), Type::RefVar(_)))
        .then_some(found?)
    }

    /// The `k` of a write the operator `name` makes through `args` when the one place it
    /// writes is rooted at `V[k]` (`dv.elem`'s `V`): a field setter's record, `OpCopyRecord`'s
    /// destination.  `OpCopyRecord` reads its source, and releases it only under
    /// `COPY_FREE_SOURCE`, whose contract makes it a callee's fresh temporary.  `k` must be a
    /// literal or an integer local nothing links to and the statement `s` does not rebind, so
    /// the guard evaluates it to the value the write uses.
    fn dc_indexed(&self, name: &str, args: &[Value], dv: &DcVar, s: &Value) -> Option<Value> {
        let (v, size) = dv.elem?;
        let place = if crate::same_read::FIELD_SETS.contains(&name) {
            args.first()?
        } else if name == "OpCopyRecord" && args.len() == 3 {
            &args[1]
        } else {
            return None;
        };
        let (root, sz, k) = index_root(self.cx.data, place)?;
        if root != v || sz != size {
            return None;
        }
        let vars = self.vars;
        if let Some(n) = int_constant(self.cx.data, k) {
            return Some(Value::Int(n));
        }
        match k.unspan() {
            Value::Var(i)
                if !vars.is_captured(*i)
                    && matches!(vars.tp(*i).base(), Type::Integer(..))
                    && !(0..vars.count())
                        .any(|w| crate::opt::body_writes::link_targets(vars, w).contains(i))
                    && !s.any_node(&mut |n| matches!(n, Value::Set(t, _) if t == i)) =>
            {
                Some(k.clone())
            }
            _ => None,
        }
    }

    /// The reference variables derived from `x` — deps reaching it, closed — or `None` where a
    /// text is borrowed from it (a `loft_str` into the source has no copy to move to).
    fn dc_derived(&self, x: u16) -> Option<Vec<u16>> {
        let vars = self.vars;
        let mut set: HashSet<u16> = HashSet::from([x]);
        loop {
            let before = set.len();
            for v in 0..vars.count() {
                if !set.contains(&v) && vars.tp(v).depend().iter().any(|d| set.contains(d)) {
                    set.insert(v);
                }
            }
            if set.len() == before {
                break;
            }
        }
        let mut derived = Vec::new();
        for &v in &set {
            if v == x {
                continue;
            }
            match vars.tp(v).base() {
                Type::Text(_) => return None,
                _ if self.shapes.get(v as usize) == Some(&super::CType::Ref) => derived.push(v),
                _ => {}
            }
        }
        Some(derived)
    }

    /// Whether statement `s` may write a store not proven apart from `dv`'s sources' — the
    /// point its copy must be taken before.  A read operator writes nothing; any other operator
    /// or call writes at most the stores its operands name (a scalar field read passes a
    /// number).  The witnesses of deferred locals name only their copies.
    fn dc_writes_source(&self, s: &Value, whole: &Value, dv: &DcVar) -> Point {
        let data = self.cx.data;
        let Some(facts) = self.store_facts() else {
            return Point::Flag;
        };
        let vars = self.vars;
        // The witnesses name only their copies, and a deferred local and its views are only
        // ever read (`reads_only`): none of them is how a statement writes a source.
        let owns: HashSet<u16> = self
            .dc
            .vars
            .iter()
            .flat_map(|(x, d)| d.derived.iter().copied().chain([*x, d.own]))
            .collect();
        // A copy is disturbed by a change to its source's RECORDS, never by the store's buffer
        // moving (a reference is store, record and position, not an address).  So besides
        // `distinct`, two owners this activation created — minted, or placed in a host's store
        // (`(R-Place)`) — own disjoint records whichever store they share: a write through one
        // changes no record of the other.
        let own_here = |w: u16| facts.fresh(w) || facts.placed.contains(&w);
        let records_apart = |t: u16, v: u16| {
            facts
                .terminal(v)
                .is_some_and(|tv| tv != t && own_here(t) && own_here(tv))
        };
        let apart = |v: u16| {
            owns.contains(&v)
                || self.dc.null_only.contains(&v)
                || (v as usize) >= vars.var_count()
                || !(crate::data::is_dbref(vars.tp(v).base())
                    || matches!(vars.tp(v).base(), Type::RefVar(_)))
                || dv
                    .sources
                    .iter()
                    .all(|t| *t != v && (facts.distinct(*t, v) || records_apart(*t, v)))
        };
        let mut flag = false;
        let mut ks: Vec<Value> = Vec::new();
        let memo = self.dc.writes.clone();
        let mut memo = memo.borrow_mut();
        s.any_node(&mut |node| {
            if let Value::Call(op, args) = node
                && (*op as usize) < data.definitions.len()
                && !read_op(data.def(*op).name())
                // An operator or callee that writes no store cannot disturb a source — the
                // question the header gate asks (`call_writes_store`), answered the same way.
                && crate::opt::writes::call_writes_store(
                    *op,
                    data,
                    &mut memo,
                    &mut HashSet::new(),
                    None,
                )
                && !args.iter().all(|a| super::mentions_apart(a, data, &apart))
            {
                if let Some(k) = self.dc_indexed(data.def(*op).name(), args, dv, whole) {
                    ks.push(k);
                    return false;
                }
                if Self::dc_trace() {
                    let names: Vec<String> = args
                        .iter()
                        .map(|a| match a.unspan() {
                            Value::Var(v) => vars.name(*v).to_string(),
                            _ => "_".into(),
                        })
                        .collect();
                    let mut failing = Vec::new();
                    for a in args {
                        a.any_node(&mut |m| {
                            if let Value::Var(v) = m
                                && !apart(*v)
                            {
                                failing.push(vars.name(*v).to_string());
                            }
                            false
                        });
                    }
                    eprintln!(
                        "deferred-copy:   copy point at {} [{}] not apart: {failing:?}",
                        data.def(*op).name(),
                        names.join(",")
                    );
                }
                flag = true;
            }
            flag
        });
        if flag {
            Point::Flag
        } else if ks.is_empty() {
            Point::Clear
        } else {
            Point::Index(ks)
        }
    }

    /// Before statement `s`: take every deferred copy `s` may disturb the source of.
    pub(super) fn dc_before(&mut self, s: &Value, ind: usize) {
        if self.dc.vars.is_empty() {
            return;
        }
        let probe: &Value = match s.unspan() {
            Value::Block(_) | Value::Loop(_) | Value::Insert(_) => return,
            Value::If(c, _, _) => c,
            other => other,
        };
        let at = node_addr(s);
        let mut due: Vec<(u16, Option<Vec<Value>>)> = Vec::new();
        for (x, dv) in &self.dc.vars {
            if dv.dead.contains(&at) {
                continue;
            }
            match self.dc_writes_source(probe, s, dv) {
                Point::Clear => {}
                Point::Index(ks) => due.push((*x, Some(ks))),
                Point::Flag => due.push((*x, None)),
            }
        }
        if due.is_empty() {
            return;
        }
        due.sort_unstable_by_key(|(x, _)| *x);
        let data = self.cx.data;
        let frame = crate::lease::Frame::new(data, data.def(self.d));
        for (x, ks) in due {
            // Only what is read at or after the point needs the copy: a view the statement
            // mentions, or one the per-path liveness finds read later (an unreached site is read).
            let dv = &self.dc.vars[&x];
            let refs: HashSet<u16> = dv.derived.iter().copied().chain([x]).collect();
            let here = s.any_node(&mut |n| matches!(n, Value::Var(v) if refs.contains(v)));
            let read = |v: u16| {
                let after = frame.read_after(s, v);
                if Self::dc_trace() && !here {
                    let at = match after {
                        None => "unreached".to_string(),
                        Some(None) => "not read".to_string(),
                        Some(Some(l)) => format!("read at line {l}"),
                    };
                    eprintln!(
                        "deferred-copy:     {} after the point: {at}",
                        self.vars.name(v)
                    );
                }
                here || after != Some(None)
            };
            let moved: Vec<u16> = dv.derived.iter().copied().filter(|d| read(*d)).collect();
            if !read(x) && moved.is_empty() {
                if Self::dc_trace() {
                    eprintln!(
                        "deferred-copy:   no point: {} is not read again",
                        self.vars.name(x)
                    );
                }
                continue;
            }
            if Self::dc_trace() {
                let names: Vec<&str> = moved.iter().map(|d| self.vars.name(*d)).collect();
                let form = if ks.is_some() { "index guard" } else { "flag" };
                eprintln!(
                    "deferred-copy:   {form} point for {}, moving {names:?}",
                    self.vars.name(x)
                );
            }
            self.dc_materialise(x, ind, ks.as_deref(), &moved);
        }
    }

    /// The copy point: if `x` still views its source — and, for an indexed write, views the
    /// element at one of `ks` — copy it, move the derived views in `moved` into the copy, and
    /// derive again the addresses held for any of them.
    fn dc_materialise(&mut self, x: u16, ind: usize, ks: Option<&[Value]>, moved: &[u16]) {
        let dv = self.dc.vars[&x].clone();
        let name = self.var(x);
        let own = self.var(dv.own);
        let flag = format!("loft_dcv{x}");
        let cond = match (ks, dv.elem) {
            (Some(ks), Some((v, size))) => {
                let vn = self.var(v);
                let tests: Vec<String> = ks
                    .iter()
                    .map(|k| {
                        let kc = match k.unspan() {
                            Value::Var(i) => format!("(int64_t) {}", self.var(*i)),
                            Value::Int(n) => format!("INT64_C({n})"),
                            _ => unreachable!("dc_indexed admits a variable or a literal"),
                        };
                        format!("loft_dc_is_elem({name}, {vn}, {size}, {kc})")
                    })
                    .collect();
                crate::rewrite_census::fired("R-DeferredCopy/index", 1);
                format!("{flag} && ({})", tests.join(" || "))
            }
            _ => flag.clone(),
        };
        self.line(ind, &format!("if ({cond}) {{"));
        self.line(ind + 1, "loft_ref loft_dcc = loft_ref_null();");
        self.line(
            ind + 1,
            &format!("loft_bind_copy(loft_types, {}, &loft_dcc, {name});", dv.tp),
        );
        for d in moved {
            let dn = self.var(*d);
            self.line(
                ind + 1,
                &format!(
                    "{dn} = loft_dc_remap(loft_types, {}, {name}, loft_dcc, {dn});",
                    dv.tp
                ),
            );
        }
        self.line(ind + 1, &format!("{name} = loft_dcc;"));
        self.line(ind + 1, &format!("{own} = loft_dcc;"));
        self.line(ind + 1, &format!("{flag} = 0;"));
        let held: Vec<(u16, String)> = self
            .rec_ptrs
            .iter()
            .filter(|(v, _, _)| *v == x || dv.derived.contains(v))
            .map(|(v, pa, _)| (*v, pa.clone()))
            .collect();
        for (v, pa) in held {
            let vn = self.var(v);
            self.line(ind + 1, &format!("{pa} = loft_rec_ptr({vn});"));
        }
        self.line(ind, "}");
        crate::rewrite_census::fired("R-DeferredCopy/point", 1);
    }

    /// `Set(var, rhs)` of a deferred local or of its witness: the deferred bind takes the
    /// source's reference and raises the flag; the witness takes the local only when it owns.
    /// `None` for any other statement.
    pub(super) fn dc_set(&mut self, set: &Value, var: u16, rhs: &Value, ind: usize) -> Option<()> {
        if let Some(dv) = self.dc.vars.get(&var)
            && dv.binds.contains(&node_addr(set))
        {
            let (src, _) = self.whole_value_bind(var, rhs)?;
            let name = self.var(var);
            let src = self.var(src);
            self.line(ind, &format!("{name} = {src};"));
            self.line(ind, &format!("loft_dcv{var} = 1;"));
            crate::rewrite_census::fired("R-DeferredCopy", 1);
            return Some(());
        }
        let data = self.cx.data;
        if let Some((x, _)) = self
            .dc
            .vars
            .iter()
            .find(|(x, dv)| dv.own == var && names_var(data, rhs, **x))
        {
            let x = *x;
            let own = self.var(var);
            let xn = self.var(x);
            self.line(ind, &format!("if (!loft_dcv{x}) {own} = {xn};"));
            return Some(());
        }
        None
    }

    /// After any other bind of a deferred local: it is a copy or a view in its own right now.
    pub(super) fn dc_after(&mut self, set: &Value, ind: usize) {
        if let Value::Set(var, _) = set.unspan()
            && let Some(dv) = self.dc.vars.get(var)
            && !dv.binds.contains(&node_addr(set))
        {
            self.line(ind, &format!("loft_dcv{var} = 0;"));
        }
    }

    /// Is `v` a deferred local or a view derived from one — a root no header may be held for?
    pub(super) fn dc_root(&self, v: u16) -> bool {
        self.dc
            .vars
            .iter()
            .any(|(x, dv)| *x == v || dv.derived.contains(&v))
    }
}

/// The statements where a copy point is not needed for the variables of `refs`: after them
/// nothing reads one again — the rest of the enclosing block reaches a `return` (or is the
/// value a `return` hands back) without mentioning one — or before them none was bound by a
/// deferred bind yet (a top-level statement ahead of the first one in `binds`).  A block that
/// ends without returning — a loop body runs again, an enclosing block goes on — counts as
/// reading, so this only drops a point no path past it can need.
fn dead_after(code: &Value, refs: &HashSet<u16>, binds: &HashSet<usize>) -> HashSet<usize> {
    fn mentions(n: &Value, refs: &HashSet<u16>) -> bool {
        n.any_node(&mut |m| matches!(m, Value::Var(v) if refs.contains(v)))
    }
    fn walk(n: &Value, refs: &HashSet<u16>, returns: bool, out: &mut HashSet<usize>) {
        match n.unspan() {
            Value::Return(e) => walk(e, refs, true, out),
            Value::Block(b) | Value::Loop(b) => {
                let ops = &b.operators;
                let ends = returns && matches!(n.unspan(), Value::Block(_));
                for (i, op) in ops.iter().enumerate() {
                    let dead = if let Value::Return(_) = op.unspan() {
                        !mentions(op, refs)
                    } else {
                        let rest = &ops[i + 1..];
                        match rest.iter().position(|o| {
                            mentions(o, refs) || matches!(o.unspan(), Value::Return(_))
                        }) {
                            Some(k) => {
                                matches!(rest[k].unspan(), Value::Return(_))
                                    && !mentions(&rest[k], refs)
                            }
                            None => ends,
                        }
                    };
                    if dead {
                        out.insert(node_addr(op));
                    }
                    walk(op, refs, false, out);
                }
            }
            other => other.for_each_child(&mut |c| walk(c, refs, false, out)),
        }
    }
    let mut out = HashSet::new();
    walk(code, refs, false, &mut out);
    // Top-level statements ahead of the first deferred bind, everything inside them included.
    if let Value::Block(b) = code.unspan() {
        let first = b
            .operators
            .iter()
            .position(|op| op.any_node(&mut |m| binds.contains(&node_addr(m))));
        for op in &b.operators[..first.unwrap_or(0)] {
            op.any_node(&mut |m| {
                out.insert(node_addr(m));
                false
            });
            out.insert(node_addr(op));
        }
    }
    out
}

/// Whether every record a value of type `tp` reaches is laid out as `loft_dc_remap` follows it:
/// structs, struct-enum variants, plain vectors, texts and scalars.  A keyed collection copies
/// its records into a new layout, so a view into one has no position to move to.
fn remappable(stores: &crate::database::Stores, tp: u16, seen: &mut HashSet<u16>) -> bool {
    use crate::database::Parts;
    if (tp as usize) >= stores.types.len() || !seen.insert(tp) {
        return true;
    }
    match &stores.types[tp as usize].parts {
        Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
            fields.iter().all(|f| remappable(stores, f.content, seen))
        }
        Parts::Enum(variants) => variants
            .iter()
            .all(|(v, _)| *v == u16::MAX || remappable(stores, *v, seen)),
        Parts::Vector(elem) => remappable(stores, *elem, seen),
        Parts::Base
        | Parts::Byte(..)
        | Parts::Short(..)
        | Parts::Int(..)
        | Parts::ShortRaw(..)
        | Parts::IntRaw(..) => true,
        _ => false,
    }
}

/// Every mention of a variable in `refs` (the deferred local `x` and its derived views) is a
/// read: the target of a bind; the first operand of a read operator; the source of another
/// variable's whole-value copy (`copies`) or of a bind of another variable of `refs`; the
/// witness's own bookkeeping (`own = x`, `OpFreeRefIfDistinct(own, x)`).  An allow-list.
fn reads_only(
    code: &Value,
    data: &crate::data::Data,
    refs: &HashSet<u16>,
    x: u16,
    own: u16,
    copies: &dyn Fn(u16, &Value) -> bool,
) -> bool {
    fn walk(
        n: &Value,
        data: &crate::data::Data,
        refs: &HashSet<u16>,
        x: u16,
        own: u16,
        copies: &dyn Fn(u16, &Value) -> bool,
    ) -> bool {
        let is_ref = |v: &Value| matches!(v.unspan(), Value::Var(w) if refs.contains(w));
        let bad = |n: &Value| {
            if FnWriter::dc_trace() {
                let mut t = format!("{n:?}");
                t.truncate(200);
                eprintln!("deferred-copy:   not a read: {t}");
            }
            false
        };
        match n.unspan() {
            Value::Var(v) if refs.contains(v) => bad(n),
            Value::Var(_) => true,
            Value::Set(t, rhs) if *t == own && names_var(data, rhs, x) => true,
            // A value block's last operator is the bound value (`op = { ncr = …; ncr }`, a `??`
            // discharge): judged as the bind of that variable, the rest of the block as itself.
            Value::Set(t, rhs)
                if matches!(rhs.unspan(), Value::Block(b)
                    if b.operators.last().is_some_and(is_ref)) =>
            {
                let Value::Block(b) = rhs.unspan() else {
                    return false;
                };
                let (tail, rest) = b.operators.split_last().expect("checked");
                (refs.contains(t) || copies(*t, tail) || bad(n))
                    && rest.iter().all(|o| walk(o, data, refs, x, own, copies))
            }
            Value::Set(t, rhs) if is_ref(rhs) => {
                (*t == own && matches!(rhs.unspan(), Value::Var(w) if *w == x)
                    || refs.contains(t)
                    || copies(*t, rhs))
                    || bad(n)
            }
            Value::Call(op, args) if (*op as usize) < data.definitions.len() => {
                let name = data.def(*op).name();
                if name == "OpFreeRefIfDistinct"
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == own)
                    && matches!(args.get(1).map(Value::unspan), Some(Value::Var(w)) if *w == x)
                {
                    return true;
                }
                // A store comparison reads only the handles' store numbers.
                if name == "OpDistinctStore" {
                    return args
                        .iter()
                        .all(|a| is_ref(a) || walk(a, data, refs, x, own, copies));
                }
                let skip_first = read_op(name) && args.first().is_some_and(is_ref);
                if FnWriter::dc_trace() {
                    for (i, a) in args.iter().enumerate() {
                        if is_ref(a) && !(i == 0 && skip_first) {
                            eprintln!("deferred-copy:   operand {i} of {name}");
                        }
                    }
                }
                args.iter()
                    .enumerate()
                    .all(|(i, a)| (i == 0 && skip_first) || walk(a, data, refs, x, own, copies))
            }
            other => {
                let mut ok = true;
                other.for_each_child(&mut |c| {
                    if FnWriter::dc_trace() && is_ref(c) {
                        let mut t = format!("{other:?}");
                        t.truncate(160);
                        eprintln!("deferred-copy:   parent: {t}");
                    }
                    ok &= walk(c, data, refs, x, own, copies);
                });
                ok
            }
        }
    }
    walk(code, data, refs, x, own, copies)
}

/// Does `v` hold the null reference at every mention in `code`?  An allow-list: a bind of
/// `null`, an argument of a function (whatever it does with a null reference, it names no store
/// that exists), an operand of a release.  Any other mention — a mint into it, a bind of a value
/// — answers no.
fn null_only(code: &Value, data: &crate::data::Data, v: u16) -> bool {
    fn walk(n: &Value, data: &crate::data::Data, v: u16) -> bool {
        match n.unspan() {
            Value::Var(w) => *w != v,
            Value::Set(t, rhs) if *t == v => matches!(rhs.unspan(), Value::Null),
            Value::Call(op, args) if (*op as usize) < data.definitions.len() => {
                let name = data.def(*op).name();
                let passes = !name.starts_with("Op") || name.starts_with("OpFree");
                args.iter().all(|a| {
                    (passes && matches!(a.unspan(), Value::Var(w) if *w == v)) || walk(a, data, v)
                })
            }
            other => {
                let mut ok = true;
                other.for_each_child(&mut |c| ok &= walk(c, data, v));
                ok
            }
        }
    }
    walk(code, data, v)
}

/// `(V, element size)` when every bind of `s` in `code` is a whole element of the vector
/// variable `V`: `OpGetVector(V, size, e)`, a value block ending in one, or a compiler temp
/// (a `??` discharge's `__ncr_N`) bound only so.  A bind of `null` is no place and is skipped.
fn elem_source(
    code: &Value,
    data: &crate::data::Data,
    vars: &crate::variables::Function,
    s: u16,
    depth: u8,
) -> Option<(u16, i64)> {
    fn class(
        rhs: &Value,
        code: &Value,
        data: &crate::data::Data,
        vars: &crate::variables::Function,
        depth: u8,
    ) -> Result<Option<(u16, i64)>, ()> {
        match rhs.unspan() {
            Value::Null => Ok(None),
            Value::Block(b) => match b.operators.last() {
                Some(tail) => class(tail, code, data, vars, depth),
                None => Err(()),
            },
            Value::Var(t) if depth < 3 && vars.is_compiler_generated(*t) => {
                elem_source(code, data, vars, *t, depth + 1)
                    .map(Some)
                    .ok_or(())
            }
            Value::Call(op, args)
                if (*op as usize) < data.definitions.len()
                    && matches!(data.def(*op).name(), "OpGetVector" | "OpGetVectorNullable")
                    && args.len() == 3 =>
            {
                match (args[0].unspan(), args[1].unspan()) {
                    (Value::Var(v), Value::Int(size)) => Ok(Some((*v, i64::from(*size)))),
                    _ => Err(()),
                }
            }
            _ => Err(()),
        }
    }
    let mut found: Option<(u16, i64)> = None;
    let mut bad = false;
    code.any_node(&mut |n| {
        if let Value::Set(t, rhs) = n
            && *t == s
        {
            match class(rhs, code, data, vars, depth) {
                Ok(Some(e)) if found.is_none_or(|f| f == e) => found = Some(e),
                Ok(None) => {}
                _ => bad = true,
            }
        }
        bad
    });
    if bad { None } else { found }
}

/// `(V, element size, k)` when the place `p` is rooted at element `k` of the vector variable
/// `V`: `OpGetVector(V, size, k)` itself, or a field or an element reached from it.  The root is
/// the element nearest the variable, so `ps[1].ops[0].a` is rooted at `ps[1]`.  A pointer field
/// or a call on the way answers `None`: the place it reaches is not owned through the element.
fn index_root<'v>(data: &crate::data::Data, p: &'v Value) -> Option<(u16, i64, &'v Value)> {
    let Value::Call(op, args) = p.unspan() else {
        return None;
    };
    if (*op as usize) >= data.definitions.len() {
        return None;
    }
    match data.def(*op).name() {
        "OpGetField" if !args.is_empty() => index_root(data, &args[0]),
        "OpGetVector" | "OpGetVectorNullable" if args.len() == 3 => match args[0].unspan() {
            Value::Var(v) => match args[1].unspan() {
                Value::Int(size) => Some((*v, i64::from(*size), &args[2])),
                _ => None,
            },
            _ => index_root(data, &args[0]),
        },
        _ => None,
    }
}

/// The value of `k` when it is an integer literal or the negation of one (`-1` is
/// `OpMinSingleInt(1)`).
fn int_constant(data: &crate::data::Data, k: &Value) -> Option<i32> {
    match k.unspan() {
        Value::Int(n) => Some(*n),
        Value::Call(op, args)
            if (*op as usize) < data.definitions.len()
                && data.def(*op).name() == "OpMinSingleInt"
                && args.len() == 1 =>
        {
            match args[0].unspan() {
                Value::Int(n) => n.checked_neg(),
                _ => None,
            }
        }
        _ => None,
    }
}
