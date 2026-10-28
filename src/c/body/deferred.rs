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
//! copy's, since nothing changed the source between the bind and that statement.  An
//! interpreter's `prog = callee_prog` then costs one copy per lowering, not one per call.
//!
//! The local must hold an owner witness (`__own_<x>`, `@FR-O-Witness`): it marks whether the
//! local owns a copy, so a deferred view is never released.  Every mention of the local, and of
//! each reference variable derived from it (`op = prog.ops[pc]`), must be a read; a derived
//! view is moved into the copy when it is taken (`loft_dc_remap`), as is any address held for
//! one of them (`@FR-R-RecPtr`).  A text borrowed from the local declines it.
//! `LOFT_NO_DEFERRED_COPY=1` copies at every bind again; `LOFT_TRACE_DEFERRED_COPY=1` names
//! each admission and decline.
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
}

/// The function's deferred locals.
#[derive(Clone, Debug, Default)]
pub(crate) struct DeferredCopies {
    pub vars: HashMap<u16, DcVar>,
    /// `call_writes_store`'s memo, shared by every copy-point test of the function.
    pub writes: std::rc::Rc<std::cell::RefCell<HashMap<u32, bool>>>,
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
        let (binds, sources) = match self.dc_binds(x, facts) {
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
            eprintln!(
                "deferred-copy: {} {} deferred ({} bind(s), {} derived view(s))",
                data.def(self.d).name(),
                vars.name(x),
                binds.len(),
                derived.len()
            );
        }
        Some(DcVar {
            own,
            derived,
            tp,
            sources,
            binds,
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
    ) -> Result<Option<(HashSet<usize>, Vec<u16>)>, ()> {
        let vars = self.vars;
        let mut binds = HashSet::new();
        let mut sources = Vec::new();
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
                    }
                    _ => bad_source = true,
                }
            }
            false
        });
        if bad_source {
            return Err(());
        }
        Ok((!binds.is_empty()).then_some((binds, sources)))
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
    fn dc_writes_source(&self, s: &Value, dv: &DcVar) -> bool {
        let data = self.cx.data;
        let Some(facts) = self.store_facts() else {
            return true;
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
        let apart = |v: u16| {
            owns.contains(&v)
                || (v as usize) >= vars.var_count()
                || !(crate::data::is_dbref(vars.tp(v).base())
                    || matches!(vars.tp(v).base(), Type::RefVar(_)))
                || dv.sources.iter().all(|t| *t != v && facts.distinct(*t, v))
        };
        let mut hit = false;
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
                if Self::dc_trace() {
                    let names: Vec<String> = args
                        .iter()
                        .map(|a| match a.unspan() {
                            Value::Var(v) => vars.name(*v).to_string(),
                            _ => "_".into(),
                        })
                        .collect();
                    eprintln!(
                        "deferred-copy:   copy point at {} [{}]",
                        data.def(*op).name(),
                        names.join(",")
                    );
                }
                hit = true;
            }
            hit
        });
        hit
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
        let mut due: Vec<u16> = self
            .dc
            .vars
            .iter()
            .filter(|(_, dv)| self.dc_writes_source(probe, dv))
            .map(|(x, _)| *x)
            .collect();
        due.sort_unstable();
        for x in due {
            self.dc_materialise(x, ind);
        }
    }

    /// The copy point: if `x` still views its source, copy it, move its derived views into the
    /// copy, and derive again the addresses held for any of them.
    fn dc_materialise(&mut self, x: u16, ind: usize) {
        let dv = self.dc.vars[&x].clone();
        let name = self.var(x);
        let own = self.var(dv.own);
        let flag = format!("loft_dcv{x}");
        self.line(ind, &format!("if ({flag}) {{"));
        self.line(ind + 1, "loft_ref loft_dcc = loft_ref_null();");
        self.line(
            ind + 1,
            &format!("loft_bind_copy(loft_types, {}, &loft_dcc, {name});", dv.tp),
        );
        for d in &dv.derived {
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
