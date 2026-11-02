// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C
//! `@FR-R-ElemFirst` on the C: a local vector consumed once as a record-literal field of an
//! append (`out += [R { f: v, … }]`) is built INSIDE the appended element.
//!
//! The admission is the shared analysis (`hoist::element_first`), the one native reads.  The C
//! spells its moves over the IR the parser wrote: at the first temp's declaration the append
//! site's own reservation, mint and vector-field zeros run (the C does not zero a fresh slot,
//! so the zeros move with the mint), and every paired temp is bound to its field slot in the
//! element instead of a store of its own; the append site then skips what already ran and the
//! paired copies, keeping its scalar sets and its finish — the length bump that keeps the
//! element invisible until the append.  The forms native admits beyond the literal-built temp
//! appended into a local vector (an append in each arm, a call-filled temp, the comprehension
//! clause) keep the plain form here, and so does a pair whose container the writer holds a
//! header, a push header or a window for: those are keyed to the statements they rewrite.
//! `LOFT_NO_ELEMENT_FIRST=1` (native's) keeps the plain form.

use crate::data::{Data, Value};
use std::collections::HashMap;

struct Pair {
    out: u16,
    elm: u16,
}

/// One function's admitted element-first pairs, and the statements the port rewrites.
#[derive(Default)]
pub(crate) struct ElemFirstPort {
    pairs: Vec<Pair>,
    /// A temp's declaration (`OpDatabase(vdb, …)`), by node address: its pair, the temp, the
    /// field offset, and whether it carries the mint.
    decls: HashMap<usize, (usize, u16, i32, bool)>,
    /// The statements written early or not at all, by node address, with their pair.
    skip: HashMap<usize, usize>,
    /// The append site's reservation, mint and zeros, written at the first declaration.
    early: Vec<Vec<Value>>,
    /// Whether a pair took the element-first form (decided at its first declaration).
    active: Vec<bool>,
}

fn named(data: &Data, d: u32, n: &str) -> bool {
    (d as usize) < data.definitions.len() && data.def(d).name() == n
}

fn var(v: Option<&Value>) -> Option<u16> {
    match v.map(Value::unspan) {
        Some(Value::Var(x)) => Some(*x),
        _ => None,
    }
}

fn int(v: Option<&Value>) -> Option<i32> {
    match v.map(Value::unspan) {
        Some(Value::Int(n)) => Some(*n),
        _ => None,
    }
}

/// Every statement list of `v`: a block's, a loop's, an `if`'s arms', an insert's.
fn lists<'v>(v: &'v Value, out: &mut Vec<&'v [Value]>) {
    match v.unspan() {
        Value::Block(b) | Value::Loop(b) => {
            out.push(&b.operators);
            for o in &b.operators {
                lists(o, out);
            }
        }
        Value::Insert(ops) => {
            out.push(ops);
            for o in ops {
                lists(o, out);
            }
        }
        other => other.for_each_child(&mut |c: &'v Value| lists(c, out)),
    }
}

fn addr(v: &Value) -> usize {
    std::ptr::from_ref(v) as usize
}

impl ElemFirstPort {
    pub(crate) fn of(cx: &super::super::Cx, d: u32) -> Self {
        let mut port = ElemFirstPort::default();
        if cx.switches.element_first_disabled {
            return port;
        }
        let data = cx.data;
        let map =
            crate::opt::hoist::element_first(data, cx.stores, d, cx.value_records().views.get(&d));
        let mut all: Vec<&[Value]> = Vec::new();
        lists(data.def(d).code(), &mut all);
        for pair in &map.pairs {
            if !pair.aliases.is_empty()
                || pair.out_fld != 65535
                || pair.binds.iter().any(|b| b.from_call)
                || map.whole_copies.iter().any(|(_, e)| *e == pair.elm)
            {
                continue;
            }
            if let Some((early, skip, decls)) = Self::locate(data, pair, &all) {
                let pi = port.pairs.len();
                port.pairs.push(Pair {
                    out: pair.out,
                    elm: pair.elm,
                });
                port.early.push(early);
                port.active.push(false);
                for a in skip {
                    port.skip.insert(a, pi);
                }
                for (a, tmp, off, first) in decls {
                    port.decls.insert(a, (pi, tmp, off, first));
                }
            }
        }
        port
    }

    /// The statements of one pair: the append site's reservation, mint and zeros (cloned, to
    /// be written early), the addresses of every statement written early or skipped, and each
    /// temp's declaration.  `None` when one is not where the analysis says it is.
    #[allow(clippy::type_complexity)]
    fn locate(
        data: &Data,
        pair: &crate::opt::hoist::ElemFirst,
        all: &[&[Value]],
    ) -> Option<(Vec<Value>, Vec<usize>, Vec<(usize, u16, i32, bool)>)> {
        let mut early: Vec<Value> = Vec::new();
        let mut skip: Vec<usize> = Vec::new();
        let mut decls: Vec<(usize, u16, i32, bool)> = Vec::new();
        let mut minted = false;
        for ops in all {
            for (j, o) in ops.iter().enumerate() {
                let Value::Set(e, m) = o.unspan() else {
                    continue;
                };
                if *e != pair.elm {
                    continue;
                }
                let Value::Call(md, margs) = m.unspan() else {
                    continue;
                };
                if !named(data, *md, "OpNewRecord") || var(margs.first()) != Some(pair.out) {
                    continue;
                }
                if minted {
                    return None;
                }
                minted = true;
                let pre = ops[..j]
                    .iter()
                    .rev()
                    .find(|p| !matches!(p.unspan(), Value::Line(_)))?;
                let Value::Call(pd, pargs) = pre.unspan() else {
                    return None;
                };
                if !named(data, *pd, "OpPreAllocVector") || var(pargs.first()) != Some(pair.out) {
                    return None;
                }
                early.push(pre.clone());
                early.push(o.clone());
                skip.push(addr(pre));
                skip.push(addr(o));
                for b in &pair.binds {
                    let zero = ops[j + 1..].iter().find(|z| {
                        matches!(z.unspan(), Value::Call(zd, za)
                            if named(data, *zd, "OpSetInt4")
                                && var(za.first()) == Some(pair.elm)
                                && int(za.get(1)) == Some(b.field_off)
                                && int(za.get(2)) == Some(0))
                    })?;
                    early.push(zero.clone());
                    skip.push(addr(zero));
                    let copy = ops[j + 1..].iter().find(|c| {
                        matches!(c.unspan(), Value::Call(cd, ca)
                            if named(data, *cd, "OpAppendVector")
                                && var(ca.get(1)) == Some(b.tmp)
                                && matches!(ca.first().map(Value::unspan), Some(Value::Call(gd, ga))
                                    if named(data, *gd, "OpGetField")
                                        && var(ga.first()) == Some(pair.elm)
                                        && int(ga.get(1)) == Some(b.field_off)))
                    })?;
                    skip.push(addr(copy));
                }
            }
        }
        if !minted {
            return None;
        }
        for b in &pair.binds {
            let (decl, bind, reset) = Self::temp_decl(data, b, all)?;
            decls.push((decl, b.tmp, b.field_off, b.first));
            skip.push(bind);
            skip.push(reset);
        }
        Some((early, skip, decls))
    }

    /// A temp's three declaration statements, each found exactly once: the store mint
    /// (`OpDatabase(vdb, …)`), the bind to its vector field and the length reset — by address.
    fn temp_decl(
        data: &Data,
        b: &crate::opt::hoist::ElemBind,
        all: &[&[Value]],
    ) -> Option<(usize, usize, usize)> {
        let (mut decl, mut bind, mut reset) = (None, None, None);
        for ops in all {
            for o in *ops {
                match o.unspan() {
                    Value::Call(dd, a)
                        if (named(data, *dd, "OpDatabase") || named(data, *dd, "OpDatabaseNP"))
                            && var(a.first()) == Some(b.vdb) =>
                    {
                        decl = decl.map_or(Some(addr(o)), |_| Some(usize::MAX));
                    }
                    Value::Set(t, x)
                        if *t == b.tmp
                            && matches!(x.unspan(), Value::Call(gd, ga)
                                if named(data, *gd, "OpGetField")
                                    && var(ga.first()) == Some(b.vdb)) =>
                    {
                        bind = bind.map_or(Some(addr(o)), |_| Some(usize::MAX));
                    }
                    Value::Call(zd, za)
                        if named(data, *zd, "OpSetInt4")
                            && var(za.first()) == Some(b.vdb)
                            && int(za.get(1)) == Some(0)
                            && int(za.get(2)) == Some(0) =>
                    {
                        reset = reset.map_or(Some(addr(o)), |_| Some(usize::MAX));
                    }
                    _ => {}
                }
            }
        }
        let (decl, bind, reset) = (decl?, bind?, reset?);
        if [decl, bind, reset].contains(&usize::MAX) {
            return None;
        }
        Some((decl, bind, reset))
    }
}

impl super::FnWriter<'_> {
    /// `@FR-R-ElemFirst` — statement `v` of an admitted pair: a temp's declaration written as
    /// the element mint (the first temp) and the bind to the element's field slot, or a
    /// statement the early mint already ran, or a paired copy, written as nothing.  `None` for
    /// every other statement, and for a pair whose first declaration declined.
    pub(super) fn elem_first_stmt(&mut self, v: &Value, ind: usize) -> Option<Result<(), String>> {
        if self.ef.pairs.is_empty() || self.gen_elem.is_some() || self.tr.is_some() {
            return None;
        }
        let keys = [addr(v), addr(v.unspan())];
        if let Some(&(pi, tmp, off, first)) = keys.iter().find_map(|k| self.ef.decls.get(k)) {
            if first {
                let out = self.ef.pairs[pi].out;
                let path: crate::opt::writes::PathKey = (out, Vec::new());
                let held = self.held_path(&path).is_some()
                    || self.push_head_based(&path).is_some()
                    || self.windows.iter().any(|(p, ..)| *p == path);
                self.ef.active[pi] = !held;
                if held {
                    return None;
                }
                let early = self.ef.early[pi].clone();
                for s in &early {
                    if let Err(e) = self.stmt(s, ind) {
                        return Some(Err(e));
                    }
                }
                crate::rewrite_census::fired("R-ElemFirst", 1);
            } else if !self.ef.active[pi] {
                return None;
            }
            let (t, e) = (self.var(tmp), self.var(self.ef.pairs[pi].elm));
            self.line(
                ind,
                &format!("{t} = (loft_ref) {{ {e}.store_nr, {e}.rec, {e}.pos + {off} }}; /* @FR-R-ElemFirst */"),
            );
            return Some(Ok(()));
        }
        if let Some(&pi) = keys.iter().find_map(|k| self.ef.skip.get(k))
            && self.ef.active[pi]
        {
            return Some(Ok(()));
        }
        None
    }
}
