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
}

impl LoopRecordPort {
    pub(crate) fn of(cx: &super::super::Cx, d: u32) -> Self {
        if std::env::var("LOFT_NO_LOOP_RECORD_REFILL").is_ok_and(|v| v != "0")
            || std::env::var("LOFT_NO_LOOP_RECORD").is_ok_and(|v| v != "0")
        {
            return Self::default();
        }
        let lr = crate::opt::loop_records::loop_records(cx.data, cx.stores, d);
        let recs: HashMap<u16, u16> = lr
            .recs
            .iter()
            .filter(|(_, r)| r.refill)
            .map(|(v, r)| (*v, r.loop_scope))
            .collect();
        if recs.is_empty() {
            return Self::default();
        }
        let decl: HashMap<u16, u16> = lr
            .recs
            .iter()
            .filter(|(v, _)| recs.contains_key(v))
            .map(|(v, r)| (*v, r.decl_block))
            .collect();
        let mut dropped = HashSet::new();
        cx.data.def(d).code().walk(&mut |n| {
            if let Value::Block(b) = n {
                collect_dropped(cx, b, &decl, &mut dropped);
            }
        });
        // The analysis answers only the refill records' zeros, and every refill record is ours.
        Self {
            recs,
            dropped,
            zeros: lr.field_zeros,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.recs.is_empty()
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
