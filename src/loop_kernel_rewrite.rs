// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I73 — native function registry: replacing a loop by the kernel that answers it

//! The rewrite that replaces a whole loop by one call to its kernel (`loop_kernels`): the
//! scope pass finds an integer-sum reduction over a vector and puts `vector_sum_int(v, acc)` in
//! its place.  A pass over the IR, so it sits with the compiler; the kernels it calls are what
//! a compiled program links, in the runtime (@PLN191).

use crate::data::{Data, DefType, Value};

// ── The rule: a reduction loop is one kernel call ───────────────────────────────────────

/// `LOFT_NO_LOOP_KERNELS=1` keeps every loop a loop — the bisect step for a wrong answer out
/// of a loop the rule replaced, on either backend.
fn disabled() -> bool {
    crate::env_once!(std::env::var_os("LOFT_NO_LOOP_KERNELS").is_some_and(|v| v != "0"))
}

/// The definitions the matcher names, looked up once per function.
struct Ops {
    vector_len: u32,
    length_vector: u32,
    add_int: u32,
    le_int: u32,
    get_int: u32,
    get_vector: u32,
    sum_int: u32,
}

impl Ops {
    fn new(data: &Data) -> Ops {
        Ops {
            vector_len: data.def_nr("t_6vector_len"),
            length_vector: data.def_nr("OpLengthVector"),
            add_int: data.def_nr("OpAddInt"),
            le_int: data.def_nr("OpLeInt"),
            get_int: data.def_nr("OpGetInt"),
            get_vector: data.def_nr("OpGetVector"),
            sum_int: data.def_nr("n_vector_sum_int"),
        }
    }
}

/// Replace every reduction loop of `d_nr` the kernels cover by its kernel call.  Decided in
/// the scope pass, beside the other loop rewrites, so both backends read the call; it changes
/// no signature.
///
/// The one shape today, `(R-BoundedNest)`'s reduction:
///
/// ```text
/// {#For  end = len(v);  idx = -1;
///   loop { i = {#Iter idx = idx + 1; if end <= idx break; idx};  { acc = acc + v[i] } } }
/// ```
///
/// over an integer vector becomes `acc = vector_sum_int(v, acc)`.  Admitted only when the
/// loop body is that one statement — nothing else in the loop, so `v` is only read — and the
/// loop's own variables (`i`, `idx`, `end`) are mentioned nowhere else in the function, so
/// dropping the loop changes no later read.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if disabled() || data.def_type(d_nr) != DefType::Function {
        return;
    }
    let ops = Ops::new(data);
    if ops.sum_int == u32::MAX {
        return;
    }
    // Asked of the body as it stands, without a copy: most functions hold no reduction, and a
    // copy of every body cost the front end ~27,000 allocations on the medium corpus.
    let code = &data.definitions[d_nr as usize].code;
    if !contains_reduction(code, code, &ops) {
        return;
    }
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    let whole = code.clone();
    let mut fired = 0;
    replace_loops(&mut code, &whole, &ops, &mut fired);
    data.definitions[d_nr as usize].code = code;
    if fired > 0 {
        crate::rewrite_census::fired("R-KernelSum", fired);
    }
}

/// Does `n` hold a loop [`reduction`] admits?
fn contains_reduction(n: &Value, whole: &Value, ops: &Ops) -> bool {
    if reduction(n, whole, ops).is_some() {
        return true;
    }
    let mut found = false;
    n.for_each_child(&mut |c| found = found || contains_reduction(c, whole, ops));
    found
}

fn replace_loops(n: &mut Value, whole: &Value, ops: &Ops, fired: &mut usize) {
    if let Some(call) = reduction(n, whole, ops) {
        *n = call;
        *fired += 1;
        return;
    }
    n.for_each_child_mut(&mut |c| replace_loops(c, whole, ops, fired));
}

/// The kernel call that answers the loop `n`, when `n` is the admitted reduction shape.
fn reduction(n: &Value, whole: &Value, ops: &Ops) -> Option<Value> {
    let Value::Block(b) = n.unspan() else {
        return None;
    };
    let stmts: Vec<&Value> = statements(&b.operators);
    let [set_end, set_idx, lp] = stmts.as_slice() else {
        return None;
    };
    // end = len(v)
    let Value::Set(end, len_call) = set_end else {
        return None;
    };
    let Value::Call(len_op, len_args) = len_call.unspan() else {
        return None;
    };
    if *len_op != ops.vector_len && *len_op != ops.length_vector {
        return None;
    }
    let [v_arg] = len_args.as_slice() else {
        return None;
    };
    let Value::Var(v) = v_arg.unspan() else {
        return None;
    };
    // idx = -1
    let Value::Set(idx, start) = set_idx else {
        return None;
    };
    if !matches!(start.unspan(), Value::Int(-1)) {
        return None;
    }
    // loop { i = {#Iter …}; { acc = acc + v[i] } }
    let Value::Loop(body) = lp else {
        return None;
    };
    let loop_stmts = statements(&body.operators);
    let [set_i, inner] = loop_stmts.as_slice() else {
        return None;
    };
    let Value::Set(i, iter) = set_i else {
        return None;
    };
    if !is_counting_step(iter, *idx, *end, ops) {
        return None;
    }
    let Value::Block(inner) = inner else {
        return None;
    };
    let body_stmts = statements(&inner.operators);
    let [add] = body_stmts.as_slice() else {
        return None;
    };
    let acc = accumulates(add, *v, *i, ops)?;
    // The loop's own variables live and die inside it.
    for own in [*end, *idx, *i] {
        if own == acc || own == *v || count(whole, own) != count(n, own) {
            return None;
        }
    }
    Some(Value::Set(
        acc,
        Box::new(Value::Call(
            ops.sum_int,
            vec![Value::Var(*v), Value::Var(acc)],
        )),
    ))
}

/// A statement list without its line markers, unwrapped.
fn statements(list: &[Value]) -> Vec<&Value> {
    list.iter()
        .map(Value::unspan)
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect()
}

/// `{#Iter idx = idx + 1; if end <= idx break; idx}` — the step of `for i in 0..end`.
fn is_counting_step(iter: &Value, idx: u16, end: u16, ops: &Ops) -> bool {
    let Value::Block(block) = iter.unspan() else {
        return false;
    };
    let steps = statements(&block.operators);
    let [inc, test, answer] = steps.as_slice() else {
        return false;
    };
    let is_var = |node: &Value, var: u16| matches!(node.unspan(), Value::Var(x) if *x == var);
    let inc_ok = matches!(inc, Value::Set(target, sum) if *target == idx
        && matches!(sum.unspan(), Value::Call(op, args) if *op == ops.add_int
            && matches!(args.as_slice(), [lhs, rhs]
                if is_var(lhs, idx) && matches!(rhs.unspan(), Value::Int(1)))));
    let test_ok = matches!(test, Value::If(cond, then, other)
        if matches!(cond.unspan(), Value::Call(op, args) if *op == ops.le_int
            && matches!(args.as_slice(), [lhs, rhs] if is_var(lhs, end) && is_var(rhs, idx)))
        && matches!(then.unspan(), Value::Break(0))
        && matches!(other.unspan(), Value::Null));
    inc_ok && test_ok && is_var(answer, idx)
}

/// `acc = acc + v[i]` with `v[i]` an 8-byte integer element: the accumulator.
fn accumulates(add: &Value, vector: u16, index: u16, ops: &Ops) -> Option<u16> {
    let Value::Set(acc, sum) = add else {
        return None;
    };
    let Value::Call(op, args) = sum.unspan() else {
        return None;
    };
    let [lhs, rhs] = args.as_slice() else {
        return None;
    };
    if *op != ops.add_int || !matches!(lhs.unspan(), Value::Var(x) if x == acc) {
        return None;
    }
    let Value::Call(get, get_args) = rhs.unspan() else {
        return None;
    };
    let [elem, Value::Int(0)] = get_args.as_slice() else {
        return None;
    };
    let Value::Call(get_vec, vec_args) = elem.unspan() else {
        return None;
    };
    let ok = *get == ops.get_int
        && *get_vec == ops.get_vector
        && matches!(vec_args.as_slice(), [on, Value::Int(8), at]
            if matches!(on.unspan(), Value::Var(x) if *x == vector)
            && matches!(at.unspan(), Value::Var(x) if *x == index));
    (ok && *acc != vector && *acc != index).then_some(*acc)
}

/// How many nodes of `n` name variable `v`, in any spelling.
fn count(n: &Value, v: u16) -> usize {
    let mut c = 0;
    n.any_node(&mut |x| {
        if crate::ir_shape::names_var(x, v) {
            c += 1;
        }
        false
    });
    c
}
