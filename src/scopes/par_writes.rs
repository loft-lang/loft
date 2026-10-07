// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! PAR worker writes: whether a `par` worker writes state its parent owns, directly or through
//! the functions it calls.

use crate::data::{Data, Value};
use crate::data::{ImpureCategory, Purity};

// ── Plan-06 phase 5b' shallow check (precise, no false positives) ────────────

/// Plan-06 phase 5b' — precise shallow par-safety check.
///
/// Walks `worker_d_nr`'s body looking for **direct** calls to fns
/// classified `Impure(ParentWrite)`.  Does NOT recurse into callee
/// bodies — so it only fires when the worker code itself contains
/// the offending call, not when a transitive callee does.  This
/// produces ZERO false positives because every `parent_write`
/// classification is explicit (came from a `#impure(parent_write)`
/// annotation in the stdlib or user code).
///
/// Trade-off vs the full `is_par_safe`: misses transitive
/// violations.  A worker that calls a user fn that calls
/// vector_add slips through.  But unlike the full check, it
/// never warns on a worker that's actually safe — making it
/// usable as a parser warning today, before the 5a annotation
/// sweep is comprehensive.
///
/// Returns `Some(callee_name)` if a direct ParentWrite call was
/// found; `None` otherwise.
#[allow(dead_code)]
#[must_use]
pub fn worker_calls_parent_write(data: &Data, worker_d_nr: u32) -> Option<String> {
    if worker_d_nr == u32::MAX || (worker_d_nr as usize) >= data.definitions.len() {
        return None;
    }
    let def = &data.definitions[worker_d_nr as usize];
    walk_shallow_parent_write(&def.code, data)
}

/// Plan-06 phase 5b' — DEEP parent-write check for par() workers.
/// Recurses through user-fn callees (those with a body) until it
/// finds a direct call to a `Purity::Impure(ParentWrite)` stdlib fn.
/// Returns the chain "worker → helper → bad_callee" or None.
///
/// Crucially, unannotated declared-only natives (Op*, n_*, t_* with
/// `code == Value::Null` and `purity == Unknown`) are treated as
/// safe — they're C primitives that don't write to parent state
/// unless explicitly tagged.  This is what avoids the 16 false
/// positives the strict `par_unsafe_reason` walk would produce.
///
/// `Purity::Impure(ParCall)` stdlib fns (parallel_for / _light)
/// are also safe — D8 R2 and D2.1.1 cover recursive Arc promotion.
pub fn worker_calls_parent_write_deep(data: &Data, worker_d_nr: u32) -> Option<String> {
    if worker_d_nr == u32::MAX || (worker_d_nr as usize) >= data.definitions.len() {
        return None;
    }
    let mut visited = std::collections::HashSet::default();
    let def = &data.definitions[worker_d_nr as usize];
    let worker_name = def.name.strip_prefix("n_").unwrap_or(&def.name).to_string();
    walk_deep_parent_write(&def.code, data, worker_d_nr, &mut visited).map(|chain| {
        if chain == worker_name {
            chain
        } else {
            format!("{worker_name}{chain}")
        }
    })
}

/// Find a parent-state write reachable from `value`, following calls into user fns.
///
/// The recursive half of `worker_calls_parent_write_deep`: it answers with the chain
/// `" → helper → bad_callee"` for the first write it reaches, and `None` when every path
/// bottoms out in a pure, host-io or unannotated-native primitive.  `visited` stops a
/// recursive call graph from looping.
///
/// This is PRODUCTION-WIRED — `parse_parallel` turns a `Some` into the C93 `Level::Error`
/// that refuses the program — so a subtree it does not enter is a refusal that does not
/// happen, and the write then runs in a worker thread against a read-only store.  That is
/// why the fallback descends via the keystone instead of naming arms: the arm list below
/// is shared with `walk_par_unsafe_reason_value`, and a wrapper missing from both is a
/// verdict issued without looking.
fn walk_deep_parent_write(
    value: &Value,
    data: &Data,
    current_fn: u32,
    visited: &mut std::collections::HashSet<u32>,
) -> Option<String> {
    match value {
        Value::Call(callee, args) => {
            // @PLN102 C93 — a RAW field/element write (`OpSet*`) whose accessor root is a
            // NON-LOCAL PARAMETER is a write to captured/parent state: in a `par` worker the
            // captured state is read-only, so this is the race the deep check must reject
            // (the tagged-stdlib ParentWrite path below only catches `+=`/`hash_set`/… — a
            // bare `s.n = v` / `cap[i] = v` lowers to `OpSet*`, a "safe" primitive, and used
            // to slip through into a codegen slot-panic).  A write to a worker-LOCAL (or the
            // hidden return-buffer destination) is fine — same locality test as
            // `first_arg_is_local_var`.
            if let Some(var) = raw_write_to_captured(*callee, args, current_fn, data) {
                return Some(format!(" writes captured `{var}`"));
            }
            if let Some(chain) = call_deep_parent_write(*callee, args, data, current_fn, visited) {
                return Some(chain);
            }
            for a in args {
                if let Some(chain) = walk_deep_parent_write(a, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        // Don't recurse into CallRef target (callee unknown until runtime).
        Value::CallRef(_, args) => {
            for a in args {
                if let Some(chain) = walk_deep_parent_write(a, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        Value::Block(b) => {
            for v in &b.operators {
                if let Some(chain) = walk_deep_parent_write(v, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        Value::Insert(vs) => {
            for v in vs {
                if let Some(chain) = walk_deep_parent_write(v, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        Value::If(c, t, e) => walk_deep_parent_write(c, data, current_fn, visited)
            .or_else(|| walk_deep_parent_write(t, data, current_fn, visited))
            .or_else(|| walk_deep_parent_write(e, data, current_fn, visited)),
        Value::Loop(body) => {
            for v in &body.operators {
                if let Some(chain) = walk_deep_parent_write(v, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        Value::Set(_, rhs) => walk_deep_parent_write(rhs, data, current_fn, visited),
        Value::Span(b) => walk_deep_parent_write(&b.1, data, current_fn, visited),
        // Every other shape descends through the keystone, so a wrapper the arms above do
        // not name still has its subtree searched.  `walk_par_unsafe_reason_value` is a
        // near-copy of this arm list and ends the same way for the same reason.
        other => {
            let mut found = None;
            other.for_each_child(&mut |c| {
                if found.is_none() {
                    found = walk_deep_parent_write(c, data, current_fn, visited);
                }
            });
            found
        }
    }
}

/// Plan-06 phase 5b' G5 — for a `ParentWrite` callee, treat the
/// call as safe when its first argument is a LOCAL variable
/// (defined within the calling function, not a parameter).  Avoids
/// the false positive on `out: vector<integer> = []; out += [i]`
/// where `OpAppendVector(out, ...)` mutates a worker-local
/// vector — `out` was just allocated locally and isn't shared
/// with the parent.
///
/// This is a heuristic: a local variable could still hold a
/// reference to parent data (e.g. via `let v = some_param_field`),
/// so the rule is conservative for the common pattern but may
/// miss adversarial aliases.  Future precision work could track
/// per-var initialiser provenance.
fn first_arg_is_local_var(args: &[Value], current_fn: u32, data: &Data) -> bool {
    // The write's ROOT, read through its accessor chain as `raw_write_to_captured` reads
    // it: `Doc { ps: np }` appends into a field OF the hidden return buffer
    // (`OpAppendVector(OpGetField(__retbuf, …), np)`), which is the worker's own output.
    let Some(v) = args.first().and_then(|a| accessor_root_var(a, data)) else {
        return false;
    };
    let v = &v;
    if current_fn == u32::MAX || (current_fn as usize) >= data.definitions.len() {
        return false;
    }
    let def = &data.definitions[current_fn as usize];
    // A local that VIEWS a parameter is not the worker's own (`captured_param_viewed_by`).
    if !def.variables.is_argument(*v) {
        return captured_param_viewed_by(def, *v).is_none();
    }
    // Plan-06 phase 5b' G5 — heap-typed return values are passed
    // via a hidden destination argument promoted by ref_return().
    // The promotion sets `argument: true` on the variable AND
    // marks the corresponding `def.attributes[…].hidden = true`.
    // Workers writing to these hidden destinations are populating
    // their own per-worker output buffer, NOT parent state.
    let name = def.variables.name(*v);
    def.attributes.iter().any(|a| a.hidden && a.name == name)
}

/// @PLN102 C93 — walk an accessor chain (`OpGet*(base, …)` / `Var`, span-transparent)
/// down to its root variable.  The base of an `OpSet*` write target is arg 0.
pub(super) fn accessor_root_var(v: &Value, data: &Data) -> Option<u16> {
    match v {
        Value::Var(n) => Some(*n),
        Value::Span(b) => accessor_root_var(&b.1, data),
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && data.definitions[*d as usize].name.starts_with("OpGet") =>
        {
            args.first().and_then(|a| accessor_root_var(a, data))
        }
        _ => None,
    }
}

/// @PLN102 C93 — is this call a RAW field/element write (`OpSet*`) whose accessor root is
/// a NON-LOCAL PARAMETER of `current_fn` (captured/parent state)?  Returns the captured
/// var's name if so.  A write to a worker-LOCAL variable, or to the hidden return-buffer
/// destination (a promoted `ref_return` output), is safe.  Mirrors `first_arg_is_local_var`.
fn raw_write_to_captured(
    callee: u32,
    args: &[Value],
    current_fn: u32,
    data: &Data,
) -> Option<String> {
    if (callee as usize) >= data.definitions.len()
        || !data.definitions[callee as usize].name.starts_with("OpSet")
    {
        return None;
    }
    let root = args.first().and_then(|a| accessor_root_var(a, data))?;
    if current_fn == u32::MAX || (current_fn as usize) >= data.definitions.len() {
        return None;
    }
    let def = &data.definitions[current_fn as usize];
    // `@FR-C-Impure`, C93 — the captured state is what the root VIEWS, not only a root that
    // is itself a parameter: `x = e.inner; x.k = …`, `x = &e.items`, `for l in e.list { l.k
    // = … }` write the parent's record through a local, and compiled to a runtime halt on the
    // read-only store (loft#1918).  A whole-value copy (`x = e`) has no dep and stays local.
    let name = captured_param_viewed_by(def, root)?;
    Some(name)
}

/// The parameter `root` is, or VIEWS through its dep chain, naming it — `None` when the chain
/// reaches no parameter, or reaches only the hidden return buffer (the worker's own output).
fn captured_param_viewed_by(def: &crate::data::Definition, root: u16) -> Option<String> {
    let mut stack = vec![root];
    let mut seen = std::collections::HashSet::new();
    while let Some(v) = stack.pop() {
        if !seen.insert(v) || v >= def.variables.count() {
            continue;
        }
        if def.variables.is_argument(v) {
            let name = def.variables.name(v).to_string();
            if !def.attributes.iter().any(|a| a.hidden && a.name == name) {
                return Some(name);
            }
            continue;
        }
        stack.extend(def.variables.tp(v).depend());
    }
    None
}

#[allow(dead_code)]
fn call_deep_parent_write(
    callee: u32,
    args: &[Value],
    data: &Data,
    current_fn: u32,
    visited: &mut std::collections::HashSet<u32>,
) -> Option<String> {
    if callee == u32::MAX || (callee as usize) >= data.definitions.len() {
        return None;
    }
    let def = &data.definitions[callee as usize];
    match def.purity {
        Purity::Pure
        | Purity::Impure(
            ImpureCategory::HostIo
            | ImpureCategory::Prng
            | ImpureCategory::Io
            | ImpureCategory::ParCall,
        ) => None,
        Purity::Impure(ImpureCategory::ParentWrite) => {
            if first_arg_is_local_var(args, current_fn, data) {
                None
            } else if def.name.starts_with("Op")
                && current_fn != u32::MAX
                && let Some(root) = args.first().and_then(|a| accessor_root_var(a, data))
            {
                // An operator is how the write is SPELLED in the IR (`v += [3]` is
                // `OpPushInt`), not a name the author wrote: say what it writes.
                let name = data.definitions[current_fn as usize].variables.name(root);
                Some(format!(" writes captured `{name}`"))
            } else {
                Some(format!(
                    " → {}",
                    def.name.strip_prefix("n_").unwrap_or(&def.name)
                ))
            }
        }
        Purity::Unknown => {
            // Declared-only native (no body) → trust as safe.
            if matches!(def.code, Value::Null) {
                None
            } else if !visited.insert(callee) {
                // Cycle — optimistic short-circuit (consistent with
                // is_par_safe / fixpoint convergence).
                None
            } else {
                walk_deep_parent_write(&def.code, data, callee, visited).map(|chain| {
                    format!(
                        " → {}{}",
                        def.name.strip_prefix("n_").unwrap_or(&def.name),
                        chain
                    )
                })
            }
        }
    }
}

#[allow(dead_code)]
fn walk_shallow_parent_write(value: &Value, data: &Data) -> Option<String> {
    // "Shallow" = the runtime callee behind a `CallRef` is never followed
    // (it is not a child node); arg expressions ARE scanned.
    let mut found = None;
    value.any_node(&mut |n| {
        if let Value::Call(callee, _) = n
            && (*callee as usize) < data.definitions.len()
            && matches!(
                data.definitions[*callee as usize].purity,
                Purity::Impure(ImpureCategory::ParentWrite)
            )
        {
            found = Some(data.definitions[*callee as usize].name.clone());
            true
        } else {
            false
        }
    });
    found
}

#[cfg(test)]
mod par_shallow_tests {
    use super::worker_calls_parent_write;
    use crate::data::{Data, DefType, ImpureCategory, Purity, Value};
    use crate::lexer::Position;

    fn pos() -> Position {
        Position {
            file: crate::lexer::no_file(),
            line: 0,
            pos: 0,
        }
    }

    #[test]
    fn direct_parent_write_call_detected() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(bad, vec![]);
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("vector_add".to_string())
        );
    }

    /// Pass-2 wave 4 regression: the hand-rolled walker had no `Return`
    /// arm (`_ => None`), so a parent-write call appearing only in
    /// `return f(...)` escaped the scan — the worker classified safe.
    /// The keystone descent sees every position.
    #[test]
    fn parent_write_in_return_value_detected() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Return(Box::new(Value::Call(bad, vec![])));
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("vector_add".to_string())
        );
    }

    /// Same hole for a tuple element (`(f(...), 1)` result shapes).
    #[test]
    fn parent_write_in_tuple_element_detected() {
        let mut d = Data::new();
        let bad = d.add_def("hash_set", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code =
            Value::Tuple(vec![Value::Call(bad, vec![]), Value::Int(1)]);
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("hash_set".to_string())
        );
    }

    #[test]
    fn pure_call_not_detected() {
        let mut d = Data::new();
        let safe = d.add_def("min", &pos(), DefType::Function);
        d.definitions[safe as usize].purity = Purity::Pure;
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(safe, vec![]);
        assert!(worker_calls_parent_write(&d, worker).is_none());
    }

    #[test]
    fn unannotated_call_not_detected() {
        // Shallow check is precise: only fires for explicit
        // ParentWrite annotations.  Unknown stays None (the full
        // is_par_safe rejects this; shallow doesn't).
        let mut d = Data::new();
        let unknown = d.add_def("mystery", &pos(), DefType::Function);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(unknown, vec![]);
        assert!(worker_calls_parent_write(&d, worker).is_none());
    }

    #[test]
    fn transitive_parent_write_not_detected() {
        // Worker calls inner; inner calls vector_add.  Shallow
        // does NOT recurse into inner — only the worker fn's
        // direct calls are checked.  Plan-06 phase 5b' (eventual)
        // adds transitive detection once 5a annotation coverage
        // is comprehensive enough not to false-positive.
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let inner = d.add_def("inner", &pos(), DefType::Function);
        d.definitions[inner as usize].code = Value::Call(bad, vec![]);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(inner, vec![]);
        assert!(worker_calls_parent_write(&d, worker).is_none());
    }

    #[test]
    fn parent_write_inside_arg_detected() {
        // bad_call(vector_add(...)) — the arg evaluation is also
        // a parent-write site.
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let safe = d.add_def("min", &pos(), DefType::Function);
        d.definitions[safe as usize].purity = Purity::Pure;
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(safe, vec![Value::Call(bad, vec![])]);
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("vector_add".to_string())
        );
    }
}

#[cfg(test)]
mod par_deep_tests {
    use super::worker_calls_parent_write_deep;
    use crate::data::{Data, DefType, ImpureCategory, Purity, Value};
    use crate::lexer::Position;

    fn pos() -> Position {
        Position {
            file: crate::lexer::no_file(),
            line: 0,
            pos: 0,
        }
    }

    /// A parent write reached only through a `Return` must still be found.
    ///
    /// `walk_deep_parent_write`'s arms are the same list as its twin
    /// `walk_par_unsafe_reason_value`'s, and neither included `Return` — so a worker whose
    /// body is `return helper(...)` was reported free of parent writes without that call ever
    /// being examined. Both now descend via the keystone; this pins the half that would
    /// otherwise drift back the moment one is edited alone.
    #[test]
    fn deep_parent_write_is_found_through_a_return() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Return(Box::new(Value::Call(bad, vec![])));
        assert!(
            worker_calls_parent_write_deep(&d, worker).is_some(),
            "a parent write behind a `return` must still be reported"
        );
    }

    #[test]
    fn deep_walks_through_user_helper() {
        // Worker calls helper; helper calls vector_add.  Deep walk
        // returns the chain.
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let helper = d.add_def("helper", &pos(), DefType::Function);
        d.definitions[helper as usize].code = Value::Call(bad, vec![]);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(helper, vec![]);
        let result = worker_calls_parent_write_deep(&d, worker);
        assert!(result.is_some());
        let chain = result.unwrap();
        assert!(chain.contains("worker"));
        assert!(chain.contains("helper"));
        assert!(chain.contains("vector_add"));
    }

    #[test]
    fn deep_skips_unannotated_native() {
        // Worker calls OpAddInt (Unknown + Value::Null) → safe.
        let mut d = Data::new();
        let op = d.add_def("OpAddInt", &pos(), DefType::Function);
        // purity defaults to Unknown, code defaults to Null
        let _ = op;
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(op, vec![]);
        assert!(worker_calls_parent_write_deep(&d, worker).is_none());
    }

    #[test]
    fn deep_handles_recursive_user_fn() {
        // Worker calls itself — visited set prevents infinite loop.
        let mut d = Data::new();
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(worker, vec![]);
        // No parent-write reachable, even with the cycle.
        assert!(worker_calls_parent_write_deep(&d, worker).is_none());
    }

    #[test]
    fn deep_par_call_callee_is_safe() {
        // ARC.md A4 (closed 2026-05-07) — `parallel_for_light` was
        // retired but any Impure(ParCall) callee still demonstrates
        // the same D8 R2 invariant: nested par() under recursive Arc
        // promotion is safe.  Use `parallel_queue` (a current
        // ParCall-purity callee) as the stand-in.
        let mut d = Data::new();
        let pf = d.add_def("parallel_queue", &pos(), DefType::Function);
        d.definitions[pf as usize].purity = Purity::Impure(ImpureCategory::ParCall);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(pf, vec![]);
        assert!(worker_calls_parent_write_deep(&d, worker).is_none());
    }

    #[test]
    fn deep_local_arg_to_parent_write_is_safe() {
        // Plan-06 phase 5b' G5 — calling a ParentWrite stdlib fn
        // on a worker-LOCAL variable is safe.  worker calls
        // OpAppendVector(local_v, ...); local_v isn't an arg.
        // Var(0) defaults to argument=false (local).
        let mut d = Data::new();
        let bad = d.add_def("OpAppendVector", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(bad, vec![Value::Var(0)]);
        assert!(worker_calls_parent_write_deep(&d, worker).is_none());
    }

    // Note: the hidden-return-arg exception (`def.attributes[…].hidden`
    // case) is exercised end-to-end by par_struct_to_vector_t4 in
    // tests/threading_chars.rs.  A unit test would need to construct a
    // full Function with a promoted hidden attribute, which the
    // parser does multi-step; the integration test is the cleaner
    // verification.
}
