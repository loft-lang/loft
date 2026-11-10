// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-R-HostStore`, the RESULT clause (@PLN219) — a recursion that answers the results of its
//! own calls builds them all in one store, answers them as they are, links them into its nodes
//! by record number and releases the ones it drops record by record.
//!
//! `simplify(e)` binds `a = simplify(l)` and `b = simplify(r)`, then answers `b`, `a`, a new
//! node over both or a fresh literal.  Each call built its result in a store of its own, every
//! node copied both subtrees into its own, and answering `a` copied it into the return buffer
//! the parser renamed `b` onto.  Where every answer of a function is a literal it builds or the
//! result of a call to itself, and those results are only read, written into a literal's field,
//! answered or freed, the whole function is rewritten:
//!
//! - a call to itself is handed the store of the first result bound before it on its path
//!   (`OpStoreBuf(a)`), or with none the caller's buffer where that names a store and no record
//!   (`OpStoreBufOnly`), so a record a caller offers is never written;
//! - a literal is claimed in that same store;
//! - a field written from a result adopts it by number (`OpAdoptChildRec`) and the local lets
//!   go of it (null): ONE owner per record — a field, the caller, or one local;
//! - a result a path drops is released by record (`OpReleaseRecIfDistinct`): the store lives on
//!   for the answer;
//! - a result answered is answered as it is: where the parser renamed the answered local onto
//!   the buffer, the local is split off again and the copy into the buffer
//!   (`materialized_view_return`) becomes the answer of the local itself.
//!
//! Decided on the settled IR of the function itself, after the frees are in.  The frame is laid
//! out again for the split local.  `LOFT_NO_HOST_RESULT=1`; `LOFT_TRACE_HOST_STORE=1` says why
//! a function is declined.

use crate::data::{Data, DefType, Type, Value};

struct Ops {
    database: u32,
    free_distinct: u32,
    set_child: u32,
    adopt: u32,
    store_buf: u32,
    store_buf_only: u32,
    release: u32,
    get_field: u32,
    copy_record: u32,
    distinct_store: u32,
    ref_alias: u32,
    get_enum: u32,
}

/// Rewrite every admitted function; the count of functions rewritten.
pub fn rewrite_program(data: &mut Data) -> usize {
    if std::env::var("LOFT_NO_HOST_RESULT").is_ok_and(|v| v != "0") {
        return 0;
    }
    // The clause builds each result in the store of the first result on its path, which is
    // the store the callee answered only where that first bind ADOPTS it (`(R-AdoptFirstBind)`).
    // With the adoption off the bind copies into a store of its own, and the second call, the
    // node and its adoptions were laid out for a store the result no longer names: `(7*7)` for
    // `(2*7)`, an offset overflow on C.  Off with the rule it stands on.
    if !crate::keys::adopt_first_bind_enabled() {
        return 0;
    }
    let ops = Ops {
        database: data.def_nr("OpDatabase"),
        free_distinct: data.def_nr("OpFreeRefIfDistinct"),
        set_child: data.def_nr("OpSetChildRec"),
        adopt: data.def_nr("OpAdoptChildRec"),
        store_buf: data.def_nr("OpStoreBuf"),
        store_buf_only: data.def_nr("OpStoreBufOnly"),
        release: data.def_nr("OpReleaseRecIfDistinct"),
        get_field: data.def_nr("OpGetField"),
        copy_record: data.def_nr("OpCopyRecord"),
        distinct_store: data.def_nr("OpDistinctStore"),
        ref_alias: data.def_nr("OpRefAlias"),
        get_enum: data.def_nr("OpGetEnum"),
    };
    if [ops.adopt, ops.store_buf, ops.store_buf_only, ops.release].contains(&u32::MAX) {
        return 0;
    }
    let trace = std::env::var("LOFT_TRACE_HOST_STORE").is_ok();
    let mut total = 0;
    for d_nr in 0..data.definitions() {
        if data.def_type(d_nr) != DefType::Function || !data.def(d_nr).is_loft_defined() {
            continue;
        }
        match admit(data, d_nr, &ops) {
            Ok(shape) => {
                if trace {
                    eprintln!(
                        "[host-store] result clause: {} admitted",
                        data.def(d_nr).name()
                    );
                }
                rewrite(data, d_nr, &ops, shape);
                data.definitions[d_nr as usize].variables.reset_intervals();
                crate::scopes::compute_function_intervals(data, d_nr);
                crate::scopes::assign_function_slots(data, d_nr);
                let _census = crate::rewrite_census::InBody::enter("ir", data.def(d_nr).name());
                crate::rewrite_census::fired("R-HostStore", 1);
                total += 1;
            }
            Err(why) => {
                if trace && why != NO_SELF_CALL {
                    eprintln!(
                        "[host-store] result clause: {} declined — {why}",
                        data.def(d_nr).name()
                    );
                }
            }
        }
    }
    total
}

const NO_SELF_CALL: &str = "no call to itself";

/// What the admission learned about a function.
struct Shape {
    /// The hidden return buffer's variable.
    buf: u16,
    /// Its argument position.
    buf_at: usize,
    /// The locals bound from calls to the function itself (the buffer among them where the
    /// parser renamed the answered local onto it).
    results: Vec<u16>,
    /// The work refs of the literals that are answers.
    minted: Vec<u16>,
    /// `__rbw_<buf>`, the entry alias of a renamed buffer.
    entry: Option<u16>,
    /// The record type of the result.
    tp: u16,
}

fn var_of(v: &Value) -> Option<u16> {
    if let Value::Var(x) = v.unspan() {
        Some(*x)
    } else {
        None
    }
}

/// The values a node answers: a block's last operator, both arms of an `if`.
fn tails<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v.unspan() {
        Value::Block(bl) if bl.name != "Object" && bl.name != "materialized_view_return" => {
            if let Some(last) = bl.operators.last() {
                tails(last, out);
            }
        }
        Value::Insert(items) => {
            if let Some(last) = items.last() {
                tails(last, out);
            }
        }
        Value::If(_, t, f) => {
            tails(t, out);
            tails(f, out);
        }
        other => out.push(other),
    }
}

/// A literal block `{ OpDatabase(w, tp); …; w }`: its work ref.
fn literal_ref(v: &Value, ops: &Ops) -> Option<u16> {
    let Value::Block(bl) = v.unspan() else {
        return None;
    };
    if bl.name != "Object" {
        return None;
    }
    let first = bl
        .operators
        .iter()
        .find(|o| !matches!(o.unspan(), Value::Line(_)))?;
    let Value::Call(db, args) = first.unspan() else {
        return None;
    };
    let w = var_of(args.first()?)?;
    (*db == ops.database && bl.operators.last().and_then(var_of) == Some(w)).then_some(w)
}

/// `{ OpDatabase(buf); OpCopyRecord(l, buf, tp); frees…; return buf }`: the local `l` the
/// parser copies into the renamed buffer.
fn view_return_local(v: &Value, buf: u16, ops: &Ops) -> Option<u16> {
    let Value::Block(bl) = v.unspan() else {
        return None;
    };
    if bl.name != "materialized_view_return" {
        return None;
    }
    let body: Vec<&Value> = bl
        .operators
        .iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    let [db, copy, frees @ .., ret] = body.as_slice() else {
        return None;
    };
    let Value::Call(mint, da) = db.unspan() else {
        return None;
    };
    let Value::Call(copy_op, ca) = copy.unspan() else {
        return None;
    };
    let Value::Return(answered) = ret.unspan() else {
        return None;
    };
    if *mint != ops.database
        || da.first().and_then(var_of) != Some(buf)
        || *copy_op != ops.copy_record
        || ca.get(1).and_then(var_of) != Some(buf)
        || var_of(answered) != Some(buf)
    {
        return None;
    }
    let all_frees = frees.iter().all(|f| {
        matches!(f.unspan(), Value::Call(op, a) if *op == ops.free_distinct
            && a.iter().all(|x| var_of(x).is_some()))
    });
    all_frees.then(|| ca.first().and_then(var_of)).flatten()
}

/// Does `v` read nothing but plain values — a function answering no heap?
fn plain_return(tp: &Type) -> bool {
    // A nullable plain value is a plain value; a nullable heap value is not.
    if let Type::Optional(inner) = tp {
        return plain_return(inner);
    }
    matches!(
        tp,
        Type::Boolean | Type::Integer(..) | Type::Float | Type::Single | Type::Character
    )
}

#[allow(clippy::too_many_lines)]
fn admit(data: &Data, d_nr: u32, ops: &Ops) -> Result<Shape, &'static str> {
    let def = data.def(d_nr);
    let Some(buf_at) = def.hidden_return_buffer_attr() else {
        return Err(NO_SELF_CALL);
    };
    // `@FR-N-Shape` — a `-> τ?` delivers through its own synthetic enum: not admitted.
    let td = match def.returned() {
        Type::Enum(td, true, _) | Type::Reference(td, _) => *td,
        Type::Optional(_) => return Err(NO_SELF_CALL),
        _ => return Err(NO_SELF_CALL),
    };
    let code = def.code();
    // Every call to itself is a bind `l = f(…, w)` handed a work ref of its own.
    let mut calls = 0;
    code.walk(&mut |n| {
        if matches!(n, Value::Call(f, _) if *f == d_nr) {
            calls += 1;
        }
    });
    if calls == 0 {
        return Err(NO_SELF_CALL);
    }
    let vars = def.variables();
    let buf = vars.var(&def.attributes()[buf_at].name);
    if buf == u16::MAX || !vars.is_argument(buf) {
        return Err("the buffer is not a parameter");
    }
    let mut results = Vec::new();
    let mut binds = 0;
    let mut bad_bind = false;
    code.walk(&mut |n| {
        if let Value::Set(l, rhs) = n
            && let Value::Call(f, args) = rhs.unspan()
            && *f == d_nr
        {
            binds += 1;
            let w = args.get(buf_at).and_then(var_of);
            if !w.is_some_and(|w| w != buf && vars.name(w).starts_with("__ref_")) {
                bad_bind = true;
            }
            if *l != buf && vars.is_argument(*l) {
                bad_bind = true;
            }
            if !results.contains(l) {
                results.push(*l);
            }
        }
    });
    if bad_bind || binds != calls {
        return Err("a call to itself that is not bound to a local with a work ref of its own");
    }
    let tracked: Vec<u16> = results
        .iter()
        .copied()
        .chain(std::iter::once(buf))
        .collect();
    // No result is an argument of a call to itself: what a call reads it may answer.
    let mut arg_reads = false;
    code.walk(&mut |n| {
        if let Value::Call(f, args) = n
            && *f == d_nr
        {
            arg_reads |= args.iter().any(|a| tracked.iter().any(|t| a.reads_var(*t)));
        }
    });
    if arg_reads {
        return Err("a result is handed to a call to itself");
    }
    // Each result is bound at most once on any path and never in a loop: at its bind it holds
    // null (its init, or the null an adoption leaves), so the bind adopts the callee's record.
    if !once_per_path(code, d_nr, &mut Vec::new()) {
        return Err("a result bound twice on one path, or in a loop");
    }
    let entry = vars.var(&format!("__rbw_{}", vars.name(buf)));
    let entry = (entry != u16::MAX).then_some(entry);
    // Every answer is a result, a literal, the copy of a result into the buffer, or null.
    let mut answer_vals: Vec<&Value> = Vec::new();
    code.walk(&mut |n| {
        if let Value::Return(e) = n {
            tails(e, &mut answer_vals);
        }
    });
    let mut expanded: Vec<&Value> = Vec::new();
    for a in &answer_vals {
        if let Some(r) = var_of(a)
            && !tracked.contains(&r)
        {
            // `__ret_N = <answer>; …; return __ret_N`
            if !vars.name(r).starts_with("__ret_") {
                return Err("an answer is a local that is not a result");
            }
            code.walk(&mut |n| {
                if let Value::Set(x, rhs) = n
                    && *x == r
                    && !matches!(rhs.unspan(), Value::Null)
                {
                    tails(rhs, &mut expanded);
                }
            });
        } else {
            expanded.push(a);
        }
    }
    let mut minted = Vec::new();
    for a in &expanded {
        match a.unspan() {
            Value::Var(r) if tracked.contains(r) => {}
            Value::Null | Value::Return(_) => {}
            Value::Call(f, _) if data.def(*f).name() == "OpNullRefSentinel" => {}
            v => {
                if let Some(w) = literal_ref(v, ops) {
                    if !vars.name(w).starts_with("__ref_") {
                        return Err("a literal built into a named local");
                    }
                    if !minted.contains(&w) {
                        minted.push(w);
                    }
                } else if let Some(l) = view_return_local(v, buf, ops) {
                    if !results.contains(&l) || l == buf {
                        return Err("the buffer takes a copy of something not a result");
                    }
                } else {
                    return Err("an answer that is not a result or a literal");
                }
            }
        }
    }
    // Every record minted is an answered literal.
    let mut stray_mint = false;
    code.walk(&mut |n| {
        if let Value::Call(f, args) = n
            && *f == ops.database
            && !args
                .first()
                .and_then(var_of)
                .is_some_and(|w| minted.contains(&w) || w == buf)
        {
            stray_mint = true;
        }
    });
    if stray_mint {
        return Err("a record minted that no answer is");
    }
    let cx = Check {
        data,
        d_nr,
        ops,
        tracked: &tracked,
        minted: &minted,
        buf,
        entry,
    };
    if !cx.valid(code) {
        return Err("a result is used other than read, adopted, answered or freed");
    }
    Ok(Shape {
        buf,
        buf_at,
        results,
        minted,
        entry,
        tp: data.def(td).known_type(),
    })
}

/// Is every call to itself bound to a local that no earlier call on the same path bound, and
/// none inside a loop?
fn once_per_path(v: &Value, d_nr: u32, bound: &mut Vec<u16>) -> bool {
    match v.unspan() {
        Value::Loop(bl) => !bl
            .operators
            .iter()
            .any(|o| o.any_node(&mut |n| matches!(n, Value::Call(f, _) if *f == d_nr))),
        Value::Set(l, rhs) if matches!(rhs.unspan(), Value::Call(f, _) if *f == d_nr) => {
            if bound.contains(l) {
                return false;
            }
            bound.push(*l);
            true
        }
        Value::If(c, t, e) => {
            if !once_per_path(c, d_nr, bound) {
                return false;
            }
            let mut other = bound.clone();
            let ok = once_per_path(t, d_nr, bound) && once_per_path(e, d_nr, &mut other);
            for l in other {
                if !bound.contains(&l) {
                    bound.push(l);
                }
            }
            ok
        }
        other => {
            let mut ok = true;
            other.for_each_child(&mut |c| ok = ok && once_per_path(c, d_nr, bound));
            ok
        }
    }
}

struct Check<'a> {
    data: &'a Data,
    d_nr: u32,
    ops: &'a Ops,
    tracked: &'a [u16],
    minted: &'a [u16],
    buf: u16,
    entry: Option<u16>,
}

impl Check<'_> {
    fn is_tracked(&self, v: &Value) -> bool {
        var_of(v).is_some_and(|x| self.tracked.contains(&x))
    }

    /// Is every mention of a result (or the buffer) in `v` one the rewrite knows: a bind from
    /// a call to itself or null, a read by a function answering a plain value, a write into an
    /// answered literal's field, an answer, a free, the buffer's entry alias and its guard?
    fn valid(&self, v: &Value) -> bool {
        let ops = self.ops;
        match v.unspan() {
            Value::Var(x) => !self.tracked.contains(x),
            Value::Set(x, rhs) if self.tracked.contains(x) => match rhs.unspan() {
                Value::Null => true,
                Value::Call(f, args) if *f == self.d_nr => args.iter().all(|a| self.valid(a)),
                _ => false,
            },
            Value::Set(x, rhs) if Some(*x) == self.entry => {
                matches!(rhs.unspan(), Value::Call(f, a) if *f == ops.ref_alias
                    && a.len() == 1 && var_of(&a[0]) == Some(self.buf))
            }
            Value::Return(e) if self.is_tracked(e) => true,
            Value::Call(f, args) if *f == ops.free_distinct => {
                args.iter().all(|a| var_of(a).is_some() || self.valid(a))
            }
            Value::Call(f, args) if *f == ops.set_child && self.is_tracked(&args[1]) => {
                let Value::Call(gf, ga) = args[0].unspan() else {
                    return false;
                };
                *gf == ops.get_field
                    && ga
                        .first()
                        .and_then(var_of)
                        .is_some_and(|w| self.minted.contains(&w))
            }
            Value::Call(f, args) if *f == ops.get_enum && self.is_tracked(&args[0]) => {
                args[1..].iter().all(|a| self.valid(a))
            }
            Value::Call(f, args)
                if *f != self.d_nr
                    && self.data.def_type(*f) == DefType::Function
                    && self.data.def(*f).is_loft_defined()
                    && plain_return(self.data.def(*f).returned()) =>
            {
                args.iter().all(|a| self.is_tracked(a) || self.valid(a))
            }
            Value::If(c, t, e) if self.distinct_store_guard(c, t) => {
                matches!(e.unspan(), Value::Null)
            }
            Value::Block(bl) if bl.name == "materialized_view_return" => {
                view_return_local(v, self.buf, ops).is_some()
            }
            other => {
                let mut ok = true;
                other.for_each_child(&mut |c| ok = ok && self.valid(c));
                ok
            }
        }
    }

    /// `if OpDistinctStore(buf, entry) OpFreeRefIfDistinct(buf, x)`: the release of a renamed
    /// buffer's local where it was rebound.
    fn distinct_store_guard(&self, c: &Value, t: &Value) -> bool {
        let ops = self.ops;
        matches!(c.unspan(), Value::Call(f, a) if *f == ops.distinct_store
            && a.first().and_then(var_of) == Some(self.buf)
            && a.get(1).and_then(var_of) == self.entry && self.entry.is_some())
            && matches!(t.unspan(), Value::Call(f, a) if *f == ops.free_distinct
                && a.first().and_then(var_of) == Some(self.buf))
    }
}

fn rewrite(data: &mut Data, d_nr: u32, ops: &Ops, mut shape: Shape) {
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    let tp = i32::from(shape.tp);
    // The parser renamed the answered local onto the buffer: split it off again.
    if let Some(at) = shape.results.iter().position(|r| *r == shape.buf) {
        let vars = &mut data.definitions[d_nr as usize].variables;
        let scope = match code.unspan() {
            Value::Block(bl) => bl.scope,
            _ => u16::MAX,
        };
        let local_tp = vars.tp(shape.buf).clone();
        let local = vars.add_unique("result", &local_tp, scope);
        undo_view_returns(&mut code, &shape, ops, local, tp);
        split_buffer(&mut code, &shape, ops, local, tp);
        if let Value::Block(bl) = code.unspan_mut() {
            bl.operators
                .insert(0, Value::Set(local, Box::new(Value::Null)));
        }
        shape.results[at] = local;
    }
    for r in &shape.results {
        data.definitions[d_nr as usize]
            .variables
            .mark_deferred_first_bind(*r);
    }
    place(&mut code, &shape, ops, &[]);
    adopt_and_release(&mut code, &shape, ops, tp);
    data.definitions[d_nr as usize].code = code;
}

/// `{ OpDatabase(buf); OpCopyRecord(l, buf); frees; return buf }` → `{ frees; release the
/// split local; return l }`: the result is answered as it is.
fn undo_view_returns(v: &mut Value, shape: &Shape, ops: &Ops, local: u16, tp: i32) {
    if let Some(l) = view_return_local(v, shape.buf, ops)
        && let Value::Block(bl) = v.unspan_mut()
    {
        let mut out = Vec::new();
        for op in bl.operators.drain(..) {
            match op.unspan() {
                Value::Line(_) => out.push(op),
                Value::Call(op_nr, args) if *op_nr == ops.free_distinct => {
                    let (freed, witness) = (var_of(&args[0]), var_of(&args[1]));
                    if freed == Some(l) && witness == Some(shape.buf) {
                        continue;
                    }
                    let witness = if witness == Some(shape.buf) {
                        Some(l)
                    } else {
                        witness
                    };
                    if let (Some(freed), Some(witness)) = (freed, witness) {
                        out.push(Value::Call(
                            ops.free_distinct,
                            vec![Value::Var(freed), Value::Var(witness)],
                        ));
                    }
                }
                _ => {}
            }
        }
        out.push(Value::Call(
            ops.release,
            vec![Value::Var(local), Value::Var(l), Value::Int(tp)],
        ));
        out.push(Value::Return(Box::new(Value::Var(l))));
        bl.operators = out;
        bl.name = "result_return";
        return;
    }
    v.for_each_child_mut(&mut |c| undo_view_returns(c, shape, ops, local, tp));
}

/// Every use of the renamed buffer as the answered local names the split local instead; the
/// release of the rebound buffer releases the local.
fn split_buffer(v: &mut Value, shape: &Shape, ops: &Ops, local: u16, tp: i32) {
    let buf = shape.buf;
    match v.unspan_mut() {
        Value::Var(x) if *x == buf => *x = local,
        Value::Set(target, _) if Some(*target) == shape.entry => {}
        Value::Set(target, rhs) => {
            if *target == buf {
                *target = local;
            }
            split_buffer(rhs, shape, ops, local, tp);
        }
        Value::If(cond, then, _)
            if matches!(cond.unspan(), Value::Call(op_nr, args) if *op_nr == ops.distinct_store
                && args.first().and_then(var_of) == Some(buf)) =>
        {
            let witness = match then.unspan() {
                Value::Call(_, args) => args.get(1).cloned().unwrap_or(Value::Null),
                _ => Value::Null,
            };
            *v.unspan_mut() = Value::Call(
                ops.release,
                vec![Value::Var(local), witness, Value::Int(tp)],
            );
        }
        other => other.for_each_child_mut(&mut |child| split_buffer(child, shape, ops, local, tp)),
    }
}

/// The store a call or a literal is placed in: the first result bound before it on its path,
/// else the caller's buffer where it names a store.
fn target(shape: &Shape, ops: &Ops, dom: &[u16]) -> Value {
    match dom.first() {
        Some(r) => Value::Call(ops.store_buf, vec![Value::Var(*r)]),
        None => Value::Call(ops.store_buf_only, vec![Value::Var(shape.buf)]),
    }
}

/// Hand every call to itself, and claim every answered literal, in the store of the first
/// result bound before it.
fn place(v: &mut Value, shape: &Shape, ops: &Ops, dom: &[u16]) {
    match v.unspan_mut() {
        Value::Block(bl) => {
            let mut dom = dom.to_vec();
            let literal = bl.name == "Object";
            let mut i = 0;
            while i < bl.operators.len() {
                if literal
                    && let Value::Call(f, a) = bl.operators[i].unspan()
                    && *f == ops.database
                    && let Some(w) = a.first().and_then(var_of)
                    && shape.minted.contains(&w)
                {
                    let at = target(shape, ops, &dom);
                    bl.operators.insert(i, Value::Set(w, Box::new(at)));
                    i += 2;
                    continue;
                }
                place(&mut bl.operators[i], shape, ops, &dom);
                if let Value::Set(l, rhs) = bl.operators[i].unspan()
                    && shape.results.contains(l)
                    && matches!(rhs.unspan(), Value::Call(..))
                    && !dom.contains(l)
                {
                    dom.push(*l);
                }
                i += 1;
            }
        }
        Value::Set(l, rhs) if shape.results.contains(l) => {
            if let Value::Call(_, args) = rhs.unspan_mut()
                && let Some(slot) = args.get_mut(shape.buf_at)
                && var_of(slot).is_some_and(|w| w != shape.buf)
            {
                *slot = target(shape, ops, dom);
            }
        }
        other => other.for_each_child_mut(&mut |c| place(c, shape, ops, dom)),
    }
}

/// A literal's field written from a result adopts it by number and the local lets go of it;
/// every free of a result releases it by record.
fn adopt_and_release(v: &mut Value, shape: &Shape, ops: &Ops, tp: i32) {
    if let Value::Block(bl) = v.unspan_mut() {
        let mut i = 0;
        while i < bl.operators.len() {
            let mut adopted = None;
            if let Value::Call(f, a) = bl.operators[i].unspan_mut()
                && *f == ops.set_child
                && let Some(l) = a.get(1).and_then(var_of)
                && shape.results.contains(&l)
            {
                *f = ops.adopt;
                if let Some(Value::Int(t)) = a.get_mut(2) {
                    *t &= i32::from(crate::keys::COPY_TP_MASK);
                }
                adopted = Some(l);
            }
            if let Some(l) = adopted {
                bl.operators
                    .insert(i + 1, Value::Set(l, Box::new(Value::Null)));
                i += 2;
                continue;
            }
            i += 1;
        }
    }
    if let Value::Call(f, a) = v.unspan_mut()
        && *f == ops.free_distinct
        && a.first()
            .and_then(var_of)
            .is_some_and(|x| shape.results.contains(&x))
    {
        *f = ops.release;
        a.push(Value::Int(tp));
        return;
    }
    v.for_each_child_mut(&mut |c| adopt_and_release(c, shape, ops, tp));
}
