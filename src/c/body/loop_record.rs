// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-LoopRecord`'s refill clause on the C: a record literal bound inside a loop, whose
//! heap is vectors of no-heap elements, keeps its store and those vectors' blocks across the
//! passes.
//!
//! The admission is the shared analysis (`opt::loop_records`), the one native reads.  The C
//! spells its three moves: the per-pass declaration and end-of-pass free are not written (one
//! free follows the loop), the mint is taken only where the local holds no store, and each of
//! the literal's vector-field zeros empties the kept vector in place on a kept pass — length 0,
//! block kept — where a fresh pass zeroes it as before.  The literal writes every field, so a
//! kept record answers what a fresh one would.  A plain no-heap record is left to the plain
//! mint: on the C a store reset costs what a fresh store does, and only the vectors' claims
//! were worth keeping.  `LOFT_NO_LOOP_RECORD_REFILL=1` keeps the plain form.

use crate::data::{Block, Value};
use std::collections::{HashMap, HashSet};

/// One function's admitted refill records, and the statements the port rewrites.
#[derive(Default)]
pub(crate) struct LoopRecordPort {
    /// The admitted locals and the loop scope each is freed after.
    pub recs: HashMap<u16, u16>,
    /// The per-pass `Set(v, null)` and end-of-pass frees, by node address: not written.
    pub dropped: HashSet<usize>,
    /// The literal's vector-field zeros (`OpSetInt4(v, off, 0)`), by node address.
    pub zeros: HashSet<usize>,
    /// `@FR-R-FnRefBuffer`: the kept locals a function-value call fills in place.
    pub fnref_bufs: HashSet<u16>,
}

impl LoopRecordPort {
    pub(crate) fn of(cx: &super::super::Cx, d: u32) -> Self {
        let mut recs: HashMap<u16, u16> = HashMap::new();
        let mut decl: HashMap<u16, u16> = HashMap::new();
        let mut zeros = HashSet::new();
        if !(std::env::var("LOFT_NO_LOOP_RECORD_REFILL").is_ok_and(|v| v != "0")
            || std::env::var("LOFT_NO_LOOP_RECORD").is_ok_and(|v| v != "0"))
        {
            let lr = crate::opt::loop_records::loop_records(cx.data, cx.stores, d);
            for (v, r) in &lr.recs {
                if r.refill {
                    recs.insert(*v, r.loop_scope);
                    decl.insert(*v, r.decl_block);
                }
            }
            // The analysis answers only the refill records' zeros, and every refill record is ours.
            zeros = lr.field_zeros;
        }
        let mut fnref_bufs = HashSet::new();
        let mut dropped = HashSet::new();
        if !std::env::var("LOFT_NO_FNREF_BUFFER").is_ok_and(|v| v != "0") {
            for (v, (loop_scope, drops)) in fnref_kept(cx, d) {
                if recs.contains_key(&v) {
                    continue;
                }
                recs.insert(v, loop_scope);
                dropped.extend(drops);
                fnref_bufs.insert(v);
            }
        }
        if recs.is_empty() {
            return Self::default();
        }
        cx.data.def(d).code().walk(&mut |n| {
            if let Value::Block(b) = n {
                collect_dropped(cx, b, &decl, &mut dropped);
            }
        });
        Self {
            recs,
            dropped,
            zeros,
            fnref_bufs,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.recs.is_empty()
    }
}

/// `@FR-R-FnRefBuffer`: the record locals bound only by calls through an unresolved function
/// value (`use_analysis::opaque_callref_bind`), every bind inside one outermost loop — each with
/// that loop and the per-pass frees and null-sets inside it.  Every other mention must be a
/// scalar field read, a free, or an argument of a call whose result is a scalar (a call that can
/// keep nothing of it), so the local's store is the binding's alone and can serve every pass.
fn fnref_kept(cx: &super::super::Cx, d: u32) -> HashMap<u16, (u16, Vec<usize>)> {
    let data = cx.data;
    let def = data.def(d);
    let vars = def.variables();
    let scalar_call = |n: &Value| -> bool {
        match n.unspan() {
            Value::Call(f, _) if (*f as usize) < data.definitions.len() => {
                let name = data.def(*f).name();
                crate::opt::loop_records::SCALAR_GETTERS.contains(&name)
                    || matches!(name, "OpFreeRef" | "OpFreeRefIfDistinct" | "OpFreeRefTag")
                    || (!name.starts_with("Op") && crate::data::is_scalar(data.def(*f).returned()))
            }
            Value::CallRef(fv, _) => matches!(
                vars.tp(*fv).base(),
                crate::data::Type::Function(_, ret, ..) if crate::data::is_scalar(ret)
            ),
            _ => false,
        }
    };
    // Candidates: bound by an opaque function-value call, owned plainly by this frame.
    let mut cands: HashSet<u16> = HashSet::new();
    def.code().walk(&mut |n| {
        if let Value::Set(v, rhs) = n
            && !vars.is_argument(*v)
            && vars.owner_witness(*v).is_none()
            && vars.entry_witness(*v).is_none()
            && crate::use_analysis::opaque_callref_bind(data, d, vars.tp(*v), rhs.unspan())
                .is_some()
        {
            cands.insert(*v);
        }
    });
    if cands.is_empty() {
        return HashMap::new();
    }
    // Every Set is a null declaration or such a call; every read is an admitted argument.
    let mut bad: HashSet<u16> = HashSet::new();
    let mut allowed: HashMap<u16, usize> = HashMap::new();
    let mut seen: HashMap<u16, usize> = HashMap::new();
    def.code().walk(&mut |n| match n {
        Value::Set(v, rhs) if cands.contains(v) => {
            let ok = matches!(rhs.unspan(), Value::Null)
                || crate::use_analysis::opaque_callref_bind(data, d, vars.tp(*v), rhs.unspan())
                    .is_some();
            if !ok {
                bad.insert(*v);
            }
        }
        Value::Var(v) if cands.contains(v) => *seen.entry(*v).or_default() += 1,
        Value::Call(_, args) | Value::CallRef(_, args) if scalar_call(n) => {
            for a in args {
                if let Value::Var(v) = a.unspan()
                    && cands.contains(v)
                {
                    *allowed.entry(*v).or_default() += 1;
                }
            }
        }
        // A function value bound to a record local is called, never copied: its own Var.
        _ => {}
    });
    // The one outermost loop holding every bind, and the frees and null-sets standing directly
    // in a block inside it — the per-pass release the kept store replaces.
    let mut binds: HashMap<u16, HashSet<Option<u16>>> = HashMap::new();
    let mut drops: HashMap<u16, Vec<usize>> = HashMap::new();
    let mut loops: Vec<u16> = Vec::new();
    scan_loops(
        cx,
        d,
        def.code(),
        &cands,
        &mut loops,
        &mut binds,
        &mut drops,
    );
    let mut out: HashMap<u16, (u16, Vec<usize>)> = HashMap::new();
    for v in &cands {
        let ok = !bad.contains(v)
            && seen.get(v).copied().unwrap_or(0) == allowed.get(v).copied().unwrap_or(0);
        let one = binds
            .get(v)
            .filter(|s| s.len() == 1)
            .and_then(|s| *s.iter().next().unwrap());
        if std::env::var("LOFT_TRACE_FNREF_BUFFER").is_ok_and(|x| x != "0") {
            eprintln!(
                "[fnref-buffer] {}: {} — bad set {}, reads {} admitted {}, one loop {:?}",
                def.name(),
                vars.name(*v),
                bad.contains(v),
                seen.get(v).copied().unwrap_or(0),
                allowed.get(v).copied().unwrap_or(0),
                one
            );
        }
        if ok && let Some(l) = one {
            out.insert(*v, (l, drops.remove(v).unwrap_or_default()));
        }
    }
    out
}

/// Each candidate's binds, by the outermost loop around each (None outside every loop), and the
/// node addresses of its `Set(v, null)` and frees standing directly in a block inside a loop.
fn scan_loops(
    cx: &super::super::Cx,
    d: u32,
    v: &Value,
    cands: &HashSet<u16>,
    loops: &mut Vec<u16>,
    binds: &mut HashMap<u16, HashSet<Option<u16>>>,
    drops: &mut HashMap<u16, Vec<usize>>,
) {
    let data = cx.data;
    let vars = data.def(d).variables();
    let block_ops = |ops: &[Value], loops: &[u16], drops: &mut HashMap<u16, Vec<usize>>| {
        if loops.is_empty() {
            return;
        }
        for op in ops {
            let o = op.unspan();
            let local = match o {
                Value::Set(lv, rhs) if matches!(rhs.unspan(), Value::Null) => Some(*lv),
                Value::Call(f, args)
                    if (*f as usize) < data.definitions.len()
                        && matches!(
                            data.def(*f).name(),
                            "OpFreeRef" | "OpFreeRefIfDistinct" | "OpFreeRefTag"
                        ) =>
                {
                    match args.first().map(Value::unspan) {
                        Some(Value::Var(lv)) => Some(*lv),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(lv) = local
                && cands.contains(&lv)
            {
                drops
                    .entry(lv)
                    .or_default()
                    .push(std::ptr::from_ref(o) as usize);
            }
        }
    };
    match v.unspan() {
        Value::Loop(b) => {
            loops.push(b.scope);
            block_ops(&b.operators, loops, drops);
            for op in &b.operators {
                scan_loops(cx, d, op, cands, loops, binds, drops);
            }
            loops.pop();
        }
        Value::Block(b) => {
            block_ops(&b.operators, loops, drops);
            for op in &b.operators {
                scan_loops(cx, d, op, cands, loops, binds, drops);
            }
        }
        Value::Set(lv, rhs) if cands.contains(lv) => {
            if !matches!(rhs.unspan(), Value::Null)
                && crate::use_analysis::opaque_callref_bind(data, d, vars.tp(*lv), rhs.unspan())
                    .is_some()
            {
                binds.entry(*lv).or_default().insert(loops.first().copied());
            }
            scan_loops(cx, d, rhs, cands, loops, binds, drops);
        }
        other => other.for_each_child(&mut |c| scan_loops(cx, d, c, cands, loops, binds, drops)),
    }
}

/// The declaring block's own `Set(v, null)` and end-of-pass free of each admitted local.  A free
/// on a `return`/`continue` path sits in an `Insert`, not directly in the block, and stays.
fn collect_dropped(
    cx: &super::super::Cx,
    b: &Block,
    decl: &HashMap<u16, u16>,
    out: &mut HashSet<usize>,
) {
    for op in &b.operators {
        let v = op.unspan();
        let local = match v {
            Value::Set(lv, rhs) if matches!(rhs.unspan(), Value::Null) => Some(*lv),
            Value::Call(f, args)
                if (*f as usize) < cx.data.definitions.len()
                    && matches!(
                        cx.data.def(*f).name(),
                        "OpFreeRef" | "OpFreeRefIfDistinct" | "OpFreeRefTag"
                    ) =>
            {
                match args.first().map(Value::unspan) {
                    Some(Value::Var(lv)) => Some(*lv),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(lv) = local
            && decl.get(&lv) == Some(&b.scope)
        {
            out.insert(std::ptr::from_ref(v) as usize);
        }
    }
}

impl super::FnWriter<'_> {
    /// A statement the port rewrites: a dropped declaration or free writes nothing, and a
    /// vector-field zero of a kept record empties the vector on a kept pass.  Answers whether
    /// `v` was handled.
    pub(super) fn loop_record_stmt(&mut self, v: &Value, ind: usize) -> bool {
        let at = std::ptr::from_ref(v.unspan()) as usize;
        if self.lr.dropped.contains(&at) {
            return true;
        }
        if !self.lr.zeros.contains(&at) {
            return false;
        }
        let Value::Call(_, args) = v.unspan() else {
            return false;
        };
        let (Some(Value::Var(r)), Some(Value::Int(off))) = (
            args.first().map(Value::unspan),
            args.get(1).map(Value::unspan),
        ) else {
            return false;
        };
        if !self.lr.recs.contains_key(r) {
            return false;
        }
        let var = self.var(*r);
        self.flush(ind);
        self.line(
            ind,
            &format!(
                "if (lrf{r}) loft_set_int4({var}, {off}, INT64_C(0)); else loft_vec_clear(loft_field({var}, {off}));"
            ),
        );
        true
    }
}
