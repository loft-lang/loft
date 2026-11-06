// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

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
//! Decided after every body is settled, on the callee's final IR.  `LOFT_NO_HOST_STORE=1`.

use crate::data::{Data, DefType, Value};

/// Rewrite every admitted write-back in the program.
pub fn rewrite_program(data: &mut Data) {
    if std::env::var("LOFT_NO_HOST_STORE").is_ok_and(|v| v != "0") {
        return;
    }
    let ids = Ids {
        set_child: data.def_nr("OpSetChildRec"),
        adopt: data.def_nr("OpAdoptChildRec"),
        store_buf: data.def_nr("OpStoreBuf"),
        get_field: data.def_nr("OpGetField"),
        from_child: data.def_nr("OpRefFromChildRec"),
    };
    if [ids.adopt, ids.store_buf].contains(&u32::MAX) {
        return;
    }
    for d_nr in 0..data.definitions() {
        if data.def_type(d_nr) != DefType::Function {
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
    n.for_each_child_mut(&mut |c| rewrite(c, data, ids, me, count));
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
                        "OpDatabase" | "OpGetField" | "OpRefIsNull" | "OpConvBoolFromRef" => i == 0,
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
