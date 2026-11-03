// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-SliceArg`'s range clause in the IR phase: `text_from_bytes(v[lo..hi])` is the
//! slice's bound statements and `text_from_byte_range(v, lo, hi)` — the bytes read where they
//! lie, so neither backend builds the slice.
//!
//! A slice used as a value is lowered to a block that mints a store for it, binds a temporary
//! `T` to the store's vector, zeroes it, computes the clamped bounds and copies the span in:
//! `{ OpDatabase(W, tp); T = OpGetField(W, 0, …); OpSetInt4(W, 0, 0); <bounds>;
//! OpSliceView(T, src, lo, hi, row); T }`.  Handed straight to `text_from_bytes`, the copy is
//! read once and dropped: `text_from_byte_range` decodes the same `[lo, hi)` of `src` (the
//! bounds already clamped by the statements kept) with the same conversion.
//!
//! Admitted where the block is exactly that — every statement that names `T` or its store
//! `W` is the mint, the bind, the zero or the one slice, and the slice's operands name
//! neither — and where, outside the block, `W` is only declared null and freed and `T` is not
//! named at all: then `W` is never minted and its frees pass a null by.  Anything else keeps
//! the slice.  The C writer's clause (`FnWriter::call`, the `n_text_from_bytes` arm) matches
//! the same block; with this pass on it finds none.
//!
//! `LOFT_NO_IR_SLICE_TEXT=1` keeps every slice (the IR clause off); `LOFT_TRACE_SLICE_TEXT=1`
//! names each site admitted and each declined with why.
use crate::data::{Data, DefType, Value, v_block};
use crate::variables::Function;

const RULE: &str = "R-SliceArg/range";

struct Ops {
    from_bytes: u32,
    from_range: u32,
    slice: [u32; 2],
    keep: [u32; 4],
    frees: [u32; 3],
    get_field: u32,
    append_text: u32,
}

/// One admitted site: the temporary `T`, its store `W`.
struct Site {
    tmp: u16,
    store: u16,
}

fn disabled() -> bool {
    // Opt-in (`LOFT_IR_SLICE_TEXT=1`) until the move protocol's comparison passes: the clause
    // answered a wrong text on its probe (the range call as a block's value).
    crate::env_once!(std::env::var("LOFT_NO_IR_SLICE_TEXT").is_ok_and(|v| v != "0"))
        || !crate::env_once!(std::env::var("LOFT_IR_SLICE_TEXT").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_SLICE_TEXT").is_ok_and(|v| v != "0"))
}

/// Rewrite every admitted `text_from_bytes(<slice block>)` of function `d_nr`.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if disabled() || data.def_type(d_nr) != DefType::Function || !data.def(d_nr).is_loft_defined() {
        return;
    }
    let ops = Ops {
        from_bytes: data.def_nr("n_text_from_bytes"),
        from_range: data.def_nr("n_text_from_byte_range"),
        slice: [data.def_nr("OpSliceView"), data.def_nr("OpSliceVector")],
        keep: [
            data.def_nr("OpDatabase"),
            data.def_nr("OpDatabaseNP"),
            data.def_nr("OpSetInt4"),
            data.def_nr("OpClearVector"),
        ],
        // Every one passes a null store by (`free_displaced`, `loft_free_displaced`).
        frees: [
            data.def_nr("OpFreeRef"),
            data.def_nr("OpFreeRecordIn"),
            data.def_nr("OpFreeRefIfDistinct"),
        ],
        get_field: data.def_nr("OpGetField"),
        append_text: data.def_nr("OpAppendText"),
    };
    if ops.from_bytes == u32::MAX || ops.from_range == u32::MAX {
        return;
    }
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    let mut sites: Vec<Site> = Vec::new();
    {
        let vars = data.def(d_nr).variables();
        let mut assigned = 0usize;
        let mut calls = 0usize;
        code.walk(&mut |n| {
            if let Some(block) = host(n, &ops) {
                assigned += 1;
                match admit(block, vars, &ops) {
                    Ok(site) => sites.push(site),
                    Err(why) => decline(data, d_nr, None, why),
                }
            }
            if let Value::Call(f, args) = n
                && *f == ops.from_bytes
                && args.len() == 1
                && slice_block(&args[0]).is_some()
            {
                calls += 1;
            }
        });
        // A call held anywhere else is left alone: as a block's value the code generator would
        // have no destination to write the text into (the parser's own text calls are always
        // assigned or appended), and inside a larger operand an earlier operand would run after
        // the hoisted bounds.
        for _ in assigned..calls {
            decline(
                data,
                d_nr,
                None,
                "the text is neither assigned nor appended to a variable",
            );
        }
        sites.retain(|s| match outside_mentions(&code, s, &ops) {
            Ok(()) => true,
            Err(why) => {
                decline(data, d_nr, Some(vars.name(s.tmp)), why);
                false
            }
        });
    }
    let mut fired = 0usize;
    if !sites.is_empty() {
        let vars = data.def(d_nr).variables();
        code.map_nodes(&mut |n| {
            // The bare statement, not a span around it: `map_nodes` reaches the node inside a
            // span too, and rewriting that one keeps the statement's source line.
            let Some(block) = host(n, &ops) else {
                return;
            };
            let Ok(site) = admit(block, vars, &ops) else {
                return;
            };
            if !sites.iter().any(|s| s.tmp == site.tmp) {
                return;
            }
            let (prelude, src, lo, hi) = split(block, &site, &ops);
            let mut stmts = prelude;
            stmts.push(rebuilt(
                n,
                Value::Call(ops.from_range, vec![src, lo, hi]),
                &ops,
            ));
            // The bounds' variables belong to the slice block's scope: the statement block that
            // replaces the assignment takes it over.
            let scope = block.scope;
            *n = v_block(stmts, crate::data::Type::Void, "slice text");
            if let Value::Block(bl) = n {
                bl.scope = scope;
            }
            fired += 1;
            if trace() {
                eprintln!(
                    "[slice-text] fn={} {} reads its span in place",
                    data.def(d_nr).name(),
                    vars.name(site.tmp)
                );
            }
        });
    }
    data.definitions[d_nr as usize].code = code;
    crate::rewrite_census::fired(RULE, fired);
}

/// A statement that holds the call where nothing is evaluated before it: the assignment
/// `x = text_from_bytes(<slice>)`, or the append `OpAppendText(w, text_from_bytes(<slice>))`
/// into a plain variable `w` — the parser's text build.  Each is rebuilt around the range call
/// with the bound statements before it, so no operand that ran before the call runs after them.
fn host<'v>(n: &'v Value, ops: &Ops) -> Option<&'v crate::data::Block> {
    match n {
        Value::Set(_, rhs) => slice_call(rhs, ops),
        Value::Call(f, args)
            if *f == ops.append_text
                && let [w, call] = args.as_slice()
                && matches!(w.unspan(), Value::Var(_)) =>
        {
            slice_call(call, ops)
        }
        _ => None,
    }
}

/// `n` (a [`host`]) with its call replaced by `range`.
fn rebuilt(n: &Value, range: Value, ops: &Ops) -> Value {
    match n {
        Value::Set(x, _) => Value::Set(*x, Box::new(range)),
        Value::Call(f, args) if *f == ops.append_text => {
            Value::Call(*f, vec![args[0].clone(), range])
        }
        _ => unreachable!("not a host"),
    }
}

/// The slice block of `text_from_bytes(<block>)`, when `v` is that call.
fn slice_call<'v>(v: &'v Value, ops: &Ops) -> Option<&'v crate::data::Block> {
    match v.unspan() {
        Value::Call(f, args) if *f == ops.from_bytes && args.len() == 1 => slice_block(&args[0]),
        _ => None,
    }
}

/// The block a slice value is lowered to, when `v` is one.
fn slice_block(v: &Value) -> Option<&crate::data::Block> {
    match v.unspan() {
        Value::Block(bl) => Some(bl),
        _ => None,
    }
}

/// The block's statements with every `Insert` opened, in order.
fn flat(ops: &[Value]) -> Vec<&Value> {
    let mut out = Vec::new();
    for op in ops {
        match op.unspan() {
            Value::Insert(inner) => out.extend(flat(inner)),
            _ => out.push(op),
        }
    }
    out
}

fn names(v: &Value, owned: &[u16]) -> bool {
    let mut hit = false;
    v.walk(&mut |n| {
        if matches!(n, Value::Var(x) if owned.contains(x)) {
            hit = true;
        }
    });
    hit
}

/// Is `block` exactly a slice value: the temporary's mint, bind and zero, the bounds, one
/// slice whose operands name neither the temporary nor its store, and the temporary as its
/// value?  The same block `FnWriter::slice_fill` accepts.
fn admit(block: &crate::data::Block, vars: &Function, ops: &Ops) -> Result<Site, &'static str> {
    let Some((last, rest)) = block.operators.split_last() else {
        return Err("an empty block");
    };
    let Value::Var(tmp) = last.unspan() else {
        return Err("its value is not the temporary");
    };
    if !vars.name(*tmp).contains("__iter_vec") {
        return Err("its value is no slice temporary");
    }
    let deps = vars.tp(*tmp).depend();
    let [store] = deps.as_slice() else {
        return Err("the temporary's store is not one variable");
    };
    let owned = [*tmp, *store];
    let mut slices = 0;
    for op in flat(rest) {
        match op.unspan() {
            Value::Call(f, args) if ops.slice.contains(f) => {
                if args.len() != 5
                    || !matches!(args[0].unspan(), Value::Var(x) if x == tmp)
                    || args[1..].iter().any(|a| names(a, &owned))
                {
                    return Err("the slice reads the temporary");
                }
                slices += 1;
            }
            Value::Call(f, args)
                if ops.keep.contains(f)
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if owned.contains(x))
                    && !args[1..].iter().any(|a| names(a, &owned)) => {}
            Value::Set(x, val) if *x == *tmp => {
                if !matches!(val.unspan(), Value::Call(f, a) if *f == ops.get_field
                    && matches!(a.first().map(Value::unspan), Some(Value::Var(w)) if w == store))
                {
                    return Err("the temporary is bound to something but its store");
                }
            }
            other if names(other, &owned) => return Err("a bound statement names the temporary"),
            _ => {}
        }
    }
    if slices != 1 {
        return Err("not one slice");
    }
    Ok(Site {
        tmp: *tmp,
        store: *store,
    })
}

/// The bound statements (every statement that names neither the temporary nor its store) and
/// the slice's source and bounds.
fn split(block: &crate::data::Block, site: &Site, ops: &Ops) -> (Vec<Value>, Value, Value, Value) {
    let owned = [site.tmp, site.store];
    let mut prelude = Vec::new();
    let mut operands = None;
    let (_, rest) = block.operators.split_last().expect("admitted");
    for op in flat(rest) {
        match op.unspan() {
            Value::Call(f, args) if ops.slice.contains(f) => {
                operands = Some((args[1].clone(), args[2].clone(), args[3].clone()));
            }
            other
                if names(other, &owned)
                    || matches!(other, Value::Set(x, _) if owned.contains(x)) => {}
            _ => prelude.push(op.clone()),
        }
    }
    let (src, lo, hi) = operands.expect("admitted: one slice");
    (prelude, src, lo, hi)
}

/// Outside its block, `W` is only declared null and freed, and `T` is never named: then the
/// store is never minted once the block is gone, and every free of it passes a null by.
fn outside_mentions(code: &Value, site: &Site, ops: &Ops) -> Result<(), &'static str> {
    let (mut store_all, mut tmp_all, mut store_freed, mut store_set) = (0, 0, 0, 0);
    let (mut store_in, mut tmp_in) = (0, 0);
    code.walk(&mut |n| match n {
        Value::Var(x) if *x == site.store => store_all += 1,
        Value::Var(x) if *x == site.tmp => tmp_all += 1,
        Value::Call(f, args)
            if ops.frees.contains(f) && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == site.store) =>
        {
            store_freed += 1;
        }
        Value::Set(x, val) if *x == site.store && !matches!(val.unspan(), Value::Null) => store_set += 1,
        Value::Block(bl) if matches!(bl.operators.last().map(Value::unspan), Some(Value::Var(x)) if *x == site.tmp) => {
            for op in &bl.operators {
                op.walk(&mut |m| match m {
                    Value::Var(x) if *x == site.store => store_in += 1,
                    Value::Var(x) if *x == site.tmp => tmp_in += 1,
                    _ => {}
                });
            }
        }
        _ => {}
    });
    if store_set > 0 {
        return Err("its store is assigned outside the slice");
    }
    if store_all != store_in + store_freed {
        return Err("its store is named outside the slice");
    }
    if tmp_all != tmp_in {
        return Err("the temporary is named outside the slice");
    }
    Ok(())
}

fn decline(data: &Data, d_nr: u32, site: Option<&str>, why: &str) {
    crate::rewrite_census::declined(RULE, site.unwrap_or("slice"), why);
    if trace() {
        eprintln!(
            "[slice-text] fn={} keeps its slice: {why}",
            data.def(d_nr).name()
        );
    }
}
