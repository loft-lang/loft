// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-R-HostStore` (@PLN219) — a recursion that writes its result back into the field it came
//! from builds that result in the field's own store.
//!
//! `t.left = insert_rec(t.left, k)` hands the callee a return buffer; a callee that answers a
//! fresh record builds it there, in a store of its own, and the write-back (`write_back_call`)
//! copied it into `t`'s store and freed the buffer's store — a store made and freed per call.
//! Where every answer of the callee is its own argument (the field's child, handed in) or its
//! return buffer, the caller hands it a buffer naming `t`'s STORE (`OpStoreBuf`): the callee's
//! `OpDatabase` claims the record there, and the field ADOPTS it by its number
//! (`OpAdoptChildRec`); the argument handed back is the child the field already holds.
//!
//! The LITERAL clause is the callee half of the same move: `Add { l: gen(…), r: gen(…) }` builds
//! the node, then calls `gen` for each child with a buffer of its own and copies the answer into
//! the node's field (`OpSetChildRec`), freeing the answer's store.  Where every answer of the
//! callee is its return buffer, the call is handed a buffer naming the NODE's store, the field
//! adopts the record by number, and the answer's store is freed only where it is not the
//! node's (a fallback copy).  A callee whose literal exits build into its buffer
//! (`@FR-R-Place`'s callee clause) is such a callee; the recursion then builds the whole tree
//! in the store its first caller names.
//!
//! Decided after every body is settled, on the callee's final IR.  `LOFT_NO_HOST_STORE=1`.

use crate::data::{Data, DefType, Value};

/// Rewrite every admitted write-back in `fresh`, the functions the scope pass is checking.
pub fn rewrite_program(data: &mut Data, fresh: &[u32]) {
    if std::env::var("LOFT_NO_HOST_STORE").is_ok_and(|v| v != "0") {
        return;
    }
    let ids = Ids {
        free_ref: data.def_nr("OpFreeRef"),
        free_distinct: data.def_nr("OpFreeRefIfDistinct"),
        set_child: data.def_nr("OpSetChildRec"),
        adopt: data.def_nr("OpAdoptChildRec"),
        store_buf: data.def_nr("OpStoreBuf"),
        get_field: data.def_nr("OpGetField"),
        from_child: data.def_nr("OpRefFromChildRec"),
    };
    if [ids.adopt, ids.store_buf].contains(&u32::MAX) {
        return;
    }
    for &d_nr in fresh {
        if data.def_type(d_nr) != DefType::Function
            || !calls(&data.definitions[d_nr as usize].code, ids.set_child)
        {
            continue;
        }
        // The proof reads the callee's code, and a recursion's callee is this function: it is
        // read from a snapshot taken before the rewrite takes the body out.
        let snapshot = data.definitions[d_nr as usize].code.clone();
        let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
        let mut n = 0;
        let me = Me {
            d_nr,
            code: &snapshot,
        };
        rewrite(&mut code, data, &ids, &me, &mut n);
        data.definitions[d_nr as usize].code = code;
        if n > 0 {
            let _census = crate::rewrite_census::InBody::enter("ir", data.def(d_nr).name());
            crate::rewrite_census::fired("R-HostStore", n);
        }
    }
    crate::host_result::rewrite_program(data);
}

/// Whether `n` calls `op` anywhere.  Both clauses rewrite only at an `OpSetChildRec` (the
/// write-back's adopt and the literal field's write), so a body without one is skipped before
/// the snapshot below, which would otherwise copy every function's IR on every compile.
fn calls(n: &Value, op: u32) -> bool {
    if matches!(n.unspan(), Value::Call(f, _) if *f == op) {
        return true;
    }
    let mut found = false;
    n.for_each_child(&mut |c| found = found || calls(c, op));
    found
}

/// The function being rewritten, with its body as it was before the rewrite.
struct Me<'a> {
    d_nr: u32,
    code: &'a Value,
}

fn code_of<'a>(data: &'a Data, me: &'a Me, f: u32) -> &'a Value {
    if f == me.d_nr {
        me.code
    } else {
        data.def(f).code()
    }
}

struct Ids {
    free_ref: u32,
    free_distinct: u32,
    set_child: u32,
    adopt: u32,
    store_buf: u32,
    get_field: u32,
    from_child: u32,
}

fn same(a: &Value, b: &Value) -> bool {
    match (a.unspan(), b.unspan()) {
        (Value::Var(x), Value::Var(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Call(f, xs), Value::Call(g, ys)) => {
            f == g && xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| same(x, y))
        }
        _ => false,
    }
}

fn rewrite(n: &mut Value, data: &Data, ids: &Ids, me: &Me, count: &mut usize) {
    if let Value::Block(bl) = n.unspan()
        && bl.name == "write_back_call"
        && std::env::var("LOFT_TRACE_HOST_STORE").is_ok()
    {
        eprintln!(
            "[host-store] write-back {:?}",
            admit_why(&bl.operators, data, ids, me)
        );
    }
    if let Value::Block(bl) = n.unspan_mut()
        && bl.name == "write_back_call"
        && bl.operators.len() >= 2
        && let Some(plan) = admit(&bl.operators, data, ids, me)
    {
        let (host, buf_at) = plan;
        if let Value::Set(_, call) = bl.operators[0].unspan_mut()
            && let Value::Call(_, args) = call.unspan_mut()
        {
            args[buf_at] = Value::Call(ids.store_buf, vec![Value::Var(host)]);
        }
        if let Value::Call(op, _) = bl.operators[1].unspan_mut() {
            *op = ids.adopt;
        }
        *count += 1;
    }
    if let Value::Block(bl) = n.unspan_mut() {
        literal_clause(&mut bl.operators, data, ids, me, count);
    }
    n.for_each_child_mut(&mut |c| rewrite(c, data, ids, me, count));
}

/// The LITERAL clause: in one block, `lift = f(…, buf)` followed by
/// `OpSetChildRec(OpGetField(node, …), lift, tp)`, where `lift` is named nowhere else but by
/// its frees — the field of a record literal handed a call's answer.  Where every answer of
/// `f` is its buffer, `f` is handed `OpStoreBuf(node)`, the field adopts the answer by number,
/// and each `OpFreeRef(lift)` becomes `OpFreeRefIfDistinct(lift, node)`: the answer's store
/// is freed only where it is not the node's.
fn literal_clause(ops: &mut [Value], data: &Data, ids: &Ids, me: &Me, count: &mut usize) {
    let trace = std::env::var("LOFT_TRACE_HOST_STORE").is_ok();
    for i in 0..ops.len().saturating_sub(1) {
        let Some((lift, host, buf_at)) = literal_site(&ops[i], &ops[i + 1], data, ids, me) else {
            continue;
        };
        let vars = data.def(me.d_nr).variables();
        let here: usize = ops.iter().map(|o| uses(o, lift)).sum();
        if !vars.is_compiler_generated(lift)
            || here != uses(me.code, lift)
            || !lift_only_freed(ops, lift, i, ids)
        {
            if trace {
                eprintln!("[host-store] literal field: its answer is named elsewhere");
            }
            continue;
        }
        if let Value::Set(_, call) = ops[i].unspan_mut()
            && let Value::Call(_, args) = call.unspan_mut()
        {
            args[buf_at] = Value::Call(ids.store_buf, vec![Value::Var(host)]);
        }
        if let Value::Call(op, args) = ops[i + 1].unspan_mut() {
            *op = ids.adopt;
            if let Some(Value::Int(tp)) = args.get_mut(2) {
                *tp &= i32::from(crate::keys::COPY_TP_MASK);
            }
        }
        for op in ops.iter_mut() {
            if let Value::Call(f, args) = op.unspan_mut()
                && *f == ids.free_ref
                && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == lift)
            {
                *f = ids.free_distinct;
                args.push(Value::Var(host));
            }
        }
        if trace {
            eprintln!("[host-store] literal field: admitted");
        }
        *count += 1;
    }
}

/// `lift = f(…)` then `OpSetChildRec(OpGetField(host, …), lift, …)` with `f` answering only its
/// buffer: the lift, the host and the buffer's argument position.
fn literal_site(
    bind: &Value,
    write: &Value,
    data: &Data,
    ids: &Ids,
    me: &Me,
) -> Option<(u16, u16, usize)> {
    let Value::Set(lift, call) = bind.unspan() else {
        return None;
    };
    let Value::Call(f, _) = call.unspan() else {
        return None;
    };
    let Value::Call(sc, sargs) = write.unspan() else {
        return None;
    };
    if *sc != ids.set_child
        || !matches!(sargs.get(1).map(Value::unspan), Some(Value::Var(x)) if x == lift)
    {
        return None;
    }
    let Value::Call(gf, ga) = sargs.first()?.unspan() else {
        return None;
    };
    let Some(Value::Var(host)) = ga.first().map(Value::unspan) else {
        return None;
    };
    if *gf != ids.get_field || data.def_type(*f) != DefType::Function {
        return None;
    }
    let buf_at = data.def(*f).hidden_return_buffer_attr()?;
    let buf = u16::try_from(buf_at).ok()?;
    let ok = answers_only(code_of(data, me, *f), u16::MAX, buf, data);
    if std::env::var("LOFT_TRACE_HOST_STORE").is_ok() && !ok {
        eprintln!(
            "[host-store] literal field: {} answers something else, or uses its buffer otherwise",
            data.def(*f).name()
        );
    }
    ok.then_some((*lift, *host, buf_at))
}

/// Is `lift` named, outside its bind at `at` and the field write after it, only by a null
/// init or an `OpFreeRef` in this block?
fn lift_only_freed(ops: &[Value], lift: u16, at: usize, ids: &Ids) -> bool {
    ops.iter().enumerate().all(|(j, op)| {
        if j == at || j == at + 1 {
            return true;
        }
        match op.unspan() {
            Value::Set(x, v) if *x == lift => matches!(v.unspan(), Value::Null),
            Value::Call(f, args)
                if *f == ids.free_ref
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == lift) =>
            {
                true
            }
            other => !other.names_var_here(lift) && !mentions(other, lift),
        }
    })
}

/// The non-null mentions of `x`: its reads and its non-null sets.
fn uses(code: &Value, var: u16) -> usize {
    let mut count = 0;
    code.walk(&mut |node| match node {
        Value::Var(seen) if *seen == var => count += 1,
        Value::Set(seen, rhs) if *seen == var && !matches!(rhs.unspan(), Value::Null) => {
            count += 1;
        }
        _ => {}
    });
    count
}

fn mentions(v: &Value, x: u16) -> bool {
    let mut hit = false;
    v.walk(&mut |n| {
        if matches!(n, Value::Var(y) | Value::Set(y, _) if *y == x) {
            hit = true;
        }
    });
    hit
}

/// Why `admit` refuses, for `LOFT_TRACE_HOST_STORE`.
fn admit_why(ops: &[Value], data: &Data, ids: &Ids, me: &Me) -> &'static str {
    if ops.len() < 2 {
        return "short block";
    }
    let Value::Set(_, call) = ops[0].unspan() else {
        return "first op is not the call's bind";
    };
    let Value::Call(f, args) = call.unspan() else {
        return "the bind is not a call";
    };
    let Value::Call(sc, sargs) = ops[1].unspan() else {
        return "second op is not a call";
    };
    if *sc != ids.set_child {
        return "second op is not OpSetChildRec";
    }
    let Some(field) = sargs.first() else {
        return "no field";
    };
    let def = data.def(*f);
    let Some(buf_at) = def.hidden_return_buffer_attr() else {
        return "no hidden buffer";
    };
    let child = Value::Call(ids.from_child, vec![field.clone()]);
    let Some(arg_at) = args.iter().position(|a| same(a, &child)) else {
        return "no argument is the field's child";
    };
    if !answers_only(code_of(data, me, *f), arg_at as u16, buf_at as u16, data) {
        return "the callee answers something else, or uses its buffer otherwise";
    }
    if admit(ops, data, ids, me).is_some() {
        "admitted"
    } else {
        "host not a variable"
    }
}

/// The host variable and the buffer's argument position of an admitted write-back.
fn admit(ops: &[Value], data: &Data, ids: &Ids, me: &Me) -> Option<(u16, usize)> {
    let Value::Set(r, call) = ops[0].unspan() else {
        return None;
    };
    let Value::Call(f, args) = call.unspan() else {
        return None;
    };
    let Value::Call(sc, sargs) = ops[1].unspan() else {
        return None;
    };
    if *sc != ids.set_child
        || !matches!(sargs.get(1).map(Value::unspan), Some(Value::Var(x)) if x == r)
    {
        return None;
    }
    let field = sargs.first()?;
    let Value::Call(gf, ga) = field.unspan() else {
        return None;
    };
    let Some(Value::Var(host)) = ga.first().map(Value::unspan) else {
        return None;
    };
    if *gf != ids.get_field {
        return None;
    }
    let def = data.def(*f);
    let buf_at = def.hidden_return_buffer_attr()?;
    let child = Value::Call(ids.from_child, vec![field.clone()]);
    let arg_at = args.iter().position(|a| same(a, &child))?;
    let (Ok(buf), Ok(arg)) = (u16::try_from(buf_at), u16::try_from(arg_at)) else {
        return None;
    };
    answers_only(code_of(data, me, *f), arg, buf, data).then_some((*host, buf_at))
}

/// Does every answer of the callee name its argument `arg` or its buffer `buf`, and does the
/// body use `buf` only as a buffer is used — minted by `OpDatabase`, its fields written, tested,
/// a witness of a release, answered?
fn answers_only(code: &Value, arg: u16, buf: u16, data: &Data) -> bool {
    fn tail(v: &Value) -> &Value {
        match v.unspan() {
            Value::Block(bl) => bl.operators.last().map_or(v, tail),
            other => other,
        }
    }
    let mut ok = true;
    let mut answers = 0;
    code.walk(&mut |n| {
        if !ok {
            return;
        }
        match n {
            Value::Return(e) => {
                answers += 1;
                if !matches!(tail(e), Value::Var(x) if *x == arg || *x == buf) {
                    if std::env::var("LOFT_TRACE_HOST_STORE").is_ok() {
                        eprintln!(
                            "[host-store]   answers {:?} (arg {arg}, buf {buf})",
                            tail(e)
                        );
                    }
                    ok = false;
                }
            }
            Value::Call(op, a) => {
                let name = data.def(*op).name();
                for (i, x) in a.iter().enumerate() {
                    if !matches!(x.unspan(), Value::Var(y) if *y == buf) {
                        continue;
                    }
                    let fine = match name {
                        "OpDatabase" | "OpGetField" | "OpRefIsNull" | "OpConvBoolFromRef"
                        | "OpStoreBuf" => i == 0,
                        "OpFreeRefIfDistinct" => i == 1,
                        n => n.starts_with("OpSet") && i == 0,
                    };
                    if !fine {
                        if std::env::var("LOFT_TRACE_HOST_STORE").is_ok() {
                            eprintln!("[host-store]   uses its buffer in {name} at {i}");
                        }
                        ok = false;
                    }
                }
            }
            Value::Set(x, _) if *x == buf => {
                if std::env::var("LOFT_TRACE_HOST_STORE").is_ok() {
                    eprintln!("[host-store]   sets its buffer");
                }
                ok = false;
            }
            _ => {}
        }
    });
    ok && answers > 0
}
