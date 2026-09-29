// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! @PLN164 B2 — a call result BUILT WHERE IT WILL LIVE (`@FR-R-Place`) and stored there by
//! RELOCATION at its last use (`@FR-R-MoveLast`).
//!
//! The shape is the record-returning style's commonest one: `v = mk(…)` bound once from a
//! loft-defined callee that writes the buffer it is handed on EVERY exit and answers nothing
//! else (B2 unit 1, `Parser::literal_exits_into_buffer`), read a few times, then given to a
//! record-literal field of an element appended to a PARAMETER's collection —
//! `X.items += [Lit { f: v, … }]` — after which `v` is dead on that path.  Today that costs a
//! store minted by the callee and a deep copy at the field.  With the buffer claimed as a
//! RECORD in `X`'s store, the field takes the bytes by relocation and nothing is minted.
//!
//! Three IR sites change and only those: the buffer's null init `__ref_N = null` becomes
//! `__ref_N = OpPlaceRecord(X, tp)`; the field's `OpCopyRecord(v, dst, tp)` becomes
//! `OpMoveRecord(v, dst, tp)`, which relocates the bytes and releases the source's block on
//! the spot; and each exit's pair — the local's free, `OpFreeRef(v)` or its record-function
//! spelling `OpFreeRefIfDistinct(v, __retbuf)`, beside the buffer's `OpFreeRefIfDistinct(__ref_N,
//! v)` — becomes ONE `OpFreeRecordIn(v, tp)` on a path that never stored the record and
//! NOTHING on a path that did.  The state at every exit is a compile-time fact the walk below establishes, so
//! it is written into the IR as which free stands there — never substituted by a runtime
//! stand-in (a zeroed source a later walk would read, or a `v = null` the interpreter lowers
//! as a store-level free of what the local held).  One IR for both backends; it runs in
//! `scopes::check` once the scan phases have settled the function's frees and before the
//! slot intervals are read off the final IR.
//!
//! The admission is `(R-Place)`'s and `(R-MoveLast)`'s decline lists read off the IR, and
//! every doubt DECLINES: a wrong admission is silent-wrong, a leak or a double free, a wrong
//! decline is the copy the program already pays.  What is asked, in order: the local is a
//! bare dense record bound exactly once, from a direct call to a loft-defined callee that
//! answers a bare record through a hidden `__retbuf`; every exit of the callee is a fresh
//! literal built into that buffer (so nothing else is ever handed back); the buffer argument
//! is a caller work-ref whose only other appearance is the exit pair; no argument of the call
//! names the local, the buffer, or the host the result will land in; and after the bind, on
//! every path, the local is READ only as the receiver of a native operation (`OpGetInt(v,
//! …)`, `OpPushFloat(OpGetField(v, …), x)`) while it still holds the record, stored ONCE into
//! a `_elm_` element that `OpNewRecord` appended to a parameter's collection, and never named
//! again on that path except by the exit frees.  A read after the store, a hand-off to a
//! loft-defined call, a rebind, a `?`/`??` (not a direct call), a destination inside a loop,
//! a second destination, a second host, a destination in a local record or through a field
//! assignment, and a path that stored the record rejoining one that still holds it (the
//! exit's free would have no one answer) — each keeps the copy.
//!
//! `LOFT_NO_PLACE_RESULT=1` keeps every copy (the switch); `LOFT_TRACE_PLACE=1` names each
//! admission and each decline.

use std::collections::{HashMap, HashSet};

use crate::data::{Block, Data, Type, Value};
use crate::variables::Function;

/// The operators the pass reads and writes, by definition number.
struct Ops {
    copy: u32,
    free_ref: u32,
    free_if_distinct: u32,
    new_record: u32,
    get_field: u32,
    place: u32,
    move_rec: u32,
    free_in: u32,
    move_field: u32,
    ref_is_null: u32,
    database: u32,
    clear: u32,
}

impl Ops {
    fn lookup(data: &Data) -> Option<Self> {
        let nr = |n: &str| {
            let d = data.def_nr(n);
            (d != u32::MAX).then_some(d)
        };
        Some(Self {
            copy: nr("OpCopyRecord")?,
            free_ref: nr("OpFreeRef")?,
            free_if_distinct: nr("OpFreeRefIfDistinct")?,
            new_record: nr("OpNewRecord")?,
            get_field: nr("OpGetField")?,
            place: nr("OpPlaceRecord")?,
            move_rec: nr("OpMoveRecord")?,
            free_in: nr("OpFreeRecordIn")?,
            move_field: nr("OpMoveField")?,
            ref_is_null: nr("OpRefIsNull")?,
            database: nr("OpDatabase")?,
            clear: nr("OpClear")?,
        })
    }
}

/// A statement's place in the body: the child index taken at each step down from the root,
/// in the one enumeration [`Cx::walk`] and [`apply`] share — so a site the analysis decided
/// on is the site the rewrite finds.
type Path = Vec<usize>;

/// One admitted bind: the local, the work-ref the call was handed, the parameter whose
/// store hosts the result, the record's runtime type id, and the sites — the field stores
/// that become moves, the exit frees on a path that still holds the record (one record free
/// each), and the frees that go: the local's on a path that stored it, and the buffer's
/// witness-guarded one everywhere.
struct Plan {
    v: u16,
    buf: u16,
    host: u16,
    tp: u16,
    moves: HashSet<Path>,
    frees_held: HashSet<Path>,
    frees_dropped: HashSet<Path>,
}

/// When the admission is asked: to REWRITE the scope pass's IR, or to PREVIEW the verdict
/// on the parser's IR before that pass has run — the copy notice's question, asked on the
/// program path where the lint family reads the program first.  The parser's IR lacks the
/// buffer's null init the scope pass prepends (`lift_vars`) and the exit frees; nothing
/// else the admission reads differs, so a preview tolerates a missing init and a rewrite
/// requires exactly one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Rewrite,
    Preview,
}

/// Rewrite every admitted bind of function `d_nr`; a no-op under the switch and for a
/// function with no admissible bind.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if !crate::keys::place_result_enabled() {
        return;
    }
    let Some(ops) = Ops::lookup(data) else {
        return;
    };
    // Admission reads the function with its code IN PLACE — a recursive callee is this
    // function, and a body taken out for the rewrite would read as no body at all — and
    // the code is taken out only to apply.  The loop clause goes first; the top-level
    // admission then reads the result, so its paths are the ones it applies to.
    let loop_plans = admitted_loops(data, d_nr, data.def(d_nr).code(), &ops);
    if !loop_plans.is_empty() {
        let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
        for plan in &loop_plans {
            apply_loop(&mut code, plan, &ops);
        }
        data.definitions[d_nr as usize].code = code;
    }
    let plans = admitted(data, d_nr, data.def(d_nr).code(), &ops, Mode::Rewrite);
    if !plans.is_empty() {
        let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
        for plan in &plans {
            let mut path = Path::new();
            apply(&mut code, &mut path, plan, &ops);
        }
        data.definitions[d_nr as usize].code = code;
    }
}

/// Would the bind of local `v` in function `d_nr` be placed?  Asked by the copy notice
/// (`use_analysis::warn_copies`) on the parser's IR, so a field the compiler will relocate
/// is not reported as a copy that "could not be moved".  One home for the admission: this
/// is the same walk the rewrite runs, in preview.
#[must_use]
pub fn admits(data: &Data, d_nr: u32, v: u16) -> bool {
    if !crate::keys::place_result_enabled() {
        return false;
    }
    let Some(ops) = Ops::lookup(data) else {
        return false;
    };
    admitted(data, d_nr, data.def(d_nr).code(), &ops, Mode::Preview)
        .iter()
        .any(|plan| plan.v == v)
}

fn admitted(data: &Data, d_nr: u32, code: &Value, ops: &Ops, mode: Mode) -> Vec<Plan> {
    let function = data.def(d_nr).variables();
    let Value::Block(top) = code.unspan() else {
        return Vec::new();
    };
    let mut plans = Vec::new();
    for (idx, stmt) in top.operators.iter().enumerate() {
        let Value::Set(v, value) = stmt.unspan() else {
            continue;
        };
        let value = value.unspan();
        let Value::Call(fn_nr, args) = value else {
            continue;
        };
        if !placeable_bind(data, function, *v, *fn_nr) {
            continue;
        }
        match admit(data, d_nr, top, idx, *v, *fn_nr, args, ops, mode) {
            Ok(plan) => {
                if mode != Mode::Preview {
                    crate::rewrite_census::fired("R-Place", 1);
                }
                if crate::keys::trace_place() {
                    eprintln!(
                        "[place{}] fn={} v={}: ADMITTED host={} tp={} moves={} frees={}",
                        if mode == Mode::Preview {
                            "-preview"
                        } else {
                            ""
                        },
                        data.def(d_nr).name(),
                        function.name(plan.v),
                        function.name(plan.host),
                        plan.tp,
                        plan.moves.len(),
                        plan.frees_held.len()
                    );
                }
                plans.push(plan);
            }
            Err(why) => {
                if crate::keys::trace_place() {
                    eprintln!(
                        "[place{}] fn={} v={}: DECLINED — {why}",
                        if mode == Mode::Preview {
                            "-preview"
                        } else {
                            ""
                        },
                        data.def(d_nr).name(),
                        function.name(*v)
                    );
                }
            }
        }
    }
    plans
}

/// The bind shapes the pass examines at all: a PLAIN local (not a parameter, not a
/// caller-side hidden buffer, not a never-free binding, not a compiler temporary) bound from
/// a direct call to a loft-defined callee that answers a bare dense record through a hidden
/// `__retbuf` buffer.  The local's tests are `use_analysis::adopts_minted_at_bind`'s; the
/// callee test differs because an all-literal callee is not B1's promoted-local shape — it
/// takes the older fresh-adopting pairing — and `admit` then asks the exits themselves.
fn placeable_bind(data: &Data, function: &Function, v: u16, fn_nr: u32) -> bool {
    if (fn_nr as usize) >= data.definitions.len() {
        return false;
    }
    let def = data.def(fn_nr);
    if !def.is_loft_defined() {
        return false;
    }
    // The return SHAPE is asked before the ownership question, because the ownership question
    // is only defined for a heap return.  `returns_borrowed_view` reads the returned deps as
    // attribute indices, and a `Type::Function` return carries a `CALLEE_FRAME`-tagged note
    // instead — a closure's own frame variable, never an attribute index — which that read
    // refuses.  A bare dense record is the only shape `@FR-R-Place` places, so asking the
    // shape first makes that precondition structural rather than a second gate beside it.
    let (shape, nullable) = def.returned().peel_optional();
    if nullable || !matches!(shape, Type::Reference(_, _)) {
        return false;
    }
    // `@FR-R-Place`'s callee clause: a callee that may hand back a store it did not mint
    // licenses nothing (`@FR-O-Proxy`, the deps proxy).
    if def.returns_borrowed_view() {
        return false;
    }
    if function.is_argument(v) || function.is_caller_hidden_buf(v) || function.is_skip_free(v) {
        return false;
    }
    // A compiler temporary (`__ret_N`, `__lift_N`, a work-ref) is never the shape.
    !function.name(v).starts_with("__")
}

#[allow(clippy::too_many_arguments)]
fn admit(
    data: &Data,
    d_nr: u32,
    top: &Block,
    idx: usize,
    v: u16,
    fn_nr: u32,
    args: &[Value],
    ops: &Ops,
    mode: Mode,
) -> Result<Plan, &'static str> {
    let function = data.def(d_nr).variables();
    // `@FR-N-Shape` — the nullability is read off the marker, not off a missing arm: a
    // `P?` local holds the sentinel on some path and is not this shape.
    let (shape, nullable) = function.tp(v).peel_optional();
    if nullable || !matches!(shape, Type::Reference(_, _)) {
        return Err("the local is not a bare dense record");
    }
    let Some(buf_idx) = data.def(fn_nr).hidden_return_buffer_attr() else {
        return Err("the callee has no hidden return buffer");
    };
    if !callee_writes_buffer_at_every_exit(data, fn_nr, buf_idx) {
        return Err("the callee does not build a fresh literal into its buffer on every exit");
    }
    let Some(Value::Var(buf)) = args.get(buf_idx).map(Value::unspan) else {
        return Err("the buffer argument is not a variable");
    };
    let buf = *buf;
    if !function.name(buf).starts_with("__ref_") {
        return Err("the buffer argument is not a caller work-ref");
    }
    let mut call_mentions = HashSet::new();
    for (i, a) in args.iter().enumerate() {
        if i == buf_idx {
            continue;
        }
        if mentions(a, buf) || mentions(a, v) {
            return Err("an argument of the call names the buffer or the local");
        }
        collect_vars(a, &mut call_mentions);
    }
    let (mut buf_inits, mut v_inits) = (0usize, 0usize);
    for stmt in &top.operators[..idx] {
        match stmt.unspan() {
            Value::Set(w, val) if *w == buf => {
                if !matches!(val.unspan(), Value::Null) {
                    return Err("the buffer is bound before the call");
                }
                buf_inits += 1;
            }
            Value::Set(w, val) if *w == v => {
                if !matches!(val.unspan(), Value::Null) {
                    return Err("the local is bound before the call");
                }
                v_inits += 1;
            }
            s => {
                if mentions(s, buf) || mentions(s, v) {
                    return Err("the buffer or the local is named before the call");
                }
            }
        }
    }
    if buf_inits > 1 || (mode == Mode::Rewrite && buf_inits != 1) {
        return Err("the buffer's null init is not one top-level statement before the call");
    }
    if v_inits > 1 {
        return Err("the local has more than one null init");
    }
    let mut cx = Cx {
        data,
        function,
        ops,
        v,
        buf,
        tp: None,
        host: None,
        in_loop: 0,
        call_mentions,
        elm_host: HashMap::new(),
        own_retbuf: own_return_buffer(data, d_nr),
        path: Vec::new(),
        moves: HashSet::new(),
        frees_held: HashSet::new(),
        frees_dropped: HashSet::new(),
    };
    let mut st = St::Held;
    for (i, stmt) in top.operators.iter().enumerate().skip(idx + 1) {
        match cx.child(i, stmt, st, Pos::Stmt)? {
            Flow::Next(n) => st = n,
            Flow::Exit => break,
        }
    }
    match (cx.host, cx.tp) {
        (Some(host), Some(tp)) => Ok(Plan {
            v,
            buf,
            host,
            tp,
            moves: cx.moves,
            frees_held: cx.frees_held,
            frees_dropped: cx.frees_dropped,
        }),
        _ => Err("no owning destination"),
    }
}

/// The variable holding function `d_nr`'s own hidden return buffer — `__retbuf`, or the
/// local the parser promoted onto it — when the function answers through one.
fn own_return_buffer(data: &Data, d_nr: u32) -> Option<u16> {
    let def = data.def(d_nr);
    let idx = def.hidden_return_buffer_attr()?;
    let name = &def.attributes().get(idx)?.name;
    let v = def.variables().var(name);
    (v != u16::MAX).then_some(v)
}

/// `(R-Place)`'s callee clause: every exit of `fn_nr` is a fresh literal written into the
/// return buffer the caller handed (B2 unit 1's contract) — or a CHAIN that hands that same
/// buffer to a callee of which this holds — and the body's last statement is such an exit,
/// so no path answers another store.  A callee whose buffer attribute was promoted onto a
/// local (`o = P { … }; …; o`) declines here exactly as unit 1 declines it: the local IS the
/// buffer, and a literal built beside it would share the record.
///
/// The buffer is the attribute at the callee's return-buffer index by WHATEVER name it
/// carries: `__retbuf`, or the `__ref_N` a chain exit renamed it to
/// (`Parser::chain_return_buffer_var`) — a chain function's exits spell that name.
///
/// An exit has four SPELLINGS in the IR and all are read: `return { Object …; buf }` (the
/// `Return` wraps the literal's block), `{ Object …; return buf }` (the `Return` is the
/// block's last operator — the form a function with text work-refs takes, their frees
/// standing between the writes and the return), the chain `return { one_buffer_chain: buf =
/// g(…, buf); …; buf }` — an exit that writes the buffer iff `g` writes ITS buffer on every
/// exit, asked recursively, a cycle declining — and for the body's last statement alone the
/// bare tail `{ Object …; buf }` (or the chain block) as the parser leaves it before the
/// scope pass wraps it (the preview's view).  A `Return` met anywhere else is an exit that
/// answers something other than the buffer.
fn callee_writes_buffer_at_every_exit(data: &Data, fn_nr: u32, buf_idx: usize) -> bool {
    chain_writes_buffer(data, fn_nr, buf_idx, &mut HashSet::new())
}

fn chain_writes_buffer(data: &Data, fn_nr: u32, buf_idx: usize, active: &mut HashSet<u32>) -> bool {
    if !active.insert(fn_nr) {
        return false;
    }
    let answer = chain_writes_buffer_inner(data, fn_nr, buf_idx, active);
    active.remove(&fn_nr);
    answer
}

fn chain_writes_buffer_inner(
    data: &Data,
    fn_nr: u32,
    buf_idx: usize,
    active: &mut HashSet<u32>,
) -> bool {
    let def = data.def(fn_nr);
    let Some(attr) = def.attributes().get(buf_idx) else {
        return false;
    };
    if attr.name != "__retbuf" && !attr.name.starts_with("__ref_") {
        return false;
    }
    let buf_var = def.variables().var(&attr.name);
    if buf_var == u16::MAX || !def.variables().is_argument(buf_var) {
        return false;
    }
    let Value::Block(bl) = def.code().unspan() else {
        return false;
    };
    let Some(last) = bl.operators.last() else {
        return false;
    };
    let mut cx = Exits {
        data,
        buf_var,
        active,
        count: 0,
        ok: true,
    };
    if tail_object_yielding(last, buf_var) || chain_block(data, last, buf_var).is_some() {
        // The tail spelling, the parser's tail before the scope pass wraps it in a
        // `Return`: the body's last statement IS the literal's block (or the chain block),
        // yielding the buffer.
        cx.count += 1;
        if let Some(target) = chain_block(data, last, buf_var) {
            cx.chain(target);
        }
        for op in &bl.operators[..bl.operators.len() - 1] {
            cx.check(op);
        }
    } else {
        if !cx.is_exit(last) {
            if crate::keys::trace_place() {
                eprintln!(
                    "[place] callee {} last statement is not an exit: {}",
                    def.name(),
                    match last.unspan() {
                        Value::Block(b) => format!("block {} deps {:?}", b.name, b.result.depend()),
                        Value::Return(_) => "return".to_string(),
                        other => format!("{other:?}").chars().take(60).collect(),
                    }
                );
            }
            return false;
        }
        cx.check(def.code());
    }
    cx.count > 0 && cx.ok
}

/// The exit walk of one callee: counts the exits and whether every one writes the buffer.
struct Exits<'a> {
    data: &'a Data,
    buf_var: u16,
    active: &'a mut HashSet<u32>,
    count: usize,
    ok: bool,
}

impl Exits<'_> {
    /// A chain exit's target writes its own buffer on every exit — or this exit does not.
    fn chain(&mut self, (g, g_buf): (u32, usize)) {
        if !chain_writes_buffer(self.data, g, g_buf, self.active) {
            self.ok = false;
        }
    }

    fn is_exit(&mut self, op: &Value) -> bool {
        match op.unspan() {
            Value::Return(inner) => {
                if crate::parser::Parser::tail_fresh_object_workref(inner) == Some(self.buf_var) {
                    return true;
                }
                if let Some(target) = chain_block(self.data, inner, self.buf_var) {
                    self.chain(target);
                    return true;
                }
                false
            }
            Value::Block(bl) => buffer_object_returning(bl, self.buf_var),
            _ => false,
        }
    }

    fn check(&mut self, node: &Value) {
        match node.unspan() {
            Value::Return(inner) => {
                self.count += 1;
                if crate::parser::Parser::tail_fresh_object_workref(inner) == Some(self.buf_var) {
                } else if let Some(target) = chain_block(self.data, inner, self.buf_var) {
                    self.chain(target);
                } else {
                    self.ok = false;
                }
            }
            Value::Block(bl) if buffer_object_returning(bl, self.buf_var) => {
                self.count += 1;
                let n = bl.operators.len();
                for op in &bl.operators[..n - 1] {
                    self.check(op);
                }
            }
            n => n.for_each_child(&mut |c| self.check(c)),
        }
    }
}

/// The chain spelling — `{ one_buffer_chain: buf = g(…, buf); …; buf }`: a block the parser
/// names so, yielding the buffer, whose one call binds the buffer from a loft-defined `g`
/// handed that same buffer at its own return-buffer index.  Answers `(g, g's buffer index)`.
/// Enforces `@FR-R-Place` (the callee clause's chain form).
fn chain_block(data: &Data, op: &Value, buf_var: u16) -> Option<(u32, usize)> {
    let Value::Block(bl) = op.unspan() else {
        return None;
    };
    if !bl.name.starts_with("one_buffer_chain")
        || !matches!(bl.operators.last().map(Value::unspan), Some(Value::Var(w)) if *w == buf_var)
    {
        return None;
    }
    let mut target = None;
    for op in &bl.operators {
        let Value::Set(w, rhs) = op.unspan() else {
            continue;
        };
        if *w != buf_var {
            continue;
        }
        let Value::Call(g, args) = rhs.unspan() else {
            return None;
        };
        if (*g as usize) >= data.definitions.len() || !data.def(*g).is_loft_defined() {
            return None;
        }
        let g_buf = data.def(*g).hidden_return_buffer_attr()?;
        if !matches!(args.get(g_buf).map(Value::unspan), Some(Value::Var(b)) if *b == buf_var) {
            return None;
        }
        if target.is_some() {
            return None;
        }
        target = Some((*g, g_buf));
    }
    target
}

/// The tail spelling: an `Object` block building into the buffer whose value is the
/// buffer — the body's last statement as the parser leaves it.
fn tail_object_yielding(op: &Value, buf_var: u16) -> bool {
    matches!(op.unspan(), Value::Block(bl) if bl.name == "Object"
        && bl.result.depend() == [buf_var]
        && matches!(bl.operators.last().map(Value::unspan), Some(Value::Var(w)) if *w == buf_var))
}

/// A literal's `Object` block that builds into the buffer and ends by returning it — the
/// second spelling of an exit.
fn buffer_object_returning(bl: &Block, buf_var: u16) -> bool {
    bl.name == "Object"
        && bl.result.depend() == [buf_var]
        && matches!(bl.operators.last().map(Value::unspan),
            Some(Value::Return(r)) if matches!(r.unspan(), Value::Var(w) if *w == buf_var))
}

/// Does `node` name variable `w` anywhere — as a `Var`, or in one of the variants that
/// carry a variable number outside a `Var` node (`Set`, `TupleGet`, `TuplePut`, `CallRef`,
/// `FnRef`, `FnRefDnr`, `Iter`)?  The second spelling is what a `Var`-only matcher misses.
fn mentions(node: &Value, w: u16) -> bool {
    let mut found = false;
    node.walk(&mut |n| {
        if names_var(n, w) {
            found = true;
        }
    });
    found
}

fn names_var(n: &Value, w: u16) -> bool {
    match n.unspan() {
        Value::Var(x)
        | Value::Set(x, _)
        | Value::TupleGet(x, _)
        | Value::TuplePut(x, _, _)
        | Value::CallRef(x, _)
        | Value::FnRefDnr(x)
        | Value::Iter(x, _, _, _)
        | Value::FnRef(_, x, _) => *x == w,
        _ => false,
    }
}

fn collect_vars(node: &Value, into: &mut HashSet<u16>) {
    node.walk(&mut |n| match n.unspan() {
        Value::Var(x)
        | Value::Set(x, _)
        | Value::TupleGet(x, _)
        | Value::TuplePut(x, _, _)
        | Value::CallRef(x, _)
        | Value::FnRefDnr(x)
        | Value::Iter(x, _, _, _)
        | Value::FnRef(_, x, _) => {
            into.insert(*x);
        }
        _ => {}
    });
}

/// A value the store hands out by VALUE — nothing that names a record or a collection, so
/// an operation answering one cannot carry a reference into the placed record out of the
/// receiver chain it was read from.
fn scalar_like(t: &Type) -> bool {
    matches!(
        t.base(),
        Type::Void
            | Type::Null
            | Type::Never
            | Type::Integer(_)
            | Type::Boolean
            | Type::Float
            | Type::Single
            | Type::Character
            | Type::Text(_)
            | Type::Enum(_, false, _)
    )
}

/// Whether the local still holds the placed record on the path being walked.
#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    Held,
    Moved,
}

/// Where a node stands: a statement, the receiver (first argument) of a native operation,
/// or any other argument position.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pos {
    Stmt,
    Recv,
    Arg,
}

/// What a walked node does to the path: continues with a state, or ends it (a `return`,
/// a `break`, a `continue`).
#[derive(Clone, Copy)]
enum Flow {
    Next(St),
    Exit,
}

type Verdict = Result<Flow, &'static str>;

struct Cx<'a> {
    data: &'a Data,
    function: &'a Function,
    ops: &'a Ops,
    v: u16,
    buf: u16,
    tp: Option<u16>,
    host: Option<u16>,
    in_loop: u32,
    /// Every variable an argument of the call names: the host must not be one of them.
    call_mentions: HashSet<u16>,
    /// The `_elm_` element locals of literal groups appended to a parameter's collection,
    /// each with the parameter that hosts it — filled as the walk meets their `OpNewRecord`.
    elm_host: HashMap<u16, u16>,
    /// This function's own hidden return buffer, when it has one: the witness the
    /// record-function form of the local's exit free names.
    own_retbuf: Option<u16>,
    /// The child indices from the root to the node being walked.
    path: Path,
    moves: HashSet<Path>,
    frees_held: HashSet<Path>,
    frees_dropped: HashSet<Path>,
}

impl Cx<'_> {
    fn names_either(&self, node: &Value) -> bool {
        mentions(node, self.v) || mentions(node, self.buf)
    }

    fn walk_stmts(&mut self, stmts: &[Value], mut st: St) -> Verdict {
        for (i, s) in stmts.iter().enumerate() {
            match self.child(i, s, st, Pos::Stmt)? {
                Flow::Next(n) => st = n,
                Flow::Exit => return Ok(Flow::Exit),
            }
        }
        Ok(Flow::Next(st))
    }

    /// Walk child `i` of the current node — the one enumeration the rewrite mirrors.
    fn child(&mut self, i: usize, node: &Value, st: St, pos: Pos) -> Verdict {
        self.path.push(i);
        let r = self.walk(node, st, pos);
        self.path.pop();
        r
    }

    /// The fallback for a node kind this walk does not model is a DECLINE whenever the
    /// node names the local or the buffer, and a pass-through otherwise: a node that names
    /// neither cannot read, move or free the record, and a node that names one in a shape
    /// not listed here is exactly the shape the rules have not admitted.
    fn walk(&mut self, node: &Value, st: St, pos: Pos) -> Verdict {
        let node = node.unspan();
        match node {
            Value::Break(_) | Value::Continue(_) => Ok(Flow::Exit),
            Value::Var(w) => {
                if *w == self.v {
                    return if pos == Pos::Recv && st == St::Held {
                        Ok(Flow::Next(st))
                    } else if st == St::Moved {
                        Err("the local is read after its move")
                    } else {
                        Err("the local is used other than as the receiver of a native read")
                    };
                }
                if *w == self.buf {
                    return Err("the buffer is named outside its admitted sites");
                }
                Ok(Flow::Next(st))
            }
            Value::Set(w, val) => {
                if *w == self.v {
                    return Err("the local is rebound");
                }
                if *w == self.buf {
                    return Err("the buffer is rebound");
                }
                self.note_element(*w, val);
                self.child(0, val, st, Pos::Arg)
            }
            Value::Call(d, args) => self.walk_call(*d, args, st, pos),
            Value::If(cond, then, otherwise) => {
                let st = match self.child(0, cond, st, Pos::Arg)? {
                    Flow::Next(s) => s,
                    Flow::Exit => return Ok(Flow::Exit),
                };
                let ft = self.child(1, then, st, Pos::Stmt)?;
                let fe = self.child(2, otherwise, st, Pos::Stmt)?;
                match (ft, fe) {
                    (Flow::Exit, Flow::Exit) => Ok(Flow::Exit),
                    (Flow::Exit, f) | (f, Flow::Exit) => Ok(f),
                    (Flow::Next(a), Flow::Next(b)) if a == b => Ok(Flow::Next(a)),
                    // One arm stored the record and the other still holds it: the exit
                    // both reach would need a free that is right on one path only.
                    (Flow::Next(_), Flow::Next(_)) => {
                        Err("a path that stored the record rejoins one that holds it")
                    }
                }
            }
            Value::Block(bl) => self.walk_stmts(&bl.operators, st),
            Value::Loop(bl) => {
                // The body may run any number of times: a destination inside it is
                // refused (`destination`), so the state after the loop is the state before.
                self.in_loop += 1;
                let r = self.walk_stmts(&bl.operators, st);
                self.in_loop -= 1;
                r?;
                Ok(Flow::Next(st))
            }
            Value::Return(x) => {
                self.child(0, x, st, Pos::Arg)?;
                Ok(Flow::Exit)
            }
            Value::Drop(x) => self.child(0, x, st, Pos::Arg),
            _ => {
                if self.names_either(node) {
                    Err("the local or the buffer is named in a shape the rewrite does not model")
                } else {
                    Ok(Flow::Next(st))
                }
            }
        }
    }

    /// A literal group's element local (`_elm_N = OpNewRecord(X, …)`) hosted by parameter
    /// `X`; any other bind of an element local forgets it.
    fn note_element(&mut self, w: u16, val: &Value) {
        let host = match val.unspan() {
            Value::Call(d, a)
                if *d == self.ops.new_record
                    && self.function.is_compiler_generated(w)
                    && self.function.name(w).starts_with("_elm_") =>
            {
                match a.first().map(Value::unspan) {
                    // A nullable host (`X?`) may be absent at the call and declines
                    // (`@FR-N-Shape`: read off the marker).
                    Some(Value::Var(x))
                        if self.function.is_argument(*x)
                            && matches!(
                                self.function.tp(*x).peel_optional(),
                                (Type::Reference(_, _), false)
                            ) =>
                    {
                        Some(*x)
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        match host {
            Some(x) => {
                self.elm_host.insert(w, x);
            }
            None => {
                self.elm_host.remove(&w);
            }
        }
    }

    fn walk_call(&mut self, d: u32, args: &[Value], st: St, pos: Pos) -> Verdict {
        let is_v = |a: &Value| matches!(a.unspan(), Value::Var(w) if *w == self.v);
        let is_buf = |a: &Value| matches!(a.unspan(), Value::Var(w) if *w == self.buf);
        if d == self.ops.copy && args.len() == 3 && is_v(&args[0]) {
            return self.destination(args, st);
        }
        // The exit free of the local, in either of its spellings — `OpFreeRef(v)`, or in a
        // function that answers a record through its own buffer the witness-guarded
        // `OpFreeRefIfDistinct(v, __retbuf)` (the local freed unless it IS the buffer's
        // store; an admitted local is never returned, so it never is): one record free
        // where the record is still held, nothing where the move already consumed it.
        let own_exit_free = (d == self.ops.free_ref && args.len() == 1 && is_v(&args[0]))
            || (d == self.ops.free_if_distinct
                && args.len() == 2
                && is_v(&args[0])
                && self
                    .own_retbuf
                    .is_some_and(|r| matches!(args[1].unspan(), Value::Var(w) if *w == r)));
        if own_exit_free {
            let path = self.path.clone();
            match st {
                St::Held => self.frees_held.insert(path),
                St::Moved => self.frees_dropped.insert(path),
            };
            return Ok(Flow::Next(st));
        }
        if d == self.ops.free_if_distinct && args.len() == 2 && is_buf(&args[0]) && is_v(&args[1]) {
            // The buffer's witness-guarded free: the buffer IS the local's record, so the
            // local's own free (above) is the whole release and this one goes.
            let path = self.path.clone();
            self.frees_dropped.insert(path);
            return Ok(Flow::Next(st));
        }
        if (d as usize) >= self.data.definitions.len() {
            return Err("a call to an unknown definition");
        }
        if !args.iter().any(|a| self.names_either(a)) {
            return Ok(Flow::Next(st));
        }
        let def = self.data.def(d);
        if def.is_loft_defined() {
            return Err("the local or the buffer is handed to a loft-defined call");
        }
        if def.name().starts_with("OpFreeRef") {
            return Err("a free names the local or the buffer outside the exit pair");
        }
        if st == St::Moved {
            return Err("the local is read after its move");
        }
        // A reference answered off the local must itself be a receiver: at any other
        // position it would be bound, stored or returned, and outlive the record it
        // names (`OpSetRef(n, fld, OpGetField(v, …))`, `w = v.field`, `return v.field`).
        if !scalar_like(def.returned())
            && pos != Pos::Recv
            && args.first().is_some_and(|a| self.names_either(a))
        {
            return Err("a reference into the local leaves the receiver chain");
        }
        let mut st = st;
        for (i, a) in args.iter().enumerate() {
            let p = if i == 0 { Pos::Recv } else { Pos::Arg };
            st = match self.child(i, a, st, p)? {
                Flow::Next(s) => s,
                Flow::Exit => return Ok(Flow::Exit),
            };
        }
        Ok(Flow::Next(st))
    }

    /// `OpCopyRecord(v, OpGetField(_elm_N, off, tp), tp)` where `_elm_N` was appended to a
    /// parameter's collection: the one owning destination `(R-Place)` admits, while the
    /// local still holds the record, outside any loop, with no flag on the copy, into a
    /// field of the record's own type, in the same host as every other destination, a host
    /// no argument of the call reaches.
    fn destination(&mut self, args: &[Value], st: St) -> Verdict {
        if self.in_loop > 0 {
            return Err("the destination is inside a loop");
        }
        if st == St::Moved {
            return Err("a second destination on one path");
        }
        let Value::Int(raw) = args[2].unspan() else {
            return Err("the copy's type operand is not a literal");
        };
        let Ok(raw) = u16::try_from(*raw) else {
            return Err("the copy's type operand is out of range");
        };
        if raw & !crate::keys::COPY_TP_MASK != 0 {
            return Err("the copy carries a flag");
        }
        let Value::Call(g, ga) = args[1].unspan() else {
            return Err("the destination is not a field");
        };
        if *g != self.ops.get_field || ga.len() != 3 {
            return Err("the destination is not a field");
        }
        let (Value::Var(elm), Value::Int(ftp)) = (ga[0].unspan(), ga[2].unspan()) else {
            return Err("the destination field is not on an element local");
        };
        if u16::try_from(*ftp) != Ok(raw) {
            return Err("the destination field's type is not the record's");
        }
        let Some(&host) = self.elm_host.get(elm) else {
            return Err("the destination is not an element appended to a parameter's collection");
        };
        if self.call_mentions.contains(&host) {
            return Err("an argument of the call reaches the destination's store");
        }
        if self.host.is_some_and(|h| h != host) {
            return Err("the destinations name two hosts");
        }
        self.host = Some(host);
        self.tp = Some(raw);
        let path = self.path.clone();
        self.moves.insert(path);
        Ok(Flow::Next(St::Moved))
    }
}

/// The rewrite, walking the SAME child enumeration as [`Cx::walk`] so a site's path names
/// the same node: the buffer's null init (found by shape — there is exactly one), the moves,
/// and the exit frees — one record free where the analysis saw the record held, nothing
/// where it saw it moved or where the buffer's witness-guarded free stood.
fn apply(node: &mut Value, path: &mut Path, plan: &Plan, ops: &Ops) {
    match node {
        Value::Span(b) => apply(&mut b.1, path, plan, ops),
        Value::Block(bl) | Value::Loop(bl) => apply_block(bl, path, plan, ops),
        Value::Set(w, val) if *w == plan.buf && matches!(val.unspan(), Value::Null) => {
            **val = Value::Call(
                ops.place,
                vec![Value::Var(plan.host), Value::Int(i32::from(plan.tp))],
            );
        }
        Value::Set(_, val) => {
            path.push(0);
            apply(val, path, plan, ops);
            path.pop();
        }
        Value::If(cond, then, otherwise) => {
            for (i, child) in [cond, then, otherwise].into_iter().enumerate() {
                path.push(i);
                apply(child, path, plan, ops);
                path.pop();
            }
        }
        Value::Return(x) | Value::Drop(x) => {
            path.push(0);
            apply(x, path, plan, ops);
            path.pop();
        }
        Value::Call(d, args) => {
            if *d == ops.copy && plan.moves.contains(path) {
                crate::rewrite_census::fired("R-MoveLast", 1);
                *d = ops.move_rec;
                args[2] = Value::Int(i32::from(plan.tp));
                return;
            }
            if (*d == ops.free_ref || *d == ops.free_if_distinct) && plan.frees_held.contains(path)
            {
                *d = ops.free_in;
                *args = vec![Value::Var(plan.v), Value::Int(i32::from(plan.tp))];
                return;
            }
            for (i, a) in args.iter_mut().enumerate() {
                path.push(i);
                apply(a, path, plan, ops);
                path.pop();
            }
        }
        _ => {}
    }
}

fn apply_block(bl: &mut Block, path: &mut Path, plan: &Plan, ops: &Ops) {
    let mut out = Vec::with_capacity(bl.operators.len());
    for (i, mut op) in bl.operators.drain(..).enumerate() {
        path.push(i);
        if plan.frees_dropped.contains(path) {
            path.pop();
            continue;
        }
        apply(&mut op, path, plan, ops);
        path.pop();
        out.push(op);
    }
    bl.operators = out;
}

// ─── The loop clause: a decoder's own shape ───────────────────────────────────────────────
//
// `@FR-R-Place`, the bind INSIDE A LOOP: `sub = read(bytes, p)` at the top of a loop body, the
// buffer the call is handed being the frame's lazy work-ref (`if buf is null { OpDatabase }
// else { OpClear }` right before the call — `@FR-O-LazyBuffer` with `@FR-H-ClearRelease`),
// the result stored ONCE in the same turn into an element minted into a HOST collection — a
// parameter's, or a local's whose backing is its own `__vdb_N` store — either whole
// (`items += [sub]`) or by its ONE heap-owning field (`items += [sub.value]`) while the
// wrapper's scalar fields are read after; the local dead by the turn's end, where its
// per-turn free stands.  Placed: the lazy mint claims the buffer IN THE HOST'S STORE
// (`OpPlaceRecord(host, tp)`), so the callee refills a record that already lives where its
// payload will land; the store becomes the SHALLOW move (`OpMoveField`: the bytes relocate
// within the store, the heap handles keep their claims, the source is zeroed so the next
// turn's `OpClear` and the callee's refill find an empty record); and the buffer's exit frees
// become no-ops, because the host store's own release covers the block — a parameter's host
// outlives the frame, and a `__vdb_N` host is released at the exit that already stood there.
// Declines, each keeping the copy: a read of the moved payload after the move, a second
// destination, a nested loop, a `return` that names the local, an argument of the call that
// reaches the host, a host bound after the loop or rebound anywhere, a buffer named anywhere
// but its init, its mint, the call, the turn's free and the exit frees.

struct LoopPlan {
    buf: u16,
    host: u16,
    tp: u16,
    move_tp: u16,
    /// The `OpDatabase(buf, tp)` inside the lazy mint, replaced by the placement.
    mint: Path,
    /// The `OpCopyRecord` that becomes the shallow move.
    mov: Path,
    /// The buffer's exit frees, made no-ops in place (a removal would shift the paths of
    /// every later plan in the function).
    drops: Vec<Path>,
}

fn admitted_loops(data: &Data, d_nr: u32, code: &Value, ops: &Ops) -> Vec<LoopPlan> {
    let function = data.def(d_nr).variables();
    let mut plans = Vec::new();
    let mut path = Path::new();
    find_loop_binds(
        data, d_nr, function, code, ops, &mut path, 0, &mut plans, code,
    );
    plans
}

#[allow(clippy::too_many_arguments)]
fn find_loop_binds(
    data: &Data,
    d_nr: u32,
    function: &Function,
    node: &Value,
    ops: &Ops,
    path: &mut Path,
    loop_depth: u32,
    plans: &mut Vec<LoopPlan>,
    top: &Value,
) {
    let stmts: &[Value] = match node.unspan() {
        Value::Block(bl) | Value::Loop(bl) => &bl.operators,
        Value::Insert(ops) => ops,
        _ => &[],
    };
    match node.unspan() {
        Value::Block(_) | Value::Loop(_) | Value::Insert(_) => {
            let depth = loop_depth + u32::from(matches!(node.unspan(), Value::Loop(_)));
            for (i, stmt) in stmts.iter().enumerate() {
                path.push(i);
                if depth > 0
                    && crate::keys::trace_place()
                    && let Value::Set(v, val) = stmt.unspan()
                    && let Value::Call(fn_nr, _) = val.unspan()
                    && (*fn_nr as usize) < data.definitions.len()
                    && data.def(*fn_nr).is_loft_defined()
                    && !placeable_bind(data, function, *v, *fn_nr)
                {
                    let def = data.def(*fn_nr);
                    let (shape, nullable) = def.returned().peel_optional();
                    eprintln!(
                        "[place] fn={} v={}: not a placeable loop bind — callee={} bare_ref={} nullable={} borrowed={} arg={} hidden={} skip_free={} name={}",
                        data.def(d_nr).name(),
                        function.name(*v),
                        def.name(),
                        matches!(shape, Type::Reference(_, _)),
                        nullable,
                        def.returns_borrowed_view(),
                        function.is_argument(*v),
                        function.is_caller_hidden_buf(*v),
                        function.is_skip_free(*v),
                        function.name(*v)
                    );
                }
                if depth > 0
                    && let Value::Set(v, val) = stmt.unspan()
                    && let Value::Call(fn_nr, args) = val.unspan()
                    && placeable_bind(data, function, *v, *fn_nr)
                {
                    match admit_loop(data, d_nr, top, stmts, path, i, *v, *fn_nr, args, ops) {
                        Ok(plan) => {
                            crate::rewrite_census::fired("R-Place", 1);
                            if crate::keys::trace_place() {
                                eprintln!(
                                    "[place] fn={} v={}: ADMITTED (loop) host={} tp={} move_tp={}",
                                    data.def(d_nr).name(),
                                    function.name(*v),
                                    function.name(plan.host),
                                    plan.tp,
                                    plan.move_tp
                                );
                            }
                            plans.push(plan);
                        }
                        Err(why) => {
                            if crate::keys::trace_place() {
                                eprintln!(
                                    "[place] fn={} v={}: DECLINED (loop) — {why}",
                                    data.def(d_nr).name(),
                                    function.name(*v)
                                );
                            }
                        }
                    }
                }
                find_loop_binds(data, d_nr, function, stmt, ops, path, depth, plans, top);
                path.pop();
            }
        }
        Value::Set(_, val) => {
            path.push(0);
            find_loop_binds(data, d_nr, function, val, ops, path, loop_depth, plans, top);
            path.pop();
        }
        Value::If(c, t, e) => {
            for (i, child) in [c, t, e].into_iter().enumerate() {
                path.push(i);
                find_loop_binds(
                    data, d_nr, function, child, ops, path, loop_depth, plans, top,
                );
                path.pop();
            }
        }
        Value::Return(x) | Value::Drop(x) => {
            path.push(0);
            find_loop_binds(data, d_nr, function, x, ops, path, loop_depth, plans, top);
            path.pop();
        }
        _ => {}
    }
}

/// The one heap-owning field of `v`'s record whose runtime type is `ftp`, with every other
/// field scalar: `Some(())` names it, `None` declines (two heap fields, or none of that type).
fn one_heap_field(data: &Data, function: &Function, v: u16, ftp: u16) -> bool {
    let Some(td) = function.tp(v).base().heap_def_nr() else {
        return false;
    };
    let mut heap_fields = 0usize;
    let mut found = false;
    for attr in data.def(td).attributes() {
        if attr.name.starts_with("__") {
            continue;
        }
        if scalar_like(&attr.typedef) && !matches!(attr.typedef.base(), Type::Text(_)) {
            continue;
        }
        heap_fields += 1;
        let ad = data.type_def_nr(&attr.typedef);
        if (ad as usize) < data.definitions.len() && data.def(ad).known_type() == ftp {
            found = true;
        }
    }
    heap_fields == 1 && found
}

/// The lazy mint right before the bind — `if buf is null { OpDatabase(buf, tp) } else
/// { OpClear(buf, tp) }` — and the path of the `OpDatabase` in its `then`, where the
/// placement lands.  Either arm may be the bare call or a one-statement block around it.
fn lazy_mint_path(
    body: &[Value],
    bind_path: &Path,
    idx: usize,
    buf: u16,
    tp: u16,
    ops: &Ops,
) -> Result<Path, &'static str> {
    if idx == 0 {
        return Err("no lazy mint before the bind");
    }
    let Value::If(cond, then_arm, else_arm) = body[idx - 1].unspan() else {
        if crate::keys::trace_place() {
            let shown = format!("{:?}", body[idx - 1].unspan());
            eprintln!(
                "[place]   before the bind: {}",
                &shown[..shown.len().min(200)]
            );
        }
        return Err("the statement before the bind is not the lazy mint");
    };
    if crate::keys::trace_place() {
        let shown = format!(
            "{:?} | {:?} | {:?}",
            cond.unspan(),
            then_arm.unspan(),
            else_arm.unspan()
        );
        eprintln!(
            "[place]   lazy mint arms: {}",
            &shown[..shown.len().min(300)]
        );
    }
    let is_call_on_buf = |node: &Value, op: u32, want_tp: bool| -> bool {
        matches!(node.unspan(), Value::Call(called, args) if *called == op
            && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == buf)
            && (!want_tp || matches!(args.get(1).map(Value::unspan), Some(Value::Int(t)) if u16::try_from(*t) == Ok(tp))))
    };
    let unwrap = |node: &Value| -> (Value, Option<usize>) {
        match node.unspan() {
            Value::Block(bl) if bl.operators.len() == 1 => (bl.operators[0].clone(), Some(0)),
            Value::Insert(list) if list.len() == 1 => (list[0].clone(), Some(0)),
            other => (other.clone(), None),
        }
    };
    let (then_call, then_idx) = unwrap(then_arm);
    let (else_call, _) = unwrap(else_arm);
    if !is_call_on_buf(cond, ops.ref_is_null, false)
        || !is_call_on_buf(&then_call, ops.database, true)
        || !is_call_on_buf(&else_call, ops.clear, true)
    {
        return Err("the statement before the bind is not the lazy mint");
    }
    let mut mint = bind_path.clone();
    mint.pop();
    mint.push(idx - 1);
    mint.push(1);
    if let Some(k) = then_idx {
        mint.push(k);
    }
    Ok(mint)
}

/// Outside the loop body: the buffer's null init before the loop and its exit frees (whose
/// paths are answered, to become no-ops); nothing else may name the buffer or the local,
/// and the host must be bound before the loop and never rebound.
fn outside_the_turn(
    function: &Function,
    top: &Value,
    bind_path: &Path,
    v: u16,
    buf: u16,
    host: u16,
    ops: &Ops,
) -> Result<Vec<Path>, &'static str> {
    let Value::Block(topbl) = top.unspan() else {
        return Err("no top block");
    };
    let loop_top = bind_path[0];
    let mut drops = Vec::new();
    let mut buf_inits = 0usize;
    let mut host_bound_before = function.is_argument(host);
    for (i, stmt) in topbl.operators.iter().enumerate() {
        if i == loop_top {
            // The loop's own top-level statement: the buffer may be named at the turn
            // (checked by the walk), at the turn's free beside it, and at exit frees in
            // the blocks around the loop (dropped, as the other statements' are).
            let mut found = Vec::new();
            collect_buf_frees(stmt, &mut vec![i], buf, ops, &mut found);
            drops.extend(found);
            let mut outer = stmt.clone();
            strip_turn(&mut outer, &bind_path[1..]);
            strip_pair_frees(&mut outer, v, buf, ops);
            strip_buf_frees(&mut outer, buf, ops);
            if mentions(&outer, buf) || mentions(&outer, v) {
                if crate::keys::trace_place() {
                    let mut hit = String::new();
                    outer.walk(&mut |n| {
                        if hit.is_empty() && (names_var(n, buf) || names_var(n, v)) {
                            hit = format!("{n:?}");
                        }
                    });
                    eprintln!("[place]   outside the turn: {}", &hit[..hit.len().min(200)]);
                }
                return Err("the buffer or the local is named in the loop outside the turn");
            }
            continue;
        }
        match stmt.unspan() {
            Value::Set(w, val) if *w == buf => {
                if !matches!(val.unspan(), Value::Null) || i > loop_top {
                    return Err("the buffer is bound outside its init");
                }
                buf_inits += 1;
                continue;
            }
            Value::Set(w, _) if *w == host => {
                if i < loop_top && !host_bound_before {
                    host_bound_before = true;
                } else {
                    return Err("the host is rebound, or bound after the loop");
                }
            }
            _ => {}
        }
        let mut found = Vec::new();
        collect_buf_frees(stmt, &mut vec![i], buf, ops, &mut found);
        drops.extend(found);
        let mut stripped = stmt.clone();
        strip_buf_frees(&mut stripped, buf, ops);
        if mentions(&stripped, buf) {
            if crate::keys::trace_place() {
                let shown = format!("{:?}", stripped.unspan());
                eprintln!(
                    "[place]   names the buffer outside: {}",
                    &shown[..shown.len().min(240)]
                );
            }
            return Err("the buffer is named outside its admitted sites");
        }
        if mentions(stmt, v) {
            return Err("the local is named outside the loop");
        }
    }
    if buf_inits != 1 {
        return Err("the buffer's null init is not one top-level statement before the loop");
    }
    if !host_bound_before {
        return Err("the host is not bound before the loop");
    }
    Ok(drops)
}

#[allow(clippy::too_many_arguments)]
fn admit_loop(
    data: &Data,
    d_nr: u32,
    top: &Value,
    body: &[Value],
    bind_path: &Path,
    idx: usize,
    v: u16,
    fn_nr: u32,
    args: &[Value],
    ops: &Ops,
) -> Result<LoopPlan, &'static str> {
    let function = data.def(d_nr).variables();
    let (shape, nullable) = function.tp(v).peel_optional();
    let Type::Reference(td, _) = shape.peel_link() else {
        return Err("the local is not a bare dense record");
    };
    if nullable || (*td as usize) >= data.definitions.len() {
        return Err("the local is not a bare dense record");
    }
    let tp = data.def(*td).known_type();
    if tp == u16::MAX {
        return Err("the local's record has no runtime type yet");
    }
    let Some(buf_idx) = data.def(fn_nr).hidden_return_buffer_attr() else {
        return Err("the callee has no hidden return buffer");
    };
    if !callee_writes_buffer_at_every_exit(data, fn_nr, buf_idx) {
        return Err("the callee does not build a fresh literal into its buffer on every exit");
    }
    let Some(Value::Var(buf)) = args.get(buf_idx).map(Value::unspan) else {
        return Err("the buffer argument is not a variable");
    };
    let buf = *buf;
    if !function.name(buf).starts_with("__ref_") {
        return Err("the buffer argument is not a caller work-ref");
    }
    let mut call_mentions = HashSet::new();
    for (i, a) in args.iter().enumerate() {
        if i == buf_idx {
            continue;
        }
        if mentions(a, buf) || mentions(a, v) {
            return Err("an argument of the call names the buffer or the local");
        }
        collect_vars(a, &mut call_mentions);
    }
    let mint = lazy_mint_path(body, bind_path, idx, buf, tp, ops)?;
    // The rest of the turn.
    let mut cx = LoopCx {
        data,
        function,
        ops,
        v,
        buf,
        tp,
        call_mentions,
        elm_host: HashMap::new(),
        views: HashMap::new(),
        path: bind_path[..bind_path.len() - 1].to_vec(),
        st: LSt::Held,
        host: None,
        move_tp: None,
        mov: None,
    };
    for (i, stmt) in body.iter().enumerate().skip(idx + 1) {
        cx.path.push(i);
        let last = i + 1 == body.len();
        let r = cx.stmt(stmt, last);
        cx.path.pop();
        r?;
    }
    let (Some(host), Some(move_tp), Some(mov)) = (cx.host, cx.move_tp, cx.mov) else {
        return Err("the turn stores the local nowhere");
    };
    let drops = outside_the_turn(function, top, bind_path, v, buf, host, ops)?;
    Ok(LoopPlan {
        buf,
        host,
        tp,
        move_tp,
        mint,
        mov,
        drops,
    })
}

/// Blank the turn (the loop body block that holds the bind) inside a clone of the loop's
/// top-level statement, so the rest of that statement can be asked what it names.
fn strip_turn(node: &mut Value, rel: &[usize]) {
    if rel.is_empty() {
        *node = Value::Null;
        return;
    }
    match node {
        Value::Span(b) => strip_turn(&mut b.1, rel),
        Value::Block(bl) | Value::Loop(bl) => {
            if rel.len() == 1 {
                // The turn IS this block's statements from the bind on: blank the block.
                *node = Value::Null;
            } else if let Some(c) = bl.operators.get_mut(rel[0]) {
                strip_turn(c, &rel[1..]);
            }
        }
        Value::Insert(ops) => {
            if rel.len() == 1 {
                *node = Value::Null;
            } else if let Some(c) = ops.get_mut(rel[0]) {
                strip_turn(c, &rel[1..]);
            }
        }
        Value::Set(_, val) => strip_turn(val, &rel[1..]),
        Value::If(c, t, e) => {
            if let Some(child) = [c, t, e].into_iter().nth(rel[0]) {
                strip_turn(child, &rel[1..]);
            }
        }
        Value::Return(x) | Value::Drop(x) => strip_turn(x, &rel[1..]),
        Value::Call(_, args) => {
            if let Some(a) = args.get_mut(rel[0]) {
                strip_turn(a, &rel[1..]);
            }
        }
        _ => {}
    }
}

fn is_pair_free(n: &Value, v: u16, buf: u16, ops: &Ops) -> bool {
    matches!(n.unspan(), Value::Call(d, a)
        if *d == ops.free_if_distinct && a.len() == 2
            && matches!(a[0].unspan(), Value::Var(w) if *w == v)
            && matches!(a[1].unspan(), Value::Var(w) if *w == buf))
}

fn strip_pair_frees(node: &mut Value, v: u16, buf: u16, ops: &Ops) {
    if is_pair_free(node, v, buf, ops) {
        *node = Value::Null;
        return;
    }
    match node {
        Value::Span(b) => strip_pair_frees(&mut b.1, v, buf, ops),
        Value::Block(bl) | Value::Loop(bl) => {
            for c in &mut bl.operators {
                strip_pair_frees(c, v, buf, ops);
            }
        }
        Value::Insert(list) => {
            for c in list {
                strip_pair_frees(c, v, buf, ops);
            }
        }
        Value::If(c, t, e) => {
            strip_pair_frees(c, v, buf, ops);
            strip_pair_frees(t, v, buf, ops);
            strip_pair_frees(e, v, buf, ops);
        }
        Value::Set(_, x) | Value::Return(x) | Value::Drop(x) => strip_pair_frees(x, v, buf, ops),
        _ => {}
    }
}

fn is_buf_free(n: &Value, buf: u16, ops: &Ops) -> bool {
    matches!(n.unspan(), Value::Call(d, a)
        if (*d == ops.free_ref || *d == ops.free_if_distinct)
            && matches!(a.first().map(Value::unspan), Some(Value::Var(b)) if *b == buf))
}

fn collect_buf_frees(node: &Value, path: &mut Path, buf: u16, ops: &Ops, out: &mut Vec<Path>) {
    if is_buf_free(node, buf, ops) {
        out.push(path.clone());
        return;
    }
    match node.unspan() {
        Value::Block(bl) | Value::Loop(bl) => {
            for (i, c) in bl.operators.iter().enumerate() {
                path.push(i);
                collect_buf_frees(c, path, buf, ops, out);
                path.pop();
            }
        }
        Value::Insert(list) => {
            for (i, c) in list.iter().enumerate() {
                path.push(i);
                collect_buf_frees(c, path, buf, ops, out);
                path.pop();
            }
        }
        Value::If(c, t, e) => {
            for (i, child) in [c, t, e].into_iter().enumerate() {
                path.push(i);
                collect_buf_frees(child, path, buf, ops, out);
                path.pop();
            }
        }
        Value::Set(_, x) | Value::Return(x) | Value::Drop(x) => {
            path.push(0);
            collect_buf_frees(x, path, buf, ops, out);
            path.pop();
        }
        _ => {}
    }
}

fn strip_buf_frees(node: &mut Value, buf: u16, ops: &Ops) {
    if is_buf_free(node, buf, ops) {
        *node = Value::Null;
        return;
    }
    match node {
        Value::Span(b) => strip_buf_frees(&mut b.1, buf, ops),
        Value::Block(bl) | Value::Loop(bl) => {
            for c in &mut bl.operators {
                strip_buf_frees(c, buf, ops);
            }
        }
        Value::Insert(list) => {
            for c in list {
                strip_buf_frees(c, buf, ops);
            }
        }
        Value::If(c, t, e) => {
            strip_buf_frees(c, buf, ops);
            strip_buf_frees(t, buf, ops);
            strip_buf_frees(e, buf, ops);
        }
        Value::Set(_, x) | Value::Return(x) | Value::Drop(x) => strip_buf_frees(x, buf, ops),
        _ => {}
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LSt {
    Held,
    Moved,
    /// The one heap-owning field at this byte offset moved out; the scalars stay readable.
    FieldMoved(i64),
}

struct LoopCx<'a> {
    data: &'a Data,
    function: &'a Function,
    ops: &'a Ops,
    v: u16,
    buf: u16,
    tp: u16,
    call_mentions: HashSet<u16>,
    elm_host: HashMap<u16, u16>,
    /// Locals bound to the one heap-owning field of `v` (`kv = kd.value`): each may be the
    /// move's source and nothing else.
    views: HashMap<u16, (i64, u16)>,
    path: Path,
    st: LSt,
    host: Option<u16>,
    move_tp: Option<u16>,
    mov: Option<Path>,
}

impl LoopCx<'_> {
    fn names_either(&self, node: &Value) -> bool {
        mentions(node, self.v)
            || mentions(node, self.buf)
            || self.views.keys().any(|w| mentions(node, *w))
    }

    /// A statement of the turn after the bind.  `last` marks the turn's final statement,
    /// where the local's per-turn free stands.
    fn stmt(&mut self, node: &Value, last: bool) -> Result<(), &'static str> {
        let n = node.unspan();
        // The turn's own free of the local against the buffer: a no-op once placed (the
        // local IS the buffer), kept as it is.
        let _ = last;
        if is_pair_free(n, self.v, self.buf, self.ops) {
            return Ok(());
        }
        self.node(n)
    }

    fn child(&mut self, i: usize, node: &Value) -> Result<(), &'static str> {
        self.path.push(i);
        let r = self.node(node);
        self.path.pop();
        r
    }

    fn node(&mut self, node: &Value) -> Result<(), &'static str> {
        match node.unspan() {
            Value::Loop(_) => Err("a nested loop in the turn"),
            Value::Break(_) | Value::Continue(_) => Ok(()),
            Value::Var(w) => {
                if *w == self.v {
                    return Err("the local is used other than as the receiver of a native read");
                }
                if *w == self.buf {
                    return Err("the buffer is named outside its admitted sites");
                }
                if self.views.contains_key(w) {
                    return Err(
                        "a view of the local's payload is used other than as the move's source",
                    );
                }
                Ok(())
            }
            Value::Set(w, val) => {
                if *w == self.v || *w == self.buf {
                    return Err("the local or the buffer is rebound");
                }
                if self.views.contains_key(w) {
                    return Err("a view of the local's payload is rebound");
                }
                // `kv = kd.value` — a view of the one heap-owning field, to be the move's source.
                if let Value::Call(g, ga) = val.unspan()
                    && *g == self.ops.get_field
                    && ga.len() == 3
                    && matches!(ga[0].unspan(), Value::Var(x) if *x == self.v)
                    && let (Value::Int(off), Value::Int(ftp)) = (ga[1].unspan(), ga[2].unspan())
                    && let Ok(ftp) = u16::try_from(*ftp)
                    && one_heap_field(self.data, self.function, self.v, ftp)
                {
                    if self.st != LSt::Held {
                        return Err("the payload is viewed after its move");
                    }
                    self.views.insert(*w, (i64::from(*off), ftp));
                    return Ok(());
                }
                self.note_element(*w, val);
                self.child(0, val)
            }
            Value::Call(d, args) => self.call(*d, args),
            Value::If(cond, then_arm, else_arm) => {
                self.child(0, cond)?;
                let before = self.st;
                self.child(1, then_arm)?;
                let after_then = self.st;
                self.st = before;
                self.child(2, else_arm)?;
                let after_else = self.st;
                // A move on one arm makes the payload unreadable after the join.
                self.st = match (after_then, after_else) {
                    (left, right) if left == right => left,
                    (LSt::Held, m) | (m, LSt::Held) => m,
                    _ => return Err("two arms move differently"),
                };
                Ok(())
            }
            Value::Block(bl) => {
                for (i, s) in bl.operators.iter().enumerate() {
                    self.child(i, s)?;
                }
                Ok(())
            }
            Value::Insert(list) => {
                for (i, s) in list.iter().enumerate() {
                    self.child(i, s)?;
                }
                Ok(())
            }
            Value::Return(x) | Value::Drop(x) => {
                if self.names_either(x) {
                    return Err("the local or the buffer leaves through a return");
                }
                self.child(0, x)
            }
            other => {
                if self.names_either(other) {
                    Err("the local or the buffer is named in a shape the rewrite does not model")
                } else {
                    Ok(())
                }
            }
        }
    }

    fn note_element(&mut self, w: u16, val: &Value) {
        let host = match val.unspan() {
            Value::Call(d, a)
                if *d == self.ops.new_record
                    && self.function.is_compiler_generated(w)
                    && self.function.name(w).starts_with("_elm_") =>
            {
                match a.first().map(Value::unspan) {
                    Some(Value::Var(x)) if self.host_ok(*x) => Some(*x),
                    _ => None,
                }
            }
            _ => None,
        };
        match host {
            Some(x) => {
                self.elm_host.insert(w, x);
            }
            None => {
                self.elm_host.remove(&w);
            }
        }
    }

    /// A host collection: a parameter that is a record or a collection, or a local whose
    /// backing is its own `__vdb_N` store (`Parser::owns_vdb_backing`'s test).
    fn host_ok(&self, x: u16) -> bool {
        if self.function.is_argument(x) {
            return matches!(
                self.function.tp(x).peel_optional(),
                (Type::Reference(_, _) | Type::Vector(_, _), false)
            );
        }
        let deps = self.function.tp(x).depend();
        deps.len() == 1 && self.function.name(deps[0]).starts_with("__vdb_")
    }

    fn call(&mut self, d: u32, args: &[Value]) -> Result<(), &'static str> {
        let is_v = |a: &Value| matches!(a.unspan(), Value::Var(w) if *w == self.v);
        if d == self.ops.copy && args.len() == 3 {
            let whole = is_v(&args[0]);
            let via_view = match args[0].unspan() {
                Value::Var(w) => self.views.get(w).copied(),
                _ => None,
            };
            let field = via_view.or(match args[0].unspan() {
                Value::Call(g, ga) if *g == self.ops.get_field && ga.len() == 3 && is_v(&ga[0]) => {
                    match (ga[1].unspan(), ga[2].unspan()) {
                        (Value::Int(off), Value::Int(ftp)) => {
                            Some((i64::from(*off), u16::try_from(*ftp).unwrap_or(u16::MAX)))
                        }
                        _ => None,
                    }
                }
                _ => None,
            });
            if whole || field.is_some() {
                return self.destination(args, field);
            }
        }
        if (d as usize) >= self.data.definitions.len() {
            return Err("a call to an unknown definition");
        }
        if !args.iter().any(|a| self.names_either(a)) {
            return Ok(());
        }
        let def = self.data.def(d);
        if is_pair_free(&Value::Call(d, args.to_vec()), self.v, self.buf, self.ops) {
            return Ok(());
        }
        if def.name().starts_with("OpFreeRef") {
            return Err("a free names the local or the buffer outside the turn's end");
        }
        if def.is_loft_defined() {
            // A scalar READ of the local may be an argument (`read(bytes, kd.next)`); the
            // local itself, or a reference into it, may not — each argument is walked, and
            // the Var arm refuses a bare local.
            for (i, a) in args.iter().enumerate() {
                self.child(i, a)?;
            }
            return Ok(());
        }
        if is_v(&args[0]) {
            if !scalar_like(def.returned()) {
                return Err("a reference into the local leaves the receiver chain");
            }
            match self.st {
                LSt::Held => {}
                LSt::Moved => return Err("the local is read after its move"),
                LSt::FieldMoved(off) => {
                    // A scalar read of another field is fine; anything at the moved field's
                    // offset is not.
                    let at = matches!(args.get(1).map(Value::unspan), Some(Value::Int(o)) if i64::from(*o) == off);
                    if at {
                        return Err("the moved payload is read after its move");
                    }
                }
            }
            for a in &args[1..] {
                if self.names_either(a) {
                    return Err("the local is named in its own read's operands");
                }
            }
            return Ok(());
        }
        if !scalar_like(def.returned()) {
            return Err("a reference into the local leaves the receiver chain");
        }
        // The local sits inside an operand (`OpAddInt(s, OpGetInt(v, off))`): each operand is
        // walked, so a nested native read of the local is judged as a receiver read.
        for (i, a) in args.iter().enumerate() {
            self.child(i, a)?;
        }
        Ok(())
    }

    fn destination(
        &mut self,
        args: &[Value],
        field: Option<(i64, u16)>,
    ) -> Result<(), &'static str> {
        if self.st != LSt::Held {
            return Err("a second destination on one path");
        }
        let Value::Int(raw) = args[2].unspan() else {
            return Err("the copy's type operand is not a literal");
        };
        let Ok(raw) = u16::try_from(*raw) else {
            return Err("the copy's type operand is out of range");
        };
        if raw & crate::keys::COPY_FREE_SOURCE != 0 {
            return Err("the copy frees its source");
        }
        let copy_tp = raw & crate::keys::COPY_TP_MASK;
        let move_tp = match field {
            None => {
                if copy_tp != self.tp {
                    return Err("the copy's type is not the record's");
                }
                self.tp
            }
            Some((_, ftp)) => {
                if ftp != copy_tp || !one_heap_field(self.data, self.function, self.v, ftp) {
                    return Err("the moved field is not the record's one heap-owning field");
                }
                ftp
            }
        };
        let elm = match args[1].unspan() {
            Value::Var(elm) => elm,
            Value::Call(g, ga)
                if *g == self.ops.get_field
                    && ga.len() == 3
                    && matches!(ga[2].unspan(), Value::Int(t) if u16::try_from(*t) == Ok(move_tp)) =>
            {
                let Value::Var(elm) = ga[0].unspan() else {
                    return Err("the destination field is not on an element local");
                };
                elm
            }
            _ => return Err("the destination is not an element local or a field of one"),
        };
        let Some(&host) = self.elm_host.get(elm) else {
            return Err("the destination is not an element appended to a host collection");
        };
        if self.call_mentions.contains(&host)
            || self
                .function
                .tp(host)
                .depend()
                .iter()
                .any(|d| self.call_mentions.contains(d))
        {
            return Err("an argument of the call reaches the destination's store");
        }
        self.host = Some(host);
        self.move_tp = Some(move_tp);
        self.mov = Some(self.path.clone());
        self.st = match field {
            None => LSt::Moved,
            Some((off, _)) => LSt::FieldMoved(off),
        };
        Ok(())
    }
}

fn node_at_mut<'a>(node: &'a mut Value, path: &[usize]) -> Option<&'a mut Value> {
    let node = match node {
        Value::Span(b) => return node_at_mut(&mut b.1, path),
        n => n,
    };
    let Some((&i, rest)) = path.split_first() else {
        return Some(node);
    };
    match node {
        Value::Block(bl) | Value::Loop(bl) => node_at_mut(bl.operators.get_mut(i)?, rest),
        Value::Insert(ops) => node_at_mut(ops.get_mut(i)?, rest),
        Value::Set(_, val) if i == 0 => node_at_mut(val, rest),
        Value::If(c, t, e) => node_at_mut([c, t, e].into_iter().nth(i)?, rest),
        Value::Return(x) | Value::Drop(x) if i == 0 => node_at_mut(x, rest),
        Value::Call(_, args) => node_at_mut(args.get_mut(i)?, rest),
        _ => None,
    }
}

fn apply_loop(code: &mut Value, plan: &LoopPlan, ops: &Ops) {
    if let Some(m) = node_at_mut(code, &plan.mint) {
        *m = Value::Set(
            plan.buf,
            Box::new(Value::Call(
                ops.place,
                vec![Value::Var(plan.host), Value::Int(i32::from(plan.tp))],
            )),
        );
    }
    if let Some(Value::Call(d, args)) = node_at_mut(code, &plan.mov).map(Value::unspan_mut) {
        crate::rewrite_census::fired("R-MoveLast", 1);
        *d = ops.move_field;
        args[2] = Value::Int(i32::from(plan.move_tp));
    }
    // The buffer's exit frees become no-ops IN PLACE: a removal would shift the statements
    // after it, and the paths of every later plan in this function (its mint, its move,
    // its own frees) were read off the code before any plan was applied — a second
    // buffer's free then landed one statement further on, on the next buffer's free,
    // and that one leaked its store on every call (the cbor decoder, 400 stores a round).
    for path in &plan.drops {
        let n = node_at_mut(code, path).expect("a loop plan's free path names a node");
        assert!(
            is_buf_free(n, plan.buf, ops),
            "a loop plan's free path no longer names its buffer's free"
        );
        *n = Value::Null;
    }
}
