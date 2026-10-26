// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-TextBorrow`'s store-read clause — the text variables that BORROW a text read out of
//! a parameter's store instead of copying it.  Here, in `opt`, because both compiled writers
//! ask it (the Rust writer and the C backend); moved from `generation::hoist`, which
//! re-exports it.

use crate::data::{Data, Type, Value};
use crate::ir_shape::{as_var, call_named};
use std::collections::{HashMap, HashSet};

use crate::opt::attrs::retbuf_var;
use crate::opt::loop_records::{IN_PLACE_SET_OPS, is_scalar};
use crate::opt::text_borrow::{TextBorrow, text_escapes_with};
use crate::opt::writes::{
    call_writes_store, in_place_only_writer, native_op_is_store_free, text_stack_ref,
};

thread_local! {
    static BUFFER_WRITERS_OPEN: std::cell::RefCell<HashSet<u32>> =
        std::cell::RefCell::new(HashSet::new());
}

/// What [`borrowed_store_texts`] admits: the discharge temps and the statement-bound text
/// locals that borrow a text read out of a PARAMETER's store.
#[derive(Default)]
pub struct StoreTextBorrows {
    /// Every admitted variable, temps and locals alike, as [`Output::text_borrowed`] reads
    /// them.
    pub borrows: HashMap<u16, TextBorrow>,
    /// The admitted LOCALS: bound by a statement of the body, so every `Set` of one binds a
    /// `&str` slot (`Output::output_set`).
    pub locals: HashSet<u16>,
    /// `@FR-R-SliceBorrow` — the admitted locals with a value slice among their bindings,
    /// counted apart for the rewrite census.
    pub slices: HashSet<u16>,
    /// The tested clause — admitted locals whose scope also writes the hidden RETURN BUFFER,
    /// each with the parameters its reads are rooted in: the borrow holds only while each of
    /// those parameters' stores is not the buffer's, which the bind tests at run time and
    /// copies otherwise.
    pub tested: HashMap<u16, Vec<u16>>,
    /// The admitted locals that borrow only the frame's own work texts (a format's, a call's)
    /// or literals: no store holds their bytes, so a store write may read them in place.
    pub work: HashSet<u16>,
}

/// `@FR-R-TextBorrow`'s store-read clause — the text variables of this function that BORROW
/// a text read out of a parameter's store instead of copying it:
///
/// * a `?` / `??` discharge temp (`__ncc_N`) bound to `OpGetText(R, off)` where `R` is a
///   parameter reached through field and element reads (`words[i]?`, `p.names[i] ?? ""`),
///   every mention of it a text-value read;
/// * a text LOCAL whose every binding is a text literal, such a read, a text parameter, or
///   another variable this clause admits, and every other mention a text-value read.
///
/// The slice points into the parameter's store, and it stays valid while that store is
/// neither grown, freed nor has the text's block released.  A parameter's store was live
/// when this frame began, so it is none of the stores the frame mints (`(H-Alloc)`: a fresh
/// store is distinct from every live one) — which lets the condition be read off the
/// operands: over the innermost statement block that holds every mention of the borrow,
/// every store writer must name only stores this frame minted ([`frame_minted_locals`]), or
/// be a scalar set in place, which moves no block.  A parameter the body rebinds, a user
/// callee that writes, a fn-ref call, a parallel arm and a generator decline.
#[must_use]
#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn borrowed_store_texts(
    data: &Data,
    def_nr: u32,
    already: &HashMap<u16, TextBorrow>,
) -> StoreTextBorrows {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    let trace = std::env::var("LOFT_TRACE_TEXT_BORROW").is_ok();
    let mut out = StoreTextBorrows::default();
    if body.any_node(&mut |n| matches!(n, Value::Yield(_))) {
        return out;
    }
    let mut rebound: HashSet<u16> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Set(v, _) = n {
            rebound.insert(*v);
        }
        false
    });
    let param_read = |to: &Value| {
        call_named(to, data, "OpGetText")
            .and_then(|a| a.first())
            .is_some_and(|r| param_rooted(r, data, vars, &rebound))
    };
    let text_local = |v: u16| {
        !vars.is_argument(v)
            && !already.contains_key(&v)
            && matches!(vars.tp(v).base(), Type::Text(_))
    };
    // `@FR-R-SliceBorrow` — a value slice of a plain text PARAMETER (`&str` of the
    // caller's, never `&text`, never rebound) or of an admitted local is one more source a
    // local may borrow: the `&str` the slice answers names the bytes its source does, so
    // nothing this frame can do moves them.  A slice of an OWNED local declines here (its
    // buffer can grow under the borrow; that clause needs the path condition).
    // `LOFT_NO_SLICE_BORROW=1` keeps the copy for this clause alone.
    let slice_disabled = std::env::var("LOFT_NO_SLICE_BORROW").is_ok_and(|v| v != "0");
    let plain_text_arg = |s: u16| {
        vars.is_argument(s) && matches!(vars.tp(s).base(), Type::Text(_)) && !rebound.contains(&s)
    };
    let slice_of =
        |to: &Value| call_named(to, data, "OpGetTextSub").and_then(|a| as_var(a.first()));
    // The discharge temps: one bind, to a parameter's text.
    let mut temps: HashSet<u16> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Set(v, to) = n
            && vars.name(*v).starts_with("__ncc_")
            && text_local(*v)
            && param_read(to)
        {
            temps.insert(*v);
        }
        false
    });
    temps.retain(|t| {
        let mut binds = 0u32;
        body.any_node(&mut |m| {
            if matches!(m, Value::Set(x, to) if x == t && !matches!(to.unspan(), Value::Null)) {
                binds += 1;
            }
            false
        });
        binds == 1
    });
    // The locals: the greatest set whose every binding is a borrowable source.
    let mut locals: HashSet<u16> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Set(v, _) = n
            && !vars.name(*v).starts_with("__")
            && text_local(*v)
        {
            locals.insert(*v);
        }
        false
    });
    let mut why: HashMap<u16, &'static str> = HashMap::new();
    // The format clause: a local bound to a format string borrows the format's work text
    // instead of copying it, when that work text is the format's own — named nowhere else
    // in the body — so only the bind's next run rewrites it.  The format must not read the
    // local it binds (its old value would still borrow the text being rewritten) and no
    // other variable may be bound from the local (the copy would outlive that rewrite).
    let fmt_own = |to: &Value| {
        own_format_work(body, to, data).is_some() || own_call_work(body, to, data).is_some()
    };
    let mut fmt_bound: HashSet<u16> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Set(v, to) = n
            && locals.contains(v)
            && fmt_own(to)
        {
            fmt_bound.insert(*v);
        }
        false
    });
    let mut fmt_refused: Vec<(u16, &'static str)> = Vec::new();
    for &v in &fmt_bound {
        let mut self_read = false;
        let mut copied = false;
        body.any_node(&mut |n| {
            match n {
                Value::Set(x, to) if *x == v && fmt_own(to) => {
                    if to.any_node(&mut |m| matches!(m, Value::Var(y) if *y == v)) {
                        self_read = true;
                    }
                }
                Value::Set(x, to) if *x != v && matches!(to.unspan(), Value::Var(y) if *y == v) => {
                    copied = true;
                }
                _ => {}
            }
            false
        });
        if self_read {
            fmt_refused.push((v, "a format binding it reads itself"));
        } else if copied {
            fmt_refused.push((v, "bound to a format and copied into another variable"));
        }
    }
    for (v, w) in fmt_refused {
        fmt_bound.remove(&v);
        locals.remove(&v);
        why.insert(v, w);
    }
    loop {
        let admitted: HashSet<u16> = temps.union(&locals).copied().collect();
        let source_ok = |to: &Value| match to.unspan() {
            Value::Text(_) => true,
            Value::Block(_) => fmt_own(to),
            Value::Call(..) if fmt_own(to) => true,
            Value::Var(s) => {
                admitted.contains(s)
                    || (vars.is_argument(*s) && matches!(vars.tp(*s).base(), Type::Text(_)))
            }
            _ => {
                param_read(to)
                    || (!slice_disabled
                        && slice_of(to).is_some_and(|s| plain_text_arg(s) || admitted.contains(&s)))
            }
        };
        let temp_ok = |to: &Value| param_read(to);
        let mut drop: Vec<(u16, &'static str)> = Vec::new();
        for &v in &admitted {
            let escape = if temps.contains(&v) {
                text_escapes_with(body, v, data, &temp_ok)
            } else {
                text_escapes_with(body, v, data, &source_ok)
            };
            if let Some(w) = escape {
                drop.push((v, w));
            }
        }
        if drop.is_empty() {
            break;
        }
        for (v, w) in drop {
            temps.remove(&v);
            locals.remove(&v);
            why.insert(v, w);
        }
    }
    // The store condition, over each borrow's scope.
    let minted = frame_minted_locals(data, vars, body, &[]);
    let minted_rb = (tested_borrow_enabled())
        .then(|| retbuf_var(data, def_nr))
        .flatten()
        .map(|rb| frame_minted_locals(data, vars, body, &[rb]));
    let mut cache: HashMap<u32, bool> = HashMap::new();
    let mut active: HashSet<u32> = HashSet::new();
    for v in temps
        .iter()
        .chain(locals.iter())
        .copied()
        .collect::<Vec<_>>()
    {
        // A borrow of the frame's own work text names no store: no store write can move it.
        if fmt_bound.contains(&v)
            && text_bound_only_by(body, v, &|to: &Value| {
                matches!(to.unspan(), Value::Text(_)) || fmt_own(to)
            })
        {
            out.work.insert(v);
            continue;
        }
        // `@FR-R-SliceBorrow` — a slice of a plain parameter names no store either: its
        // bytes are the caller's `&str`, which no store write of this frame can move.
        if !slice_disabled
            && text_bound_only_by(body, v, &|to: &Value| {
                matches!(to.unspan(), Value::Text(_)) || slice_of(to).is_some_and(plain_text_arg)
            })
        {
            continue;
        }
        let scope = mention_scope(body, v);
        if let Some(w) = foreign_store_writer(scope, data, &minted, &mut cache, &mut active, None) {
            // The tested clause: a local whose only other writers name the return buffer
            // (or a callee's buffer it is handed) borrows while its parameters' stores are
            // not the buffer's — a run-time test at each bind, a copy where it fails.
            if let Some(m) = minted_rb.as_ref()
                && locals.contains(&v)
            {
                let narrow = tested_scope(body, v, data);
                let tested = match foreign_store_writer(
                    narrow,
                    data,
                    m,
                    &mut cache,
                    &mut active,
                    Some(vars),
                ) {
                    Some(w2) => Err(w2),
                    None => param_roots_of(body, v, data, vars)
                        .ok_or("a binding not rooted in a parameter"),
                };
                match tested {
                    Ok(roots) => {
                        out.tested.insert(v, roots);
                        continue;
                    }
                    Err(w2) if trace => eprintln!(
                        "[text-borrow] {}: `{}` not tested either — {w2} (over {})",
                        def.name(),
                        vars.name(v),
                        match narrow {
                            Value::Block(b) => b.name,
                            _ => "the body",
                        }
                    ),
                    Err(_) => {}
                }
            }
            temps.remove(&v);
            locals.remove(&v);
            why.insert(v, w);
        }
    }
    // A local whose source was just refused has lost its borrow's source: repeat the
    // binding test until nothing moves.  A TESTED local is no source: its bind may answer
    // its own copy, which the next bind overwrites.
    loop {
        let admitted: HashSet<u16> = temps
            .union(&locals)
            .copied()
            .filter(|v| !out.tested.contains_key(v))
            .collect();
        let source_ok = |to: &Value| match to.unspan() {
            Value::Text(_) => true,
            Value::Block(_) => fmt_own(to),
            Value::Call(..) if fmt_own(to) => true,
            Value::Var(s) => {
                admitted.contains(s)
                    || (vars.is_argument(*s) && matches!(vars.tp(*s).base(), Type::Text(_)))
            }
            _ => {
                param_read(to)
                    || (!slice_disabled
                        && slice_of(to).is_some_and(|s| plain_text_arg(s) || admitted.contains(&s)))
            }
        };
        let lost: Vec<u16> = locals
            .iter()
            .copied()
            .filter(|&v| text_escapes_with(body, v, data, &source_ok).is_some())
            .collect();
        if lost.is_empty() {
            break;
        }
        for v in lost {
            locals.remove(&v);
            why.insert(v, "bound from a text that does not borrow");
        }
    }
    out.tested.retain(|v, _| locals.contains(v));
    out.work.retain(|v| locals.contains(v) || temps.contains(v));
    // `@FR-R-SliceBorrow` — the admitted locals with a slice among their bindings, for the
    // census and the trace.
    let slices: HashSet<u16> = locals
        .iter()
        .copied()
        .filter(|&v| {
            body.any_node(
                &mut |n| matches!(n, Value::Set(x, to) if *x == v && slice_of(to).is_some()),
            )
        })
        .collect();
    if trace {
        for v in temps.iter().chain(locals.iter()) {
            let what = if slices.contains(v) {
                "a slice of a parameter's text"
            } else if out.tested.contains_key(v) {
                "a parameter's text while its store is not the return buffer's"
            } else if fmt_bound.contains(v) {
                "a format's or a call's work text"
            } else {
                "a parameter's text"
            };
            eprintln!(
                "[text-borrow] {}: `{}` borrows {what}",
                def.name(),
                vars.name(*v)
            );
        }
        for (v, w) in &why {
            eprintln!(
                "[text-borrow] {}: `{}` declined — {w}",
                def.name(),
                vars.name(*v)
            );
        }
    }
    for v in temps.iter().chain(locals.iter()) {
        out.borrows.insert(*v, TextBorrow { read: None });
    }
    out.locals = locals;
    out.slices = slices;
    out
}

/// `@FR-R-TextBorrow`'s format clause — the work text of the format string `to` when that
/// text is the format's OWN: `to` is a `Formatted string` block (its work text set first and
/// answered last), and nothing else in `body` names the work text, so only another run of
/// this block rewrites it.  The frame's own bookkeeping of the work text is no other use:
/// its declaration (a text literal) and its `OpFreeText` as statements of the function's
/// outermost block, which run before the first and after the last statement that could
/// bind or read a borrow of it.
fn own_format_work(body: &Value, to: &Value, data: &Data) -> Option<u16> {
    let Value::Block(b) = to.unspan() else {
        return None;
    };
    if b.name != "Formatted string" {
        return None;
    }
    let work = match b.operators.first().map(Value::unspan) {
        Some(Value::Set(w, _)) => *w,
        _ => return None,
    };
    if !matches!(b.operators.last().map(Value::unspan), Some(Value::Var(w)) if *w == work) {
        return None;
    }
    let mentions = |node: &Value| {
        let mut c = 0usize;
        node.any_node(&mut |n| {
            if matches!(n, Value::Var(x) | Value::Set(x, _) if *x == work) {
                c += 1;
            }
            false
        });
        c
    };
    let bookkeeping = match body.unspan() {
        Value::Block(top) => top
            .operators
            .iter()
            .filter(|st| match st.unspan() {
                Value::Set(w, init) => *w == work && matches!(init.unspan(), Value::Text(_)),
                Value::Call(d, args) => {
                    (*d as usize) < data.definitions.len()
                        && data.def(*d).name() == "OpFreeText"
                        && matches!(args.as_slice(), [a] if matches!(a.unspan(), Value::Var(w) if *w == work))
                }
                _ => false,
            })
            .count(),
        _ => 0,
    };
    (mentions(body) == mentions(to) + bookkeeping).then_some(work)
}

/// `@FR-R-TextBorrow`'s call clause — the work text of the call `to` when the call's result
/// IS that work text and the text is the call's OWN: `to` calls a user function with one
/// hidden `&text` work buffer whose every return (and tail) is that buffer, the caller hands
/// it a `default ref` block over its own work local, and nothing else in `body` names that
/// local but its declaration and release (the format clause's bookkeeping).  Only another
/// run of this call rewrites the text, so a local bound to it may borrow it as the format
/// clause's local borrows its format.  `LOFT_NO_CALL_TEXT_BORROW=1` turns the clause off.
fn own_call_work(body: &Value, to: &Value, data: &Data) -> Option<u16> {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| !std::env::var("LOFT_NO_CALL_TEXT_BORROW").is_ok_and(|v| v != "0")) {
        return None;
    }
    let Value::Call(d, args) = to.unspan() else {
        return None;
    };
    if (*d as usize) >= data.definitions.len() {
        return None;
    }
    let callee = data.def(*d);
    if matches!(callee.code(), Value::Null)
        || !matches!(callee.returned().base(), Type::Text(_))
        || callee.text_work_buffers() != 1
    {
        return None;
    }
    let wi = callee.attributes().iter().position(|a| {
        a.hidden && matches!(a.typedef.base(), Type::RefVar(t) if matches!(t.base(), Type::Text(_)))
    })?;
    let out = callee.variables().var(&callee.attributes()[wi].name);
    if out == u16::MAX {
        return None;
    }
    // Every return of the callee answers its work buffer.
    let is_out = |v: &Value| matches!(v.unspan(), Value::Var(x) if *x == out);
    if callee
        .code()
        .any_node(&mut |n| matches!(n, Value::Return(r) if !is_out(r)))
    {
        return None;
    }
    if let Value::Block(top) = callee.code().unspan()
        && let Some(tail) = top.operators.last()
        && !matches!(tail.unspan(), Value::Return(_))
        && !is_out(tail)
    {
        return None;
    }
    // The caller's work local, handed through a `default ref` block.
    let Value::Block(b) = args.get(wi)?.unspan() else {
        return None;
    };
    if b.name != "default ref" {
        return None;
    }
    let work = match b.operators.first().map(Value::unspan) {
        Some(Value::Set(w, init)) if matches!(init.unspan(), Value::Text(_)) => *w,
        _ => return None,
    };
    let mentions = |node: &Value| {
        let mut c = 0usize;
        node.any_node(&mut |n| {
            if matches!(n, Value::Var(x) | Value::Set(x, _) if *x == work) {
                c += 1;
            }
            false
        });
        c
    };
    // The declaration among the outermost statements, and EVERY release of the work local:
    // an early exit releases it before its `return`, and a release emits no code on native
    // (the work `String` lives to the frame's end), so none ends the borrow's text early.
    let declared = match body.unspan() {
        Value::Block(top) => top
            .operators
            .iter()
            .filter(|st| {
                matches!(st.unspan(), Value::Set(w, init)
                    if *w == work && matches!(init.unspan(), Value::Text(_)))
            })
            .count(),
        _ => 0,
    };
    let mut released = 0usize;
    body.any_node(&mut |n| {
        if let Value::Call(f, a) = n
            && (*f as usize) < data.definitions.len()
            && data.def(*f).name() == "OpFreeText"
            && matches!(a.as_slice(), [x] if matches!(x.unspan(), Value::Var(w) if *w == work))
        {
            released += 1;
        }
        false
    });
    let bookkeeping = declared + released;
    (mentions(body) == mentions(to) + bookkeeping).then_some(work)
}

/// Is every binding of `v` in `body` one that `ok` accepts (a null declaration aside)?
fn text_bound_only_by(body: &Value, v: u16, ok: &dyn Fn(&Value) -> bool) -> bool {
    !body.any_node(&mut |n| {
        matches!(n, Value::Set(x, to) if *x == v && !matches!(to.unspan(), Value::Null) && !ok(to))
    })
}

/// Is `r` a PARAMETER reached through field and element reads — a place in a store that was
/// live when the frame began?  The parameter must be a plain one the body never rebinds.
fn param_rooted(
    r: &Value,
    data: &Data,
    vars: &crate::variables::Function,
    rebound: &HashSet<u16>,
) -> bool {
    match r.unspan() {
        Value::Var(a) => {
            vars.is_argument(*a)
                && !matches!(vars.tp(*a).base(), Type::RefVar(_))
                && !rebound.contains(a)
        }
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && matches!(
                    data.def(*d).name(),
                    "OpGetField" | "OpGetVector" | "OpGetVectorNullable"
                ) =>
        {
            args.first()
                .is_some_and(|a| param_rooted(a, data, vars, rebound))
        }
        _ => false,
    }
}

/// Can a value of `tp` name a store?  The reference, the struct-enum and every collection.
fn names_a_store(tp: &Type) -> bool {
    matches!(
        tp.base(),
        Type::Reference(..)
            | Type::Enum(_, true, _)
            | Type::Vector(..)
            | Type::Sorted(..)
            | Type::Index(..)
            | Type::Radix(..)
            | Type::Trie(..)
            | Type::Hash(..)
    )
}

/// The locals of a body whose every value lies in a store the FRAME minted: the greatest set
/// of non-parameter store locals whose every binding is null, another such local, or a
/// field, element or record read through one — with `OpDatabase(v, tp)`, which gives `v` a
/// store of its own, the one slot write admitted.  A local handed to anything else by its
/// slot (a link, a `&` parameter) is out: what that does to it is not this body's to read.
pub(crate) fn frame_minted_locals(
    data: &Data,
    vars: &crate::variables::Function,
    body: &Value,
    seeds: &[u16],
) -> HashSet<u16> {
    let mut set: HashSet<u16> = (0..vars.count())
        .filter(|&v| (!vars.is_argument(v) || seeds.contains(&v)) && names_a_store(vars.tp(v)))
        .collect();
    body.any_node(&mut |n| {
        if let Value::Call(d, args) = n
            && (*d as usize) < data.definitions.len()
        {
            let def = data.def(*d);
            let slot_write = matches!(def.name(), "OpDatabase" | "OpDatabaseNP");
            // A callee's hidden RETURN BUFFER is handed by slot so the callee can build into
            // it; the store it builds in is that buffer's own (a fresh one from a null slot),
            // so the local stays minted.  What the callee writes elsewhere is the writer
            // test's question (`callee_writes_only_its_buffer`).
            let buffer_at = (!matches!(def.code(), Value::Null))
                .then(|| def.hidden_return_buffer_attr())
                .flatten();
            for (i, a) in args.iter().enumerate() {
                if let Value::Var(x) = a.unspan() {
                    let by_slot = Some(i) != buffer_at
                        && def.attributes().get(i).is_some_and(|at| {
                            (at.constant && !(slot_write && i == 0))
                                || matches!(at.typedef.base(), Type::RefVar(_))
                        });
                    if by_slot {
                        set.remove(x);
                    }
                }
            }
        }
        false
    });
    loop {
        let mut out: Vec<u16> = Vec::new();
        body.any_node(&mut |n| {
            if let Value::Set(v, to) = n
                && set.contains(v)
                && !matches!(to.unspan(), Value::Null)
                && !minted_expr(to, data, &set)
            {
                out.push(*v);
            }
            false
        });
        if out.is_empty() {
            return set;
        }
        for v in out {
            set.remove(&v);
        }
    }
}

/// Is `e` a place in a store one of `minted`'s locals holds?
fn minted_expr(e: &Value, data: &Data, minted: &HashSet<u16>) -> bool {
    match e.unspan() {
        Value::Var(v) => minted.contains(v),
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && matches!(
                    data.def(*d).name(),
                    "OpGetField"
                        | "OpGetVector"
                        | "OpGetVectorNullable"
                        | "OpGetRecord"
                        | "OpNewRecord"
                ) =>
        {
            args.first().is_some_and(|a| minted_expr(a, data, minted))
        }
        // A user call that builds into its hidden return buffer answers that buffer: the
        // value is in whatever store the buffer argument names.
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && !matches!(data.def(*d).code(), Value::Null) =>
        {
            data.def(*d)
                .hidden_return_buffer_attr()
                .and_then(|i| args.get(i))
                .is_some_and(|a| minted_expr(a, data, minted))
        }
        _ => false,
    }
}

/// The tested clause's scope of text local `v`: the innermost block holding every mention of
/// `v` but its LITERAL binds, whose result is no text — the body when none is.  A literal
/// bind holds static bytes no write can move, so the borrow lives from a store-read bind to
/// the uses after it; and a block whose value is not a text cannot hand the borrow out (a
/// text leaving otherwise is an escape `text_escapes` refuses).
fn tested_scope<'a>(body: &'a Value, v: u16, data: &Data) -> &'a Value {
    // A literal bind, the null text's bind and the text's release (`OpFreeText`, no code for
    // a borrow) are not mentions the borrow lives across.
    fn count(node: &Value, v: u16, data: &Data) -> usize {
        let mut c = 0usize;
        let mut skip = 0usize;
        node.any_node(&mut |n| {
            match n {
                Value::Set(x, to)
                    if *x == v
                        && (matches!(to.unspan(), Value::Text(_) | Value::Null)
                            || call_named(to, data, "OpConvTextFromNull").is_some()) =>
                {
                    skip += 1;
                }
                Value::Call(_, args)
                    if call_named(n, data, "OpFreeText").is_some()
                        && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == v) =>
                {
                    skip += 1;
                }
                Value::Var(x) | Value::Set(x, _) if *x == v => c += 1,
                _ => {}
            }
            false
        });
        // A skipped release's operand is a `Var` the walk counted on its own.
        c.saturating_sub(skip)
    }
    fn size(node: &Value) -> usize {
        let mut s = 0usize;
        node.any_node(&mut |_| {
            s += 1;
            false
        });
        s
    }
    fn walk<'a>(
        node: &'a Value,
        v: u16,
        data: &Data,
        total: usize,
        best: &mut Option<(&'a Value, usize)>,
    ) {
        if let Value::Block(b) = node
            && !matches!(b.result.base(), Type::Text(_))
            && count(node, v, data) == total
        {
            let s = size(node);
            if best.is_none_or(|(_, bs)| s < bs) {
                *best = Some((node, s));
            }
        }
        node.for_each_child(&mut |c| walk(c, v, data, total, best));
    }
    let total = count(body, v, data);
    let mut best: Option<(&Value, usize)> = None;
    walk(body, v, data, total, &mut best);
    best.map_or(body, |(b, _)| b)
}

/// The innermost statement block (a `void` block) holding every mention of `v` — the body
/// itself when none does.  A value a statement block's statements compute does not outlive
/// the block unless it is bound to a variable, and a bound borrow is a variable of its own.
fn mention_scope(body: &Value, v: u16) -> &Value {
    let count = |node: &Value| {
        let mut c = 0usize;
        node.any_node(&mut |n| {
            if matches!(n, Value::Var(x) | Value::Set(x, _) if *x == v) {
                c += 1;
            }
            false
        });
        c
    };
    let total = count(body);
    let mut best: Option<(&Value, usize)> = None;
    let size = |node: &Value| {
        let mut s = 0usize;
        node.any_node(&mut |_| {
            s += 1;
            false
        });
        s
    };
    let mut blocks: Vec<&Value> = Vec::new();
    collect_void_blocks(body, &mut blocks);
    for b in blocks {
        if count(b) == total {
            let s = size(b);
            if best.is_none_or(|(_, bs)| s < bs) {
                best = Some((b, s));
            }
        }
    }
    best.map_or(body, |(b, _)| b)
}

fn collect_void_blocks<'a>(node: &'a Value, out: &mut Vec<&'a Value>) {
    match node {
        Value::Block(b) => {
            if matches!(b.result.base(), Type::Void) {
                out.push(node);
            }
            for o in &b.operators {
                collect_void_blocks(o, out);
            }
        }
        Value::Loop(b) => {
            for o in &b.operators {
                collect_void_blocks(o, out);
            }
        }
        Value::Span(inner) => collect_void_blocks(&inner.1, out),
        Value::Set(_, to) => collect_void_blocks(to, out),
        Value::Call(_, args) => {
            for a in args {
                collect_void_blocks(a, out);
            }
        }
        Value::If(c, t, e) => {
            collect_void_blocks(c, out);
            collect_void_blocks(t, out);
            collect_void_blocks(e, out);
        }
        Value::Insert(ops) => {
            for o in ops {
                collect_void_blocks(o, out);
            }
        }
        Value::Drop(inner) | Value::Return(inner) => collect_void_blocks(inner, out),
        _ => {}
    }
}

/// `LOFT_NO_TESTED_BORROW=1` — `@FR-R-TextBorrow`'s tested clause off: a text local whose
/// scope writes the return buffer copies again.
fn tested_borrow_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("LOFT_NO_TESTED_BORROW").is_ok_and(|v| v != "0"))
}

/// The parameters text local `v`'s reads are rooted in — every binding a text literal, a
/// plain text parameter (the caller's `&str`, no store) or `OpGetText(R, f)` with `R` a
/// projection of a parameter — or `None` when a binding is anything else.
fn param_roots_of(
    body: &Value,
    v: u16,
    data: &Data,
    vars: &crate::variables::Function,
) -> Option<Vec<u16>> {
    fn root(r: &Value, data: &Data, vars: &crate::variables::Function) -> Option<u16> {
        match r.unspan() {
            Value::Var(p) if vars.is_argument(*p) => Some(*p),
            Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                root(args.first()?, data, vars)
            }
            _ => None,
        }
    }
    let mut roots: Vec<u16> = Vec::new();
    let mut ok = true;
    body.any_node(&mut |n| {
        if let Value::Set(x, to) = n
            && *x == v
        {
            match to.unspan() {
                Value::Text(_) | Value::Null => {}
                Value::Var(s) if vars.is_argument(*s) => {}
                _ => match call_named(to, data, "OpGetText")
                    .and_then(|a| a.first())
                    .and_then(|r| root(r, data, vars))
                {
                    Some(p) if !roots.contains(&p) => roots.push(p),
                    Some(_) => {}
                    None => ok = false,
                },
            }
        }
        false
    });
    (ok && !roots.is_empty()).then_some(roots)
}

/// The tested clause's callee — does user function `d` write no store but the hidden return
/// buffer it is handed, and is that buffer, at this call, a store in `minted`?  Its own body
/// answers by [`foreign_store_writer`] with its buffer seeded as minted; a call back into a
/// body under check answers yes.
fn callee_writes_only_its_buffer(
    d: u32,
    args: &[Value],
    data: &Data,
    minted: &HashSet<u16>,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
) -> bool {
    let def = data.def(d);
    let Some(attr) = def.hidden_return_buffer_attr() else {
        return false;
    };
    if !matches!(args.get(attr).map(Value::unspan), Some(Value::Var(x)) if minted.contains(x)) {
        return false;
    }
    let Some(rb) = retbuf_var(data, d) else {
        return false;
    };
    // A recursive call answers yes while its own body is being checked: the property is
    // that every write lands in the buffer handed in, so a write that breaks it is in a
    // body this walk reads (the greatest fixpoint, as `in_place_only_writer` assumes none).
    if !BUFFER_WRITERS_OPEN.with(|o| o.borrow_mut().insert(d)) {
        return true;
    }
    let cvars = def.variables();
    let inner = frame_minted_locals(data, cvars, def.code(), &[rb]);
    let clean =
        foreign_store_writer(def.code(), data, &inner, cache, active, Some(cvars)).is_none();
    BUFFER_WRITERS_OPEN.with(|o| o.borrow_mut().remove(&d));
    clean
}

/// The first reason running `node` might grow, free or rewrite a text block of a store the
/// frame did NOT mint — or `None` when every store writer names only `minted` stores or sets
/// a scalar in place.  A native writer must NAME what it writes: a store operand, or a slot
/// operand spelled as a variable (`OpDatabase(v, tp)`), each one minted; a writer that names
/// none (a generator's resume, a store-minting cast), or takes an operand of another kind (a
/// fn-ref, a tuple), is refused.  A user callee may run only if it writes no store, or writes
/// one only in place.  Given the variable table, the address of a TEXT local
/// (`OpCreateStack`, the slot a callee formats into) is no writer either: it names the frame's
/// own `String`, never store memory — the same allowance the header gate makes.
pub(crate) fn foreign_store_writer(
    node: &Value,
    data: &Data,
    minted: &HashSet<u16>,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
    vars: Option<&crate::variables::Function>,
) -> Option<&'static str> {
    let mut why = None;
    node.any_node(&mut |n| match n {
        Value::CallRef(_, _) => {
            why = Some("a fn-ref call in its scope");
            true
        }
        Value::Parallel(_) | Value::Yield(_) => {
            why = Some("a parallel arm or a yield in its scope");
            true
        }
        Value::Call(d, args) => {
            if (*d as usize) >= data.definitions.len() {
                why = Some("an unknown call in its scope");
                return true;
            }
            let def = data.def(*d);
            let name = def.name();
            if !matches!(def.code(), Value::Null) {
                if call_writes_store(*d, data, cache, active, None)
                    && !in_place_only_writer(*d, data, cache, active, None)
                    && !(vars.is_some()
                        && callee_writes_only_its_buffer(*d, args, data, minted, cache, active))
                {
                    why = Some("a callee that writes a store in its scope");
                    return true;
                }
                return false;
            }
            // Given the variable table, `OpPutRef` is no writer either: it binds a reference
            // LOCAL (the slot rule has already taken that local out of `minted`, so a write
            // through it later is judged as one through a store it may name).
            if native_op_is_store_free(def)
                || IN_PLACE_SET_OPS.contains(&name)
                || text_stack_ref(name, args, vars)
                || (vars.is_some() && name == "OpPutRef")
            {
                return false;
            }
            let mut named = false;
            for (i, a) in args.iter().enumerate() {
                let Some(at) = def.attributes().get(i) else {
                    if name == "OpGetRecord" || name == "OpSetKeyed" {
                        continue; // a trailing key value
                    }
                    why = Some("a writer with an untyped operand in its scope");
                    return true;
                };
                if at.constant {
                    if let Value::Var(x) = a.unspan() {
                        if !minted.contains(x) {
                            why = Some("a writer to a slot the frame did not mint in its scope");
                            return true;
                        }
                        named = true;
                    }
                } else if names_a_store(&at.typedef) {
                    // `OpAppendVector`'s source is READ (copied from), never written.
                    if name == "OpAppendVector" && i == 1 {
                        continue;
                    }
                    if !minted_expr(a, data, minted) {
                        why = Some("a writer to a store the frame did not mint in its scope");
                        return true;
                    }
                    named = true;
                } else if !(is_scalar(&at.typedef) || matches!(at.typedef.base(), Type::Text(_))) {
                    why = Some("a writer with an operand of another kind in its scope");
                    return true;
                }
            }
            if !named {
                why = Some("a writer that names no store in its scope");
                return true;
            }
            false
        }
        _ => false,
    });
    why
}
