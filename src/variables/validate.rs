// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I61 — Stack slot allocator

//! Slot validation and variable table dump: assert no overlapping live intervals,
//! scope-parent analysis, and debug output.
//!
//! Every helper here is ultimately reachable from `validate_slots`, which is
//! only called under `#[cfg(any(debug_assertions, test))]`.  The loft package
//! disables `debug_assertions` in its hot interpreter profile
//! (`[profile.dev.package.loft]`), so under the default clippy invocation the
//! whole helper tree appears unused.  `#![allow(dead_code)]` acknowledges that:
//! the items are exercised by `cargo test`, by any caller enabling
//! `debug_assertions`, and by `LOFT_SLOT_V2=validate` runtime callers.
#![allow(dead_code)]

use crate::data::Value;
use crate::data::{Context, Data, Type};

use std::collections::HashMap;
use std::io::{Error, Write};

use super::Variable;
use super::slots_v2::{SlotKind, slot_kind};
use super::{Function, size};

fn short_type(tp: &Type) -> String {
    match tp {
        Type::Optional(inner) => format!("{}?", short_type(inner)),
        Type::Unknown(_) => "?".to_string(),
        Type::Null => "null".to_string(),
        Type::Void | Type::Never => "void".to_string(),
        Type::Integer(_) => "int".to_string(),
        Type::Boolean => "bool".to_string(),
        Type::Float => "float".to_string(),
        Type::Single => "single".to_string(),
        Type::Character => "char".to_string(),
        Type::Text(_) => "text".to_string(),
        Type::Keys => "keys".to_string(),
        Type::Enum(t, _, _) => format!("enum({t})"),
        Type::Reference(t, _) => format!("ref({t})"),
        Type::RefVar(inner) => format!("&{}", short_type(inner)),
        Type::Vector(inner, _) => format!("vec<{}>", short_type(inner)),
        Type::Routine(t) => format!("routine({t})"),
        Type::Iterator(inner, _) => format!("iter<{}>", short_type(inner)),
        Type::Sorted(t, _, _) => format!("sorted({t})"),
        Type::Index(t, _, _) => format!("index({t})"),
        Type::Radix(t, _, _) => format!("spatial({t})"),
        Type::Trie(t, _, _) => format!("trie({t})"),
        Type::Hash(t, _, _) => format!("hash({t})"),
        Type::Function(..) => "fn".to_string(),
        Type::Rewritten(inner) => format!("~{}", short_type(inner)),
        Type::Tuple(elems) => {
            let es: Vec<String> = elems.iter().map(short_type).collect();
            format!("({})", es.join(","))
        }
    }
}

/// Build a map from each scope number → its parent scope number, by walking the IR tree.
/// Scopes with no parent (e.g. the root block) are not in the map.
///
/// If a scope number appears more than once in the tree (e.g. a synthetic block sharing
/// a scope number with an outer block), keep the first-seen parent — it is the
/// structurally outermost one.  Never insert a self-loop (`scope == parent`).
fn build_scope_parents(val: &Value, parent: u16, parents: &mut HashMap<u16, u16>) {
    match val {
        Value::Block(bl) | Value::Loop(bl) => {
            // Guard: never insert a self-loop; keep the first-seen parent.
            if bl.scope != parent {
                parents.entry(bl.scope).or_insert(parent);
            }
            for op in &bl.operators {
                build_scope_parents(op, bl.scope, parents);
            }
        }
        Value::If(cond, t, f) => {
            build_scope_parents(cond, parent, parents);
            build_scope_parents(t, parent, parents);
            build_scope_parents(f, parent, parents);
        }
        Value::Set(_, inner) | Value::Drop(inner) | Value::Return(inner) => {
            build_scope_parents(inner, parent, parents);
        }
        Value::Call(_, args) | Value::CallRef(_, args) => {
            for a in args {
                build_scope_parents(a, parent, parents);
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                build_scope_parents(op, parent, parents);
            }
        }
        Value::Iter(_, create, next, extra) => {
            build_scope_parents(create, parent, parents);
            build_scope_parents(next, parent, parents);
            build_scope_parents(extra, parent, parents);
        }
        Value::Span(b) => build_scope_parents(&b.1, parent, parents),
        _ => {}
    }
}

/// Returns true if `ancestor` is a strict ancestor of `child` in the scope tree.
fn is_scope_ancestor(ancestor: u16, child: u16, parents: &HashMap<u16, u16>) -> bool {
    let mut cur = child;
    let mut steps = 0u32;
    loop {
        assert!(
            steps <= 10_000,
            "is_scope_ancestor: cycle in scope parent map after {steps} steps \
             (ancestor={ancestor}, child={child}, cur={cur}). \
             This indicates build_scope_parents inserted a scope with itself as parent."
        );
        steps += 1;
        match parents.get(&cur) {
            Some(&p) if p == ancestor => return true,
            Some(&p) if p == cur => return false, // self-loop → not an ancestor
            Some(&p) => cur = p,
            None => return false,
        }
    }
}

/// Returns true if scope SA and scope SB can be physically concurrent, i.e., one is an
/// ancestor of the other (or they are equal).  Variables in sibling branches of the IR tree
/// cannot be simultaneously on the stack, so byte-range overlap between them is allowed.
fn scopes_can_conflict(sa: u16, sb: u16, parents: &HashMap<u16, u16>) -> bool {
    // u16::MAX = "no scope" (global or argument) — always treat as possible conflict.
    if sa == u16::MAX || sb == u16::MAX {
        return true;
    }
    sa == sb || is_scope_ancestor(sa, sb, parents) || is_scope_ancestor(sb, sa, parents)
}

/// Scan `vars` for the first pair of variables whose stack slots AND live intervals both
/// overlap AND whose scopes are in the same execution branch (i.e. one scope is an ancestor
/// of the other).  Variables in sibling branches cannot be simultaneously on the stack.
pub(super) fn find_conflict(
    vars: &[Variable],
    scope_parents: &HashMap<u16, u16>,
) -> Option<(usize, u16, usize, u16)> {
    for left_idx in 0..vars.len() {
        let left = &vars[left_idx];
        if left.stack_pos == u16::MAX || left.first_def == u32::MAX {
            continue;
        }
        let left_size = size(&left.type_def, &Context::Variable);
        if left_size == 0 {
            continue;
        }
        let left_slot_end = left.stack_pos + left_size;
        for (right_idx, right) in vars.iter().enumerate().skip(left_idx + 1) {
            if right.stack_pos == u16::MAX || right.first_def == u32::MAX {
                continue;
            }
            let right_size = size(&right.type_def, &Context::Variable);
            if right_size == 0 {
                continue;
            }
            let right_slot_end = right.stack_pos + right_size;
            let slots_overlap = left.stack_pos < right_slot_end && right.stack_pos < left_slot_end;
            let intervals_overlap =
                left.first_def <= right.last_use && right.first_def <= left.last_use;
            if slots_overlap && intervals_overlap {
                // Same name + same slot = sequential reuse of one logical variable across
                // block scopes.  The compiler creates a fresh Variable entry per block but
                // assigns it the same slot; the overlap in live ranges is a conservative
                // artefact of compute_intervals, not a real runtime conflict.
                if left.name == right.name && left.stack_pos == right.stack_pos {
                    continue;
                }
                // Variables in sibling (or cousin) scope branches cannot physically overlap:
                // one block exits before the other starts.  The live-interval overlap is an
                // artefact of OpFreeRef/OpFreeText tracking across scope boundaries.
                if !scopes_can_conflict(left.scope, right.scope, scope_parents) {
                    continue;
                }
                // S34 Option A: a work variable moved down to the same slot as an outer
                // variable is marked skip_free so that only the outer variable emits
                // OpFreeRef.  The slot is intentionally aliased — not a real conflict.
                if left.skip_free || right.skip_free {
                    continue;
                }
                return Some((left_idx, left_slot_end, right_idx, right_slot_end));
            }
        }
    }
    None
}

/// Plan-04 Phase 2a: runtime-contract axis for slot-reuse compatibility.
///
/// A slot's `SlotKind` determines which drop opcode fires at scope
/// exit (`OpFreeText` for `RefSlot` of size 24, `OpFreeRef` for
/// `RefSlot` of size 12, none for `Inline`).  V2's placement allows
/// dead-slot reuse only between compatible kinds / sizes (SPEC.md
/// Compute the frame's `local_start` — the first byte above the
/// argument + return-address prefix.  Matches the formula in
/// `src/scopes.rs:156–164`: `sum(arg sizes) + 4`.
fn compute_local_start(function: &Function) -> u16 {
    let mut total: u16 = 0;
    for &a in &function.arguments() {
        total += size(function.tp(a), &Context::Argument);
    }
    total + 4
}

/// I2 — argument isolation.  Every non-argument variable must be
/// placed at or above `local_start`, so no local can overlap the
/// argument region.  Returns `Some((idx, local_start))` on the
/// first violation.
fn check_i2_arg_isolation(vars: &[Variable], local_start: u16) -> Option<(usize, u16)> {
    for (idx, v) in vars.iter().enumerate() {
        if v.argument {
            continue;
        }
        if v.stack_pos == u16::MAX {
            continue;
        }
        let sz = size(&v.type_def, &Context::Variable);
        if sz == 0 {
            continue;
        }
        if v.stack_pos < local_start {
            return Some((idx, local_start));
        }
    }
    None
}

/// I4 — every defined variable is placed.  Any variable with a
/// populated live interval and non-zero size must have a valid
/// `stack_pos`.  Returns the var index on the first violation.
fn check_i4_every_var_placed(vars: &[Variable]) -> Option<usize> {
    for (idx, v) in vars.iter().enumerate() {
        if v.argument {
            continue;
        }
        if v.first_def == u32::MAX {
            continue;
        }
        let sz = size(&v.type_def, &Context::Variable);
        if sz == 0 {
            continue;
        }
        if v.stack_pos == u16::MAX {
            return Some(idx);
        }
    }
    None
}

/// I8 — total-claim capacity bound.  Returns the first placed local whose
/// byte range `[stack_pos, stack_pos + size)` exceeds the u16-addressable
/// frame (`> u16::MAX`).  Slot offsets are u16, so anything past that has
/// overflowed the claim a function can address — a sign of alignment-padding
/// bloat or a runaway offset (@PLAN53 cluster 2).
fn check_i8_total_claim(vars: &[Variable]) -> Option<usize> {
    vars.iter().enumerate().find_map(|(idx, v)| {
        if v.argument || v.stack_pos == u16::MAX {
            return None;
        }
        let end = u32::from(v.stack_pos) + u32::from(size(&v.type_def, &Context::Variable));
        (end > u32::from(u16::MAX)).then_some(idx)
    })
}

/// @PLAN53 cluster 2 — alignment invariant for the **aligned V2 layout only**.
/// Every placed local must sit at an `align(tp)`-aligned slot offset so an
/// `addr`/`addr_mut::<T>` into it is sound.  Panics on the first violation.
///
/// Run ONLY on V2's output via the `LOFT_SLOT_V2=validate` shadow path — NOT
/// in the unconditional `validate_slots` path, because V1's tight layout is
/// legitimately unaligned and would trip it.
pub fn validate_alignment(function: &Function) {
    for (idx, v) in function.variables.iter().enumerate() {
        if v.argument || v.stack_pos == u16::MAX {
            continue;
        }
        let al = u16::from(crate::variables::align(&v.type_def)).max(1);
        assert!(
            v.stack_pos % al == 0,
            "[ALIGN] variable '{}' (idx={idx}) in function '{}' at slot {} is not \
             {al}-aligned (type {}) — aligned-V2-allocator bug",
            v.name,
            function.name,
            v.stack_pos,
            short_type(&v.type_def),
        );
    }
}

/// I5 — kind-consistency on overlapping-slot reuse.  For any pair
/// of variables whose slot ranges overlap spatially AND whose live
/// intervals are disjoint (the reuse case), kinds must match; for
/// RefSlot reuse, `(slot, size)` must coincide fully.
///
/// Returns `Some((left_idx, right_idx))` on the first violation.
fn check_i5_kind_consistency(vars: &[Variable]) -> Option<(usize, usize)> {
    for li in 0..vars.len() {
        let l = &vars[li];
        if l.stack_pos == u16::MAX || l.first_def == u32::MAX {
            continue;
        }
        let l_size = size(&l.type_def, &Context::Variable);
        if l_size == 0 {
            continue;
        }
        let l_end = l.stack_pos + l_size;
        let l_kind = slot_kind(&l.type_def);
        for (ri, r) in vars.iter().enumerate().skip(li + 1) {
            if r.stack_pos == u16::MAX || r.first_def == u32::MAX {
                continue;
            }
            let r_size = size(&r.type_def, &Context::Variable);
            if r_size == 0 {
                continue;
            }
            let r_end = r.stack_pos + r_size;
            let slots_overlap = l.stack_pos < r_end && r.stack_pos < l_end;
            if !slots_overlap {
                continue;
            }
            // Live-interval overlap is I1's concern — not I5's.
            let intervals_overlap = l.first_def <= r.last_use && r.first_def <= l.last_use;
            if intervals_overlap {
                continue;
            }
            let r_kind = slot_kind(&r.type_def);
            if l_kind != r_kind {
                return Some((li, ri));
            }
            if matches!(l_kind, SlotKind::RefSlot)
                && (l.stack_pos != r.stack_pos || l_size != r_size)
            {
                return Some((li, ri));
            }
        }
    }
    None
}

/// I7 — declared-scope zone-1 frame consistency.
///
/// For every local `V` with:
/// - declared scope `S` that corresponds to a non-loop Block in the IR,
/// - size ≤ 8 bytes (zone-1-qualifying under V1's allocator),
///
/// `V.stack_pos` must lie in `[frame_base(S), frame_base(S) + block.var_size)`
/// where `frame_base(S)` is the current IR-walk TOS at entry to S's
/// Block node, including zone-1 pre-reserves of all ancestor Blocks.
///
/// **What this catches:** allocators that place a zone-1 var outside
/// its declared scope's pre-reserved region — the exact failure
/// class that under the V2-drive attempt (plan-04 retraction) put
/// body-scope variables at inner-match-arm TOS, producing `Incorrect
/// var X[slot] versus TOS` runtime panics.  V1's zone-1 pre-claim
/// loop (`slots.rs:85-171`) satisfies this invariant by construction;
/// any future allocator that changes placement must continue to.
///
/// **What this does NOT catch:** zone-2 (large-type) placement —
/// V1 places these sequentially, interleaved with child-scope frames,
/// so there is no clean per-scope bound.  Loop-scope placement is
/// also excluded because loops share the eval stack with the enclosing
/// frame and skip zone 1 entirely.
///
/// Returns `Some((idx, frame_base, frame_top))` on the first violation.
fn check_i7_scope_frame(
    vars: &[Variable],
    function: &Function,
    code: &Value,
    local_start: u16,
) -> Option<(usize, u16, u16)> {
    let frames = compute_scope_frame_bases(code, local_start);
    for (idx, v) in vars.iter().enumerate() {
        if v.argument || v.stack_pos == u16::MAX {
            continue;
        }
        if v.scope == u16::MAX || v.scope == 0 {
            continue;
        }
        // I7 only governs zone-1 (≤ 8 byte) vars in non-loop scopes.
        // Zone-2 vars are sequentially placed across scope boundaries
        // and have no per-scope upper bound.  Loops skip zone 1 entirely.
        let sz = size(&v.type_def, &Context::Variable);
        if sz == 0 || sz > 8 {
            continue;
        }
        if function.is_loop_scope(v.scope) {
            continue;
        }
        let Some(&(base, zone1_size)) = frames.get(&v.scope) else {
            continue;
        };
        let top = base + zone1_size;
        if v.stack_pos < base || v.stack_pos + sz > top {
            return Some((idx, base, top));
        }
    }
    None
}

/// Walk the IR tree and record each non-loop Block's frame base
/// (inherited from parent's frame base + parent's zone-1 size) plus
/// its zone-1 `var_size`.  `frame_base(root) = local_start`.  Returns
/// `HashMap<scope, (frame_base, zone1_size)>`.
///
/// Only non-loop Blocks are recorded — loops have `var_size = 0` and
/// no zone-1 region, so I7 skips them.
fn compute_scope_frame_bases(code: &Value, local_start: u16) -> HashMap<u16, (u16, u16)> {
    let mut frames: HashMap<u16, (u16, u16)> = HashMap::new();
    walk_frame_bases(code, local_start, &mut frames);
    frames
}

fn walk_frame_bases(val: &Value, current_base: u16, frames: &mut HashMap<u16, (u16, u16)>) {
    match val {
        Value::Block(bl) => {
            let zone1 = bl.var_size;
            frames.entry(bl.scope).or_insert((current_base, zone1));
            // Child scopes inherit `current_base + zone1` as their base
            // — zone 1 is pre-reserved before any child scope starts.
            let child_base = current_base + zone1;
            for op in &bl.operators {
                walk_frame_bases(op, child_base, frames);
            }
        }
        Value::Loop(lp) => {
            // Loops have var_size = 0 and are excluded from I7; still
            // recurse into children so nested blocks are recorded.
            for op in &lp.operators {
                walk_frame_bases(op, current_base, frames);
            }
        }
        Value::If(c, t, f) => {
            walk_frame_bases(c, current_base, frames);
            walk_frame_bases(t, current_base, frames);
            walk_frame_bases(f, current_base, frames);
        }
        Value::Set(_, inner) | Value::Drop(inner) | Value::Return(inner) => {
            walk_frame_bases(inner, current_base, frames);
        }
        Value::Call(_, args) | Value::CallRef(_, args) => {
            for a in args {
                walk_frame_bases(a, current_base, frames);
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                walk_frame_bases(op, current_base, frames);
            }
        }
        Value::Iter(_, c, n, e) => {
            walk_frame_bases(c, current_base, frames);
            walk_frame_bases(n, current_base, frames);
            walk_frame_bases(e, current_base, frames);
        }
        Value::Span(b) => walk_frame_bases(&b.1, current_base, frames),
        _ => {}
    }
}

/// I6 — loop-iteration safety (defence-in-depth against
/// `compute_intervals` regressions).  For every loop scope `L`
/// with seq range `[s, e]`, every pair `(V, W)` sharing a slot
/// (same `stack_pos`) must satisfy one of:
///   - both entirely inside `L`,
///   - both entirely outside `L`,
///   - lifetimes disjoint (already covered by I1).
///
/// Returns `Some((loop_scope, left_idx, right_idx))` on the first
/// violation.
fn check_i6_loop_iteration(vars: &[Variable], function: &Function) -> Option<(u16, usize, usize)> {
    // Enumerate the loop scopes referenced by any placed variable.
    let mut loop_scopes: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
    for v in vars {
        if v.stack_pos == u16::MAX {
            continue;
        }
        if v.scope != u16::MAX && function.is_loop_scope(v.scope) {
            loop_scopes.insert(v.scope);
        }
    }
    for loop_scope in &loop_scopes {
        let Some((s, e)) = function.loop_seq_range(*loop_scope) else {
            continue;
        };
        for li in 0..vars.len() {
            let l = &vars[li];
            if l.stack_pos == u16::MAX || l.first_def == u32::MAX {
                continue;
            }
            for (ri, r) in vars.iter().enumerate().skip(li + 1) {
                if r.stack_pos == u16::MAX || r.first_def == u32::MAX {
                    continue;
                }
                if l.stack_pos != r.stack_pos {
                    continue;
                }
                let disjoint = l.last_use < r.first_def || r.last_use < l.first_def;
                if disjoint {
                    continue;
                }
                let l_inside = l.first_def >= s && l.last_use < e;
                let l_outside = l.last_use < s || l.first_def >= e;
                let r_inside = r.first_def >= s && r.last_use < e;
                let r_outside = r.last_use < s || r.first_def >= e;
                if l_inside && r_inside {
                    continue;
                }
                if l_outside && r_outside {
                    continue;
                }
                return Some((*loop_scope, li, ri));
            }
        }
    }
    None
}

/// Assert that slot placements satisfy invariants I1–I6 from
/// [`SPEC.md § 5a`](../doc/claude/plans/finished/04-slot-assignment-redesign/SPEC.md).
///
/// Unconditionally compiled so the `LOFT_SLOT_V2=validate` shadow
/// path (scopes.rs) can invoke it from any build profile.  The
/// call site in `state/codegen.rs` remains gated on
/// `#[cfg(any(debug_assertions, test))]`, so release builds pay
/// no cost unless the env var opts in.
///
/// On the first failure, logs the full variable table and IR code
/// before panicking with a distinct `[I1]` … `[I6]` prefix so the
/// failing invariant is self-identifying.
/// `scope_blind` — when true (the V2 shadow path), skip I7 (declared-scope
/// zone-1 frame): V2 is a scope-blind single-pool allocator, so the V1
/// two-zone "slot within its scope's zone" invariant does not apply.  All
/// other invariants (I1 overlap, I2, I5, I6, I8) hold for both allocators.
pub fn validate_slots(function: &Function, data: &Data, def_nr: u32, scope_blind: bool) {
    let vars = &function.variables;
    let local_start = compute_local_start(function);

    // ── I4: every defined variable is placed ─────────────────────────────
    if let Some(idx) = check_i4_every_var_placed(vars) {
        let v = &vars[idx];
        panic!(
            "[I4] variable '{}' (idx={idx}) in function '{}' has \
             first_def={} and size={} but no slot assigned",
            v.name,
            function.name,
            v.first_def,
            size(&v.type_def, &Context::Variable),
        );
    }

    // ── I8: total-claim capacity bound ───────────────────────────────────
    // Slot offsets are u16, so a slot whose byte range exceeds u16::MAX has
    // overflowed the claim a function can address (and `frame_hwm`'s
    // saturating_add would silently mask it).  Guards alignment-padding
    // bloat / runaway offsets pushing a slot outside the function's total
    // claim (@PLAN53 cluster 2).
    if let Some(idx) = check_i8_total_claim(vars) {
        let v = &vars[idx];
        let end = u32::from(v.stack_pos) + u32::from(size(&v.type_def, &Context::Variable));
        panic!(
            "[I8] variable '{}' (idx={idx}) in function '{}' has slot \
             [{}, {end}) exceeding the u16-addressable frame claim \
             (limit {}); likely alignment-padding bloat or a runaway offset",
            v.name,
            function.name,
            v.stack_pos,
            u16::MAX,
        );
    }

    // ── I2: argument isolation ───────────────────────────────────────────
    if let Some((idx, ls)) = check_i2_arg_isolation(vars, local_start) {
        let v = &vars[idx];
        panic!(
            "[I2] variable '{}' in function '{}' at slot {} dips \
             below local_start={ls} (argument region)",
            v.name, function.name, v.stack_pos,
        );
    }

    // ── I5: kind-consistency on overlapping-slot reuse ───────────────────
    if let Some((li, ri)) = check_i5_kind_consistency(vars) {
        let l = &vars[li];
        let r = &vars[ri];
        panic!(
            "[I5] variables '{}' ({} slot [{}, {})) and '{}' ({} slot [{}, {})) \
             in function '{}' share overlapping slot ranges across disjoint \
             lifetimes but differ in kind or size",
            l.name,
            short_type(&l.type_def),
            l.stack_pos,
            l.stack_pos + size(&l.type_def, &Context::Variable),
            r.name,
            short_type(&r.type_def),
            r.stack_pos,
            r.stack_pos + size(&r.type_def, &Context::Variable),
            function.name,
        );
    }

    // ── I7: declared-scope zone-1 frame consistency ─────────────────────
    // Skipped for the scope-blind V2 allocator (zone frames are a V1 concept).
    if let Some((idx, base, top)) = (!scope_blind)
        .then(|| check_i7_scope_frame(vars, function, &data.def(def_nr).code, local_start))
        .flatten()
    {
        let v = &vars[idx];
        let sz = size(&v.type_def, &Context::Variable);
        panic!(
            "[I7] variable '{}' (scope {}, size {sz}B) in function '{}' has slot {} \
             outside zone-1 frame [{base}, {top}).  The allocator placed the var \
             outside its declared scope's pre-reserved region; codegen will emit \
             reads that panic with `Incorrect var X[{}] versus TOS` at runtime.  \
             Likely cause: allocator used an inner scope's TOS instead of the \
             declared scope's frame_base.",
            v.name, v.scope, function.name, v.stack_pos, v.stack_pos,
        );
    }

    // ── I6: loop-iteration safety ────────────────────────────────────────
    if let Some((loop_scope, li, ri)) = check_i6_loop_iteration(vars, function) {
        let l = &vars[li];
        let r = &vars[ri];
        panic!(
            "[I6] variables '{}' (live [{}, {}]) and '{}' (live [{}, {}]) \
             in function '{}' share slot {} but straddle loop scope {} \
             (one inside, one outside, or one crossing the boundary)",
            l.name,
            l.first_def,
            l.last_use,
            r.name,
            r.first_def,
            r.last_use,
            function.name,
            l.stack_pos,
            loop_scope,
        );
    }

    // ── I1: spatial+temporal overlap (historical check) ──────────────────
    // Build scope parent map from the IR tree so find_conflict can skip sibling-branch conflicts.
    let mut scope_parents: HashMap<u16, u16> = HashMap::new();
    build_scope_parents(&data.def(def_nr).code, u16::MAX, &mut scope_parents);

    let Some((left_idx, left_slot_end, right_idx, right_slot_end)) =
        find_conflict(vars, &scope_parents)
    else {
        return;
    };
    let left = &vars[left_idx];
    let right = &vars[right_idx];
    // Log full diagnostics before panicking so the cause is immediately clear.
    eprintln!("\n=== Slot conflict in function '{}' ===\n", function.name);
    eprintln!("  Conflicting pair:");
    eprintln!(
        "  * '{}'  slot [{}, {left_slot_end})  live [{}, {}]",
        left.name, left.stack_pos, left.first_def, left.last_use
    );
    eprintln!(
        "  * '{}'  slot [{}, {right_slot_end})  live [{}, {}]",
        right.name, right.stack_pos, right.first_def, right.last_use
    );
    eprintln!();
    eprintln!(
        "  {:<4} {:<2} {:<20} {:<14} {:<16} {:<12} {:<12} {:<14}",
        "#", "", "name", "type", "scope", "slot", "pre", "live"
    );
    eprintln!("  {}", "-".repeat(96));
    for (idx, var) in vars.iter().enumerate() {
        let vs = size(&var.type_def, &Context::Variable);
        let slot_str = if var.stack_pos == u16::MAX {
            "-".to_string()
        } else {
            format!("[{}, {})", var.stack_pos, var.stack_pos + vs)
        };
        let pre_str = if var.pre_assigned_pos == u16::MAX || var.pre_assigned_pos == var.stack_pos {
            String::new()
        } else {
            format!("[{}, {})", var.pre_assigned_pos, var.pre_assigned_pos + vs)
        };
        let live_str = if var.first_def == u32::MAX {
            "-".to_string()
        } else {
            format!("[{}, {}]", var.first_def, var.last_use)
        };
        let mark = if idx == left_idx || idx == right_idx {
            "*"
        } else {
            " "
        };
        // Show scope number; append "L seq:[s..e)" for loop scopes so physical-TOS
        // decisions are immediately visible without reading the full IR.
        let scope_str = if var.scope == u16::MAX {
            "-".to_string()
        } else if let Some((s, e)) = function.loop_seq_range(var.scope) {
            format!("{}L seq:[{}..{})", var.scope, s, e)
        } else {
            var.scope.to_string()
        };
        eprintln!(
            "  {idx:<4} {mark:<2} {:<20} {:<14} {scope_str:<16} {slot_str:<12} {pre_str:<12} {live_str:<14}",
            var.name,
            short_type(&var.type_def),
        );
    }
    eprintln!();
    eprintln!("=== IR code for '{}' ===", function.name);
    let mut buf: Vec<u8> = Vec::new();
    let mut vars_copy = Function::copy(function);
    if data
        .show_code(&mut buf, &mut vars_copy, &data.def(def_nr).code, 0, true)
        .is_ok()
    {
        eprintln!("{}", String::from_utf8_lossy(&buf));
    }
    panic!(
        "[I1] Variables '{}' (slot [{}, {left_slot_end}), live [{}, {}]) and '{}' (slot [{}, {right_slot_end}), live [{}, {}]) \
         share a stack slot while both live in function '{}'",
        left.name,
        left.stack_pos,
        left.first_def,
        left.last_use,
        right.name,
        right.stack_pos,
        right.first_def,
        right.last_use,
        function.name,
    );
}

/// Write the variable table for `function` to `f`.
///
/// Columns: index, argument flag, name, short type, scope, stack slot range, live interval.
/// Variables with no slot (`stack_pos == u16::MAX`) or no definition are still listed.
///
/// # Errors
/// Propagates any I/O error from the writer.
/// `LOFT_VAR_TABLE=<substring>` — print each matching function's variable table
/// after `scopes::check`, with every type dep rendered as `name(index)`.
///
/// The IR dump names variables but never numbers them, so a body and a variable
/// table that disagree read as one consistent story. loft#666 is that story: the
/// operation read `e` while the binding's type dep printed `__retbuf`, and only the
/// index says whether those are one variable or two. Printing the index beside the
/// name — and resolving each dep to `name(index)` — makes a code/table desync
/// visible instead of inferred, for this bug and the next one in its family.
pub fn dump_var_tables(data: &Data, from: u32) {
    let Some(want) = std::env::var_os("LOFT_VAR_TABLE") else {
        return;
    };
    let want = want.to_string_lossy().to_string();
    for d_nr in from..data.definitions() {
        let def = data.def(d_nr);
        if !def.name().contains(&want) || *def.code() == Value::Null {
            continue;
        }
        let vars = def.variables();
        eprintln!("[vartable] {} — returned {:?}", def.name(), def.returned());
        for (idx, var) in vars.variables.iter().enumerate() {
            let deps: Vec<String> = var
                .type_def
                .depend()
                .iter()
                .map(|d| format!("{}({d})", vars.name(*d)))
                .collect();
            let scope = if var.scope == u16::MAX {
                "-".to_string()
            } else {
                var.scope.to_string()
            };
            let idx16 = u16::try_from(idx).unwrap_or(u16::MAX);
            eprintln!(
                "[vartable]   {idx:<3} {:<18} {:<14} scope={scope:<4} {}{}{}{}{}{}{}",
                var.name,
                short_type(&var.type_def),
                if var.argument { "arg " } else { "" },
                if var.defined { "def " } else { "" },
                if var.skip_free { "skipfree " } else { "" },
                if vars.is_inline_ref(idx16) {
                    "inlineref "
                } else {
                    ""
                },
                // @PLN130 F9 — the `&` at a struct projection is invisible in the IR
                // (`c = &v[0]` and `c = v[0]` emit the same ops), so this column is the
                // only place the two spellings can be told apart.
                if vars.is_amp_link(idx16) {
                    "amplink "
                } else {
                    ""
                },
                if vars.owns_store(idx16) { "OWNS " } else { "" },
                if deps.is_empty() {
                    String::new()
                } else {
                    format!("deps=[{}]", deps.join(", "))
                },
            );
        }
    }
}

pub fn dump_variables(f: &mut dyn Write, function: &Function, data: &Data) -> Result<(), Error> {
    writeln!(
        f,
        "  {:<4} {:<4} {:<20} {:<14} {:<6} {:<12} live",
        "#", "arg", "name", "type", "scope", "slot"
    )?;
    writeln!(f, "  {}", "-".repeat(70))?;
    for (idx, var) in function.variables.iter().enumerate() {
        let vs = size(&var.type_def, &Context::Variable);
        let slot_str = if var.stack_pos == u16::MAX {
            "-".to_string()
        } else {
            format!("[{}, {})", var.stack_pos, var.stack_pos + vs)
        };
        let live_str = if var.first_def == u32::MAX {
            "-".to_string()
        } else {
            format!("[{}, {}]", var.first_def, var.last_use)
        };
        let scope_str = if var.scope == u16::MAX {
            "-".to_string()
        } else {
            var.scope.to_string()
        };
        let arg_flag = if var.argument { "arg" } else { "" };
        let type_str = short_type(&var.type_def);
        writeln!(
            f,
            "  {idx:<4} {arg_flag:<4} {:<20} {type_str:<14} {scope_str:<6} {slot_str:<12} {live_str}",
            var.name
        )?;
        let _ = data; // reserved for future type name resolution
    }
    writeln!(f)
}

#[cfg(test)]
mod invariant_tests {
    //! Regression guards for the invariant-check helpers introduced in
    //! plan-04 Phase 2a.  Each test constructs a minimal `Function`
    //! with a deliberately broken placement and asserts the
    //! corresponding invariant fires.

    use super::*;
    use crate::data::{Deps, IntegerSpec};

    const INT: Type = Type::Integer(IntegerSpec::signed32());

    fn mk_fn() -> Function {
        Function::new("test_fn", "test")
    }

    fn add_arg(f: &mut Function, name: &str, tp: &Type) -> u16 {
        let v = f.add_unique(name, tp, 0);
        f.variables[v as usize].argument = true;
        // Arguments keep stack_pos == u16::MAX during assign_slots;
        // codegen assigns their actual position.  For I2 tests we
        // leave it unassigned — what matters is the total
        // arg-region size used by `compute_local_start`.
        v
    }

    fn add_local(
        f: &mut Function,
        name: &str,
        tp: &Type,
        slot: u16,
        first_def: u32,
        last_use: u32,
    ) -> u16 {
        let v = f.add_unique(name, tp, 0);
        f.variables[v as usize].stack_pos = slot;
        f.variables[v as usize].first_def = first_def;
        f.variables[v as usize].last_use = last_use;
        v
    }

    // ── I2 — argument isolation ──────────────────────────────────────

    #[test]
    fn i2_local_below_local_start_flagged() {
        let mut f = mk_fn();
        // Arg: one integer (size 8) + 4-byte return address = local_start 12.
        add_arg(&mut f, "arg_a", &INT);
        // Local placed at slot 4 — violates I2.
        add_local(&mut f, "bad", &INT, 4, 10, 20);
        assert_eq!(
            check_i2_arg_isolation(&f.variables, compute_local_start(&f)).map(|(i, _)| i),
            Some(1)
        );
    }

    #[test]
    fn i2_local_at_or_above_local_start_ok() {
        let mut f = mk_fn();
        add_arg(&mut f, "arg_a", &INT); // local_start = 12
        add_local(&mut f, "good", &INT, 12, 10, 20);
        assert!(check_i2_arg_isolation(&f.variables, compute_local_start(&f)).is_none());
    }

    // ── I4 — every defined variable is placed ────────────────────────

    #[test]
    fn i4_missing_slot_flagged() {
        let mut f = mk_fn();
        // Variable has first_def + non-zero size but no stack_pos.
        let v = f.add_unique("orphan", &INT, 0);
        f.variables[v as usize].first_def = 5;
        f.variables[v as usize].last_use = 10;
        // stack_pos left at u16::MAX.
        assert_eq!(check_i4_every_var_placed(&f.variables), Some(v as usize));
    }

    #[test]
    fn i4_placed_variable_passes() {
        let mut f = mk_fn();
        add_local(&mut f, "placed", &INT, 4, 5, 10);
        assert_eq!(check_i4_every_var_placed(&f.variables), None);
    }

    #[test]
    fn i4_unused_variable_ok() {
        let mut f = mk_fn();
        // Variable declared but never used (first_def stays u32::MAX).
        f.add_unique("unused", &INT, 0);
        assert_eq!(check_i4_every_var_placed(&f.variables), None);
    }

    // ── I5 — kind-consistency on overlapping-slot reuse ──────────────

    #[test]
    fn i5_same_slot_disjoint_lifetimes_matching_refslot_ok() {
        // Two Text vars at identical (slot, size) with disjoint
        // lifetimes — permitted RefSlot reuse.
        let mut f = mk_fn();
        let text = Type::Text(Deps::none());
        add_local(&mut f, "t1", &text, 4, 0, 10);
        add_local(&mut f, "t2", &text, 4, 11, 20);
        assert_eq!(check_i5_kind_consistency(&f.variables), None);
    }

    #[test]
    fn i5_kind_mismatch_on_shared_slot_flagged() {
        // RefSlot (Text, 24B) dies; an Inline integer (8B) takes a
        // slot that overlaps the Text's range.  Disjoint lifetimes
        // but kinds differ → I5 must fire.
        let mut f = mk_fn();
        let text = Type::Text(Deps::none());
        add_local(&mut f, "t", &text, 4, 0, 10);
        add_local(&mut f, "i", &INT, 4, 11, 20);
        assert!(check_i5_kind_consistency(&f.variables).is_some());
    }

    #[test]
    fn i5_refslot_size_mismatch_flagged() {
        // Both RefSlot, same start slot, different sizes (24 B Text
        // vs 12 B DbRef).  Disjoint lifetimes.  Must fire.
        let mut f = mk_fn();
        let text = Type::Text(Deps::none());
        let refer = Type::Reference(0, Deps::none());
        add_local(&mut f, "t", &text, 4, 0, 10);
        add_local(&mut f, "r", &refer, 4, 11, 20);
        assert!(check_i5_kind_consistency(&f.variables).is_some());
    }

    #[test]
    fn i5_two_inline_sharing_slot_partial_overlap_ok() {
        // Inline vars can have partial-range overlap with disjoint
        // lifetimes — no drop-opcode issue.  E.g., 1-byte cond at
        // slot 4 dies, 8-byte int starts at slot 4 covering [4, 12).
        let mut f = mk_fn();
        add_local(&mut f, "cond", &Type::Boolean, 4, 0, 3);
        add_local(&mut f, "n", &INT, 4, 4, 20);
        assert_eq!(check_i5_kind_consistency(&f.variables), None);
    }

    #[test]
    fn i5_refslot_same_size_different_types_ok() {
        // Two 12-B RefSlots (Reference and Vector) with disjoint
        // lifetimes — size and slot match, kinds match (both
        // RefSlot), both drop via OpFreeRef.  Permitted.
        let mut f = mk_fn();
        let r = Type::Reference(0, Deps::none());
        let v = Type::Vector(Box::new(INT), Deps::none());
        add_local(&mut f, "r", &r, 4, 0, 10);
        add_local(&mut f, "vec", &v, 4, 11, 20);
        assert_eq!(check_i5_kind_consistency(&f.variables), None);
    }

    // ── I7 — declared-scope zone-1 frame consistency ─────────────────

    use crate::data::Block;

    /// Build a single-Block root Value with the given scope and var_size.
    fn mk_block(scope: u16, var_size: u16) -> Value {
        Value::Block(Box::new(Block {
            operators: Vec::new(),
            result: Type::Void,
            name: "",
            scope,
            var_size,
        }))
    }

    /// Build a nested Block-in-Block root: parent at scope 1 with
    /// zone1=`parent_size`, child at scope 2 with zone1=`child_size`.
    fn mk_nested_block(parent_size: u16, child_scope: u16, child_size: u16) -> Value {
        Value::Block(Box::new(Block {
            operators: vec![mk_block(child_scope, child_size)],
            result: Type::Void,
            name: "",
            scope: 1,
            var_size: parent_size,
        }))
    }

    fn set_scope(f: &mut Function, v: u16, scope: u16) {
        f.variables[v as usize].scope = scope;
    }

    #[test]
    fn i7_slot_inside_scope_frame_ok() {
        let mut f = mk_fn();
        add_arg(&mut f, "a", &INT); // local_start = 12
        // Single Block, scope 1, zone1_size = 16 → frame [12, 28).
        let code = mk_block(1, 16);
        let v = add_local(&mut f, "x", &INT, 12, 5, 10);
        set_scope(&mut f, v, 1);
        assert_eq!(
            check_i7_scope_frame(&f.variables, &f, &code, compute_local_start(&f)),
            None,
        );
    }

    #[test]
    fn i7_slot_above_frame_top_flagged() {
        let mut f = mk_fn();
        add_arg(&mut f, "a", &INT);
        // scope 1, zone1 = 8 → frame [12, 20).  Var at slot 20 overflows.
        let code = mk_block(1, 8);
        let v = add_local(&mut f, "x", &INT, 20, 5, 10);
        set_scope(&mut f, v, 1);
        let result = check_i7_scope_frame(&f.variables, &f, &code, compute_local_start(&f));
        assert!(result.is_some());
        let (idx, _base, _top) = result.unwrap();
        assert_eq!(idx, 1);
    }

    // I8 (@PLAN53 cluster 2): a slot whose byte range exceeds the
    // u16-addressable frame is flagged; normal slots pass.
    #[test]
    fn i8_slot_exceeding_u16_claim_flagged() {
        let mut f = mk_fn();
        // INT is 8 bytes; placing at u16::MAX - 4 gives end = u16::MAX + 4.
        add_local(&mut f, "x", &INT, u16::MAX - 4, 5, 10);
        assert_eq!(check_i8_total_claim(&f.variables), Some(0));
    }

    #[test]
    fn i8_normal_slot_ok() {
        let mut f = mk_fn();
        add_local(&mut f, "x", &INT, 12, 5, 10);
        assert_eq!(check_i8_total_claim(&f.variables), None);
    }

    #[test]
    fn i7_slot_below_frame_base_flagged() {
        let mut f = mk_fn();
        add_arg(&mut f, "a", &INT);
        // scope 1 at [12, 28).  Child scope 2 frame_base = 28.
        // Place a scope-2 var at slot 12 — below its frame_base.
        let code = mk_nested_block(16, 2, 8);
        let v = add_local(&mut f, "x", &INT, 12, 5, 10);
        set_scope(&mut f, v, 2);
        let result = check_i7_scope_frame(&f.variables, &f, &code, compute_local_start(&f));
        assert!(result.is_some());
    }

    #[test]
    fn i7_arg_is_skipped() {
        let mut f = mk_fn();
        // Args live below local_start; I7 must not flag them.
        let a = add_arg(&mut f, "a", &INT);
        f.variables[a as usize].stack_pos = 0;
        f.variables[a as usize].scope = 0;
        let code = mk_block(1, 8);
        assert_eq!(
            check_i7_scope_frame(&f.variables, &f, &code, compute_local_start(&f)),
            None,
        );
    }

    #[test]
    fn i7_unscoped_var_is_skipped() {
        let mut f = mk_fn();
        add_arg(&mut f, "a", &INT);
        let code = mk_block(1, 8);
        // Var with scope 0 (argument-level, not a block) — I7 skips.
        let v = add_local(&mut f, "x", &INT, 100, 5, 10);
        assert_eq!(f.variables[v as usize].scope, 0);
        assert_eq!(
            check_i7_scope_frame(&f.variables, &f, &code, compute_local_start(&f)),
            None,
        );
        // Also: a var with scope 5 (not present in IR) is skipped
        // because `frames` has no entry.
        set_scope(&mut f, v, 5);
        assert_eq!(
            check_i7_scope_frame(&f.variables, &f, &code, compute_local_start(&f)),
            None,
        );
    }

    #[test]
    fn i7_zone_2_var_is_skipped() {
        let mut f = mk_fn();
        add_arg(&mut f, "a", &INT);
        // Text is 24 bytes (> 8) → zone 2, skipped by I7.
        // Place it well above scope 1's frame; I7 must still pass.
        let code = mk_block(1, 8);
        let v = add_local(&mut f, "t", &Type::Text(Deps::none()), 100, 5, 10);
        set_scope(&mut f, v, 1);
        assert_eq!(
            check_i7_scope_frame(&f.variables, &f, &code, compute_local_start(&f)),
            None,
        );
    }

    #[test]
    fn i7_loop_scope_var_is_skipped() {
        let mut f = mk_fn();
        add_arg(&mut f, "a", &INT);
        // Mark scope 2 as a loop scope — loops skip zone 1 entirely.
        f.mark_loop_scope(2);
        let code = mk_nested_block(8, 2, 0);
        // Var in scope 2 at slot 100 would fail frame-consistency if
        // scope 2 were a Block, but because it's a Loop I7 skips.
        let v = add_local(&mut f, "x", &INT, 100, 5, 10);
        set_scope(&mut f, v, 2);
        assert_eq!(
            check_i7_scope_frame(&f.variables, &f, &code, compute_local_start(&f)),
            None,
        );
    }
}
