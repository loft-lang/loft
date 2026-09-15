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
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    let plans = admitted(data, d_nr, &code, &ops, Mode::Rewrite);
    for plan in &plans {
        let mut path = Path::new();
        apply(&mut code, &mut path, plan, &ops);
    }
    data.definitions[d_nr as usize].code = code;
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
    if !def.is_loft_defined() || def.returns_borrowed_view() {
        return false;
    }
    let (shape, nullable) = def.returned().peel_optional();
    if nullable || !matches!(shape, Type::Reference(_, _)) {
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
/// `__retbuf` the caller handed (B2 unit 1's contract), and the body's last statement is
/// such an exit, so no path answers another store.  A callee whose buffer attribute was
/// promoted onto a local (`o = P { … }; …; o`) declines here exactly as unit 1 declines it:
/// the local IS the buffer, and a literal built beside it would share the record.
///
/// An exit has three SPELLINGS in the IR and all are read: `return { Object …; __retbuf }`
/// (the `Return` wraps the literal's block), `{ Object …; return __retbuf }` (the `Return`
/// is the block's last operator — the form a function with text work-refs takes, their
/// frees standing between the writes and the return), and for the body's last statement
/// alone the bare tail `{ Object …; __retbuf }` as the parser leaves it before the scope
/// pass wraps it (the preview's view).  A `Return` met anywhere else is an exit that answers
/// something other than the buffer.
fn callee_writes_buffer_at_every_exit(data: &Data, fn_nr: u32, buf_idx: usize) -> bool {
    let def = data.def(fn_nr);
    if def
        .attributes()
        .get(buf_idx)
        .is_none_or(|a| a.name != "__retbuf")
    {
        return false;
    }
    let buf_var = def.variables().var("__retbuf");
    if buf_var == u16::MAX || !def.variables().is_argument(buf_var) {
        return false;
    }
    let Value::Block(bl) = def.code().unspan() else {
        return false;
    };
    let Some(last) = bl.operators.last() else {
        return false;
    };
    let (mut exits, mut all_into_buffer) = (0usize, true);
    if tail_object_yielding(last, buf_var) {
        // The third spelling, the parser's tail before the scope pass wraps it in a
        // `Return`: the body's last statement IS the literal's block, yielding the buffer.
        exits += 1;
        for op in &bl.operators[..bl.operators.len() - 1] {
            check_exits(op, buf_var, &mut exits, &mut all_into_buffer);
        }
    } else {
        if !is_exit(last, buf_var) {
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
        check_exits(def.code(), buf_var, &mut exits, &mut all_into_buffer);
    }
    exits > 0 && all_into_buffer
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

/// Is `op` an exit that answers the buffer, in either spelling?
fn is_exit(op: &Value, buf_var: u16) -> bool {
    match op.unspan() {
        Value::Return(inner) => {
            crate::parser::Parser::tail_fresh_object_workref(inner) == Some(buf_var)
        }
        Value::Block(bl) => buffer_object_returning(bl, buf_var),
        _ => false,
    }
}

fn check_exits(node: &Value, buf_var: u16, exits: &mut usize, ok: &mut bool) {
    match node.unspan() {
        Value::Return(inner) => {
            *exits += 1;
            if crate::parser::Parser::tail_fresh_object_workref(inner) != Some(buf_var) {
                *ok = false;
            }
        }
        Value::Block(bl) if buffer_object_returning(bl, buf_var) => {
            *exits += 1;
            let n = bl.operators.len();
            for op in &bl.operators[..n - 1] {
                check_exits(op, buf_var, exits, ok);
            }
        }
        n => n.for_each_child(&mut |c| check_exits(c, buf_var, exits, ok)),
    }
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
