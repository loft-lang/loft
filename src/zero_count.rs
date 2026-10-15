// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-ZeroCount` — a text's CHARACTER count compared against the zero boundary is its
//! BYTE count compared against it.
//!
//! `len(t) > 0` counts every character of `t` (`chars().count()`) to learn whether there is
//! one.  A text has no characters exactly when it has no bytes, the absent text included
//! (`@FR-L-Null-Text`: both counts answer 0 for it), and neither count is ever negative, so
//! every comparison that only tells zero from non-zero answers the same over either count —
//! and the byte count is one length read.  The comparisons admitted: `len(t) ⋈ 0` and
//! `0 ⋈ len(t)` for `⋈ ∈ { ==, !=, <, <= }` (`>` and `>=` arrive swapped), and the two
//! spellings of the boundary at 1, `len(t) >= 1` (`OpLeInt(1, len)`) and `len(t) < 1`
//! (`OpLtInt(len, 1)`).  `len(t) == 1` and `len(t) > 1` count characters and are left alone.
//! Both spellings of the count are folded: the stdlib call `t_4text_len` and its inlined
//! operator `OpLengthText`.  One IR rewrite on the closed program, so both backends get it;
//! `LOFT_NO_ZERO_COUNT=1` is the switch.
use crate::data::{Data, DefType, Value};

struct Ops {
    eq: u32,
    ne: u32,
    lt: u32,
    le: u32,
    /// `(character count, byte count)` per spelling: the call and the operator.
    counts: [(u32, u32); 2],
}

impl Ops {
    fn lookup(data: &Data) -> Option<Self> {
        let nr = |n: &str| {
            let d = data.def_nr(n);
            (d != u32::MAX).then_some(d)
        };
        Some(Self {
            eq: nr("OpEqInt")?,
            ne: nr("OpNeInt")?,
            lt: nr("OpLtInt")?,
            le: nr("OpLeInt")?,
            counts: [
                (nr("t_4text_len")?, nr("t_4text_size")?),
                (nr("OpLengthText")?, nr("OpSizeText")?),
            ],
        })
    }

    /// The byte count that answers for character count `d`, when `d` is one.
    fn bytes_for(&self, d: u32) -> Option<u32> {
        self.counts.iter().find(|(c, _)| *c == d).map(|(_, b)| *b)
    }
}

fn int(v: &Value) -> Option<i32> {
    match v.unspan() {
        Value::Int(n) => Some(*n),
        _ => None,
    }
}

/// Does comparison `op(a, b)`, with the count on side `count_left`, only tell zero from
/// non-zero?
fn at_boundary(ops: &Ops, op: u32, k: i32, count_left: bool) -> bool {
    if k == 0 {
        return op == ops.eq || op == ops.ne || op == ops.lt || op == ops.le;
    }
    // `1 <= len` (len >= 1) and `len < 1`: the boundary between 0 and 1.
    k == 1 && ((op == ops.le && !count_left) || (op == ops.lt && count_left) || op == ops.eq)
}

/// Fold every admitted comparison of the program; a no-op under the switch.
pub fn rewrite_program(data: &mut Data) {
    if !crate::keys::zero_count_enabled() {
        return;
    }
    let Some(ops) = Ops::lookup(data) else {
        return;
    };
    let mut fired = 0usize;
    for d_nr in 0..data.definitions.len() {
        let d = d_nr as u32;
        if data.def_type(d) != DefType::Function || !data.def(d).is_loft_defined() {
            continue;
        }
        let mut code = std::mem::replace(&mut data.definitions[d_nr].code, Value::Null);
        code.map_nodes(&mut |n| {
            let Value::Call(op, args) = n else {
                return;
            };
            let [a, b] = args.as_mut_slice() else {
                return;
            };
            let (count, k, count_left) = match (int(a), int(b)) {
                (None, Some(k)) => (a, k, true),
                (Some(k), None) => (b, k, false),
                _ => return,
            };
            if !at_boundary(&ops, *op, k, count_left) {
                return;
            }
            if let Value::Call(c, cargs) = count.unspan_mut()
                && cargs.len() == 1
                && let Some(bytes) = ops.bytes_for(*c)
            {
                *c = bytes;
                fired += 1;
            }
        });
        data.definitions[d_nr].code = code;
    }
    crate::rewrite_census::fired("R-ZeroCount", fired);
}
