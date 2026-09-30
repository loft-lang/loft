// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Loop kernels: a whole loop as ONE Rust function both backends call.
//!
//! Each kernel is an ordinary stdlib function — declared without a loft body in
//! `default/*.loft`, implemented here, and reached by the interpreter through its single
//! native-call operator (`OpStaticCall`, a row in `native::FUNCTIONS`) and by `--native`
//! through `codegen_runtime` (a row in `CODEGEN_RUNTIME_FNS`).  Adding a kernel therefore adds
//! a function, never an operator: the opcode table does not grow with the library.
//!
//! What a kernel buys is the interpreter's per-element cost.  A loop the interpreter
//! dispatches element by element costs several operators per element; the kernel runs the
//! loop in compiled Rust, the way native already does (@PLN180 § Kernels).  A kernel answers
//! exactly what the loop it replaces answers — null propagation, overflow reports and all —
//! so replacing a loop by a call is a representation change, never a semantics one.

use crate::data::{Data, DefType, Value};
use crate::keys::DbRef;
use crate::store::Store;
use crate::vector;

/// `LOFT_HOIST_VERIFY=1` re-checks every plain block the sum admits against the checked add,
/// as the native emission does under the same switch.
fn verify() -> bool {
    crate::env_once!(std::env::var_os("LOFT_HOIST_VERIFY").is_some_and(|v| v != "0"))
}

/// `acc + v[0] + v[1] + … + v[len-1]` over an integer vector, with loft's integer add:
/// a null operand gives null and stays null, an overflow reports once and gives null.
///
/// The plain part runs [`vector::sum_blocks_i64`] — blocks whose elements and running total
/// are provably far from the i64 edge, summed vectorised — and the elements it leaves are
/// added one by one with the checked add, which is where a null or an overflow is met.
#[must_use]
pub fn vector_sum_int(stores: &[Store], v: &DbRef, acc: i64) -> i64 {
    let len = i64::from(vector::length_vector(v, stores));
    if len == 0 {
        return acc;
    }
    let header = vector::vec_header(v, stores);
    let base = vector::vec_base(&header, stores);
    // SAFETY: `base` is `vec_base` of the header just read; nothing below writes a store.
    let (mut acc, at) = unsafe {
        if verify() {
            vector::sum_blocks_i64::<true>(base, header.len, 0, len, acc)
        } else {
            vector::sum_blocks_i64::<false>(base, header.len, 0, len, acc)
        }
    };
    for i in at..len {
        // SAFETY: as above; `i < len`.
        let x = unsafe {
            vector::get_elem_at::<i64, false>(&header, base, v, 8, i, 0, i64::MIN, stores)
        };
        acc = crate::ops::op_add_int(acc, x);
    }
    acc
}

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
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    let whole = code.clone();
    let mut fired = 0;
    replace_loops(&mut code, &whole, &ops, &mut fired);
    data.definitions[d_nr as usize].code = code;
    if fired > 0 {
        crate::rewrite_census::fired("R-KernelSum", fired);
    }
}

fn replace_loops(n: &mut Value, whole: &Value, ops: &Ops, fired: &mut usize) {
    if let Some(call) = reduction(n, whole, ops) {
        *n = call;
        *fired += 1;
        return;
    }
    each_child_mut(n, &mut |c| replace_loops(c, whole, ops, fired));
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
    let lstmts = statements(&body.operators);
    let [set_i, inner] = lstmts.as_slice() else {
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
    let istmts = statements(&inner.operators);
    let [add] = istmts.as_slice() else {
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
    let Value::Block(b) = iter.unspan() else {
        return false;
    };
    let s = statements(&b.operators);
    let [inc, test, answer] = s.as_slice() else {
        return false;
    };
    let is_var = |n: &Value, v: u16| matches!(n.unspan(), Value::Var(x) if *x == v);
    let inc_ok = matches!(inc, Value::Set(x, e) if *x == idx
        && matches!(e.unspan(), Value::Call(op, a) if *op == ops.add_int
            && matches!(a.as_slice(), [l, r] if is_var(l, idx) && matches!(r.unspan(), Value::Int(1)))));
    let test_ok = matches!(test, Value::If(c, t, e)
        if matches!(c.unspan(), Value::Call(op, a) if *op == ops.le_int
            && matches!(a.as_slice(), [l, r] if is_var(l, end) && is_var(r, idx)))
        && matches!(t.unspan(), Value::Break(0))
        && matches!(e.unspan(), Value::Null));
    inc_ok && test_ok && is_var(answer, idx)
}

/// `acc = acc + v[i]` with `v[i]` an 8-byte integer element: the accumulator.
fn accumulates(add: &Value, v: u16, i: u16, ops: &Ops) -> Option<u16> {
    let Value::Set(acc, e) = add else {
        return None;
    };
    let Value::Call(op, a) = e.unspan() else {
        return None;
    };
    let [l, r] = a.as_slice() else {
        return None;
    };
    if *op != ops.add_int || !matches!(l.unspan(), Value::Var(x) if x == acc) {
        return None;
    }
    let Value::Call(get, ga) = r.unspan() else {
        return None;
    };
    let [elem, Value::Int(0)] = ga.as_slice() else {
        return None;
    };
    let Value::Call(gv, va) = elem.unspan() else {
        return None;
    };
    let ok = *get == ops.get_int
        && *gv == ops.get_vector
        && matches!(va.as_slice(), [vv, Value::Int(8), ii]
            if matches!(vv.unspan(), Value::Var(x) if *x == v)
            && matches!(ii.unspan(), Value::Var(x) if *x == i));
    (ok && *acc != v && *acc != i).then_some(*acc)
}

/// How many nodes of `n` name variable `v`, in any spelling.
fn count(n: &Value, v: u16) -> usize {
    let mut c = 0;
    n.any_node(&mut |x| {
        if crate::value_record::names_var(x, v) {
            c += 1;
        }
        false
    });
    c
}

/// Call `f` on each direct child of `n`, mutably.
fn each_child_mut(n: &mut Value, f: &mut impl FnMut(&mut Value)) {
    match n {
        Value::Span(s) => f(&mut s.1),
        Value::Call(_, a)
        | Value::CallRef(_, a)
        | Value::Insert(a)
        | Value::Tuple(a)
        | Value::Parallel(a) => a.iter_mut().for_each(f),
        Value::Block(b) | Value::Loop(b) => b.operators.iter_mut().for_each(f),
        Value::Set(_, x)
        | Value::Return(x)
        | Value::Drop(x)
        | Value::Yield(x)
        | Value::TuplePut(_, _, x) => f(x),
        Value::If(a, b, c) | Value::Iter(_, a, b, c) => {
            f(a);
            f(b);
            f(c);
        }
        _ => {}
    }
}
