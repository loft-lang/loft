// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — shared by the native Rust and the C99 generators

//! `@FR-R-AppendTwin` — `X += f(args)` built in X.
//!
//! A function returning a vector builds it in the buffer its caller hands it, after clearing
//! that buffer; a caller that APPENDS the result then copies it into its own vector.  The
//! append twin `f__ap(args, X)` is f's body with X as the buffer and none of its clears: the
//! body's appends land after X's own elements, and the copy is gone.  That is sound exactly
//! when the body never OBSERVES what the buffer held on entry, which [`eligible`] decides from
//! the IR: every mention of the buffer, or of a view of it, is a leading clear or mint, an
//! append into it, a call handing it to an eligible callee AS that callee's buffer, or the
//! value flowing out as the result.  Anything else — a length read, an index, a walk, the
//! buffer passed as an ordinary argument, a free — reads its content, and declines.
//!
//! The caller's half ([`call_site`]) asks that no argument reach X: a caller never hands a
//! buffer one of its other arguments reaches (loft#1895 closed the one rebind that did), so
//! X's root must be the frame's own buffer or a local, and no argument may name that root or
//! a local whose dependencies do.  Both compiled backends write the twin (the C as `f__ap`,
//! `c::body`); the interpreter copies as before and is the
//! reference.

use crate::data::{Data, Value};

fn trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_TRACE_APPEND_TWIN").is_some())
}
use crate::variables::Function;
use std::collections::{HashMap, HashSet};

/// The function's hidden vector return buffer, if it has one.
#[must_use]
pub fn buffer(data: &Data, d_nr: u32) -> Option<u16> {
    let def = data.def(d_nr);
    if !matches!(def.returned().base(), crate::data::Type::Vector(_, _)) {
        return None;
    }
    super::hoist::retbuf_var(data, d_nr)
}

pub use crate::opt::body_writes::views;

#[derive(Clone, Copy, PartialEq)]
enum Pos {
    /// The value is a statement's or a result's: a view here only flows on.
    Flow,
    /// The value is an operand: a view here is read.
    Operand,
}

struct Scan<'a> {
    data: &'a Data,
    vars: &'a Function,
    b: u16,
    /// The buffer's views: those whose dependencies reach it, and every alias the body binds
    /// of one (`w = OpRefAlias(B)`, a rebind witness) — its uses are the buffer's too.
    vb: HashSet<u16>,
    written: bool,
    memo: &'a mut HashMap<u32, bool>,
    /// The first node that declined, for `LOFT_TRACE_APPEND_TWIN`.
    why: Option<String>,
}

impl Scan<'_> {
    fn is_view(&self, v: &Value) -> bool {
        matches!(v.unspan(), Value::Var(x) if self.vb.contains(x))
    }

    fn name(&self, d: u32) -> &str {
        self.data.def(d).name()
    }

    /// Scan every argument as an operand: none may be a view of the buffer.
    fn operands(&mut self, args: &[Value]) -> bool {
        args.iter().all(|a| self.scan(a, Pos::Operand))
    }

    fn scan(&mut self, v: &Value, pos: Pos) -> bool {
        let ok = self.scan_node(v, pos);
        if !ok && self.why.is_none() {
            self.why = Some(match v.unspan() {
                Value::Call(d, _) => format!("{} (written={})", self.name(*d), self.written),
                Value::Var(x) => format!("a view read as an operand: {}", self.vars.name(*x)),
                other => format!("{:?}", std::mem::discriminant(other)),
            });
        }
        ok
    }

    fn scan_node(&mut self, v: &Value, pos: Pos) -> bool {
        match v.unspan() {
            Value::Var(x) => !self.vb.contains(x) || pos == Pos::Flow,
            Value::Set(x, rhs) => {
                if let Value::Call(d, a) = rhs.unspan()
                    && self.name(*d) == "OpRefAlias"
                    && a.first().is_some_and(|f| self.is_view(f))
                {
                    self.vb.insert(*x);
                    return self.operands(&a[1..]);
                }
                if self.vb.contains(x) {
                    // `view = OpGetField(B, …)` / `w = OpRefAlias(B)`: deriving a view.
                    if let Value::Call(d, a) = rhs.unspan()
                        && matches!(self.name(*d), "OpGetField" | "OpRefAlias")
                        && a.first().is_some_and(|f| self.is_view(f))
                    {
                        return self.operands(&a[1..]);
                    }
                    // A local typed as a view that is handed ANOTHER store — the return tail
                    // `__ret_tail_N = OpConstRef(…)` of `return NAMES` — carries that store
                    // out as the result, and a twin's caller never reads the result: the
                    // destination would miss the exit's value.
                    if !self.yields_buffer(rhs) {
                        if self.why.is_none() {
                            let shape: String =
                                format!("{:?}", rhs.unspan()).chars().take(120).collect();
                            self.why = Some(format!(
                                "a view handed another store: {} = {shape}",
                                self.vars.name(*x)
                            ));
                        }
                        return false;
                    }
                    self.scan(rhs, Pos::Flow)
                } else {
                    self.scan(rhs, Pos::Operand)
                }
            }
            Value::Call(d, args) => self.call(*d, args),
            Value::Block(bl) | Value::Loop(bl) => {
                let n = bl.operators.len();
                bl.operators
                    .iter()
                    .enumerate()
                    .all(|(i, op)| self.scan(op, if i + 1 == n { pos } else { Pos::Flow }))
            }
            Value::Insert(ops) => ops.iter().all(|op| self.scan(op, Pos::Flow)),
            Value::If(c, t, e) => {
                self.scan(c, Pos::Operand) && self.scan(t, pos) && self.scan(e, pos)
            }
            // Every exit answers the buffer: the twin's caller ignores the answer.
            Value::Return(r) => {
                if !self.yields_buffer(r) {
                    if self.why.is_none() {
                        let shape: String = format!("{:?}", r.unspan()).chars().take(80).collect();
                        self.why = Some(format!("an exit answering another store: {shape}"));
                    }
                    return false;
                }
                self.scan(r, Pos::Flow)
            }
            Value::Drop(r) => self.scan(r, Pos::Operand),
            Value::CallRef(..) | Value::Parallel(_) | Value::Yield(_) => false,
            other => {
                // Any other node is read as an operand: the walk below reaches every
                // variable it names, and a view among them is a read.
                let mut ok = true;
                other.walk(&mut |n| {
                    if let Value::Var(x) = n
                        && self.vb.contains(x)
                    {
                        ok = false;
                    }
                });
                ok
            }
        }
    }

    /// Does a value bound to a buffer view hold the buffer — a view, a null init, a block or
    /// both arms of an `if` that end in one, or a call answering the buffer it is handed?
    /// The mint around the buffer (`{ if … { B = OpDatabase(B) } else { clear } B }`) ends
    /// in B; a constant, a parameter or another store does not.
    fn yields_buffer(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Null => true,
            Value::Var(x) => *x == self.b || self.vb.contains(x),
            Value::Block(bl) => bl.operators.last().is_some_and(|l| self.yields_buffer(l)),
            Value::Insert(ops) => ops.last().is_some_and(|l| self.yields_buffer(l)),
            Value::If(_, t, e) => self.yields_buffer(t) && self.yields_buffer(e),
            // `return B` ending a block — the copy tail `{ …; return __retbuf }` a body with an
            // `if` gets — answers what it returns (loft#2116).
            Value::Return(r) => self.yields_buffer(r),
            // The null a match's fallthrough answers: appended, it adds nothing — as the twin's
            // exit adds nothing.
            Value::Call(d, args)
                if args.is_empty() && self.data.def(*d).name() == "OpNullRefSentinel" =>
            {
                true
            }
            Value::Call(d, args) => {
                let def = self.data.def(*d);
                let ok = matches!(def.name(), "OpGetField" | "OpRefAlias")
                    && args.first().is_some_and(|f| self.is_view(f))
                    || def.is_loft_defined()
                        && def
                            .hidden_return_buffer_attr()
                            .is_some_and(|at| args.get(at).is_some_and(|a| self.is_view(a)));
                if !ok && trace() {
                    eprintln!(
                        "append-twin:   not the buffer: {} -> {:?}",
                        def.name(),
                        def.returned()
                    );
                }
                ok
            }
            other => {
                if trace() {
                    eprintln!(
                        "append-twin:   not the buffer: {:?}",
                        std::mem::discriminant(other)
                    );
                }
                false
            }
        }
    }

    fn call(&mut self, d: u32, args: &[Value]) -> bool {
        let name = self.name(d).to_string();
        let first_is_view = args.first().is_some_and(|a| self.is_view(a));
        let first_is_b =
            matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == self.b);
        match name.as_str() {
            // The leading forms: before the body writes, clearing or minting the buffer
            // changes nothing a twin's caller can see — the twin skips them.
            "OpClearVector" if first_is_view => !self.written && args.len() == 1,
            "OpDatabase" if first_is_b => !self.written && self.operands(&args[1..]),
            "OpSetInt4" if first_is_b => {
                !self.written
                    && matches!(args.get(1).map(Value::unspan), Some(Value::Int(0)))
                    && matches!(args.get(2).map(Value::unspan), Some(Value::Int(0)))
            }
            // Appends into the buffer: the values may not read it.  `OpSliceVector` is the
            // `(R-ByteCopy)` append of a span of ANOTHER vector — its source a value too.
            // `OpInsertVector` is not among them: it inserts at an index counted from the
            // buffer's start, which in a twin holds the caller's elements.
            "OpAppendVector" | "OpAppendTextBytes" | "OpPreAllocVector" | "OpSliceVector"
                if first_is_view =>
            {
                self.written = true;
                self.operands(&args[1..])
            }
            n if n.starts_with("OpPush") && first_is_view => {
                self.written = true;
                self.operands(&args[1..])
            }
            // `OpReplaceVector(B, view)` — the self-replace a buffer's own view delivers.
            "OpReplaceVector" if first_is_b && args.get(1).is_some_and(|a| self.is_view(a)) => {
                self.operands(&args[2..])
            }
            _ => {
                let def = self.data.def(d);
                if def.is_loft_defined()
                    && let Some(at) = def.hidden_return_buffer_attr()
                    && args.get(at).is_some_and(|a| self.is_view(a))
                {
                    // The buffer handed on as the callee's own buffer.
                    self.written = true;
                    let others: Vec<&Value> = args
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != at)
                        .map(|(_, a)| a)
                        .collect();
                    return others.iter().all(|a| self.scan(a, Pos::Operand))
                        && eligible(self.data, d, self.memo);
                }
                self.operands(args)
            }
        }
    }
}

/// Does `d_nr` have an append twin?  A fixpoint over the call graph: a function in progress
/// counts as eligible, so a recursive builder (`encode`'s array arm) can call its own twin.
pub fn eligible(data: &Data, d_nr: u32, memo: &mut HashMap<u32, bool>) -> bool {
    if let Some(&known) = memo.get(&d_nr) {
        return known;
    }
    let def = data.def(d_nr);
    if !def.is_loft_defined() {
        return false;
    }
    let Some(b) = buffer(data, d_nr) else {
        memo.insert(d_nr, false);
        return false;
    };
    memo.insert(d_nr, true);
    let vars = def.variables();
    let mut scan = Scan {
        data,
        vars,
        b,
        vb: views(vars, b),
        written: false,
        memo,
        why: None,
    };
    // The body's tail is its last exit, unless it is a `return` the walk asks itself.
    let tail_ok = match def.code().unspan() {
        Value::Block(bl) => bl
            .operators
            .last()
            .is_none_or(|t| matches!(t.unspan(), Value::Return(_)) || scan.yields_buffer(t)),
        _ => false,
    };
    if !tail_ok {
        scan.why = Some("the body's tail answers another store".to_string());
    }
    let ok = tail_ok && scan.scan(def.code(), Pos::Flow);
    let why = scan.why.take();
    memo.insert(d_nr, ok);
    if trace() {
        if ok {
            eprintln!("append-twin: {} has a twin", def.name());
        } else {
            eprintln!(
                "append-twin: {} observes its buffer at {}",
                def.name(),
                why.as_deref().unwrap_or("?")
            );
        }
    }
    ok
}

/// `OpAppendVector(X, f(args…, buf), tp)` where `f` has an append twin: answers the callee,
/// its arguments, the position of its buffer argument and X.  X's root is the frame's own
/// return buffer or a local, and no argument names that root or a local whose dependencies
/// reach it.
pub struct Site<'a> {
    pub callee: u32,
    pub args: &'a [Value],
    pub at: usize,
    pub dest: &'a Value,
}

#[must_use]
pub fn call_site<'a>(
    stmt: &'a Value,
    data: &Data,
    d_nr: u32,
    memo: &mut HashMap<u32, bool>,
) -> Option<Site<'a>> {
    let Value::Call(op, a) = stmt.unspan() else {
        return None;
    };
    if data.def(*op).name() != "OpAppendVector" {
        return None;
    }
    let [dest, src, _] = &a[..] else { return None };
    let Value::Call(callee, args) = src.unspan() else {
        return None;
    };
    let at = data.def(*callee).hidden_return_buffer_attr()?;
    if !matches!(args.get(at).map(Value::unspan), Some(Value::Var(_))) {
        return None;
    }
    let decline = |why: &str| {
        if trace() {
            eprintln!(
                "append-twin: {} += {}(…) declined — {why}",
                data.def(d_nr).name(),
                data.def(*callee).name()
            );
        }
    };
    let Some((root, _)) = super::hoist::vector_path(data, dest) else {
        decline("the destination is not a vector path");
        return None;
    };
    let vars = data.def(d_nr).variables();
    let own = buffer(data, d_nr);
    // A WORK BUFFER the caller lends (`@FR-R-WorkBuffer`) is this frame's alone — each call
    // site lends its own — so no other argument can be a view of it, which is all the
    // parameter decline guards against.
    if vars.is_argument(root)
        && own.is_none_or(|b| !views(vars, b).contains(&root))
        && !super::hoist::work_buffer_arg(data, d_nr, root)
    {
        decline("the destination is a parameter");
        return None;
    }
    let reach: HashSet<u16> = views(vars, root);
    let clean = args.iter().enumerate().all(|(i, x)| {
        if i == at {
            return true;
        }
        let mut ok = true;
        x.walk(&mut |n| {
            if let Value::Var(v) = n
                && reach.contains(v)
            {
                ok = false;
            }
        });
        ok
    });
    if !clean {
        decline("an argument reaches the destination");
        return None;
    }
    if !eligible(data, *callee, memo) {
        decline("the callee observes its buffer");
        return None;
    }
    Some(Site {
        callee: *callee,
        args,
        at,
        dest,
    })
}

/// `LOFT_NO_APPEND_WINDOW=1` — every append twin finds its buffer through the store.
#[must_use]
pub fn window_enabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("LOFT_NO_APPEND_WINDOW").is_ok_and(|v| v != "0"))
}

/// `@FR-R-AppendTwin`'s WINDOW clause — the element size of `d_nr`'s append twin's buffer when
/// that twin can write through its caller's push window: it has an append twin ([`eligible`]),
/// and every WRITE into its buffer, or a view of it, is one a window takes — a scalar push, a
/// reservation, a text's bytes, a scalar vector's elements — or a call handing the buffer to a
/// callee whose own twin takes the window, all of one element size.  A slice, a replace or an
/// insert into the buffer declines, and so does a mixture of sizes.  `None` when it cannot.
///
/// A fixpoint over the call graph like [`eligible`]: a function in progress counts as taking
/// the window at any size, so a recursive builder takes its own.  Each question starts a fresh
/// memo, so an answer never rests on a callee a cycle later declined.
#[must_use]
pub fn window_size(data: &Data, stores: &crate::database::Stores, d_nr: u32) -> Option<u32> {
    if !window_enabled() {
        return None;
    }
    let mut memo = HashMap::new();
    window_of(data, stores, d_nr, &mut memo).filter(|s| *s != 0)
}

/// What a write into a window path adds to [`window_of`]'s answer: its element size, `0` for an
/// operation that writes nothing a window carries, `None` for one a window cannot take.
/// `self_replace` is whether the second operand is a view of the buffer too.
fn window_write(
    data: &Data,
    stores: &crate::database::Stores,
    name: &str,
    args: &[Value],
    self_replace: bool,
) -> Option<u32> {
    let size_of = |at: usize| match args.get(at).map(Value::unspan) {
        Some(Value::Int(tp)) => u16::try_from(*tp)
            .ok()
            .filter(|tp| scalar_elem(stores, *tp))
            .map(|tp| u32::from(stores.size(tp))),
        _ => None,
    };
    match name {
        "OpAppendTextBytes" => Some(1),
        "OpPreAllocVector" => match args.get(2).map(Value::unspan) {
            Some(Value::Int(s)) if *s > 0 => u32::try_from(*s).ok(),
            _ => None,
        },
        "OpAppendVector" => size_of(2),
        "OpSliceVector" => size_of(4),
        // The self-replace a buffer's own view delivers changes nothing.
        "OpReplaceVector" if self_replace => Some(0),
        "OpReplaceVector" | "OpInsertVector" => None,
        n if n.starts_with("OpPush") => {
            crate::opt::writes::fused_push(data, n, args).map(|fp| fp.size)
        }
        _ => Some(0),
    }
}

/// A scalar element type — what a window copies as bytes: the base numbers and booleans (not
/// text, type 5) and the narrow integer kinds, as the C writer's slice asks.
fn scalar_elem(stores: &crate::database::Stores, tp: u16) -> bool {
    (tp <= 6 && tp != 5)
        || matches!(
            stores.types.get(tp as usize).map(|t| &t.parts),
            Some(
                crate::database::Parts::Byte(..)
                    | crate::database::Parts::Short(..)
                    | crate::database::Parts::ShortRaw(..)
                    | crate::database::Parts::Int(..)
                    | crate::database::Parts::IntRaw(..)
            )
        )
}

/// [`window_size`]'s walk; `Some(0)` is a function in progress, whose size its writes decide.
fn window_of(
    data: &Data,
    stores: &crate::database::Stores,
    d_nr: u32,
    memo: &mut HashMap<u32, Option<u32>>,
) -> Option<u32> {
    if let Some(known) = memo.get(&d_nr) {
        return *known;
    }
    let mut em = HashMap::new();
    if !eligible(data, d_nr, &mut em) {
        memo.insert(d_nr, None);
        return None;
    }
    let b = buffer(data, d_nr)?;
    memo.insert(d_nr, Some(0));
    let def = data.def(d_nr);
    let vars = def.variables();
    let mut vb = views(vars, b);
    vb.insert(b);
    // Views the body binds itself (`v = OpGetField(B, …)`, `w = OpRefAlias(B)`).
    def.code().walk(&mut |n| {
        if let Value::Set(x, rhs) = n
            && let Value::Call(op, a) = rhs.unspan()
            && matches!(data.def(*op).name(), "OpGetField" | "OpRefAlias")
            && a.first()
                .is_some_and(|f| matches!(f.unspan(), Value::Var(y) if vb.contains(y)))
        {
            vb.insert(*x);
        }
    });
    let on_view =
        |v: Option<&Value>| matches!(v.map(Value::unspan), Some(Value::Var(x)) if vb.contains(x));
    let mut size = 0u32;
    let mut ok = true;
    let mut why = String::new();
    let take = |s: u32, size: &mut u32, ok: &mut bool| {
        if s == 0 {
            return;
        }
        if *size == 0 {
            *size = s;
        } else if *size != s {
            *ok = false;
        }
    };
    let mut callees = Vec::new();
    def.code().walk(&mut |n| {
        let Value::Call(op, args) = n else { return };
        let name = data.def(*op).name();
        let was = ok;
        if on_view(args.first()) {
            match window_write(data, stores, name, args, on_view(args.get(1))) {
                Some(s) => take(s, &mut size, &mut ok),
                None => ok = false,
            }
        }
        if was && !ok && why.is_empty() {
            why = format!("{name} (size {size})");
        }
        let callee = data.def(*op);
        if callee.is_loft_defined()
            && let Some(at) = callee.hidden_return_buffer_attr()
            && on_view(args.get(at))
        {
            callees.push(*op);
        }
    });
    for c in callees {
        let was = ok;
        match window_of(data, stores, c, memo) {
            Some(s) => take(s, &mut size, &mut ok),
            None => ok = false,
        }
        if was && !ok && why.is_empty() {
            why = format!("the callee {}", data.def(c).name());
        }
    }
    let answer = (ok && size != 0).then_some(size);
    memo.insert(d_nr, answer);
    if trace() {
        eprintln!(
            "append-twin: {} {}",
            def.name(),
            match answer {
                Some(s) => format!("takes its caller's window ({s}-byte elements)"),
                None => format!("keeps the store form (no window): {why}"),
            }
        );
    }
    answer
}
