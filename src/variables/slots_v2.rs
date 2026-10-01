// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I61 — Stack slot allocator

//! Plan-04 Phase 2 / @PLAN53 cluster 2 — single-pool, scope-blind,
//! **aligned** slot allocator.
//!
//! Replaces the two-zone design in `slots.rs`.  V2 sorts live
//! intervals by `(live_start, var_nr)` and greedy-places each at
//! the lowest `align(tp)`-aligned slot that does not conflict with
//! any still-live interval of compatible `SlotKind` and size (see
//! `SPEC.md § 2` at
//! `doc/claude/plans/finished/04-slot-assignment-redesign/SPEC.md`).
//! Sizes stay tight; alignment is achieved by padding between
//! values, and the lowest-slot search backfills the padding holes
//! with smaller locals.
//!
//! `LOFT_SLOT_V2=mode[:filter]` selects a per-function shadow (see
//! `v2_mode_for`): `validate` runs V2 alongside V1 and asserts the
//! invariants (I1–I8 + alignment) then restores V1; `report` dumps
//! a V1-vs-V2 slot comparison; `drive` keeps the V2 layout (correct
//! execution requires the S4 eval-TOS switch — see the cluster-2
//! fix design).  Without the env var, V2 is inert — V1 drives
//! codegen untouched.

use super::{Function, size};
use crate::data::{Context, Type};
use std::collections::HashMap;

/// Classification of a slot's runtime drop-opcode semantics.  Two
/// RefSlots with matching size share a drop opcode (`OpFreeRef`
/// for 12-B DbRef slots, `OpFreeText` for 24-B Text slots); Inline
/// slots have no drop opcode.  V2's placement refuses reuse
/// between incompatible kinds / sizes, matching invariant I5 in
/// `validate.rs::check_i5_kind_consistency`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    Inline,
    RefSlot,
}

/// V2's allocator output.
#[derive(Clone, Debug, Default)]
pub struct AllocatorResult {
    /// Per-variable slot assignments.  Sorted by `var_nr` for
    /// stable comparison against V1.
    pub slots: Vec<SlotAssignment>,
    /// Function-level high-water mark.  Used by codegen's
    /// function-entry `OpReserveFrame` (§ 3.1 follow-up) and as
    /// the O1 optimality metric.
    pub hwm: u16,
    /// Per-scope zone-1-style reserve, computed as
    /// `max(slot + size) - frame_base(scope)` per § 3.1.  Keeps
    /// bytecode codegen's per-block `OpReserveFrame(var_size)`
    /// emission unchanged.
    pub per_block_var_size: std::collections::HashMap<u16, u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotAssignment {
    pub var_nr: u16,
    pub slot: u16,
}

/// Map a Type to its SlotKind.  Any owned `DbRef`-handle type —
/// Reference, Vector, Hash, Sorted, Index, Radix, Iterator, Enum
/// (struct variant), or RefVar — is `RefSlot`.  Text is also
/// `RefSlot` (size 24 B, uses `OpFreeText`); every RefSlot of size
/// 12 B shares the `OpFreeRef` drop opcode.  Everything else is
/// `Inline`.
pub fn slot_kind(tp: &Type) -> SlotKind {
    match tp {
        // @PLN25 slice (c): `Optional(τ)` shares `τ`'s storage (same size/align per
        // `size`/`align`), so it MUST carry `τ`'s slot kind. Missing this mis-classified a
        // heap-backed `text?`/`vector?` local as `Inline` — losing its drop opcode, which in
        // turn left its live interval un-extended so the allocator coalesced two live
        // nullable locals onto one slot (interp read the wiped slot back empty).
        Type::Optional(inner) => slot_kind(inner),
        Type::Text(_)
        | Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Sorted(_, _, _)
        | Type::Radix(_, _, _)
        | Type::Trie(_, _, _)
        | Type::Iterator(_, _)
        | Type::Enum(_, true, _)
        | Type::RefVar(_) => SlotKind::RefSlot,
        _ => SlotKind::Inline,
    }
}

/// V2 entry point — scope-blind, kind-aware, aligned interval-graph
/// greedy allocator (the SPEC § 2 design).
///
/// Each local is placed at the lowest `align(tp)`-aligned offset
/// ≥ `local_start` that does NOT conflict with any already-placed
/// local, where conflict = overlapping slot range AND (overlapping
/// live interval OR incompatible `SlotKind`/size OR loop-straddle
/// OR non-exact partial overlap).  Sizes stay tight; alignment is
/// padding, and the lowest-slot search backfills the holes with
/// smaller locals.  Argument / SRet slots are pinned as always-live
/// fixed intervals so locals never collide with them.
///
/// Does not mutate `function`.  The caller applies
/// `AllocatorResult` via `apply_v2_result`.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn assign_slots_v2(
    function: &Function,
    local_start: u16,
    aliases: &[(u16, u16)],
) -> AllocatorResult {
    // @PLAN53 cluster 2 — see the fn doc for the algorithm.  This replaced an
    // earlier TOS-reset IR-walk that reused slots across sibling scopes WITHOUT
    // a kind check and so violated I5 (an int and a ref sharing a slot have
    // different drop opcodes).
    struct Iv {
        var_nr: u16,
        ls: u32,
        le: u32,
        size: u16,
        align: u16,
        kind: SlotKind,
        slot: u16,
    }
    // Only an index that gets an interval of its own can lend its slot.
    let aliases: Vec<(u16, u16)> = aliases
        .iter()
        .copied()
        .filter(|&(_, ix)| {
            !function.is_argument(ix)
                && function.first_def(ix) != u32::MAX
                && size(function.tp(ix), &Context::Variable) != 0
        })
        .collect();
    let mut ivs: Vec<Iv> = Vec::new();
    for v in 0..function.next_var() {
        // A loop variable sharing its range index's slot gets no interval of its own: it
        // takes the index's slot below (`slot_alias`).
        if function.is_argument(v) || aliases.iter().any(|&(lv, _)| lv == v) {
            continue;
        }
        let fd = function.first_def(v);
        if fd == u32::MAX {
            continue;
        }
        let tp = function.tp(v);
        let sz = size(tp, &Context::Variable);
        if sz == 0 {
            continue;
        }
        ivs.push(Iv {
            var_nr: v,
            ls: fd,
            le: function.variables[v as usize].last_use,
            size: sz,
            align: u16::from(super::align(tp)).max(1),
            kind: slot_kind(tp),
            slot: 0,
        });
    }
    // PINNED: argument-typed vars (incl. hidden SRet return buffers) already
    // own a fixed slot set by the caller / V1 — V2 must NOT place a local on
    // top of them.  Treat each as an always-live ([0, MAX]) fixed interval the
    // greedy search avoids.  (@PLAN53 cluster 2: without this, V2 placed
    // locals over SRet args' slots → spurious I5/I6.)
    let pinned: Vec<Iv> = (0..function.next_var())
        .filter(|&v| {
            function.is_argument(v) && function.variables[v as usize].stack_pos != u16::MAX
        })
        .map(|v| {
            let tp = function.tp(v);
            Iv {
                var_nr: v,
                ls: 0,
                le: u32::MAX,
                size: size(tp, &Context::Variable),
                align: u16::from(super::align(tp)).max(1),
                kind: slot_kind(tp),
                slot: function.variables[v as usize].stack_pos,
            }
        })
        .collect();
    // Greedy by live-start, then var_nr for determinism.
    ivs.sort_by_key(|i| (i.ls, i.var_nr));
    // Loop scopes (I6): two vars may share a slot only if, for every loop,
    // they are BOTH inside `[s,e)` or BOTH outside — never straddling (a
    // loop-local recurs each iteration, so it conflicts with anything live
    // across the loop boundary).
    let loop_ranges: Vec<(u32, u32)> = {
        let mut seen = std::collections::BTreeSet::new();
        let mut r = Vec::new();
        for v in 0..function.next_var() {
            let sc = function.variables[v as usize].scope;
            if sc != u16::MAX
                && function.is_loop_scope(sc)
                && seen.insert(sc)
                && let Some(range) = function.loop_seq_range(sc)
            {
                r.push(range);
            }
        }
        r
    };
    let mut hwm = local_start;
    for idx in 0..ivs.len() {
        let (sz, al, ls, le, kind) = {
            let i = &ivs[idx];
            (i.size, i.align, i.ls, i.le, i.kind)
        };
        let mut pos = local_start.next_multiple_of(al);
        'search: loop {
            let end = pos + sz;
            for p in pinned.iter().chain(ivs[..idx].iter()) {
                let overlap_range = pos < p.slot + p.size && p.slot < end;
                if !overlap_range {
                    continue;
                }
                // Reuse is only valid as an EXACT slot match (same start+size);
                // any PARTIAL overlap is always a conflict (it would alias only
                // part of a live value).
                let exact = pos == p.slot && sz == p.size;
                let life_overlap = ls <= p.le && p.ls <= le;
                let incompatible = p.kind != kind || p.size != sz;
                // I6: forbid sharing that straddles any loop scope.
                let straddle = loop_ranges.iter().any(|&(s, e)| {
                    let iv_in = ls >= s && le < e;
                    let iv_out = le < s || ls >= e;
                    let p_in = p.ls >= s && p.le < e;
                    let p_out = p.le < s || p.ls >= e;
                    !((iv_in && p_in) || (iv_out && p_out))
                });
                if !exact || life_overlap || incompatible || straddle {
                    // Jump past the blocker, re-align, retry.
                    pos = (p.slot + p.size).next_multiple_of(al);
                    continue 'search;
                }
            }
            break;
        }
        ivs[idx].slot = pos;
        hwm = hwm.max(pos + sz);
    }
    let mut slots: Vec<SlotAssignment> = ivs
        .iter()
        .map(|i| SlotAssignment {
            var_nr: i.var_nr,
            slot: i.slot,
        })
        .collect();
    // The index's slot straddles its loop, so the I6 rule above already kept every other
    // local off it for the loop's whole extent, where all of the loop variable's uses lie.
    for &(lv, ix) in &aliases {
        if let Some(i) = ivs.iter().find(|i| i.var_nr == ix) {
            slots.push(SlotAssignment {
                var_nr: lv,
                slot: i.slot,
            });
        }
    }
    slots.sort_by_key(|s| s.var_nr);
    // Scope-blind: one function-entry reserve (frame_hwm) covers every slot;
    // no per-block reserves.
    AllocatorResult {
        slots,
        hwm,
        per_block_var_size: HashMap::new(),
    }
}

// No synthetic Rust-level unit tests here.  V2 correctness is
// verified against real `.loft` programs:
//
//   - `tests/scripts/96-slot-assign.loft` — patterns historically
//     prone to slot-assignment bugs; executed by `loft_suite` in
//     `tests/wrap.rs`.
//   - `tests/slot_v2_baseline.rs` — 29 Phase-0 fixtures, each a
//     `code!("fn test() { … }")` block exercising one shape
//     (is-capture aliasing, loop-carry, block-return, sibling-scope
//     reuse, late-local-in-nested-for, …).
//   - `tests/issues.rs::p178_is_capture_slot_alias`
//   - `tests/issues.rs::p185_slot_alias_on_late_local_in_nested_for`
//
// A slot-inspection suite can live in a separate integration test
// that loads specific `.loft` files from `tests/scripts/` and
// asserts structural slot properties — it never hand-constructs
// `Function` objects, it points to the `.loft` file + function
// name that holds the problem.

/// Apply an `AllocatorResult` to the function: set each variable's
/// `stack_pos`, and populate `var_size` on every Block node in the
/// IR tree so codegen's existing `OpReserveFrame(var_size)` call
/// per block (`src/state/codegen.rs:1896`) reserves the right
/// amount of space.  Loop nodes keep `var_size = 0` — their
/// internal vars are placed at TOS one-by-one during codegen's
/// walk and the loop wrapper does not pre-reserve.
pub fn apply_v2_result(
    function: &mut Function,
    code: &mut crate::data::Value,
    result: &AllocatorResult,
) {
    // Plan-22 02d-vii follow-up — `LOFT_LOG=slots:<fn>` trace.
    // Logs each slot decision + WHY any var was skipped, BEFORE
    // applying the result.  Helps diagnose "Incorrect var X[65535]"
    // codegen panics where the allocator silently skipped X.
    if let Some(target) = crate::log_config::slots_trace_target()
        && function.name.contains(&target)
    {
        eprintln!("[slots] fn={} hwm={}", function.name, result.hwm);
        let assigned: std::collections::HashSet<u16> =
            result.slots.iter().map(|s| s.var_nr).collect();
        for v_nr in 0..function.next_var() {
            let name = function.name(v_nr);
            let tp = function.tp(v_nr);
            if let Some(s) = result.slots.iter().find(|s| s.var_nr == v_nr) {
                eprintln!(
                    "[slots]   ASSIGN  v_nr={v_nr:<3} name={name:<20} slot={slot:<4} type={tp:?}",
                    slot = s.slot,
                );
            } else if !assigned.contains(&v_nr) {
                let reason = slot_skip_reason(function, v_nr);
                eprintln!(
                    "[slots]   SKIP    v_nr={v_nr:<3} name={name:<20} type={tp:?}  reason={reason}",
                );
            }
        }
    }
    for s in &result.slots {
        function.set_stack_pos(s.var_nr, s.slot);
    }
    apply_var_size(code, &result.per_block_var_size);
}

/// Plan-22 02d-vii — explain why `assign_slots_v2` skipped a var.
/// Mirrors the bail-out conditions in the allocator's interval loop.
fn slot_skip_reason(function: &Function, v_nr: u16) -> &'static str {
    if (v_nr as usize) >= function.next_var() as usize {
        return "v_nr out of range";
    }
    if function.is_argument(v_nr) {
        return "is_argument (allocated externally by caller)";
    }
    let tp = function.tp(v_nr);
    let sz = crate::variables::size(tp, &crate::variables::Context::Variable);
    if sz == 0 {
        return "size(type) == 0 (zero-byte var)";
    }
    if function.first_def(v_nr) == u32::MAX {
        return "no first_def (no Set IR for this var — read-only or unused)";
    }
    "no Set IR walked (var referenced but never assigned via Value::Set)"
}

fn apply_var_size(val: &mut crate::data::Value, sizes: &HashMap<u16, u16>) {
    use crate::data::Value;
    match val {
        Value::Block(bl) => {
            bl.var_size = sizes.get(&bl.scope).copied().unwrap_or(0);
            for op in &mut bl.operators {
                apply_var_size(op, sizes);
            }
        }
        Value::Loop(lp) => {
            lp.var_size = 0;
            for op in &mut lp.operators {
                apply_var_size(op, sizes);
            }
        }
        Value::If(c, t, f) => {
            apply_var_size(c, sizes);
            apply_var_size(t, sizes);
            apply_var_size(f, sizes);
        }
        Value::Set(_, inner) | Value::Drop(inner) | Value::Return(inner) => {
            apply_var_size(inner, sizes);
        }
        Value::Call(_, args) | Value::CallRef(_, args) => {
            for a in args {
                apply_var_size(a, sizes);
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                apply_var_size(op, sizes);
            }
        }
        Value::Iter(_, c, n, e) => {
            apply_var_size(c, sizes);
            apply_var_size(n, sizes);
            apply_var_size(e, sizes);
        }
        Value::Span(b) => apply_var_size(&mut b.1, sizes),
        _ => {}
    }
}
