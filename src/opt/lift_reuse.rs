// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks
//! `@FR-R-LiftReuse` in the IR phase: lifted call results (`__lift_N`) that are tuples of
//! scalars and whose live ranges over the body's top-level operators do not overlap share one
//! variable.  A straight-line builder (`m.add_vertex(vertex(vec3(..), vec3(..), vec2(..)))` per
//! line) lifted three or four records per line into variables of their own, each
//! null-initialised at entry: 512 locals and 4096 initialisations for 128 lines, which a C
//! compiler pays for (MSVC `cl` 65 s on `a-function-of-many-record-temporaries-compiles-in-seconds`).
//!
//! The admission, per lift `v`:
//!
//! - its type is a tuple whose every element is a scalar (`value_field_type`): a value with no
//!   store, no heap and no borrow, so another lift writing the shared variable can disturb
//!   nothing but the variable itself;
//! - its live range is the top-level operators from its first mention to its last (every
//!   spelling: `Value::reads_var`), skipping entry initialisations `v = <literal>` ahead of it;
//! - the first mention WRITES: a direct `Set(v, rhs)` operator with `rhs` not naming `v` — the
//!   body's top-level operators run in order, so it dominates every later mention however
//!   nested — or one operator holding every mention, whose own first mention is such a `Set`
//!   in an operator list holding the rest ([`dominated`]).  So no read of `v` can see a value
//!   written outside its range.
//!
//! The rewrite: in order of first mention, a lift takes a variable of its type whose last range
//! ended before its own begins, or keeps its own; it is renumbered onto that variable's first
//! lift.  A renumbered lift's entry initialisation goes, as it is dead — the range's first
//! mention writes; the shared variable keeps its own, which declares it at the top level for
//! the rustc writer.  The frame is laid out again.
//!
//! The fallback is safe in one direction: a lift any clause declines keeps its own variable
//! and its initialisation, exactly as before.  `LOFT_NO_LIFT_REUSE=1` turns the rule off on
//! every backend; `LOFT_TRACE_LIFT_REUSE=1` names each function and how many lifts it shared.
use crate::data::{Data, DefType, Type, Value};
use crate::opt::value_records::layout::value_field_type;

fn enabled() -> bool {
    !crate::env_once!(std::env::var("LOFT_NO_LIFT_REUSE").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_LIFT_REUSE").is_ok_and(|v| v != "0"))
}

/// Rewrite every function of the program; answers how many lifts were renumbered onto
/// another lift's variable.
pub fn rewrite_program(data: &mut Data) -> usize {
    if !enabled() {
        return 0;
    }
    let mut total = 0usize;
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type != DefType::Function
            || matches!(def.code(), Value::Null)
            || def.is_operator()
            || !def.rust().is_empty()
        {
            continue;
        }
        let Some(plan) = plan(data, d) else {
            continue;
        };
        let _census = crate::rewrite_census::InBody::enter("ir", data.def(d).name());
        let def = &mut data.definitions[d as usize];
        if let Value::Block(bl) = def.code.unspan_mut() {
            let ops = std::mem::take(&mut bl.operators);
            bl.operators = ops
                .into_iter()
                .enumerate()
                .filter(|(i, _)| !plan.inits.contains(i))
                .map(|(_, op)| op)
                .collect();
        }
        // A shared variable is live across top-level operators of the body, so the body's scope
        // is the one that encloses every use (`@FR-B-Scope`); its first lift's may be a block's.
        let body_scope = match def.code.unspan() {
            Value::Block(bl) => bl.scope,
            _ => 0,
        };
        for &(from, to) in &plan.renames {
            def.code.renumber_frame_var(from, to);
            def.variables.renumber_frame_in_types(from, to);
            def.variables.move_scope(to, body_scope);
        }
        def.variables.reset_intervals();
        crate::scopes::compute_function_intervals(data, d);
        crate::scopes::assign_function_slots(data, d);
        if trace() {
            eprintln!(
                "[lift-reuse] fn={} {} lift(s) share {} variable(s)",
                data.def(d).name(),
                plan.renames.len() + plan.kept,
                plan.kept
            );
        }
        crate::rewrite_census::fired("R-LiftReuse", plan.renames.len());
        total += plan.renames.len();
    }
    total
}

/// What the rewrite does to one function.
struct Plan {
    /// `(lift, the lift whose variable it takes)`.
    renames: Vec<(u16, u16)>,
    /// The top-level operators that are a renumbered lift's entry initialisation, which go.
    inits: std::collections::HashSet<usize>,
    /// How many shared variables remain.
    kept: usize,
}

/// The plan for function `d`, or `None` where no lift takes another's variable.
fn plan(data: &Data, d: u32) -> Option<Plan> {
    let def = data.def(d);
    let vars = def.variables();
    let Value::Block(bl) = def.code().unspan() else {
        return None;
    };
    let ops = &bl.operators;
    // Each admitted lift's live range over the body's top-level operators.
    let mut ranges: Vec<(usize, usize, u16)> = Vec::new();
    let mut entry: std::collections::HashMap<u16, Vec<usize>> = std::collections::HashMap::new();
    for v in 0..vars.count() {
        if !vars.name(v).starts_with("__lift_") || vars.is_argument(v) || !scalar_tuple(vars.tp(v))
        {
            continue;
        }
        if let Some((first, last, inits)) = live_range(ops, v) {
            ranges.push((first, last, v));
            entry.insert(v, inits);
        }
    }
    if ranges.len() < 2 {
        return None;
    }
    ranges.sort_unstable();
    // Per type, the shared variables: `(the variable, the last operator it is live in)`.  A
    // lift takes a variable of its type whose range ended before its own begins.
    let mut shared: Vec<(Type, Vec<(u16, usize)>)> = Vec::new();
    let mut renames = Vec::new();
    for &(first, last, v) in &ranges {
        let tp = vars.tp(v);
        let i = shared.iter().position(|(t, _)| t == tp).unwrap_or_else(|| {
            shared.push((tp.clone(), Vec::new()));
            shared.len() - 1
        });
        let slots = &mut shared[i].1;
        if let Some(slot) = slots.iter_mut().find(|(_, end)| *end < first) {
            renames.push((v, slot.0));
            slot.1 = last;
        } else {
            slots.push((v, last));
        }
    }
    if renames.is_empty() {
        return None;
    }
    let kept = shared.iter().map(|(_, s)| s.len()).sum();
    // A shared variable keeps its own entry initialisation: the rustc writer declares a local
    // at its first assignment, and that must stand at the top level, ahead of every lift that
    // takes the variable.
    let inits = renames
        .iter()
        .flat_map(|(v, _)| entry[v].iter().copied())
        .collect();
    Some(Plan {
        renames,
        inits,
        kept,
    })
}

/// A tuple whose every element is a scalar: a value with no store and nothing it borrows.
fn scalar_tuple(tp: &Type) -> bool {
    match tp {
        // A nullable tuple carries a presence the shared variable would hand from one lift to
        // the next; `(R-ValueRecord)` keeps such a value a record anyway.
        Type::Optional(_) => false,
        Type::Tuple(elems) => {
            !elems.is_empty() && elems.iter().all(|e| value_field_type(e).is_some())
        }
        _ => false,
    }
}

/// How many nodes of `n` name variable `v`.
fn count(n: &Value, v: u16) -> usize {
    let mut c = 0;
    n.walk(&mut |x| {
        if x.names_var_here(v) {
            c += 1;
        }
    });
    c
}

/// An entry initialisation of `v`: `v = <expression naming no variable and calling nothing>`.
fn entry_init(op: &Value, v: u16) -> bool {
    let Value::Set(x, rhs) = op.unspan() else {
        return false;
    };
    *x == v && !rhs.any_node(&mut |n| n.var_named_here().is_some() || matches!(n, Value::Call(..)))
}

/// The top-level operators `v` is live over, `(first, last)`: entry initialisations ahead of
/// the first are skipped, and the first must WRITE `v` before anything reads it — a direct
/// `Set(v, rhs)` operator whose `rhs` does not name `v` (the body's top-level operators run in
/// order, so it dominates every later mention, however nested), or a single operator holding
/// every mention whose first is such a write ([`dominated`]).
/// The third answer is the operators that are `v`'s entry initialisations.
fn live_range(ops: &[Value], v: u16) -> Option<(usize, usize, Vec<usize>)> {
    let mut first = None;
    let mut mine = Vec::new();
    let mut last = 0;
    for (i, op) in ops.iter().enumerate() {
        if !op.reads_var(v) {
            continue;
        }
        if first.is_none() && entry_init(op, v) {
            mine.push(i);
            continue;
        }
        first.get_or_insert(i);
        last = i;
    }
    let first = first?;
    let writes = match ops[first].unspan() {
        Value::Set(x, rhs) if *x == v => !rhs.reads_var(v),
        _ => last == first && dominated(&ops[first], v, count(&ops[first], v)),
    };
    // A lift with no entry initialisation has no top-level declaration to share.
    (writes && !mine.is_empty()).then_some((first, last, mine))
}

/// Is the first mention of `v` in `node` a `Set(v, rhs)` that is a direct operator of an
/// operator list holding all `total` of its mentions, with `rhs` not naming `v`?  Answers no
/// for any other shape (an `If`, a call argument, a loop header): the lift keeps its variable.
fn dominated(node: &Value, v: u16, total: usize) -> bool {
    match node.unspan() {
        Value::Block(bl) | Value::Loop(bl) => in_list(&bl.operators, v, total),
        Value::Insert(xs) => in_list(xs, v, total),
        Value::Drop(x) => dominated(x, v, total),
        _ => false,
    }
}

fn in_list(ops: &[Value], v: u16, total: usize) -> bool {
    let Some(k) = ops.iter().position(|op| op.reads_var(v)) else {
        return false;
    };
    if ops[k..].iter().map(|op| count(op, v)).sum::<usize>() != total {
        return false;
    }
    match ops[k].unspan() {
        Value::Set(x, rhs) if *x == v => !rhs.reads_var(v),
        _ => dominated(&ops[k], v, total),
    }
}
