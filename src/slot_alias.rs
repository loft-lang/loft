// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! A counted loop's variable lives in its range index's own slot on the interpreter.
//!
//! `for i in a..b { … }` steps a hidden index (`i#index`) and copies it into `i` at the top of
//! every round — `i = {#Iter range: i#index += 1; if … break; i#index}`.  The copy is a
//! `VarInt` and a `PutInt` per round, measured at 15–22 % of a tight loop's time.  Where
//! nothing but that copy ever writes `i`, and nothing inside the body writes the index, the two
//! hold the same value for the whole body, so the slot allocator gives `i` the index's slot
//! (`variables::assign_slots_v2`) and the bytecode generator emits the iterator without the
//! copy (`generate_set`, which sees the two positions agree).  The validator accepts exactly
//! these pairs.  The IR is unchanged, so `--native` — which names variables, not slots — is
//! unchanged too.
//!
//! The index's slot is already reserved for the whole loop: it is bound before the loop and
//! read by the iterator on every round, so it straddles the loop and the allocator's
//! loop-straddle rule (I6) keeps every other local off it.  Sharing it with `i`, whose uses all
//! lie inside the loop, adds no conflict.
//!
//! A write to either would break the agreement, so a pair is declined when `i` is named by
//! anything but a read and its one iterator `Set` — a second `Set` (`i = i * 2`), or an
//! address taken (`OpCreateStack(i)`: a `&integer` argument or a `r = &i` link) — or when the
//! index is written inside the body or has its address taken.  A wrong decline is the copy the
//! loop already pays.
//!
//! One visible difference, in the live debugger only: editing `i` mid-loop now edits the loop's
//! counter, so the loop continues from the edited value, as a C `for` would.
//!
//! `LOFT_NO_LOOP_VAR_ALIAS=1` keeps every copy (`keys::loop_var_alias_enabled`).
use crate::data::{Data, Value};
use crate::generation::hoist;

/// The `(loop variable, range index)` pairs of function `d_nr` that share one slot.  A pure
/// function of the IR: the allocator and the validator each ask it and get the same answer.
#[must_use]
pub fn range_slot_aliases(data: &Data, d_nr: u32) -> Vec<(u16, u16)> {
    if !crate::keys::loop_var_alias_enabled() {
        return Vec::new();
    }
    let def = data.def(d_nr);
    let code = &def.code;
    let vars = &def.variables;
    let create_stack = data.def_nr("OpCreateStack");
    let mut pairs: Vec<(u16, u16, &crate::data::Block)> = Vec::new();
    code.any_node(&mut |n| {
        if let Value::Loop(lp) = n
            && let Ok(rc) = hoist::range_counters(lp, data)
        {
            pairs.push((rc.loop_var, rc.index, lp));
        }
        false
    });
    let mut out = Vec::new();
    for (lv, ix, lp) in pairs {
        if lv == ix
            || lv >= vars.next_var()
            || ix >= vars.next_var()
            || vars.is_argument(lv)
            || vars.is_argument(ix)
            || vars.tp(lv) != vars.tp(ix)
            || out.iter().any(|&(a, b)| a == lv || b == lv || a == ix)
        {
            continue;
        }
        let body = &lp.operators[1..];
        if only_iterator_sets(code, lv, create_stack) && index_kept(code, body, ix, create_stack) {
            out.push((lv, ix));
        }
    }
    out
}

/// Every naming of `lv` in the function is a read outside an `OpCreateStack`, except exactly
/// one `Set` — its iterator's.
fn only_iterator_sets(code: &Value, lv: u16, create_stack: u32) -> bool {
    let mut sets = 0;
    let mut other = false;
    walk(code, false, create_stack, &mut |n, under_addr| match n {
        Value::Var(x) if *x == lv => other |= under_addr,
        Value::Set(x, _) if *x == lv => sets += 1,
        _ if n.names_var_here(lv) => other = true,
        _ => {}
    });
    sets == 1 && !other
}

/// `ix` is never written inside the loop `body` and never has its address taken: every naming
/// is a read, or a `Set` outside the body (its seed, and the step inside the iterator).
fn index_kept(code: &Value, body: &[Value], ix: u16, create_stack: u32) -> bool {
    let mut other = false;
    walk(code, false, create_stack, &mut |n, under_addr| match n {
        Value::Var(x) if *x == ix => other |= under_addr,
        Value::Set(x, _) if *x == ix => {}
        _ if n.names_var_here(ix) => other = true,
        _ => {}
    });
    if other {
        return false;
    }
    !body
        .iter()
        .any(|s| s.any_node(&mut |n| matches!(n, Value::Set(x, _) if *x == ix)))
}

/// Visit every node with whether it stands directly under an `OpCreateStack` — the one IR
/// spelling of a frame slot's address — looking through `Span` position wrappers.
fn walk(v: &Value, under_addr: bool, create_stack: u32, f: &mut impl FnMut(&Value, bool)) {
    f(v, under_addr);
    let addr = matches!(v, Value::Call(d, _) if *d == create_stack)
        || (under_addr && matches!(v, Value::Span(_)));
    v.for_each_child(&mut |c| walk(c, addr, create_stack, f));
}
