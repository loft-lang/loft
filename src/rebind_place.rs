// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-Rebind` — `x = f(x, …)` hands the callee x's OWN record as its return buffer.
//!
//! The immutable-update idiom, `doc = delete_range(doc, a, b)`: the callee reads its
//! by-value parameter, builds `Doc { buf: d.buf, pieces: np }` and the caller copies that
//! record over the local it came from — the buffer copied whole, twice, for a piece list
//! that changed.  With the local's own record handed in as the hidden `__retbuf`, the
//! parameter and the buffer are ONE record: the callee's reads land where they always did,
//! its exit literal writes the fields in place — a field whose value views that same field
//! (`buf: d.buf`) is `@FR-H-CopySelf` and costs nothing, a field built in a store of its
//! own is copied in after the old field's owned heap is released (`@FR-H-ClearRelease`) —
//! and an exit `return d` answers the buffer itself.  The call site keeps its `Set`: the
//! result IS x's record, and the bind's own identity test (`FreeRefIfDistinct` of the old
//! store against the new one) already frees nothing when the two are one.
//!
//! ONE IR site changes: the hidden buffer argument of the call, the caller's `__ref_N`,
//! becomes `x`.  The buffer variable stays null for the rest of the frame and its exit free
//! is a no-op; the callee's guarded mint sees a record offered and mints nothing.
//!
//! The admission is `(R-Rebind)`'s decline list read off the IR, and every doubt DECLINES —
//! a wrong admission is silent-wrong, a wrong decline is the copy the program already
//! pays.  The caller's half: x is a plain owned dense record local (never a parameter, a
//! compiler temp, a view, a witnessed or view-elided local, nor a `τ?`), x is exactly one
//! by-value argument of the call and no other argument reaches its store, and the callee
//! is a loft-bodied function that is not this one.  The callee's half is read off ITS
//! IR, since the parser lowered it before this pass runs (`Parser::rebind_safe_literal`):
//! every exit answers the parameter that receives x or is a literal built into the
//! unpromoted `__retbuf` whose writes consult the parameter only through the staged temps
//! that precede them or the self-view replace of the same vector field, and no vector
//! field keeps a zeroing default.  A chain exit, a text or reference field read off the
//! parameter, a vector read at another of its fields and a promoted buffer each decline.
//!
//! `LOFT_NO_REBIND_PLACE=1` keeps every copy (the switch, read by the parser too);
//! `LOFT_TRACE_REBIND=1` names each admission and each decline.

use std::collections::HashSet;

use crate::data::{Block, Data, DefType, Type, Value};
use crate::database::Stores;
use crate::variables::Function;

/// Rewrite every admitted rebind of function `d_nr`; a no-op under the switch.
pub fn rewrite(data: &mut Data, database: &Stores, d_nr: u32) {
    if !crate::keys::rebind_place_enabled() || data.def_type(d_nr) != DefType::Function {
        return;
    }
    // Both facts below are read off the body BEFORE it is taken out for the rewrite: the
    // oracle's definitions (asked on an empty body it answers its fail-open default), and
    // which locals a binding makes a view of a place.
    let defs = crate::use_analysis::function_defs(data, d_nr);
    let projected = projected_locals(data, d_nr);
    let facts = Facts {
        defs: &defs,
        projected: &projected,
    };
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    visit(&mut code, data, database, d_nr, &facts);
    data.definitions[d_nr as usize].code = code;
}

/// What the rewrite reads off the whole body before it walks it.
struct Facts<'a> {
    defs: &'a crate::use_analysis::Defs,
    projected: &'a HashSet<u16>,
}

/// The locals one of whose bindings is a PROJECTION — `v[i]`, `o.f`, `t.0` — and so a
/// view of that place, whatever the other bindings did.
fn projected_locals(data: &Data, d_nr: u32) -> HashSet<u16> {
    let mut out = HashSet::new();
    data.def(d_nr).code().walk(&mut |n| {
        if let Value::Set(v, rhs) = n {
            let rhs = match rhs.unspan() {
                Value::Insert(steps) => steps.last().map(Value::unspan),
                other => Some(other),
            };
            match rhs {
                Some(Value::TupleGet(..)) => {
                    out.insert(*v);
                }
                Some(Value::Call(d, _))
                    if (*d as usize) < data.definitions.len()
                        && crate::use_analysis::is_projection_op(data, *d) =>
                {
                    out.insert(*v);
                }
                _ => {}
            }
        }
    });
    out
}

fn visit(v: &mut Value, data: &Data, database: &Stores, d_nr: u32, facts: &Facts) {
    if let Value::Set(x, val) = v {
        let x = *x;
        if let Value::Call(f, args) = val.unspan_mut() {
            let f = *f;
            let candidate = args
                .iter()
                .any(|a| matches!(a.unspan(), Value::Var(v) if *v == x));
            match admit(data, database, d_nr, x, f, args, facts) {
                Ok(b) => {
                    if crate::keys::trace_rebind() {
                        eprintln!(
                            "[rebind] fn={} x={} callee={} ADMITTED: the local's record is the buffer",
                            data.def(d_nr).name(),
                            data.def(d_nr).variables().name(x),
                            data.def(f).name()
                        );
                    }
                    args[b] = Value::Var(x);
                }
                Err(reason) => {
                    if candidate && crate::keys::trace_rebind() {
                        eprintln!(
                            "[rebind] fn={} x={} callee={} DECLINED: {reason}",
                            data.def(d_nr).name(),
                            data.def(d_nr).variables().name(x),
                            data.def(f).name()
                        );
                    }
                }
            }
        }
    }
    v.for_each_child_mut(&mut |c| visit(c, data, database, d_nr, facts));
}

/// The caller's half of the admission; `Ok` carries the hidden buffer's argument index.
fn admit(
    data: &Data,
    database: &Stores,
    d_nr: u32,
    local: u16,
    callee_nr: u32,
    args: &[Value],
    facts: &Facts,
) -> Result<usize, &'static str> {
    if callee_nr == d_nr {
        return Err("a recursive call");
    }
    if data.def_type(callee_nr) != DefType::Function {
        return Err("the callee is not a function");
    }
    let callee = data.def(callee_nr);
    if *callee.code() == Value::Null || !callee.rust().is_empty() || !callee.native().is_empty() {
        return Err("the callee has no loft body");
    }
    if callee.name().contains("__lambda") {
        return Err("the callee is a lambda");
    }
    let buf_idx = callee
        .hidden_return_buffer_attr()
        .ok_or("the callee has no hidden return buffer")?;
    if callee.attributes()[buf_idx].name != "__retbuf" {
        return Err("the callee's buffer was promoted onto a local");
    }
    if args.len() <= buf_idx {
        return Err("the call carries no buffer argument");
    }
    let func: &Function = data.def(d_nr).variables();
    let Value::Var(buf) = args[buf_idx].unspan() else {
        return Err("the buffer argument is not the caller's work-ref");
    };
    if !func.is_compiler_generated(*buf) || !func.name(*buf).starts_with("__ref_") {
        return Err("the buffer argument is not the caller's work-ref");
    }
    if func.is_argument(local) || func.is_compiler_generated(local) {
        return Err("the local is a parameter or a compiler temp");
    }
    let Type::Reference(td, _) = func.tp(local) else {
        return Err("the local is not a dense record");
    };
    let td = *td;
    // `@FR-O-Owner` — the local must OWN the store the callee is about to write.  A plain
    // local bound by a whole-value bind owns what it was handed whatever the value's own
    // verdict was: a minted store is adopted, a BORROWED result — `do_d = delete_range(d,
    // 0, 0)`, a callee that may answer its argument — is copied at the bind (`@FR-B-Copy`,
    // the interpreter's `CopyRefOrNull`, native's `OpCopyRecord`), and a JOIN adopts or
    // copies per execution (`OpBindOrCopy`).  So the oracle's VERDICT on the value is not
    // the question (it joins `Owned` with `Borrowed` to `Unknown` for exactly this shape);
    // its EVIDENCE is read, to rule out a local with no derived binding — the fail-open
    // default, a parameter, a delivery buffer — and the bindings themselves are read: one
    // whose right-hand side is a PROJECTION (`v[i]`, `o.f`, `t.0`) makes the local a view
    // of that place, and a bind that did not copy is `is_view_elided` below.
    let (own, evidence) =
        crate::use_analysis::ownership_evidence_with(data, d_nr, local, facts.defs);
    if crate::keys::trace_rebind() {
        eprintln!(
            "[rebind]   oracle for {}: {own:?} by {evidence:?}",
            func.name(local)
        );
    }
    if !matches!(
        evidence,
        crate::use_analysis::OwnEvidence::Derived | crate::use_analysis::OwnEvidence::Minted
    ) {
        return Err("the local has no derived binding");
    }
    if facts.projected.contains(&local) {
        return Err("a binding of the local views a place");
    }
    // @FR-O-Proxy asks copy — chooses alias-vs-copy (hand the local's own record in, or
    // keep the copy); authorises no free.  A dep on another place is a view whatever the
    // oracle joined for the binding.
    if !func.tp(local).depend().is_empty() {
        return Err("the local views another place");
    }
    if func.is_view_elided(local) {
        return Err("the local's bind was elided to a view");
    }
    if func.var(&format!("__own_{}", func.name(local))) != u16::MAX {
        return Err("the local carries an owner witness");
    }
    if callee.attributes()[buf_idx].typedef.base().heap_def_nr() != Some(td) {
        return Err("the buffer's record type is not the local's");
    }
    let viewers = func.store_viewers(local);
    let mut param_idx: Option<usize> = None;
    for (i, arg) in args.iter().enumerate() {
        if i == buf_idx {
            continue;
        }
        if matches!(arg.unspan(), Value::Var(v) if *v == local) {
            if param_idx.is_some() {
                return Err("the local is handed in twice");
            }
            param_idx = Some(i);
            continue;
        }
        if arg.reads_var(local) || viewers.iter().any(|&w| arg.reads_var(w)) {
            return Err("another argument reaches the local's store");
        }
    }
    let param_idx = param_idx.ok_or("the local is not an argument of the call")?;
    let attr = &callee.attributes()[param_idx];
    if attr.hidden || matches!(attr.typedef, Type::RefVar(_)) {
        return Err("the parameter receiving the local is not by value");
    }
    if attr.typedef.base().heap_def_nr() != Some(td) {
        return Err("the parameter's record type is not the local's");
    }
    callee_safe(data, database, callee_nr, param_idx, td)?;
    Ok(buf_idx)
}

/// The callee's half: every exit answers parameter `k` or a literal built into `__retbuf`
/// that is safe when the two are one record.
fn callee_safe(
    data: &Data,
    database: &Stores,
    f: u32,
    k: usize,
    td: u32,
) -> Result<(), &'static str> {
    let callee = data.def(f);
    let vars = callee.variables();
    let buf_var = vars.var("__retbuf");
    if buf_var == u16::MAX || !vars.is_argument(buf_var) {
        return Err("the callee's buffer is not its `__retbuf` argument");
    }
    let p_var = vars.var(&callee.attributes()[k].name);
    if p_var == u16::MAX || !vars.is_argument(p_var) {
        return Err("the callee's parameter has no variable");
    }
    let struct_tp = data.def(td).known_type();
    let vec_offs: HashSet<i32> = (0..data.attributes(td))
        .filter(|&aid| matches!(data.attr_type(td, aid).base(), Type::Vector(_, _)))
        .map(|aid| i32::from(database.position(struct_tp, &data.attr_name(td, aid))))
        .collect();
    let mut viewers = vars.store_viewers(p_var);
    viewers.insert(p_var);
    let cx = Cx {
        data,
        vars,
        buf_var,
        p_var,
        viewers,
        vec_offs,
        get_field: data.def_nr("OpGetField"),
    };
    let Value::Block(body) = callee.code().unspan() else {
        return Err("the callee's body is not a block");
    };
    // Every `return`, wherever it stands.
    let mut err: Option<&'static str> = None;
    callee.code().walk(&mut |n| {
        if err.is_none()
            && let Value::Return(inner) = n
            && let Err(r) = cx.exit_ok(inner)
        {
            err = Some(r);
        }
    });
    if let Some(r) = err {
        return Err(r);
    }
    // The bare tail, where the body's last statement is not a `return`.
    match body.operators.last() {
        None => return Err("the callee's body is empty"),
        Some(last) if !matches!(last.unspan(), Value::Return(_)) => cx.exit_ok(last)?,
        Some(_) => {}
    }
    // Every literal that yields the buffer, whichever spelling its exit took.
    let mut err: Option<&'static str> = None;
    callee.code().walk(&mut |n| {
        if err.is_none()
            && let Value::Block(bl) = n
            && bl.name == "Object"
            && cx.yields_buffer(bl)
            && let Err(r) = cx.object_ok(bl)
        {
            err = Some(r);
        }
    });
    err.map_or(Ok(()), Err)
}

struct Cx<'a> {
    data: &'a Data,
    vars: &'a Function,
    buf_var: u16,
    p_var: u16,
    viewers: HashSet<u16>,
    vec_offs: HashSet<i32>,
    get_field: u32,
}

impl Cx<'_> {
    fn reads_viewer(&self, v: &Value) -> bool {
        self.viewers.iter().any(|&w| v.reads_var(w))
    }

    /// `Var(v)` or `OpGetField(Var(v), …)`.
    fn targets(&self, a: &Value, v: u16) -> bool {
        match a.unspan() {
            Value::Var(x) => *x == v,
            Value::Call(d, args) if *d == self.get_field => {
                matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == v)
            }
            _ => false,
        }
    }

    /// The field offset of `OpGetField(Var(v), Int(off), _)`.
    fn field_off(&self, a: &Value, v: u16) -> Option<i32> {
        match a.unspan() {
            Value::Call(d, args)
                if *d == self.get_field
                    && args.len() == 3
                    && matches!(args[0].unspan(), Value::Var(x) if *x == v) =>
            {
                match args[1].unspan() {
                    Value::Int(off) => Some(*off),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn yields_buffer(&self, bl: &Block) -> bool {
        match bl.operators.last().map(Value::unspan) {
            Some(Value::Var(v)) => *v == self.buf_var,
            Some(Value::Return(r)) => matches!(r.unspan(), Value::Var(v) if *v == self.buf_var),
            _ => false,
        }
    }

    /// An exit answers the parameter, the buffer, or a literal that yields the buffer.
    fn exit_ok(&self, e: &Value) -> Result<(), &'static str> {
        match e.unspan() {
            Value::Var(v) if *v == self.p_var || *v == self.buf_var => Ok(()),
            Value::Return(inner) => self.exit_ok(inner),
            Value::Block(bl) if bl.name == "Object" && self.yields_buffer(bl) => Ok(()),
            _ => Err("an exit answers neither the parameter nor the buffer"),
        }
    }

    /// The literal's writes are safe when the buffer IS the parameter's record.
    fn object_ok(&self, bl: &Block) -> Result<(), &'static str> {
        let Some((_, ops)) = bl.operators.split_last() else {
            return Err("an empty literal");
        };
        let mut seen_write = false;
        for op in ops {
            match op.unspan() {
                Value::If(_, then_arm, else_arm) => {
                    let is_mint = |v: &Value| {
                        matches!(v.unspan(), Value::Call(d, args)
                            if self.data.def(*d).name() == "OpDatabase"
                            && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == self.buf_var))
                    };
                    let guarded_mint = (is_mint(else_arm)
                        && matches!(then_arm.unspan(), Value::Null))
                        || (is_mint(then_arm) && matches!(else_arm.unspan(), Value::Null));
                    if !guarded_mint {
                        return Err("a branch in the literal that is not the guarded mint");
                    }
                }
                Value::Set(target, expr) => {
                    if self.vars.is_argument(*target) {
                        return Err("the literal writes an argument");
                    }
                    if seen_write && self.reads_viewer(expr) {
                        return Err("a read of the parameter after a write to the buffer");
                    }
                }
                Value::Call(def_nr, args) => {
                    let name = self.data.def(*def_nr).name();
                    if name == "OpDatabase" {
                        return Err("the literal mints unguarded");
                    }
                    if name.starts_with("OpFree") {
                        continue;
                    }
                    let Some(first) = args.first() else {
                        continue;
                    };
                    if self.targets(first, self.buf_var) {
                        if name == "OpSetInt4"
                            && matches!(args.get(1).map(Value::unspan), Some(Value::Int(off)) if self.vec_offs.contains(off))
                        {
                            return Err("a vector field keeps its zeroing default");
                        }
                        if name == "OpReplaceVector"
                            && args.len() == 3
                            && let Some(off) = self.field_off(first, self.buf_var)
                            && self.field_off(&args[1], self.p_var) == Some(off)
                        {
                            seen_write = true;
                            continue;
                        }
                        if args[1..].iter().any(|a| self.reads_viewer(a)) {
                            return Err("a write to the buffer consults the parameter");
                        }
                        seen_write = true;
                    } else {
                        if self.targets(first, self.p_var)
                            && !name.starts_with("OpGet")
                            && !name.starts_with("OpConv")
                        {
                            return Err("the literal writes through the parameter");
                        }
                        if seen_write && args.iter().any(|a| self.reads_viewer(a)) {
                            return Err("a read of the parameter after a write to the buffer");
                        }
                    }
                }
                _ => return Err("a statement in the literal this pass does not read"),
            }
        }
        Ok(())
    }
}
