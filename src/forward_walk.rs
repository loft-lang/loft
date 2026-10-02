// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-ForwardWalk` — a forward walk over a vector the loop body cannot resize, decided in
//! the IR phase for both backends.
//!
//! `for p in v` lowers to a walk that re-reads the vector's length every round and also tests
//! `p#index < 0`: an `#remove` inside the body shrinks the vector and steps the index back, and
//! the reverse step sets the index below zero.  When the body can do neither, the walk is a
//! counted loop: the length is read once before it, the iterator becomes the counted range's
//! `Iter range` block over the same `p#index`, and the element read opens the body.  Then
//! `(R-Rotate)` and `(R-LoopSlot)` apply as they do to `for i in 0..n`.
//!
//! "Cannot resize" is decided by shape, and every unrecognised shape declines (keeping the walk
//! as it is):
//! - the walked vector — the source local and the walk's own `_vector_N` copy of the handle —
//!   appears in the body only as a direct argument of an element read or of the length;
//! - neither those two nor `p#index` is assigned in the body;
//! - no closure captures the source, and the body calls no function through a reference;
//! - when the source is a parameter, no operator or call in the body receives a `&` parameter,
//!   which the caller may have passed the same vector as.
//!
//! `LOFT_NO_FORWARD_WALK=1` keeps every walk; `LOFT_TRACE_FORWARD_WALK=1` names each one rewritten
//! or declined, and why.
use crate::data::{Block, Data, DefType, Type, Value};

fn off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_FORWARD_WALK").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_FORWARD_WALK").is_ok_and(|v| v != "0"))
}

/// Operators that read a vector without changing its length — the only places the walked
/// vector may appear in the body.
const READ_OPS: [&str; 5] = [
    "OpGetVector",
    "OpGetVectorNullable",
    "OpLengthVector",
    "OpVecGetInt",
    "OpVecGetIntNullable",
];

struct Ops {
    add_int: u32,
    le_int: u32,
    lt_int: u32,
    length_vector: u32,
    get_vector_nullable: u32,
}

/// One matched walk: the parts the rewrite keeps.
struct Walk {
    vec_t: u16,
    src: u16,
    idx: u16,
    lv: u16,
    /// The element read as the walk spells it — the element reference
    /// `OpGetVectorNullable(v, size, idx)`, or a scalar element's value read over it
    /// (`OpGetInt(…, 0)`) — rebuilt over the counter.
    read: Value,
    iter_scope: u16,
}

fn int(v: &Value) -> Option<i32> {
    match v.unspan() {
        Value::Int(k) => Some(*k),
        _ => None,
    }
}

fn var(v: &Value) -> Option<u16> {
    match v.unspan() {
        Value::Var(n) => Some(*n),
        _ => None,
    }
}

fn call(v: &Value, op: u32) -> Option<&[Value]> {
    match v.unspan() {
        Value::Call(d, args) if *d == op => Some(args),
        _ => None,
    }
}

/// `if <c> break else null`, either spelling of the break.
fn break_test(v: &Value) -> Option<&Value> {
    let Value::If(c, then_arm, else_arm) = v.unspan() else {
        return None;
    };
    let brk = match then_arm.unspan() {
        Value::Break(0) => true,
        Value::Block(b) => {
            b.operators.len() == 1 && matches!(b.operators[0].unspan(), Value::Break(0))
        }
        _ => false,
    };
    (brk && matches!(else_arm.unspan(), Value::Null)).then_some(c)
}

fn plain(ops: &[Value]) -> Vec<&Value> {
    ops.iter()
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect()
}

/// The walk a `For block` lowers `for p in v` to, forward, over a vector local or parameter —
/// or `None` for any other shape.
fn match_walk(ops: &Ops, block: &Block) -> Option<(Walk, usize)> {
    if block.name != "For block" {
        return None;
    }
    let st = plain(&block.operators);
    let [
        Value::Set(vec_t, src),
        Value::Set(idx, start),
        Value::Loop(lp),
    ] = st.as_slice()
    else {
        return None;
    };
    let src = var(src)?;
    if int(start)? != -1 || lp.name != "For loop" {
        return None;
    }
    let body = plain(&lp.operators);
    if body.len() < 3 {
        return None;
    }
    let Value::Set(lv, next) = body[0] else {
        return None;
    };
    let Value::Block(nb) = next.unspan() else {
        return None;
    };
    let nops = plain(&nb.operators);
    let [Value::Set(step_var, step), read] = nops.as_slice() else {
        return None;
    };
    let step_args = call(step, ops.add_int)?;
    if nb.name != "iter next"
        || *step_var != *idx
        || var(&step_args[0])? != *idx
        || int(&step_args[1])? != 1
    {
        return None;
    }
    // The element reference, alone or under one value read with literal operands.
    let elem = match read.unspan() {
        Value::Call(d, a) if *d == ops.get_vector_nullable => read,
        Value::Call(_, a) if !a.is_empty() && a[1..].iter().all(|x| int(x).is_some()) => &a[0],
        _ => return None,
    };
    let rargs = call(elem, ops.get_vector_nullable)?;
    if var(&rargs[0])? != *vec_t || var(&rargs[2])? != *idx {
        return None;
    }
    int(&rargs[1])?;
    // The two tests the forward walk carries: past the end, and below the start.
    let end = call(break_test(body[1])?, ops.le_int)?;
    let len_args = call(&end[0], ops.length_vector)?;
    if var(&len_args[0])? != *vec_t || var(&end[1])? != *idx {
        return None;
    }
    let below = call(break_test(body[2])?, ops.lt_int)?;
    if var(&below[0])? != *idx || int(&below[1])? != 0 {
        return None;
    }
    let first_body = lp.operators.iter().position(|o| std::ptr::eq(o, body[2]))? + 1;
    Some((
        Walk {
            vec_t: *vec_t,
            src,
            idx: *idx,
            lv: *lv,
            read: (*read).clone(),
            iter_scope: nb.scope,
        },
        first_body,
    ))
}

/// A `&` parameter: the one kind through which a function writes a vector its caller can
/// also have passed under another name.  A by-value vector parameter is either read-only or a
/// result buffer the scope pass promoted, which the caller always fills from its own fresh
/// work buffer, never from a vector the program named.
fn by_reference(tp: &Type) -> bool {
    matches!(tp, Type::RefVar(_))
}

/// Can `body` resize the walked vector?  `Err` names why a shape is declined.
fn check_body(
    data: &Data,
    vars: &crate::variables::Function,
    w: &Walk,
    node: &Value,
) -> Result<(), &'static str> {
    let alias = |n: u16| n == w.vec_t || n == w.src;
    let src_is_param = vars.is_argument(w.src);
    match node {
        Value::Var(n) if alias(*n) => Err("the vector is used other than by an element read"),
        Value::Set(n, e) => {
            if alias(*n) || *n == w.idx {
                return Err("the body assigns the vector or the index");
            }
            check_body(data, vars, w, e)
        }
        Value::CallRef(..) | Value::Yield(_) | Value::Parallel(_) => {
            Err("a call through a reference, a yield or a par in the body")
        }
        Value::Call(op, args) => {
            let name = data.def(*op).name();
            let reads = READ_OPS.contains(&name);
            for a in args {
                if let Some(n) = var(a) {
                    if alias(n) {
                        if !reads {
                            return Err("the vector is handed to an operator that may resize it");
                        }
                        continue;
                    }
                    if src_is_param && n != w.src && vars.is_argument(n) && by_reference(vars.tp(n))
                    {
                        return Err("a `&` parameter, which may be the same vector");
                    }
                }
                check_body(data, vars, w, a)?;
            }
            Ok(())
        }
        other => {
            let mut res = Ok(());
            let mut node = other.clone();
            node.for_each_child_mut(&mut |c| {
                if res.is_ok() {
                    res = check_body(data, vars, w, c);
                }
            });
            res
        }
    }
}

/// Rewrite every admitted walk in `v`; answers how many.
fn rewrite_in(
    data: &Data,
    ops: &Ops,
    vars: &mut crate::variables::Function,
    v: &mut Value,
    fname: &str,
) -> usize {
    let mut n = 0;
    v.for_each_child_mut(&mut |c| n += rewrite_in(data, ops, vars, c, fname));
    let Value::Block(block) = v else {
        return n;
    };
    let Some((w, first_body)) = match_walk(ops, block) else {
        return n;
    };
    let decline = if vars.is_captured(w.src) {
        Err("a closure captures the vector")
    } else {
        let Some(Value::Loop(lp)) = block.operators.iter().find(|o| matches!(o, Value::Loop(_)))
        else {
            return n;
        };
        lp.operators[first_body..]
            .iter()
            .try_for_each(|o| check_body(data, vars, &w, o))
    };
    if let Err(why) = decline {
        if trace() {
            eprintln!("forward-walk: {fname} {} declined: {why}", vars.name(w.lv));
        }
        return n;
    }
    if trace() {
        eprintln!("forward-walk: {fname} {} counted", vars.name(w.lv));
    }
    let int_tp = vars.tp(w.idx).clone();
    let len = vars.add_unique("walk_len", &int_tp, block.scope);
    let at = vars.add_unique("walk_at", &int_tp, w.iter_scope);
    // The length, read once before the loop, after the handle is taken.
    let set_vec = block
        .operators
        .iter()
        .position(|o| matches!(o, Value::Set(x, _) if *x == w.vec_t))
        .expect("matched");
    block.operators.insert(
        set_vec + 1,
        Value::Set(
            len,
            Box::new(Value::Call(ops.length_vector, vec![Value::Var(w.vec_t)])),
        ),
    );
    let lp = block
        .operators
        .iter_mut()
        .find_map(|o| match o {
            Value::Loop(lp) => Some(lp),
            _ => None,
        })
        .expect("matched");
    let body: Vec<Value> = lp.operators.drain(first_body..).collect();
    let iter = Value::Block(Box::new(Block {
        name: "Iter range",
        operators: vec![
            Value::Set(
                w.idx,
                Box::new(Value::Call(
                    ops.add_int,
                    vec![Value::Var(w.idx), Value::Int(1)],
                )),
            ),
            Value::If(
                Box::new(Value::Call(
                    ops.le_int,
                    vec![Value::Var(len), Value::Var(w.idx)],
                )),
                Box::new(Value::Break(0)),
                Box::new(Value::Null),
            ),
            Value::Var(w.idx),
        ],
        result: int_tp,
        scope: w.iter_scope,
        var_size: 0,
    }));
    let mut read = w.read.clone();
    read.map_nodes(&mut |n| {
        if matches!(n, Value::Var(x) if *x == w.idx) {
            *n = Value::Var(at);
        }
    });
    let element = Value::Set(w.lv, Box::new(read));
    lp.operators = vec![Value::Set(at, Box::new(iter)), element];
    lp.operators.extend(body);
    n + 1
}

/// Rewrite every admitted forward walk in the program; answers how many.
pub fn rewrite_program(data: &mut Data) -> usize {
    if off() {
        return 0;
    }
    let ops = Ops {
        add_int: data.def_nr("OpAddInt"),
        le_int: data.def_nr("OpLeInt"),
        lt_int: data.def_nr("OpLtInt"),
        length_vector: data.def_nr("OpLengthVector"),
        get_vector_nullable: data.def_nr("OpGetVectorNullable"),
    };
    let mut total = 0;
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type != DefType::Function || matches!(def.code(), Value::Null) {
            continue;
        }
        let walks = def
            .code()
            .any_node(&mut |n| matches!(n, Value::Block(b) if b.name == "For block"));
        if !walks {
            continue;
        }
        let fname = def.name().to_string();
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let mut vars = data.definitions[d as usize].variables.clone();
        let n = rewrite_in(data, &ops, &mut vars, &mut code, &fname);
        data.definitions[d as usize].variables = vars;
        data.definitions[d as usize].code = code;
        if n > 0 {
            data.definitions[d as usize].variables.reset_intervals();
            crate::scopes::compute_function_intervals(data, d);
            crate::scopes::assign_function_slots(data, d);
            total += n;
        }
    }
    crate::rewrite_census::fired("R-ForwardWalk", total);
    total
}
