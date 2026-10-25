// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-TailSelf` — a SELF-call in tail position becomes a jump to the function's entry,
//! decided in the IR phase for both backends (loft-lang/plans#202 P5a and P5b).
//!
//! The body of a function whose result is a scalar or nothing is wrapped in a loop.  Every
//! self-call in TAIL position — the value a `return` answers, the value an `if` arm or a block
//! answers in that position, the last statement of a void body, the value of the `__ret_N`
//! temporary the scope pass hands to a `return` — binds its arguments to fresh temporaries,
//! releases what the frame owns (the scope pass's own release ops for the locals, the ones
//! that would have run after the call returned), binds the parameters from the temporaries,
//! and continues the loop; every other value in that position returns.  So a `count(n - 1,
//! acc + 1)` that is the last thing a function does takes no frame, and the recursion a
//! compiler's loops are written as runs to any depth.  A self-call anywhere else stays a call,
//! and a tail call inside a user loop continues the function's loop by the loops between.
//!
//! A scalar parameter is rebound by a copy, as a call binds it.  A parameter held in a store
//! (a record, a vector, a collection, a text, an iterator) and a `&` parameter are pointers the
//! frame never owned — a call SHARES them with the caller (calls.md `(F-ParamHeap)`) — so their
//! rebind is exact when the argument is such a pointer too: the parameter itself (then nothing
//! moves), another parameter, or a part of one (`p.next`, `v[i]`, a projection rooted at a
//! parameter).  An argument that MAKES a value — a call, a literal, an operator result — or
//! names a local the jump releases would have to be handed over into the frame's ownership,
//! which a call does not do either; that binding has no spelling here, and the heap RESULT
//! (delivered into the caller's hidden buffer) has none yet.  The frame's LOCALS may be held
//! in a store (a text, a vector, a record, the compiler's own `__work_N` buffers): each is
//! declared afresh by the body, so the jump releases them where the return would have.
//!
//! What this pass cannot make a jump is REFUSED by the parser (@C155, `definitions.rs`), with
//! the same predicate (`admit_frame`), so a self-call in tail position is never silently a
//! call that stops at `MAX_CALL_DEPTH`.
//!
//! Unlike the leaf inliner, this runs in an `open_world` program and in a coverage run too:
//! the depth a program reaches is a property of the language, not of the lane, so an
//! `--interpret` run, a `--native` semantics run (which ships the live tier and so is
//! `open_world`), a shipped binary and `loft test` must agree on it.  A live flip therefore
//! reaches a tail-recursive function at its next entry from outside, as it reaches a `while`
//! loop; coverage still sees the function entered by the call that starts the recursion.
//! `LOFT_NO_TAIL_SELF=1` keeps the calls; `LOFT_TRACE_TAIL_SELF=1` names each rewrite and
//! each decline.

use crate::data::{Data, DefType, Definition, Type, Value, is_scalar, v_block, v_loop, v_set};
use crate::lexer::Position;
use crate::variables::Function;

fn off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_TAIL_SELF").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_TAIL_SELF").is_ok_and(|v| v != "0"))
}

/// Rewrite every admitted function of the program; answers the number of self-calls made
/// jumps, which the rewrite census records as `R-TailSelf`.
pub fn rewrite_program(data: &mut Data) -> usize {
    if off() {
        return 0;
    }
    let mut jumps = 0usize;
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type != DefType::Function
            || matches!(def.code(), Value::Null)
            || def.is_operator()
            || !def.rust().is_empty()
            || !def.native().is_empty()
        {
            continue;
        }
        if tail_calls(def.code(), d, def.variables(), data).is_empty() {
            continue;
        }
        let name = def.name().to_string();
        if let Err(why) = admit_frame(
            def.variables(),
            def.returned(),
            def.code(),
            data,
            &declared_params(def),
            d,
        ) {
            if trace() {
                eprintln!("tail-self: {name} declined: {why}");
            }
            crate::rewrite_census::declined("R-TailSelf", &name, &why);
            continue;
        }
        let _census_body = crate::rewrite_census::InBody::enter("ir", &name);
        let voids = matches!(data.def(d).returned().base(), Type::Void | Type::Null);
        let params = declared_params(data.def(d));
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let scope = match &code {
            Value::Block(b) => b.scope,
            _ => 0,
        };
        let vars = &mut data.definitions[d as usize].variables;
        let temps: Vec<u16> = params
            .iter()
            .map(|&p| {
                // The temporary OWNS what it holds for the length of the jump — never a view
                // into another variable — so the parameter's borrow deps do not travel with
                // the type: a reference typed as a view gets no slot of its own.
                let tp = vars.tp(p).without_deps();
                vars.add_unique("tail", &tp, scope)
            })
            .collect();
        let n = {
            let mut cx = Cx {
                d,
                params: &params,
                temps: &temps,
                data,
                vars: data.def(d).variables(),
                put_ref: data.def_nr("OpPutRef"),
                jumps: 0,
            };
            rewrite_body(&mut code, voids, &mut cx);
            cx.jumps
        };
        data.definitions[d as usize].code = code;
        if n == 0 {
            // No jump: the finder's tail call has no counterpart in the lowered body.  The
            // loop is inert, and the census records the function so the shape gets a clause.
            if trace() {
                eprintln!("tail-self: {name}: no tail call left in the lowered body");
            }
            crate::rewrite_census::declined("R-TailSelf", &name, "lowered shape not matched");
        } else if trace() {
            eprintln!("tail-self: fn {name} — {n} self-call(s) made jumps");
        }
        jumps += n;
        data.definitions[d as usize].variables.reset_intervals();
        crate::scopes::compute_function_intervals(data, d);
        crate::scopes::assign_function_slots(data, d);
    }
    crate::rewrite_census::fired("R-TailSelf", jumps);
    jumps
}

/// The declared parameters, as variable numbers: the definition's attributes that are not
/// hidden, in order.  A hidden attribute is a buffer a lowering added after the signature (a
/// promoted text local, a return buffer, an apart vector), and the table's own `argument` flag
/// marks those too, so neither is the test for "a parameter the call binds".
pub(crate) fn declared_params(def: &Definition) -> Vec<u16> {
    def.attributes()
        .iter()
        .enumerate()
        .filter(|(_, a)| !a.hidden)
        .map(|(i, _)| i as u16)
        .collect()
}

/// Can this frame be reused by a jump?  The one predicate for the parser's refusal (@C155,
/// `definitions.rs`, on the source shape) and for this pass (on the lowered one): a result
/// that is a scalar or nothing; no `&` local; every argument a tail self-call hands to a
/// parameter held in a store, or to a `&` parameter, is a pointer the frame does not own (a
/// parameter or a projection of one); no generator, `parallel` block or closure in the body.
/// A local may be held in a store — the jump releases it.  `params` is `declared_params`'
/// list.  `Err` names what blocks it, with the variable and its type, for the refusal's text.
pub(crate) fn admit_frame(
    vars: &Function,
    result: &Type,
    code: &Value,
    data: &Data,
    params: &[u16],
    d: u32,
) -> Result<(), String> {
    if !(is_scalar(result) || matches!(result.base(), Type::Void | Type::Null)) {
        return Err(format!(
            "its result is a `{}`, which is delivered into the caller's buffer, not a scalar",
            result.source_name(data)
        ));
    }
    for v in 0..vars.count() {
        if matches!(vars.tp(v).base(), Type::RefVar(_)) && !params.contains(&v) {
            return Err(format!("local `{}` is a `&` reference", vars.name(v)));
        }
    }
    for call in tail_calls(code, d, vars, data) {
        for (i, &p) in params.iter().enumerate() {
            let tp = vars.tp(p);
            if is_scalar(tp) {
                continue;
            }
            let Some(arg) = call.args.get(i) else {
                continue;
            };
            let identity = matches!(arg.unspan(), Value::Var(q) if *q == p);
            if identity {
                continue;
            }
            // A text and a `&` parameter have no raw pointer move in the bytecode (a text's
            // assignment owns, a `&` assignment writes through), so they pass through only
            // as themselves.
            if !crate::data::is_dbref_slot(tp.base()) {
                return Err(format!(
                    "parameter `{}` (a `{}`) can only be passed through as itself here",
                    vars.name(p),
                    tp.source_name(data)
                ));
            }
            if !borrows_from_params(arg, params, data) {
                return Err(format!(
                    "the call's argument for parameter `{}` (a `{}`) makes a value this frame \
                     would have to own — a fresh value, or a local the jump releases — where a \
                     call only shares; pass the parameter itself or a part of it (`{0}`, \
                     `{0}.field`, `{0}[i]`)",
                    vars.name(p),
                    tp.source_name(data)
                ));
            }
        }
    }
    // A heap parameter REBOUND in the body (`p = S { … }`, `(F-ParamRebind)`) holds a store
    // this frame owns, which the epilogue releases: passed through at the jump it would be read
    // into its temporary and then freed under it.  A `&` parameter written through keeps its
    // pointer, so the write is no rebind.
    for &p in params {
        let tp = vars.tp(p);
        if is_scalar(tp) || matches!(tp.base(), Type::RefVar(_)) {
            continue;
        }
        if writes_variable(code, p, data) {
            return Err(format!(
                "parameter `{}` (a `{}`) is rebound in the body, so this frame owns its store",
                vars.name(p),
                tp.source_name(data)
            ));
        }
    }
    let blocked = code.any_node(&mut |n| {
        matches!(
            n,
            Value::Yield(_) | Value::Parallel(_) | Value::FnRef(_, _, _) | Value::FnRefDnr(_)
        )
    });
    if blocked {
        return Err("the body holds a generator, a `parallel` block or a closure".to_string());
    }
    Ok(())
}

/// Does the body write the VARIABLE `v` itself — not a field or element through it?  A `Set`,
/// or an op that takes the variable as its destination: the parser lowers a record rebind to
/// `OpInitRefSentinel(p); OpDatabase(p, …)` and a text one to a clear and appends, and the
/// scope pass marks a rebindable parameter with `OpFreeRefIfDistinct(p, witness)`.  A field
/// write (`OpSetInt(p, …)`) is a mutation through the pointer, which `(F-ParamGrow)` allows,
/// and is not one of these.
fn writes_variable(code: &Value, v: u16, data: &Data) -> bool {
    code.any_node(&mut |n| match n {
        Value::Set(x, _) => *x == v,
        Value::Call(op, items) if !items.is_empty() => {
            let name = data.def(*op).name();
            matches!(items[0].unspan(), Value::Var(x) if *x == v)
                && (name.starts_with("OpInit")
                    || name.starts_with("OpDatabase")
                    || name.starts_with("OpFree")
                    || name.starts_with("OpClear")
                    || name.starts_with("OpAppend")
                    || name.starts_with("OpCreate")
                    || name == "OpPutRef")
        }
        _ => false,
    })
}

/// Is `arg` a pointer the frame does not own: a parameter, or a projection (a field, an
/// element, a reference read) rooted at one?  Anything else — a call's result, a literal, an
/// operator's result, a local — is a value the frame makes or releases, which a call never
/// hands over into the callee's ownership either.
fn borrows_from_params(arg: &Value, params: &[u16], data: &Data) -> bool {
    match arg.unspan() {
        Value::Var(q) => params.contains(q),
        Value::Call(op, items) if !items.is_empty() => {
            let name = data.def(*op).name();
            (crate::use_analysis::is_projection_op(data, *op) || name == "OpGetDbRef")
                && borrows_from_params(&items[0], params, data)
        }
        _ => false,
    }
}

/// Is `v` a release the scope pass emits at a scope's end — a free, a clear, a closure drop?
fn is_release(data: &Data, v: &Value) -> bool {
    matches!(v.unspan(), Value::Call(op, _) if {
        let name = data.def(*op).name();
        name.starts_with("OpFree") || name.starts_with("OpClear") || name == "OpDropFnRef"
    })
}

fn is_line(v: &Value) -> bool {
    matches!(v.unspan(), Value::Line(_))
}

/// The index of a block's value: its last operator that is not a source-line marker.
fn last_value(ops: &[Value]) -> Option<usize> {
    ops.iter().rposition(|op| !is_line(op))
}

/// The scope pass's return shape: `ops[k]` is `__ret_N = E`, then releases (and line markers)
/// only, then at `ops[j]` `return __ret_N` — or, for a block in value position, `__ret_N` as
/// the block's last operator.  Answers `(k, j)`; `E` is then in value position with those
/// releases as the epilogue a jump must run.
fn ret_pattern(
    ops: &[Value],
    vars: &Function,
    data: &Data,
    in_value: bool,
) -> Option<(usize, usize)> {
    let last = last_value(ops);
    for (k, op) in ops.iter().enumerate() {
        let Value::Set(r, _) = op.unspan() else {
            continue;
        };
        if !vars.name(*r).starts_with("__ret_") {
            continue;
        }
        let mut j = k + 1;
        while j < ops.len() && (is_release(data, &ops[j]) || is_line(&ops[j])) {
            j += 1;
        }
        if j >= ops.len() {
            return None;
        }
        match ops[j].unspan() {
            Value::Return(e) if matches!(e.unspan(), Value::Var(x) if x == r) => {
                return Some((k, j));
            }
            Value::Var(x) if x == r && in_value && Some(j) == last => return Some((k, j)),
            _ => {}
        }
    }
    None
}

/// The last operator of a void body that is not a release: a self-call there is the tail,
/// and the releases after it are its epilogue.
fn void_tail(ops: &[Value], data: &Data) -> Option<usize> {
    ops.iter()
        .rposition(|op| !is_line(op) && !is_release(data, op))
}

/// A self-call in tail position: where it stands (the innermost `Span` on the way to it) and
/// the arguments it passes.
pub(crate) struct TailCall {
    pub(crate) at: Option<Position>,
    pub(crate) args: Vec<Value>,
}

/// Every self-call in tail position of `code` (a function's body).  The gate for the pass and
/// for the parser's refusal, so it reads both the source shape and the lowered one: a
/// `return`'s value, an `if` arm's or a block's value in that position, the last statement of
/// a void body, the `__ret_N` pattern, and the value assigned to a hidden `&` result buffer (a
/// heap result's delivery, which the parser lowers before this is asked).
pub(crate) fn tail_calls(code: &Value, d: u32, vars: &Function, data: &Data) -> Vec<TailCall> {
    let f = Finder { d, vars, data };
    let mut out = Vec::new();
    f.block(code.unspan(), true, None, &mut out);
    out
}

struct Finder<'a> {
    d: u32,
    vars: &'a Function,
    data: &'a Data,
}

impl Finder<'_> {
    /// `in_value`: the block's last value is a tail (a result or a void body's last statement).
    fn block(&self, v: &Value, in_value: bool, at: Option<Position>, out: &mut Vec<TailCall>) {
        let Value::Block(b) = v.unspan() else {
            return;
        };
        let ops = &b.operators;
        let last = last_value(ops);
        let pat = ret_pattern(ops, self.vars, self.data, in_value);
        let vtail = if in_value {
            void_tail(ops, self.data)
        } else {
            None
        };
        for (i, op) in ops.iter().enumerate() {
            if let Some((k, _)) = pat
                && i == k
                && let Value::Set(_, e) = op.unspan()
            {
                self.value(e, at, out);
            } else if pat.is_some_and(|(_, j)| j == i) {
                // The pattern's own `return __ret_N`, no tail of its own.
            } else if in_value && (Some(i) == last || Some(i) == vtail) {
                self.value(op, at, out);
            } else {
                self.statement(op, at, out);
            }
        }
    }

    fn value(&self, v: &Value, at: Option<Position>, out: &mut Vec<TailCall>) {
        match v {
            Value::Span(bx) => self.value(&bx.1, Some(bx.0), out),
            Value::Call(c, args) if *c == self.d => out.push(TailCall {
                at,
                args: args.clone(),
            }),
            Value::If(_, a, b) => {
                self.value(a, at, out);
                self.value(b, at, out);
            }
            Value::Block(_) => self.block(v, true, at, out),
            Value::Return(e) => self.value(e, at, out),
            other => self.statement(other, at, out),
        }
    }

    fn statement(&self, v: &Value, at: Option<Position>, out: &mut Vec<TailCall>) {
        match v {
            Value::Span(bx) => self.statement(&bx.1, Some(bx.0), out),
            Value::Return(e) => self.value(e, at, out),
            Value::Block(_) => self.block(v, false, at, out),
            // A heap result is delivered into the compiler's hidden `&` buffer: `___acc = f(…)`
            // is the tail call of a text-returning function, which the parser lowers this way.
            // A user local promoted to the buffer keeps its own name, and `t = f(…); t` is the
            // bound spelling that keeps a call, whatever the result's type.
            Value::Set(x, e)
                if *x < self.vars.count()
                    && self.vars.is_argument(*x)
                    && self.vars.name(*x).starts_with("__")
                    && matches!(self.vars.tp(*x).base(), Type::RefVar(_)) =>
            {
                self.value(e, at, out);
            }
            other => other.for_each_child(&mut |c| self.statement(c, at, out)),
        }
    }
}

/// What a value position means to the value that is not a jump.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pos {
    /// The value the function answers: it becomes `return value`.
    Return,
    /// The value of a `__ret_N = …` whose releases and `return` follow: it stays the value,
    /// so the releases run on that path.
    Keep,
    /// The last statement of a void body: it stays a statement.
    Void,
}

struct Cx<'a> {
    d: u32,
    params: &'a [u16],
    temps: &'a [u16],
    data: &'a Data,
    vars: &'a Function,
    /// `OpPutRef`: the raw slot store the scope pass binds a witness with — a pointer moved
    /// without a release of what the slot held, which is what a shared parameter needs.
    put_ref: u32,
    jumps: usize,
}

/// Wrap the body in the loop.
fn rewrite_body(code: &mut Value, voids: bool, cx: &mut Cx) {
    let Value::Block(b) = code else {
        return;
    };
    let ops = std::mem::take(&mut b.operators);
    let pos = if voids { Pos::Void } else { Pos::Return };
    let mut body = block(ops, Some(pos), &[], 0, cx);
    if voids {
        // A void body that reaches its end returns; only a jump stays in the loop.
        body.push(Value::Return(Box::new(Value::Null)));
    }
    b.operators = vec![v_loop(body, "tail_self")];
}

/// A block's operators.  `pos` is `Some` when the block's last value is in value position;
/// `epi` the releases an enclosing `__ret_N` pattern runs after this block yields, which a
/// jump from inside it must run first.
fn block(
    ops: Vec<Value>,
    pos: Option<Pos>,
    epi: &[Value],
    depth: usize,
    cx: &mut Cx,
) -> Vec<Value> {
    let last = last_value(&ops);
    let pat = ret_pattern(&ops, cx.vars, cx.data, pos.is_some());
    let frees: Vec<Value> = match pat {
        Some((k, j)) => {
            let mut f: Vec<Value> = ops[k + 1..j]
                .iter()
                .filter(|op| is_release(cx.data, op))
                .cloned()
                .collect();
            f.extend_from_slice(epi);
            f
        }
        None => Vec::new(),
    };
    let vtail = if pos == Some(Pos::Void) {
        void_tail(&ops, cx.data)
    } else {
        None
    };
    let trailing: Vec<Value> = match vtail {
        Some(m) => {
            let mut f: Vec<Value> = ops[m + 1..]
                .iter()
                .filter(|op| is_release(cx.data, op))
                .cloned()
                .collect();
            f.extend_from_slice(epi);
            f
        }
        None => Vec::new(),
    };
    let mut out = Vec::with_capacity(ops.len());
    for (i, op) in ops.into_iter().enumerate() {
        let op = if let Some((k, _)) = pat
            && i == k
        {
            let (span, set) = unspan_owned(op);
            let Value::Set(r, e) = set else {
                unreachable!("ret_pattern found a `Set`")
            };
            respan(
                span,
                Value::Set(r, Box::new(value(*e, Pos::Keep, &frees, depth, cx))),
            )
        } else if pat.is_some_and(|(_, j)| j == i) {
            op
        } else if Some(i) == vtail {
            value(op, Pos::Void, &trailing, depth, cx)
        } else if let Some(p) = pos
            && Some(i) == last
        {
            value(op, p, epi, depth, cx)
        } else {
            let mut op = op;
            statement(&mut op, epi, depth, cx);
            op
        };
        out.push(op);
    }
    out
}

fn unspan_owned(v: Value) -> (Option<Position>, Value) {
    match v {
        Value::Span(bx) => {
            let (p, inner) = *bx;
            (Some(p), inner)
        }
        other => (None, other),
    }
}

fn respan(span: Option<Position>, v: Value) -> Value {
    match span {
        Some(p) => Value::Span(Box::new((p, v))),
        None => v,
    }
}

/// A value in a value position: a self-call becomes the jump, an `if` or block hands the
/// position to its arms or its value, a `return` is a value position of its own (its releases
/// already ran before it, so it carries none), anything else answers as `pos` says.  The
/// fallback is right because the value position of a non-void function holds the VALUE: a
/// statement node there (`Set`, `Insert`, a void call) would make the function's value void,
/// which the parser refuses before this pass runs — so every other node is the expression the
/// function answers.  A block visited in `Return` position ends in a terminator on every
/// path, so its result is `never`, as the parser types a block that ends in `return`.
fn value(v: Value, pos: Pos, epi: &[Value], depth: usize, cx: &mut Cx) -> Value {
    match v {
        Value::Span(bx) => {
            let (p, inner) = *bx;
            Value::Span(Box::new((p, value(inner, pos, epi, depth, cx))))
        }
        Value::Call(c, args) if c == cx.d => {
            cx.jumps += 1;
            jump(args, epi, depth, cx)
        }
        Value::If(cond, then, otherwise) => Value::If(
            cond,
            Box::new(value(*then, pos, epi, depth, cx)),
            Box::new(value(*otherwise, pos, epi, depth, cx)),
        ),
        Value::Block(mut b) => {
            let ops = std::mem::take(&mut b.operators);
            b.operators = block(ops, Some(pos), epi, depth, cx);
            if pos == Pos::Return {
                b.result = Type::Never;
            }
            Value::Block(b)
        }
        Value::Return(e) => value(*e, Pos::Return, &[], depth, cx),
        mut other => {
            statement(&mut other, epi, depth, cx);
            match pos {
                Pos::Return => Value::Return(Box::new(other)),
                Pos::Keep | Pos::Void => other,
            }
        }
    }
}

/// A statement outside a value position: its explicit `return`s are value positions of their
/// own, a block below it may carry the `__ret_N` pattern, and a loop below it is one level the
/// jump has to count.
fn statement(v: &mut Value, epi: &[Value], depth: usize, cx: &mut Cx) {
    match v {
        Value::Span(bx) => statement(&mut bx.1, epi, depth, cx),
        Value::Return(e) => {
            let inner = std::mem::replace(&mut **e, Value::Null);
            *v = value(inner, Pos::Return, &[], depth, cx);
        }
        Value::Block(b) => {
            let ops = std::mem::take(&mut b.operators);
            b.operators = block(ops, None, epi, depth, cx);
        }
        Value::Loop(b) => {
            for op in &mut b.operators {
                statement(op, epi, depth + 1, cx);
            }
        }
        Value::Iter(..) => {
            v.for_each_child_mut(&mut |c| statement(c, epi, depth + 1, cx));
        }
        _ => v.for_each_child_mut(&mut |c| statement(c, epi, depth, cx)),
    }
}

/// The jump: every argument into its temporary, the frame's releases (after the reads, as
/// `(O-Detach)` asks), every parameter from its temporary — so an argument that reads a
/// parameter reads the value before any rebind — then the loop continues, by the depth of the
/// user loops between the call and the function's own loop.  A parameter passed through as
/// itself is not touched: it already holds what the call would bind.
fn jump(args: Vec<Value>, epi: &[Value], depth: usize, cx: &Cx) -> Value {
    let mut ops = Vec::with_capacity(2 * cx.params.len() + epi.len() + 1);
    let mut bound: Vec<(u16, u16, bool)> = Vec::with_capacity(cx.params.len());
    for ((&p, &t), a) in cx.params.iter().zip(cx.temps).zip(args) {
        if matches!(a.unspan(), Value::Var(q) if *q == p) {
            continue;
        }
        // The temporary's `Set` is its first write — a declaration the bytecode stores
        // without a release, and the slot assignment reads as the variable's birth.
        ops.push(v_set(t, a));
        // A `Set` on a store-held PARAMETER is an owning assignment in the bytecode (it
        // releases what the slot held), and that slot holds the caller's store: the pointer is
        // moved with the raw slot store instead, on both backends.
        bound.push((p, t, crate::data::is_dbref_slot(cx.vars.tp(p).base())));
    }
    ops.extend_from_slice(epi);
    for (p, t, pointer) in bound {
        if pointer {
            ops.push(Value::Call(cx.put_ref, vec![Value::Var(p), Value::Var(t)]));
        } else {
            ops.push(v_set(p, Value::Var(t)));
        }
    }
    ops.push(Value::Continue(depth as u16));
    v_block(ops, Type::Never, "tail_jump")
}
