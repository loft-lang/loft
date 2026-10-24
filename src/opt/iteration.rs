// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! The shape of a `for e in v` loop as the parser lowers it: its head (loop variable, `#index`,
//! vector operand, element size) and the index a step cannot overflow.  Asked by the
//! non-sentinel and range analyses and by the native generator's record addressing.

use crate::data::{Block, Data, Value};
use crate::loop_shape::is_break_block;

/// The head of a vector iteration, as the parser lowers `for e in v`: the loop variable,
/// the `#index` it steps, the vector operand and the element size.  ONE home — the record
/// address (`@FR-R-RecPtr`'s base clause) reads the shape here and so does the non-sentinel
/// pass, which seeds the index as never-null off the loop's own bound (`@FR-R-Counter`).
pub struct VectorIteration<'a> {
    pub loop_var: u16,
    pub index: u16,
    pub vector: &'a Value,
    pub size: u32,
    /// The constant the head adds to the element's position: 0 for a plain element, and
    /// whatever `OpGetField(element, off, _)` wrappers sum to — a struct-enum element is
    /// bound through one at offset 0.
    pub offset: u32,
}

/// Parse the binding `e = { idx = idx + 1; OpGetVectorNullable(vec, size, idx) }` — the
/// first statement of a `for e in v` loop.  Shape only: anything else (a reversed or
/// filtered walk, a keyed collection's cursor, a text or range iterator) answers `None`,
/// and the caller keeps the general emission.
#[must_use]
pub fn iteration_head<'a>(stmt: &'a Value, data: &Data) -> Option<VectorIteration<'a>> {
    let (loop_var, index, read) = iteration_step(stmt, data)?;
    element_read(loop_var, read, data).filter(|it| it.index == index)
}

/// `@FR-R-RecPtr`'s base clause, the counted form `@FR-R-ForwardWalk` leaves: the binding
/// `e = <element read>` of an element at an index local — a walk rewritten as a counted
/// loop reads its element as the body's first statement, indexed by the range's counter.
/// The caller checks the index against the held header's length itself, so any index local
/// is sound here.
#[must_use]
pub fn element_binding<'a>(stmt: &'a Value, data: &Data) -> Option<VectorIteration<'a>> {
    let Value::Set(loop_var, read) = stmt.unspan() else {
        return None;
    };
    element_read(*loop_var, read, data)
}

/// The element read of a vector walk: `OpGetVectorNullable(vec, size, idx)`, possibly behind
/// `OpGetField(element, const off, _)` wrappers — each adds a constant to the element's
/// position and leaves its record alone (a struct-enum element is bound through one at
/// offset 0).
fn element_read<'a>(loop_var: u16, read: &'a Value, data: &Data) -> Option<VectorIteration<'a>> {
    let mut read: &Value = read;
    let mut offset: i64 = 0;
    while let Value::Call(gd, gargs) = read.unspan()
        && (*gd as usize) < data.definitions.len()
        && data.def(*gd).name() == "OpGetField"
        && gargs.len() == 3
        && let Value::Int(off) = gargs[1].unspan()
    {
        offset += i64::from(*off);
        read = &gargs[0];
    }
    let Value::Call(rd, rargs) = read.unspan() else {
        return None;
    };
    if (*rd as usize) >= data.definitions.len()
        || data.def(*rd).name() != "OpGetVectorNullable"
        || rargs.len() != 3
    {
        return None;
    }
    let Value::Var(index) = rargs[2].unspan() else {
        return None;
    };
    let Value::Int(size) = rargs[1].unspan() else {
        return None;
    };
    Some(VectorIteration {
        loop_var,
        index: *index,
        vector: &rargs[0],
        size: u32::try_from(*size).ok()?,
        offset: u32::try_from(offset).ok()?,
    })
}

/// The half of [`iteration_head`] every vector walk shares: the binding
/// `e = { idx = idx + 1; <read> }` — its loop variable, the `#index` it steps and the
/// element read — with the read's shape left to the caller (a record element, or a text
/// element's `OpGetText` over one).  Shape only; anything else answers `None`.
pub(crate) fn iteration_step<'a>(stmt: &'a Value, data: &Data) -> Option<(u16, u16, &'a Value)> {
    let Value::Set(loop_var, iter) = stmt.unspan() else {
        return None;
    };
    let Value::Block(it) = iter.unspan() else {
        return None;
    };
    let ops: Vec<&Value> = it
        .operators
        .iter()
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect();
    let [step, read] = ops[..] else {
        return None;
    };
    let Value::Set(index, stepped) = step.unspan() else {
        return None;
    };
    let Value::Call(sd, sargs) = stepped.unspan() else {
        return None;
    };
    if (*sd as usize) >= data.definitions.len()
        || data.def(*sd).name() != "OpAddInt"
        || sargs.len() != 2
        || !matches!(sargs[0].unspan(), Value::Var(c) if c == index)
        || !matches!(sargs[1].unspan(), Value::Int(1))
    {
        return None;
    }
    Some((*loop_var, *index, read))
}

/// `@FR-R-Counter`'s iteration clause — the `#index` of a `for e in v` loop whose step
/// cannot overflow: the loop's first statement is the [`iteration_head`], its second the
/// parser's own bound `if len(vec) <= index { break }` over the SAME vector operand, and
/// nothing else in the loop assigns the index.  The index is then at most one past a
/// vector's length — a `u32` — whatever the body does to the vector, since the bound runs
/// between every two steps.  Answers the index variable; the caller owns the rest of the
/// proof (every OTHER assignment in the function, and escapes).
#[must_use]
pub fn bounded_iteration_index(lp: &Block, data: &Data) -> Option<u16> {
    let mut stmts = lp.operators.iter().filter(|o| !matches!(o, Value::Line(_)));
    let head = iteration_head(stmts.next()?, data)?;
    let Value::If(cond, on_true, _) = stmts.next()?.unspan() else {
        return None;
    };
    let Value::Call(cd, cargs) = cond.unspan() else {
        return None;
    };
    if !is_break_block(on_true)
        || (*cd as usize) >= data.definitions.len()
        || data.def(*cd).name() != "OpLeInt"
        || cargs.len() != 2
        || !matches!(cargs[1].unspan(), Value::Var(c) if *c == head.index)
    {
        return None;
    }
    let Value::Call(ld, largs) = cargs[0].unspan() else {
        return None;
    };
    if (*ld as usize) >= data.definitions.len()
        || data.def(*ld).name() != "OpLengthVector"
        || largs.len() != 1
        || largs[0].unspan() != head.vector.unspan()
    {
        return None;
    }
    // Any assignment to the index besides the head's own step voids the bound.
    let mut sets = 0usize;
    for op in &lp.operators {
        op.any_node(&mut |n| {
            if matches!(n, Value::Set(v, _) | Value::TuplePut(v, _, _) if *v == head.index) {
                sets += 1;
            }
            false
        });
    }
    (sets == 1).then_some(head.index)
}
