// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-CtorLiteral`, decided in the IR phase for both backends.
//!
//! A call of a CONSTRUCTOR — a function whose body is one record literal over its parameters
//! (`fn form_new(h, lens: vector<integer>, turns: vector<integer>) -> Form { Form { … } }`) —
//! that declares a local, `ef = form_new(eh, [la, lb, lc], [ea, eb, ec])`, is replaced by the
//! literal the callee would have built, written into `ef` itself: `ef = Form { fm_h0: …,
//! fm_len: [la, lb, lc], fm_turn: [ea, eb, ec] }`.  A vector-literal argument is then built IN
//! ITS FIELD, where the call built it in a buffer of its own and the callee copied it whole —
//! the copy Rust's move does not make.  The literal form is the one the record rewrites
//! already serve (`@FR-R-LoopRecord`'s refill clause keeps the record and its vectors across a
//! loop's passes).
//!
//! The callee is admitted when its body is the literal group and nothing else: the return
//! buffer's mint, a write of every field — a scalar computed from scalar parameters by
//! operators, a vector field's zero, a vector parameter appended whole into its field — and
//! the buffer as the result.  The call site is admitted when every argument is an operator
//! expression over variables and literals (so the field writes may interleave with them and
//! nothing they read can change), or a vector literal of such elements, or a vector variable;
//! when the call DECLARES its target (the target's only assignment, never read by an
//! argument); and when the call's pooled return buffer is used by nothing else — its prep,
//! its frees.  The prep goes, and the target's free that skipped the buffer frees the target.
//! `LOFT_NO_CTOR_LITERAL=1` keeps every call; `LOFT_TRACE_CTOR_LITERAL=1` names each
//! rewritten call and each declined constructor.
use crate::data::{Data, DefType, Deps, Type, Value};
use std::collections::HashMap;

fn off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_CTOR_LITERAL").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_CTOR_LITERAL").is_ok_and(|v| v != "0"))
}

/// What a constructor's body writes, over its own variable numbers.
struct Ctor {
    name: String,
    /// The store type the buffer is minted as.
    tp: i32,
    /// The hidden return buffer's variable.
    buf: u16,
    /// The parameters in call order (the buffer last).
    params: Vec<u16>,
    /// The literal group after the mint, in order: zeros, scalar sets, vector fills.
    group: Vec<Value>,
}

fn name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

fn is_var(v: &Value, x: u16) -> bool {
    matches!(v.unspan(), Value::Var(w) if *w == x)
}

fn literal(v: &Value) -> bool {
    matches!(
        v.unspan(),
        Value::Int(_)
            | Value::Long(_)
            | Value::Float(_)
            | Value::Single(_)
            | Value::Boolean(_)
            | Value::Null
    )
}

/// An operator over variables and literals that neither writes nor reads a store: the
/// operator families of integer, float and boolean arithmetic, comparison and conversion.
/// Evaluating one earlier or later, or twice, answers the same.
fn scalar_expr(data: &Data, v: &Value) -> bool {
    match v.unspan() {
        Value::Var(_) => true,
        Value::Call(op, args) => {
            const FAMILIES: [&str; 22] = [
                "OpAdd", "OpMin", "OpMul", "OpDiv", "OpRem", "OpConv", "OpEq", "OpNe", "OpLt",
                "OpLe", "OpGt", "OpGe", "OpAnd", "OpOr", "OpNot", "OpNeg", "OpEor", "OpLand",
                "OpLor", "OpSLeft", "OpSRight", "OpAbs",
            ];
            let n = name(data, *op);
            FAMILIES.iter().any(|f| n.starts_with(f))
                && !n.contains("Text")
                && args.iter().all(|a| scalar_expr(data, a))
        }
        Value::Block(b) if b.name == "Inline" => {
            b.operators
                .iter()
                .filter(|o| !matches!(o, Value::Line(_)))
                .count()
                == 1
                && b.operators
                    .iter()
                    .filter(|o| !matches!(o, Value::Line(_)))
                    .all(|o| scalar_expr(data, o))
        }
        Value::If(c, t, e) => scalar_expr(data, c) && scalar_expr(data, t) && scalar_expr(data, e),
        other => literal(other),
    }
}

fn mentions(v: &Value, x: u16) -> bool {
    v.any_node(&mut |n| match n {
        Value::Var(w) | Value::Set(w, _) => *w == x,
        _ => false,
    })
}

/// The type the constructor's prologue mints its buffer as: `if <present> {} else
/// OpDatabase(buf, tp)`.
fn mint_type(data: &Data, prologue: &Value, buf: u16) -> Result<i32, &'static str> {
    match prologue.unspan() {
        Value::If(_, _, f) => match f.unspan() {
            Value::Call(op, a)
                if name(data, *op) == "OpDatabase" && a.len() == 2 && is_var(&a[0], buf) =>
            {
                match a[1].unspan() {
                    Value::Int(t) => Ok(*t),
                    _ => Err("a mint of a computed type"),
                }
            }
            _ => Err("a mint other than the buffer's prologue"),
        },
        _ => Err("a mint other than the buffer's prologue"),
    }
}

/// The literal group's writes: a scalar field from operators over the scalar parameters, a
/// vector field's zero, a vector parameter appended whole into its field — each vector
/// parameter exactly once.
fn check_writes(
    data: &Data,
    writes: &[&Value],
    buf: u16,
    params: &[u16],
    fv: &crate::variables::Function,
) -> Result<(), &'static str> {
    let scalar_params: Vec<u16> = params
        .iter()
        .copied()
        .filter(|p| !matches!(fv.tp(*p), Type::Vector(_, _)))
        .collect();
    let mut vec_uses: HashMap<u16, usize> = HashMap::new();
    for w in writes {
        let Value::Call(op, a) = w.unspan() else {
            return Err("a statement in the literal that is not a write");
        };
        let n = name(data, *op);
        if n == "OpAppendVector" {
            // `OpAppendVector(OpGetField(buf, off, vtp), p, 0)` — a vector parameter, whole.
            let field_ok = matches!(a.first().map(Value::unspan), Some(Value::Call(g, ga))
                if name(data, *g) == "OpGetField" && ga.len() == 3 && is_var(&ga[0], buf)
                    && matches!(ga[1].unspan(), Value::Int(_)) && matches!(ga[2].unspan(), Value::Int(_)));
            let Some(Value::Var(p)) = a.get(1).map(Value::unspan) else {
                return Err("a vector field filled from something other than a parameter");
            };
            if !field_ok
                || !params.contains(p)
                || !matches!(a.get(2).map(Value::unspan), Some(Value::Int(0)))
            {
                return Err("a vector field filled from something other than a parameter");
            }
            *vec_uses.entry(*p).or_insert(0) += 1;
            continue;
        }
        if !n.starts_with("OpSet") || n.contains("Text") || a.len() != 3 || !is_var(&a[0], buf) {
            return Err("a write of a kind the literal does not hold");
        }
        if !matches!(a[1].unspan(), Value::Int(_)) {
            return Err("a write at a computed offset");
        }
        let val = &a[2];
        if !scalar_expr(data, val) {
            return Err("a field computed by more than operators");
        }
        let mut bad = false;
        val.any_node(&mut |x| {
            if let Value::Var(v) = x
                && !scalar_params.contains(v)
            {
                bad = true;
            }
            false
        });
        if bad {
            return Err("a field that reads something other than a scalar parameter");
        }
    }
    // Every vector parameter is used exactly once, by its fill; the unused locals stay unused.
    for p in params {
        if matches!(fv.tp(*p), Type::Vector(_, _)) && vec_uses.get(p).copied() != Some(1) {
            return Err("a vector parameter read other than once, whole");
        }
    }
    Ok(())
}

/// The constructor a call of `d` is replaced by, or why not.
fn admit(data: &Data, d: u32) -> Result<Ctor, &'static str> {
    let def = data.def(d);
    if def.def_type != DefType::Function {
        return Err("not a function");
    }
    let Value::Block(body) = def.code() else {
        return Err("no loft body");
    };
    let Some(bi) = crate::generation::hoist::ret_buffer_attr(def) else {
        return Err("no record result");
    };
    if bi + 1 != def.attributes.len() {
        return Err("the return buffer is not the last parameter");
    }
    let fv = &def.variables;
    let mut params = Vec::new();
    for a in &def.attributes {
        let v = fv.var(&a.name);
        if v == u16::MAX || !fv.is_argument(v) || fv.is_captured(v) {
            return Err("a parameter without its variable");
        }
        params.push(v);
    }
    let buf = params[bi];
    // The body: unused locals declared null, then `return <object>` or the object itself.
    let ops: Vec<&Value> = body
        .operators
        .iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    let Some((last, init)) = ops.split_last() else {
        return Err("an empty body");
    };
    let mut unused = Vec::new();
    for s in init {
        let Value::Set(v, rhs) = s.unspan() else {
            return Err("a statement before the literal");
        };
        if !matches!(rhs.unspan(), Value::Null) {
            return Err("a statement before the literal");
        }
        unused.push(*v);
    }
    let object = match last.unspan() {
        Value::Return(e) => e.unspan(),
        e => e,
    };
    let Value::Block(obj) = object else {
        return Err("a result other than a record literal");
    };
    let group: Vec<&Value> = obj
        .operators
        .iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    let Some((tail, init)) = group.split_last() else {
        return Err("an empty literal");
    };
    if !is_var(tail, buf) {
        return Err("a literal that does not answer its buffer");
    }
    let Some((prologue, writes)) = init.split_first() else {
        return Err("no mint");
    };
    let tp = mint_type(data, prologue, buf)?;
    check_writes(data, writes, buf, &params[..bi], fv)?;
    if unused
        .iter()
        .any(|u| writes.iter().any(|w| mentions(w, *u)))
    {
        return Err("a local the literal reads");
    }
    Ok(Ctor {
        name: def.name().to_string(),
        tp,
        buf,
        params,
        group: writes.iter().map(|w| (*w).clone()).collect(),
    })
}

/// The pushes of a vector-literal argument, retargeted at `field`: `OpPreAllocVector` and
/// each `OpPush…` of the literal's own vector, in order.  `None` for any other shape — a
/// block whose statements are not the literal's mint, view, zero, reservation and pushes, an
/// element that is not an operator expression, an element type that differs from the field's,
/// or a block that reads `target`.
fn literal_pushes(
    data: &Data,
    block: &Value,
    field: &Value,
    vtp: i32,
    target: u16,
) -> Option<(u16, Vec<Value>)> {
    let Value::Block(b) = block.unspan() else {
        return None;
    };
    if b.name != "Vector" || mentions(block, target) {
        return None;
    }
    let ops: Vec<&Value> = b
        .operators
        .iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    let (last, init) = ops.split_last()?;
    let Value::Var(view) = last.unspan() else {
        return None;
    };
    let mut vdb: Option<u16> = None;
    let mut out = Vec::new();
    for s in init {
        match s.unspan() {
            Value::Set(v, rhs) if *v == *view => {
                let Value::Call(g, ga) = rhs.unspan() else {
                    return None;
                };
                if name(data, *g) != "OpGetField"
                    || ga.len() != 3
                    || !matches!(ga[1].unspan(), Value::Int(0))
                    || !matches!(ga[2].unspan(), Value::Int(t) if *t == vtp)
                {
                    return None;
                }
                let Value::Var(d) = ga[0].unspan() else {
                    return None;
                };
                if vdb.is_some_and(|x| x != *d) {
                    return None;
                }
                vdb = Some(*d);
            }
            Value::Call(op, a) => {
                let n = name(data, *op);
                let on_vdb = a
                    .first()
                    .is_some_and(|x| matches!(x.unspan(), Value::Var(_)))
                    && vdb.is_none_or(|d| is_var(&a[0], d));
                match n {
                    "OpDatabase" if a.len() == 2 && on_vdb => {
                        if let Value::Var(d) = a[0].unspan() {
                            vdb = Some(*d);
                        }
                    }
                    "OpSetInt4"
                        if a.len() == 3
                            && on_vdb
                            && matches!(a[1].unspan(), Value::Int(0))
                            && matches!(a[2].unspan(), Value::Int(0)) => {}
                    // The reservation only where it reserves more than the first append's
                    // eleven elements: a kept vector has its record, and a shorter literal's
                    // first push claims the same.
                    "OpPreAllocVector" if a.len() == 3 && is_var(&a[0], *view) => {
                        if !matches!(a[1].unspan(), Value::Int(k) if *k <= 11) {
                            out.push(Value::Call(
                                *op,
                                vec![field.clone(), a[1].clone(), a[2].clone()],
                            ));
                        }
                    }
                    _ if n.starts_with("OpPush")
                        && a.len() == 2
                        && is_var(&a[0], *view)
                        && scalar_expr(data, &a[1])
                        && !mentions(&a[1], *view) =>
                    {
                        out.push(Value::Call(*op, vec![field.clone(), a[1].clone()]));
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
    Some((vdb?, out))
}

/// A copy of a constructor's write over the call's names.
fn remap(v: &Value, map: &HashMap<u16, Value>, scope: u16) -> Value {
    match v {
        Value::Var(n) => map.get(n).cloned().unwrap_or(Value::Var(*n)),
        Value::Span(s) => Value::Span(Box::new((s.0, remap(&s.1, map, scope)))),
        Value::Call(op, args) => {
            Value::Call(*op, args.iter().map(|a| remap(a, map, scope)).collect())
        }
        Value::If(c, t, e) => Value::If(
            Box::new(remap(c, map, scope)),
            Box::new(remap(t, map, scope)),
            Box::new(remap(e, map, scope)),
        ),
        Value::Block(b) => Value::Block(Box::new(crate::data::Block {
            name: b.name,
            operators: b
                .operators
                .iter()
                .filter(|o| !matches!(o, Value::Line(_)))
                .map(|o| remap(o, map, scope))
                .collect(),
            result: b.result.clone(),
            scope,
            var_size: 0,
        })),
        other => other.clone(),
    }
}

/// The mentions of the pooled buffer `buf` in a caller: every one must be its declaration,
/// its prep (`if null mint else clear`), a free, or the one call being rewritten.
fn buffer_only_pooled(data: &Data, body: &Value, buf: u16, target: u16) -> bool {
    fn walk(data: &Data, node: &Value, buf: u16, target: u16, calls: &mut usize, ok: &mut bool) {
        if !*ok {
            return;
        }
        let here = node.unspan();
        match here {
            Value::Var(var) if *var == buf => {
                *ok = false;
                return;
            }
            Value::Set(var, rhs) if *var == buf => {
                if !matches!(rhs.unspan(), Value::Null) {
                    *ok = false;
                }
                return;
            }
            Value::If(cond, then, other) if prep_of(data, cond, then, other, buf) => return,
            Value::Call(op, args) => {
                let nm = name(data, *op);
                let last_is_buf = args.last().is_some_and(|x| is_var(x, buf));
                if (nm == "OpFreeRef" && args.len() == 1 && is_var(&args[0], buf))
                    || (nm == "OpFreeRefIfDistinct"
                        && args.len() == 2
                        && ((is_var(&args[0], target) && is_var(&args[1], buf))
                            || (is_var(&args[0], buf) && is_var(&args[1], target))))
                {
                    return;
                }
                if last_is_buf && !nm.starts_with("Op") {
                    *calls += 1;
                    for x in &args[..args.len() - 1] {
                        walk(data, x, buf, target, calls, ok);
                    }
                    return;
                }
            }
            _ => {}
        }
        here.for_each_child(&mut |child| walk(data, child, buf, target, calls, ok));
    }
    let mut calls = 0;
    let mut ok = true;
    walk(data, body, buf, target, &mut calls, &mut ok);
    ok && calls == 1
}

/// `if OpRefIsNull(buf) { OpDatabase(buf, tp) } else OpClear(buf, tp)` — the scope pass's
/// prep of a pooled call buffer.
fn prep_of(data: &Data, cond: &Value, then: &Value, other: &Value, buf: u16) -> bool {
    let call_on = |arm: &Value, want: &str| {
        let stmt = match arm.unspan() {
            Value::Insert(items) if items.len() == 1 => items[0].unspan(),
            Value::Block(b) if b.operators.len() == 1 => b.operators[0].unspan(),
            plain => plain,
        };
        matches!(stmt, Value::Call(op, args) if name(data, *op) == want && args.first().is_some_and(|x| is_var(x, buf)))
    };
    call_on(cond, "OpRefIsNull") && call_on(then, "OpDatabase") && call_on(other, "OpClear")
}

/// The vector buffers a rewritten call's literal arguments were built in: each must be used
/// by nothing but its declaration, its block and its free, which the rewrite leaves dead.
fn vdb_only_literal(data: &Data, body: &Value, vdb: u16) -> bool {
    let mut uses = 0usize;
    body.any_node(&mut |n| {
        match n {
            Value::Call(op, a) if a.first().is_some_and(|x| is_var(x, vdb)) => {
                let nm = name(data, *op);
                if !matches!(nm, "OpFreeRef" | "OpDatabase" | "OpSetInt4" | "OpGetField") {
                    uses += 100;
                }
                if nm == "OpDatabase" {
                    uses += 1;
                }
            }
            Value::Set(w, rhs) if *w == vdb && !matches!(rhs.unspan(), Value::Null) => uses += 100,
            _ => {}
        }
        false
    });
    uses == 1
}

/// One rewrite candidate in a block: the statement `x = ctor(args)` at `at`.
struct Site {
    target: u16,
    ctor: u32,
    buf: u16,
}

fn site_of(stmt: &Value, ctors: &HashMap<u32, Ctor>) -> Option<Site> {
    let Value::Set(x, rhs) = stmt.unspan() else {
        return None;
    };
    let Value::Call(d, args) = rhs.unspan() else {
        return None;
    };
    let c = ctors.get(d)?;
    if args.len() != c.params.len() {
        return None;
    }
    let Value::Var(buf) = args.last()?.unspan() else {
        return None;
    };
    Some(Site {
        target: *x,
        ctor: *d,
        buf: *buf,
    })
}

/// The statements that replace `stmt` (a [`Site`]), or why the call stays.
fn expand(
    data: &Data,
    stmt: &Value,
    c: &Ctor,
    site: &Site,
    scope: u16,
    body: &Value,
) -> Result<Vec<Value>, &'static str> {
    let Value::Set(_, rhs) = stmt.unspan() else {
        return Err("not an assignment");
    };
    let Value::Call(_, args) = rhs.unspan() else {
        return Err("not a call");
    };
    let x = site.target;
    let target = Value::Var(x);
    let mut map: HashMap<u16, Value> = HashMap::new();
    map.insert(c.buf, target.clone());
    let mut literal_args: HashMap<u16, Value> = HashMap::new();
    for (p, a) in c.params.iter().zip(args.iter()).take(c.params.len() - 1) {
        if mentions(a, x) {
            return Err("an argument reads the target");
        }
        match a.unspan() {
            Value::Block(b) if b.name == "Vector" => {
                literal_args.insert(*p, a.clone());
            }
            _ if scalar_expr(data, a) || literal(a) => {
                map.insert(*p, a.unspan().clone());
            }
            _ => return Err("an argument other than operators, a vector literal or a variable"),
        }
    }
    // Profitable only where a vector literal would otherwise be built in a buffer of its own
    // and copied: a call of scalars alone already travels as a value record (`R-ValueRecord`),
    // no store at all, and the literal would mint one.
    if literal_args.is_empty() {
        return Err("no vector-literal argument (a call of scalars travels as a value record)");
    }
    let db = data.def_nr("OpDatabase");
    let mut out = vec![
        Value::Set(x, Box::new(Value::Null)),
        Value::Call(db, vec![target.clone(), Value::Int(c.tp)]),
    ];
    for w in &c.group {
        if let Value::Call(op, a) = w.unspan()
            && name(data, *op) == "OpAppendVector"
            && let Some(Value::Var(p)) = a.get(1).map(Value::unspan)
            && let Some(lit) = literal_args.get(p)
        {
            let field = remap(&a[0], &map, scope);
            let Value::Call(_, ga) = field.unspan() else {
                return Err("a vector field the literal cannot name");
            };
            let Value::Int(vtp) = ga[2].unspan() else {
                return Err("a vector field the literal cannot name");
            };
            let (vdb, pushes) = literal_pushes(data, lit, &field, *vtp, x)
                .ok_or("a vector literal of a shape other than pushes of operator expressions")?;
            if !vdb_only_literal(data, body, vdb) {
                return Err("a vector literal's buffer is used elsewhere");
            }
            out.extend(pushes);
            continue;
        }
        out.push(remap(w, &map, scope));
    }
    Ok(out)
}

/// Drop the pooled buffer's prep and turn the target's skip-the-buffer free into its own.
fn retire_buffer(data: &Data, v: &mut Value, target: u16, buf: u16, free_ref: u32) {
    v.for_each_child_mut(&mut |c| retire_buffer(data, c, target, buf, free_ref));
    // The buffer's free that skips the target (`OpFreeRefIfDistinct(buf, target)`, where the
    // target was the buffer) frees a store nothing mints any more.
    let dead = |o: &Value| match o.unspan() {
        Value::If(c, t, f) => prep_of(data, c, t, f, buf),
        Value::Call(op, a) => {
            name(data, *op) == "OpFreeRefIfDistinct"
                && a.len() == 2
                && is_var(&a[0], buf)
                && is_var(&a[1], target)
        }
        _ => false,
    };
    if let Value::Block(b) = v {
        b.operators.retain(|o| !dead(o));
    }
    if let Value::Insert(items) = v {
        items.retain(|o| !dead(o));
    }
    if let Value::Call(op, a) = v
        && name(data, *op) == "OpFreeRefIfDistinct"
        && a.len() == 2
        && is_var(&a[0], target)
        && is_var(&a[1], buf)
    {
        *v = Value::Call(free_ref, vec![Value::Var(target)]);
    }
}

/// The call sites in `caller` whose target and buffer pass the whole-body checks, as
/// `(target, buffer)` pairs: the target is assigned once, a plain record local and not a
/// parameter, and the buffer is used by nothing but its pool's prep and frees.
fn admitted_sites(
    data: &Data,
    caller: u32,
    body: &Value,
    ctors: &HashMap<u32, Ctor>,
) -> Vec<(u16, u16)> {
    let vars = &data.def(caller).variables;
    let mut sets: HashMap<u16, usize> = HashMap::new();
    body.any_node(&mut |n| {
        if let Value::Set(v, _) = n {
            *sets.entry(*v).or_insert(0) += 1;
        }
        false
    });
    let mut ok_sites: Vec<(u16, u16)> = Vec::new();
    body.any_node(&mut |n| {
        if let Some(site) = site_of(n, ctors)
            && site.ctor != caller
        {
            let why = if sets.get(&site.target) != Some(&1) {
                Some("the target is assigned more than once")
            } else if vars.is_argument(site.target) {
                Some("the target is a parameter (the function's own return buffer)")
            } else if vars.is_captured(site.target)
                || !matches!(vars.tp(site.target), Type::Reference(_, _))
            {
                Some("the target is not a plain record local")
            } else if !buffer_only_pooled(data, body, site.buf, site.target) {
                Some("the return buffer is used elsewhere")
            } else {
                None
            };
            match why {
                None => ok_sites.push((site.target, site.buf)),
                Some(w) if trace() => eprintln!(
                    "ctor-literal: {} kept a call in {}: {w}",
                    ctors[&site.ctor].name,
                    data.def(caller).name()
                ),
                Some(_) => {}
            }
        }
        false
    });
    ok_sites
}

/// `@FR-R-CtorLiteral` over the whole program; answers the number of calls rewritten.
pub fn rewrite_program(data: &mut Data) -> usize {
    if off() || data.open_world || data.observes_entries {
        return 0;
    }
    let mut ctors: HashMap<u32, Ctor> = HashMap::new();
    for d in 0..data.definitions() {
        if data.def(d).def_type != DefType::Function || !data.def(d).name().starts_with("n_") {
            continue;
        }
        match admit(data, d) {
            Ok(c) => {
                ctors.insert(d, c);
            }
            Err(why) => {
                if trace() && data.def(d).name().ends_with("_new") {
                    eprintln!("ctor-literal: {} declined: {why}", data.def(d).name());
                }
            }
        }
    }
    if ctors.is_empty() {
        return 0;
    }
    let free_ref = data.def_nr("OpFreeRef");
    let mut total = 0usize;
    for caller in 0..data.definitions() {
        let def = data.def(caller);
        if def.def_type != DefType::Function || matches!(def.code(), Value::Null) {
            continue;
        }
        let blocked = def
            .code()
            .any_node(&mut |n| matches!(n, Value::Yield(_) | Value::Parallel(_)));
        let calls = def.code().any_node(
            &mut |n| matches!(n, Value::Call(d, _) if *d != caller && ctors.contains_key(d)),
        );
        if blocked || !calls {
            continue;
        }
        let body = def.code().clone();
        let ok_sites = admitted_sites(data, caller, &body, &ctors);
        if ok_sites.is_empty() {
            continue;
        }
        let _census = crate::rewrite_census::InBody::enter("ir", data.def(caller).name());
        let mut code = std::mem::replace(&mut data.definitions[caller as usize].code, Value::Null);
        let mut done = Vec::new();
        // Only the sites the whole-body checks admitted: a site whose target or buffer did
        // not pass is filtered by its pair.
        let caller_name = data.def(caller).name().to_string();
        rewrite_in(
            data,
            &caller_name,
            &mut code,
            &ctors,
            &body,
            &ok_sites,
            &mut done,
        );
        for (target, buf) in &done {
            retire_buffer(data, &mut code, *target, *buf, free_ref);
        }
        data.definitions[caller as usize].code = code;
        if !done.is_empty() {
            let vars = &mut data.definitions[caller as usize].variables;
            for (target, _) in &done {
                if let Type::Reference(d, _) = vars.tp(*target).clone() {
                    vars.set_type(*target, Type::Reference(d, Deps::none()));
                }
            }
            vars.reset_intervals();
            crate::scopes::compute_function_intervals(data, caller);
            crate::scopes::assign_function_slots(data, caller);
            total += done.len();
        }
    }
    crate::rewrite_census::fired("R-CtorLiteral", total);
    total
}

/// Rewrite every site under `v` whose target and buffer passed the whole-body checks,
/// recording each rewritten site's target and buffer.
fn rewrite_in(
    data: &Data,
    caller: &str,
    v: &mut Value,
    ctors: &HashMap<u32, Ctor>,
    whole: &Value,
    ok: &[(u16, u16)],
    done: &mut Vec<(u16, u16)>,
) {
    v.for_each_child_mut(&mut |c| rewrite_in(data, caller, c, ctors, whole, ok, done));
    let Value::Block(b) = v else {
        return;
    };
    let scope = b.scope;
    let mut i = 0;
    while i < b.operators.len() {
        let Some(site) =
            site_of(&b.operators[i], ctors).filter(|s| ok.contains(&(s.target, s.buf)))
        else {
            i += 1;
            continue;
        };
        let ctor = &ctors[&site.ctor];
        match expand(data, &b.operators[i], ctor, &site, scope, whole) {
            Ok(new) => {
                if trace() {
                    eprintln!("ctor-literal: {} written in place in {caller}", ctor.name);
                }
                let n = new.len();
                b.operators.splice(i..=i, new);
                done.push((site.target, site.buf));
                i += n;
            }
            Err(why) => {
                if trace() {
                    eprintln!("ctor-literal: {} kept a call in {caller}: {why}", ctor.name);
                }
                i += 1;
            }
        }
    }
}
