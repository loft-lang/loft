// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! One function's body as C: statements, expressions and their C shapes.
//!
//! Every temporary is declared at the top of the function, so one created inside a branch is
//! still in scope where it is freed.  An owned text temporary is freed after the statement
//! that made it; every `return` first copies its value out, then frees the function's texts.

use super::{CType, Cx, c_name, held, is_operator, type_word};
use crate::data::{Block, Value};
use crate::variables::Function;
use std::fmt::Write as _;

/// A C-safe name for variable `v`: its number first, so two loft names that sanitise alike
/// stay apart.
pub(crate) fn var_name(name: &str, v: u16) -> String {
    let clean: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("v{v}_{clean}")
}

/// The shape of a C expression's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K {
    /// `int64_t`.
    Int,
    /// A C condition, 0 or 1.
    Cond,
    /// A stored boolean, `uint8_t` 0, 1 or `LOFT_BOOL_NULL`.
    Bool,
    /// A `loft_str` view.
    View,
    /// A `loft_ref`.
    Ref,
    /// A plain enum value, `uint8_t`.
    Enum,
    /// `double`.
    Float,
    /// `float`.
    Single,
    /// A character's code point, `uint32_t`; 0 is null.
    Char,
    /// A tuple struct, by its number.
    Tup(u32),
    /// No value.
    Void,
}

/// A C expression and the shape of its value.
struct E {
    code: String,
    kind: K,
}

impl E {
    fn new(code: impl Into<String>, kind: K) -> E {
        E {
            code: code.into(),
            kind,
        }
    }
}

/// The expression kind that reads a held value of shape `c`.
fn kind_of(c: CType) -> K {
    match c {
        CType::Int => K::Int,
        CType::Bool => K::Bool,
        CType::Text => K::View,
        CType::Ref => K::Ref,
        CType::Enum => K::Enum,
        CType::Float => K::Float,
        CType::Single => K::Single,
        CType::Char => K::Char,
        CType::Tuple(id) => K::Tup(id),
        CType::Void => K::Void,
    }
}

/// One function's body under construction.
pub(crate) struct FnWriter<'a> {
    cx: &'a Cx<'a>,
    /// The function's definition number.
    d: u32,
    vars: &'a Function,
    code: &'a Value,
    /// The C shape of every variable, parameters first.
    shapes: Vec<CType>,
    /// Which variables are the function's parameters (`Function::arguments`).
    params: Vec<bool>,
    /// Which parameters are `&` links: a pointer to the caller's variable, read and written
    /// through (`(*v)`), as the bytecode reads and writes through the stack reference.
    links: Vec<bool>,
    ret: CType,
    body: String,
    /// Every temporary: its name and shape, declared at the top.
    temps: Vec<(String, CType)>,
    /// The owned text temporaries the current statement made.
    pending: Vec<String>,
    /// The label number of each enclosing loop, innermost last.
    loops: Vec<u32>,
    labels: u32,
    /// Which variables a statement written so far has bound: a later `Set` is a rebind.
    bound: Vec<bool>,
}

impl<'a> FnWriter<'a> {
    pub(crate) fn new(cx: &'a Cx<'a>, d: u32, shapes: Vec<CType>, ret: CType) -> FnWriter<'a> {
        let def = cx.data.def(d);
        FnWriter {
            cx,
            d,
            vars: def.variables(),
            code: def.code(),
            shapes,
            params: {
                let vars = def.variables();
                let mut p = vec![false; vars.count() as usize];
                for a in vars.arguments() {
                    p[a as usize] = true;
                }
                p
            },
            links: (0..def.variables().count())
                .map(|v| matches!(def.variables().tp(v), crate::data::Type::RefVar(_)))
                .collect(),
            ret,
            body: String::new(),
            temps: Vec::new(),
            pending: Vec::new(),
            loops: Vec::new(),
            labels: 0,
            bound: vec![false; def.variables().count() as usize],
        }
    }

    /// The body: the locals, the temporaries and the statements.
    pub(crate) fn write(mut self) -> Result<String, String> {
        let code = self.code;
        if self.ret == CType::Void {
            self.stmt(code, 1)?;
            self.epilogue(1);
        } else {
            self.ret_value(Some(code), 1)?;
        }
        let mut out = String::new();
        for v in (0..self.shapes.len()).filter(|v| !self.params[*v]) {
            let c = self.shapes[v];
            let _ = writeln!(
                out,
                "    {} {} = {};",
                held(c),
                self.var(v as u16),
                self.cx.initial(c)
            );
        }
        for (t, c) in &self.temps {
            let _ = writeln!(out, "    {} {t} = {};", held(*c), self.cx.initial(*c));
        }
        out.push_str(&self.body);
        Ok(out)
    }

    fn var(&self, v: u16) -> String {
        let name = var_name(self.vars.name(v), v);
        if self.links[v as usize] {
            format!("(*{name})")
        } else {
            name
        }
    }

    /// A text variable this function holds as a `loft_text`: a local, or the caller's
    /// through a link; a plain text parameter is a borrowed `loft_str`.
    fn text_held(&self, v: u16) -> bool {
        self.shapes[v as usize] == CType::Text
            && (!self.params[v as usize] || self.links[v as usize])
    }

    fn line(&mut self, ind: usize, s: &str) {
        let _ = writeln!(self.body, "{}{s}", "    ".repeat(ind));
    }

    fn temp(&mut self, c: CType) -> String {
        let t = format!("t{}", self.temps.len() + 1);
        self.temps.push((t.clone(), c));
        t
    }

    /// Free the owned text temporaries the statement just finished made.
    fn flush(&mut self, ind: usize) {
        for t in std::mem::take(&mut self.pending) {
            self.line(ind, &format!("loft_text_free(&{t});"));
        }
    }

    /// Free every text the function holds: its locals and its temporaries.
    fn epilogue(&mut self, ind: usize) {
        let mut texts: Vec<String> = (0..self.shapes.len())
            .filter(|&v| !self.params[v] && self.shapes[v] == CType::Text)
            .map(|v| self.var(v as u16))
            .collect();
        texts.extend(
            self.temps
                .iter()
                .filter(|(_, c)| *c == CType::Text)
                .map(|(t, _)| t.clone()),
        );
        for t in texts {
            self.line(ind, &format!("loft_text_free(&{t});"));
        }
    }

    /// The loop label `depth` loops out from the innermost.
    fn label(&self, depth: u16) -> Result<u32, String> {
        let n = self.loops.len();
        if (depth as usize) < n {
            Ok(self.loops[n - 1 - depth as usize])
        } else {
            Err(format!("a break or continue {depth} loops out of {n}"))
        }
    }

    /// Return `v`'s value (or nothing): copy it out, free the function's texts, return.
    fn ret_value(&mut self, v: Option<&Value>, ind: usize) -> Result<(), String> {
        let v = v.filter(|v| !matches!(v.unspan(), Value::Null));
        match (self.ret, v) {
            (CType::Void, Some(v)) => {
                self.stmt(v, ind)?;
                self.epilogue(ind);
                self.line(ind, "return;");
            }
            (CType::Void, None) => {
                self.epilogue(ind);
                self.line(ind, "return;");
            }
            (c, v) => {
                let r = self.temp(c);
                match v {
                    Some(v) => self.assign(v, &r, c, ind)?,
                    None => self.null_into(&r, c, ind),
                }
                self.ret_held(&r, ind);
            }
        }
        Ok(())
    }

    /// Return the value temporary `r` already holds, of the function's result shape.
    fn ret_held(&mut self, r: &str, ind: usize) {
        if self.ret == CType::Text {
            // The result leaves the function: its buffer moves out before the
            // epilogue frees every text the function holds.
            self.line(ind, &format!("loft_text {r}_out = {r};"));
            self.line(ind, &format!("{r}.p = 0;"));
            self.epilogue(ind);
            self.line(ind, &format!("return {r}_out;"));
        } else {
            self.epilogue(ind);
            self.line(ind, &format!("return {r};"));
        }
    }

    /// A value the scopes pass left for the bytecode's evaluation stack: `[value, frees…,
    /// return null]`, where `OpReturn` answers the value still on the stack top.  The
    /// positions of the value and of the return, when `ops` ends that way; the native
    /// writer's `detect_ref_tail_capture` is the same reading.
    fn tail_capture(&self, ops: &[Value]) -> Option<(usize, usize)> {
        if self.ret == CType::Void {
            return None;
        }
        let ret = ops
            .iter()
            .rposition(|o| !matches!(o.unspan(), Value::Line(_)))?;
        let Value::Return(x) = ops[ret].unspan() else {
            return None;
        };
        if !matches!(x.unspan(), Value::Null) && !self.is_free(x) {
            return None;
        }
        let at = ops[..ret]
            .iter()
            .rposition(|o| !matches!(o.unspan(), Value::Line(_)) && !self.is_free(o))?;
        (self.shape_of(&ops[at]).ok() == Some(self.ret)).then_some((at, ret))
    }

    /// A scope-exit free, or the store lock the cleanup window may hold.
    fn is_free(&self, v: &Value) -> bool {
        matches!(v.unspan(), Value::Call(d, _) if {
            let name = self.cx.data.def(*d).name();
            name.starts_with("OpFree") || name == "n_set_store_lock"
        })
    }

    /// `ops` for their effect, returning the value a `tail_capture` names.
    fn ops_stmt(&mut self, ops: &[Value], ind: usize) -> Result<(), String> {
        let Some((at, ret)) = self.tail_capture(ops) else {
            return ops.iter().try_for_each(|o| self.stmt(o, ind));
        };
        ops[..at].iter().try_for_each(|o| self.stmt(o, ind))?;
        let r = self.temp(self.ret);
        self.assign(&ops[at], &r, self.ret, ind)?;
        self.flush(ind);
        ops[at + 1..ret]
            .iter()
            .try_for_each(|o| self.stmt(o, ind))?;
        if let Value::Return(x) = ops[ret].unspan() {
            self.stmt(x, ind)?;
        }
        self.ret_held(&r, ind);
        Ok(())
    }

    fn null_into(&mut self, target: &str, c: CType, ind: usize) {
        let s = match c {
            CType::Int => format!("{target} = LOFT_NULL;"),
            CType::Bool => format!("{target} = LOFT_BOOL_NULL;"),
            CType::Text => format!("loft_text_set(&{target}, loft_null_text());"),
            CType::Ref => format!("{target} = loft_ref_null();"),
            CType::Enum => format!("{target} = 255;"),
            CType::Float => format!("{target} = (double) NAN;"),
            CType::Single => format!("{target} = (float) NAN;"),
            CType::Char => format!("{target} = 0;"),
            CType::Tuple(id) => {
                for (i, e) in self.cx.tuple_shapes(id).into_iter().enumerate() {
                    self.null_into(&format!("{target}.f{i}"), e, ind);
                }
                return;
            }
            CType::Void => return,
        };
        self.line(ind, &s);
    }

    /// Emit `v` for its effect.
    fn stmt(&mut self, v: &Value, ind: usize) -> Result<(), String> {
        match v {
            Value::Null | Value::Line(_) => Ok(()),
            Value::Span(b) => self.stmt(&b.1, ind),
            Value::Block(b) => self.block_stmt(b, ind),
            Value::Insert(ops) => ops.iter().try_for_each(|o| self.stmt(o, ind)),
            Value::Set(var, x) => self.set_stmt(*var, x, ind),
            Value::TuplePut(var, i, x) => {
                let CType::Tuple(id) = self.shapes[*var as usize] else {
                    return Err("a tuple write into a non-tuple".into());
                };
                let shapes = self.cx.tuple_shapes(id);
                let c = *shapes
                    .get(*i as usize)
                    .ok_or("a tuple index out of range")?;
                let target = format!("{}.f{i}", self.var(*var));
                self.assign(x, &target, c, ind)?;
                self.flush(ind);
                Ok(())
            }
            Value::If(c, t, e) => {
                let c = self.expr(c, ind)?;
                let c = as_cond(c)?;
                self.line(ind, &format!("if ({}) {{", bare(&c)));
                self.stmt(t, ind + 1)?;
                if !matches!(e.unspan(), Value::Null) {
                    self.line(ind, "} else {");
                    self.stmt(e, ind + 1)?;
                }
                self.line(ind, "}");
                self.flush(ind);
                Ok(())
            }
            Value::Loop(b) => {
                self.labels += 1;
                let l = self.labels;
                self.loops.push(l);
                self.line(ind, "for (;;) {");
                self.block_stmt(b, ind + 1)?;
                self.line(ind, &format!("cont_{l}:;"));
                self.line(ind, "}");
                self.line(ind, &format!("brk_{l}:;"));
                self.loops.pop();
                Ok(())
            }
            Value::Break(n) => {
                let l = self.label(*n)?;
                self.flush(ind);
                self.line(ind, &format!("goto brk_{l};"));
                Ok(())
            }
            Value::Continue(n) => {
                let l = self.label(*n)?;
                self.flush(ind);
                self.line(ind, &format!("goto cont_{l};"));
                Ok(())
            }
            Value::Return(x) => self.ret_value(Some(x), ind),
            Value::Drop(_) | Value::Call(..) => {
                let x = if let Value::Drop(x) = v { &**x } else { v };
                let e = self.expr(x, ind)?;
                if e.kind == K::Void {
                    self.line(ind, &format!("{};", e.code));
                } else {
                    self.line(ind, &format!("(void) {};", e.code));
                }
                self.flush(ind);
                Ok(())
            }
            other => Err(unsupported(other)),
        }
    }

    /// `v = src` binding one record variable to another of a type that copies
    /// (`@FR-B-Copy`, @C86): the source and the store type — the bytecode writer's
    /// `gen_set_first_ref_var_copy` test, `record_copy_source` and `copies_as`.
    fn whole_value_bind(&self, v: u16, x: &Value) -> Option<(u16, u16)> {
        let Value::Var(src) = x.unspan() else {
            return None;
        };
        let data = self.cx.data;
        let d = self.vars.tp(v).base().heap_def_nr()?;
        let sd = self.vars.record_copy_source(data, v, *src)?;
        data.copies_as(d, sd)
            .then(|| (*src, data.def(d).known_type()))
    }

    /// `v = f(…)` binding a record variable to a loft function's result that the bind must
    /// copy rather than adopt — the bytecode writer's first-bind test: a callee that hands
    /// over a fresh store, an elided view and a minted store are adopted; anything else is
    /// copied, its source freed when `call_return_frees_source` says the copy owns it.  The
    /// record's store type and that flag; `None` to adopt.
    fn call_copy(&self, v: u16, x: &Value) -> Result<Option<(u16, bool)>, String> {
        use crate::data::Type;
        use crate::use_analysis as ua;
        let data = self.cx.data;
        let (Type::Reference(d, _) | Type::Enum(d, true, _)) =
            ua::first_bind_shape(self.vars.tp(v)).clone()
        else {
            return Ok(None);
        };
        let Some(f) = ua::callee_of(data, self.d, x) else {
            return Ok(None);
        };
        if !data.def(f).is_loft_defined() {
            return Ok(None);
        }
        if crate::keys::join_own_enabled()
            && !self.vars.is_view_elided(v)
            && matches!(ua::ownership_of(data, self.d, x), ua::Own::Join { base } if base != u16::MAX)
        {
            return Err(
                "a bind of a result that is fresh on one arm and borrowed on the other".into(),
            );
        }
        if data.def(f).return_adopts_fresh_store()
            || self.vars.is_view_elided(v)
            || ua::adopts_minted_at_bind(data, self.vars, v, x)
        {
            return Ok(None);
        }
        Ok(Some((
            data.def(d).known_type(),
            ua::call_return_frees_source(data, self.d, x),
        )))
    }

    /// `v = c[i]` binding an owning record variable to an element of a container: the
    /// element is copied out (the bytecode writer's `gen_set_first_ref_elem_copy` test — no
    /// dependency, no owner witness, a projection of a container).  The store type.
    fn element_copy(&self, v: u16, x: &Value) -> Option<u16> {
        let tp = self.vars.tp(v);
        let d = tp.base().heap_def_nr()?;
        (tp.depend().is_empty()
            && self.vars.owner_witness(v).is_none()
            && crate::use_analysis::projection_container_var(self.cx.data, x).is_some())
        .then(|| self.cx.data.def(d).known_type())
    }

    /// Register record type `tp` for the deep copy, or refuse a kind it does not walk yet.
    fn copyable(&self, tp: u16) -> Result<u16, String> {
        self.cx.copy_type(tp)?;
        Ok(tp)
    }

    /// `p = x` for a `&` record parameter: the caller's variable takes the new record and the
    /// record it held is released unless the new one lives in its store (the bytecode's
    /// `SetStackRef` + `FreeRefIfDistinct`, @FR-B-Ref-Repoint).
    fn link_rebind(&mut self, var: u16, x: &Value, ind: usize) -> Result<(), String> {
        let crate::data::Type::RefVar(inner) = self.vars.tp(var) else {
            return Err("a link that is not a parameter".into());
        };
        if !matches!(**inner, crate::data::Type::Reference(..))
            || !matches!(x.unspan(), Value::Block(_))
        {
            return Err(format!("a rebind of a & {} parameter", type_word(inner)));
        }
        let e = self.expr(x, ind)?;
        let r = self.temp(CType::Ref);
        let old = self.temp(CType::Ref);
        let name = self.var(var);
        self.line(ind, &format!("{r} = {};", as_ref(e)?));
        self.line(ind, &format!("{old} = {name};"));
        self.line(ind, &format!("{name} = {r};"));
        self.line(ind, &format!("loft_free_displaced({old}, {name});"));
        self.flush(ind);
        Ok(())
    }

    /// `var = x`: a copy where loft copies a record, else the plain assignment.
    fn set_stmt(&mut self, var: u16, x: &Value, ind: usize) -> Result<(), String> {
        if self.links[var as usize] && self.shapes[var as usize] == CType::Ref {
            return self.link_rebind(var, x, ind);
        }
        let first = !std::mem::replace(&mut self.bound[var as usize], true);
        // An owned vector variable bound to null gets a store of its own whose record holds
        // the (empty) vector — the bytecode writer's `gen_set_first_vector_null`: the variable
        // then names that field.  A skip-free, inline or lazy-buffer one, and a view of another
        // variable (bound to its target later), keep the sentinel.  A lazy buffer's LATER
        // `Set(v, Null)` is its mint (`@FR-O-LazyBuffer`): the scopes pass guards it with
        // `OpRefIsNull(v)`.
        let lazy = self.vars.is_lazy_buffer(var);
        if matches!(x.unspan(), Value::Null)
            && let crate::data::Type::Vector(_, dep) = self.vars.tp(var)
            && dep.is_empty()
            && (if lazy {
                !first
            } else {
                !(self.vars.is_skip_free(var) || self.vars.is_inline_ref(var))
            })
        {
            let name = self.var(var);
            self.line(ind, &format!("{name} = loft_ref_null();"));
            self.line(ind, &format!("loft_database(&{name}, 8);"));
            return Ok(());
        }
        let c = self.shapes[var as usize];
        if c == CType::Text && !self.text_held(var) {
            return Err("assigns a text parameter".into());
        }
        let name = self.var(var);
        if let Some((tp, frees)) = self.call_copy(var, x)? {
            let bytes = self.copyable(tp)?;
            let e = self.expr(x, ind)?;
            let r = self.temp(CType::Ref);
            self.line(ind, &format!("{r} = {};", as_ref(e)?));
            self.line(
                ind,
                &format!("loft_bind_copy(loft_types, {bytes}, &{name}, {r});"),
            );
            if frees {
                // The stores of the call's reference arguments are protected, as the bytecode
                // writer protects them (`protectable_ref_args`): a result that is a view into
                // one of them is not the callee's to give away.
                let mut keep = vec![format!("{r}.store_nr != {name}.store_nr")];
                for a in crate::use_analysis::protectable_ref_args(self.cx.data, self.d, x).0 {
                    keep.push(format!("{r}.store_nr != {}.store_nr", self.var(a)));
                }
                self.line(
                    ind,
                    &format!("if ({}) loft_free_ref({r});", keep.join(" && ")),
                );
            }
            self.flush(ind);
            return Ok(());
        }
        if let Some(tp) = self.element_copy(var, x) {
            let bytes = self.copyable(tp)?;
            let e = self.expr(x, ind)?;
            let r = self.temp(CType::Ref);
            self.line(ind, &format!("{r} = {};", as_ref(e)?));
            self.line(
                ind,
                &format!("loft_bind_copy(loft_types, {bytes}, &{name}, {r});"),
            );
            self.flush(ind);
            return Ok(());
        }
        if let Some((src, tp)) = self.whole_value_bind(var, x) {
            let bytes = self.copyable(tp)?;
            let src = self.var(src);
            self.line(
                ind,
                &format!("loft_bind_copy(loft_types, {bytes}, &{name}, {src});"),
            );
            return Ok(());
        }
        self.assign(x, &name, c, ind)?;
        self.flush(ind);
        Ok(())
    }

    fn block_stmt(&mut self, b: &Block, ind: usize) -> Result<(), String> {
        self.ops_stmt(&b.operators, ind)
    }

    /// Emit the statements `v` needs and put its value in `target`, of shape `c`.
    fn assign(&mut self, v: &Value, target: &str, c: CType, ind: usize) -> Result<(), String> {
        match v.unspan() {
            Value::Null => {
                self.null_into(target, c, ind);
                Ok(())
            }
            Value::Block(b) => self.assign_seq(&b.operators, target, c, ind),
            Value::Insert(ops) => self.assign_seq(ops, target, c, ind),
            Value::If(cond, t, e) => {
                let cond = self.expr(cond, ind)?;
                let cond = as_cond(cond)?;
                self.line(ind, &format!("if ({}) {{", bare(&cond)));
                self.assign(t, target, c, ind + 1)?;
                self.line(ind, "} else {");
                self.assign(e, target, c, ind + 1)?;
                self.line(ind, "}");
                Ok(())
            }
            Value::Tuple(vals) => {
                let CType::Tuple(id) = c else {
                    return Err("a tuple literal into a non-tuple".into());
                };
                let shapes = self.cx.tuple_shapes(id);
                if shapes.len() != vals.len() {
                    return Err("a tuple literal of another arity".into());
                }
                for (i, (x, s)) in vals.iter().zip(shapes).enumerate() {
                    self.assign(x, &format!("{target}.f{i}"), s, ind)?;
                }
                Ok(())
            }
            v @ (Value::Return(_) | Value::Break(_) | Value::Continue(_)) => self.stmt(v, ind),
            v => {
                let e = self.expr(v, ind)?;
                let s = match c {
                    CType::Text => format!("loft_text_set(&{target}, {});", as_view(e)?),
                    CType::Void => format!("{};", e.code),
                    c => format!("{target} = {};", as_shape(e, c)?),
                };
                self.line(ind, &s);
                Ok(())
            }
        }
    }

    fn assign_seq(
        &mut self,
        ops: &[Value],
        target: &str,
        c: CType,
        ind: usize,
    ) -> Result<(), String> {
        if self.tail_capture(ops).is_some() {
            return self.ops_stmt(ops, ind);
        }
        let Some((last, rest)) = ops.split_last() else {
            return Err("a value block with no operators".into());
        };
        rest.iter().try_for_each(|o| self.stmt(o, ind))?;
        self.assign(last, target, c, ind)
    }

    /// The C shape `v` evaluates to, when it is a compound the caller must hold.
    fn shape_of(&self, v: &Value) -> Result<CType, String> {
        let cx = self.cx;
        match v.unspan() {
            Value::Block(b) => cx
                .ctype(&b.result)
                .ok_or_else(|| format!("a block of type {}", type_word(&b.result))),
            Value::Insert(ops) => ops.last().map_or(Ok(CType::Void), |l| self.shape_of(l)),
            Value::If(_, t, e) => match self.shape_of(t) {
                Ok(CType::Void) | Err(_) => self.shape_of(e),
                ok => ok,
            },
            Value::Int(_) | Value::Long(_) => Ok(CType::Int),
            Value::Enum(..) => Ok(CType::Enum),
            Value::Float(_) => Ok(CType::Float),
            Value::Single(_) => Ok(CType::Single),
            Value::Boolean(_) => Ok(CType::Bool),
            Value::Text(_) => Ok(CType::Text),
            Value::Var(v) => Ok(self.shapes[*v as usize]),
            Value::TupleGet(v, i) => match self.shapes[*v as usize] {
                CType::Tuple(id) => cx
                    .tuple_shapes(id)
                    .get(*i as usize)
                    .copied()
                    .ok_or_else(|| "a tuple index out of range".into()),
                _ => Err("a tuple read of a non-tuple".into()),
            },
            Value::Tuple(vals) => {
                let shapes = vals
                    .iter()
                    .map(|x| self.shape_of(x))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(CType::Tuple(cx.tuple_id(shapes)))
            }
            Value::Call(d, _) if !is_operator(cx.data, *d) => {
                let tp = cx.data.def(*d).returned();
                cx.ctype(tp)
                    .ok_or_else(|| format!("a call returning {}", type_word(tp)))
            }
            Value::Call(d, _) => {
                let name = cx.data.def(*d).name();
                op_shape(name).ok_or_else(|| format!("operator {name}"))
            }
            other => Err(unsupported(other)),
        }
    }

    /// A C expression for `v`'s value; the statements it needs are emitted first.
    fn expr(&mut self, v: &Value, ind: usize) -> Result<E, String> {
        match v.unspan() {
            Value::Int(i) => Ok(E::new(format!("INT64_C({i})"), K::Int)),
            Value::Long(l) if *l == i64::MIN => Ok(E::new("LOFT_NULL", K::Int)),
            Value::Long(l) => Ok(E::new(c_long(*l), K::Int)),
            Value::Boolean(b) => Ok(E::new(if *b { "1" } else { "0" }, K::Cond)),
            Value::Enum(v, _) => Ok(E::new(format!("(uint8_t) {v}"), K::Enum)),
            Value::Float(f) => Ok(E::new(
                format!("loft_f64(UINT64_C({:#x}))", f.to_bits()),
                K::Float,
            )),
            Value::Single(f) => Ok(E::new(
                format!("loft_f32(UINT32_C({:#x}))", f.to_bits()),
                K::Single,
            )),
            Value::Text(s) => Ok(E::new(
                format!("loft_lit({}, {})", c_string(s), s.len()),
                K::View,
            )),
            Value::Var(var) => {
                let name = self.var(*var);
                Ok(match self.shapes[*var as usize] {
                    CType::Text if self.text_held(*var) => {
                        E::new(format!("loft_view(&{name})"), K::View)
                    }
                    CType::Void => return Err("a void variable".into()),
                    c => E::new(name, kind_of(c)),
                })
            }
            Value::TupleGet(var, i) => {
                let c = self.shape_of(&Value::TupleGet(*var, *i))?;
                Ok(E::new(format!("{}.f{i}", self.var(*var)), kind_of(c)))
            }
            Value::Call(d, args) => self.call(*d, args, ind),
            compound @ (Value::Block(_) | Value::Insert(_) | Value::If(..) | Value::Tuple(_)) => {
                let c = self.shape_of(compound)?;
                self.nested(|w| {
                    if c == CType::Void {
                        w.stmt(compound, ind)?;
                        Ok(E::new("(void) 0", K::Void))
                    } else {
                        let t = w.temp(c);
                        w.assign(compound, &t, c, ind)?;
                        Ok(w.held(t, c))
                    }
                })
            }
            Value::Null => Err("a null without a type".into()),
            other => Err(unsupported(other)),
        }
    }

    /// Statements written inside an expression: they free only the temporaries they make, as
    /// the enclosing expression's are still to be read once they are done.
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let outer = std::mem::take(&mut self.pending);
        let r = f(self);
        let inner = std::mem::replace(&mut self.pending, outer);
        self.pending.extend(inner);
        r
    }

    /// The expression reading temporary `t` of shape `c`.
    fn held(&mut self, t: String, c: CType) -> E {
        match c {
            CType::Text => {
                self.pending.push(t.clone());
                E::new(format!("loft_view(&{t})"), K::View)
            }
            CType::Void => E::new("(void) 0", K::Void),
            c => E::new(t, kind_of(c)),
        }
    }

    /// The name of the text local `v` an operator writes into.
    fn text_dest(&self, v: &Value) -> Result<String, String> {
        match v.unspan() {
            Value::Var(var) if self.text_held(*var) => Ok(self.var(*var)),
            _ => Err("a text operator writing elsewhere than a text local".into()),
        }
    }

    /// Evaluate `args` in order: anything but a variable or a literal goes into a temporary
    /// first, because C leaves the order of a call's arguments open and loft does not.
    fn args(&mut self, args: &[Value], ind: usize) -> Result<Vec<E>, String> {
        let mut out = Vec::with_capacity(args.len());
        for a in args {
            let e = self.expr(a, ind)?;
            let simple = matches!(
                a.unspan(),
                Value::Var(_) | Value::Int(_) | Value::Long(_) | Value::Boolean(_) | Value::Text(_)
            );
            if simple || matches!(e.kind, K::View | K::Void) {
                out.push(e);
                continue;
            }
            let (c, code) = match e.kind {
                K::Int => (CType::Int, e.code),
                K::Ref => (CType::Ref, e.code),
                K::Tup(id) => (CType::Tuple(id), e.code),
                K::Enum => (CType::Enum, e.code),
                K::Float => (CType::Float, e.code),
                K::Single => (CType::Single, e.code),
                K::Char => (CType::Char, e.code),
                _ => (CType::Bool, as_bool(e)?),
            };
            let t = self.temp(c);
            self.line(ind, &format!("{t} = {code};"));
            out.push(E::new(t, kind_of(c)));
        }
        Ok(out)
    }

    fn call(&mut self, d: u32, args: &[Value], ind: usize) -> Result<E, String> {
        let cx = self.cx;
        if is_operator(cx.data, d) {
            return self.operator(cx.data.def(d).name(), args, ind);
        }
        let def = cx.data.def(d);
        let callee = def.original_name();
        let ret = cx
            .ctype(def.returned())
            .ok_or_else(|| format!("a call to {callee} returning {}", type_word(def.returned())))?;
        let shapes: Vec<CType> = def
            .attributes()
            .iter()
            .map(|a| {
                let tp = match &a.typedef {
                    crate::data::Type::RefVar(inner) => inner,
                    tp => tp,
                };
                cx.ctype(tp).ok_or_else(|| {
                    format!(
                        "a call to {callee} with a {} parameter",
                        type_word(&a.typedef)
                    )
                })
            })
            .collect::<Result<_, _>>()?;
        if shapes.len() != args.len() {
            return Err("a call with default arguments left out".into());
        }
        let name = super::native_c(def.name()).map_or_else(|| c_name(cx.data, d), str::to_string);
        let linked: Vec<bool> = def
            .attributes()
            .iter()
            .map(|a| matches!(a.typedef, crate::data::Type::RefVar(_)))
            .collect();
        // A link argument is the address of the caller's variable; the others are values,
        // evaluated in order.
        let mut xs = vec![String::new(); args.len()];
        let mut values = Vec::new();
        for (i, a) in args.iter().enumerate() {
            if linked[i] {
                xs[i] = self.link_arg(a, shapes[i], ind)?;
            } else {
                values.push(i);
            }
        }
        let vals: Vec<Value> = values.iter().map(|&i| args[i].clone()).collect();
        let es = self.args(&vals, ind)?;
        for (e, &i) in es.into_iter().zip(&values) {
            xs[i] = as_shape(e, shapes[i])?;
        }
        let call = format!("{name}({})", xs.join(", "));
        Ok(match ret {
            CType::Text => {
                let t = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_free(&{t});"));
                self.line(ind, &format!("{t} = {call};"));
                self.held(t, CType::Text)
            }
            c => E::new(call, kind_of(c)),
        })
    }

    /// The address a `&` argument passes: the caller's own variable, or a link it holds.
    fn link_arg(&mut self, a: &Value, c: CType, ind: usize) -> Result<String, String> {
        let v = match a.unspan() {
            // A text result's buffer: `{ __work = ""; OpCreateStack(__work) }`.
            Value::Block(b) => {
                let Some((last, rest)) = b.operators.split_last() else {
                    return Err("a link block with no operators".into());
                };
                self.nested(|w| rest.iter().try_for_each(|o| w.stmt(o, ind)))?;
                return self.link_arg(last, c, ind);
            }
            Value::Call(op, xs) if self.cx.data.def(*op).name() == "OpCreateStack" => {
                match xs.as_slice() {
                    [x] => match x.unspan() {
                        Value::Var(v) => *v,
                        _ => return Err("a link to a computed place".into()),
                    },
                    _ => return Err("OpCreateStack without one operand".into()),
                }
            }
            Value::Var(v) if self.links[*v as usize] => *v,
            Value::Call(op, _) => {
                return Err(format!("a link made by {}", self.cx.data.def(*op).name()));
            }
            other => return Err(format!("a link to {}", unsupported(other))),
        };
        if self.shapes[v as usize] != c || (c == CType::Text && !self.text_held(v)) {
            return Err("a link to a variable of another shape".into());
        }
        Ok(format!("&{}", self.var(v)))
    }

    fn one(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        let es = self.args(args, ind)?;
        let [a] = <[E; 1]>::try_from(es).map_err(|_| format!("{name} without one operand"))?;
        Ok(a)
    }

    fn two(&mut self, name: &str, args: &[Value], ind: usize) -> Result<(E, E), String> {
        let es = self.args(args, ind)?;
        let [a, b] = <[E; 2]>::try_from(es).map_err(|_| format!("{name} without two operands"))?;
        Ok((a, b))
    }

    fn operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        // The text writes through a `&` parameter: the C names the caller's text the same way.
        let name = match name {
            "OpAppendStackText" => "OpAppendText",
            "OpClearStackText" => "OpClearText",
            "OpFormatStackInt" => "OpFormatInt",
            "OpFormatStackFloat" => "OpFormatFloat",
            "OpFormatStackSingle" => "OpFormatSingle",
            "OpFormatStackText" => "OpFormatText",
            "OpFormatStackDatabase" => "OpFormatDatabase",
            n => n,
        };
        let arith = match name {
            "OpAddInt" | "OpAddLong" => Some("loft_add"),
            "OpMinInt" | "OpMinLong" => Some("loft_sub"),
            "OpMulInt" | "OpMulLong" => Some("loft_mul"),
            "OpDivInt" | "OpDivLong" => Some("loft_div"),
            "OpRemInt" | "OpRemLong" => Some("loft_rem"),
            "OpLandInt" | "OpLandLong" => Some("loft_and"),
            "OpLorInt" | "OpLorLong" => Some("loft_or"),
            "OpEorInt" | "OpEorLong" => Some("loft_xor"),
            "OpSLeftInt" | "OpSLeftLong" => Some("loft_shl"),
            "OpSRightInt" | "OpSRightLong" => Some("loft_shr"),
            _ => None,
        };
        if let Some(f) = arith {
            let (a, b) = self.two(name, args, ind)?;
            return Ok(E::new(
                format!("{f}({}, {})", as_int(a)?, as_int(b)?),
                K::Int,
            ));
        }
        let cmp = match name {
            "OpEqInt" | "OpEqLong" => Some("=="),
            "OpNeInt" | "OpNeLong" => Some("!="),
            "OpLtInt" | "OpLtLong" => Some("<"),
            "OpLeInt" | "OpLeLong" => Some("<="),
            _ => None,
        };
        if let Some(op) = cmp {
            let (a, b) = self.two(name, args, ind)?;
            return Ok(E::new(
                format!("({} {op} {})", as_int(a)?, as_int(b)?),
                K::Cond,
            ));
        }
        self.text_operator(name, args, ind)
    }

    /// The boolean and text operators, and printing.
    fn text_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match name {
            "OpEqBool" | "OpNeBool" => {
                let (a, b) = self.two(name, args, ind)?;
                let op = if name == "OpEqBool" { "==" } else { "!=" };
                Ok(E::new(
                    format!("({} {op} {})", as_bool(a)?, as_bool(b)?),
                    K::Cond,
                ))
            }
            "OpNot" => {
                let es = self.args(args, ind)?;
                let [a] =
                    <[E; 1]>::try_from(es).map_err(|_| "OpNot without one operand".to_string())?;
                Ok(E::new(format!("(!{})", as_cond(a)?), K::Cond))
            }
            "OpEqText" | "OpNeText" => {
                let (a, b) = self.two(name, args, ind)?;
                let neg = if name == "OpNeText" { "!" } else { "" };
                Ok(E::new(
                    format!("{neg}loft_str_eq({}, {})", as_view(a)?, as_view(b)?),
                    K::Cond,
                ))
            }
            "OpAppendText" => {
                let [dest, val] = args else {
                    return Err("OpAppendText without two operands".into());
                };
                let dest = self.text_dest(dest)?;
                let val = self.expr(val, ind)?;
                Ok(E::new(
                    format!("loft_text_concat(&{dest}, {})", as_view(val)?),
                    K::Void,
                ))
            }
            "OpFormatInt" | "OpFormatLong" => self.format_long(name, args, ind),
            "OpFormatText" => {
                let [dest, val, width, dir, token] = args else {
                    return Err("OpFormatText with an unexpected operand list".into());
                };
                let dest = self.text_dest(dest)?;
                let (Value::Int(dir), Value::Int(token)) = (dir.unspan(), token.unspan()) else {
                    return Err("OpFormatText with a computed format".into());
                };
                let (val, width) = self.two(name, &[(*val).clone(), (*width).clone()], ind)?;
                Ok(E::new(
                    format!(
                        "loft_format_text(&{dest}, {}, {}, {dir}, {token})",
                        as_view(val)?,
                        as_int(width)?
                    ),
                    K::Void,
                ))
            }
            "OpFormatDatabase" => self.format_database(args, ind),
            "OpFreeText" => {
                let [dest] = args else {
                    return Err("OpFreeText without one operand".into());
                };
                let dest = self.text_dest(dest)?;
                Ok(E::new(format!("loft_text_free(&{dest})"), K::Void))
            }
            "OpPrint" => {
                let es = self.args(args, ind)?;
                let [a] = <[E; 1]>::try_from(es)
                    .map_err(|_| "OpPrint without one operand".to_string())?;
                Ok(E::new(format!("loft_print({})", as_view(a)?), K::Void))
            }
            _ => self.scalar_operator(name, args, ind),
        }
    }

    /// An integer into a format string: `ops::format_long`.
    fn format_long(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        let [dest, val, radix, width, token, plus, note, dir] = args else {
            return Err(format!("{name} with an unexpected operand list"));
        };
        let dest = self.text_dest(dest)?;
        let (
            Value::Int(radix),
            Value::Int(token),
            Value::Boolean(plus),
            Value::Boolean(note),
            Value::Int(dir),
        ) = (
            radix.unspan(),
            token.unspan(),
            plus.unspan(),
            note.unspan(),
            dir.unspan(),
        )
        else {
            return Err(format!("{name} with a computed format"));
        };
        let (val, width) = self.two(name, &[(*val).clone(), (*width).clone()], ind)?;
        Ok(E::new(
            format!(
                "loft_format_long(&{dest}, {}, {radix}, {}, {token}, {}, {}, {dir})",
                as_int(val)?,
                as_int(width)?,
                u8::from(*plus),
                u8::from(*note)
            ),
            K::Void,
        ))
    }

    /// `{v}` of a record or a vector: the type's render function (`show.rs`).
    fn format_database(&mut self, args: &[Value], ind: usize) -> Result<E, String> {
        let [dest, val, tp, fmt] = args else {
            return Err("OpFormatDatabase with an unexpected operand list".into());
        };
        let dest = self.text_dest(dest)?;
        let tp = u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
        let fmt = Self::literal(fmt, "format")?;
        self.cx.show_type(tp)?;
        let val = self.expr(val, ind)?;
        Ok(E::new(
            format!(
                "{}(&{dest}, {}, {fmt}, 0)",
                super::Cx::show_fn(tp),
                as_ref(val)?
            ),
            K::Void,
        ))
    }

    /// The conversions and the text measures.
    fn scalar_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match name {
            "OpLengthText" | "OpSizeText" => {
                let es = self.args(args, ind)?;
                let [a] =
                    <[E; 1]>::try_from(es).map_err(|_| format!("{name} without one operand"))?;
                let f = if name == "OpLengthText" {
                    "loft_text_length"
                } else {
                    "loft_text_size"
                };
                Ok(E::new(format!("{f}({})", as_view(a)?), K::Int))
            }
            "OpRangeDefault" => {
                let [v, lo, hi, dflt] = args else {
                    return Err("OpRangeDefault with an unexpected operand list".into());
                };
                let lo = Self::literal(lo, "range bound")?;
                let hi = Self::literal(hi, "range bound")?;
                let dflt = Self::literal(dflt, "range default")?;
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!(
                        "loft_range_default({}, {}, {}, {})",
                        as_int(v)?,
                        c_long(lo),
                        c_long(hi),
                        c_long(dflt)
                    ),
                    K::Int,
                ))
            }
            "OpMinSingleInt" | "OpMinSingleLong" => {
                let es = self.args(args, ind)?;
                let [a] =
                    <[E; 1]>::try_from(es).map_err(|_| format!("{name} without one operand"))?;
                Ok(E::new(format!("loft_neg({})", as_int(a)?), K::Int))
            }
            "OpConvBoolFromInt" | "OpConvBoolFromLong" => {
                let es = self.args(args, ind)?;
                let [a] =
                    <[E; 1]>::try_from(es).map_err(|_| format!("{name} without one operand"))?;
                Ok(E::new(format!("({} != LOFT_NULL)", as_int(a)?), K::Cond))
            }
            "OpConvIntFromNull" | "OpConvLongFromNull" => Ok(E::new("LOFT_NULL", K::Int)),
            "OpConvEnumFromNull" => Ok(E::new("(uint8_t) 255", K::Enum)),
            "OpConstEnum" => self.expr(args.first().ok_or("OpConstEnum without a value")?, ind),
            "OpConvIntFromEnum" => {
                let a = self.one(name, args, ind)?;
                let a = as_enum(a)?;
                Ok(E::new(
                    format!("({a} == 255 ? LOFT_NULL : (int64_t) {a})"),
                    K::Int,
                ))
            }
            "OpCastEnumFromInt" => {
                let a = self.one(name, args, ind)?;
                let a = as_int(a)?;
                Ok(E::new(
                    format!("({a} == LOFT_NULL ? (uint8_t) 255 : (uint8_t) {a})"),
                    K::Enum,
                ))
            }
            "OpConvBoolFromEnum" => {
                let a = self.one(name, args, ind)?;
                let a = as_enum(a)?;
                Ok(E::new(format!("({a} != 255 && {a} != 0)"), K::Cond))
            }
            "OpCastTextFromEnum" => {
                let [v, tp] = args else {
                    return Err("OpCastTextFromEnum with an unexpected operand list".into());
                };
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type out of range".to_string())?;
                let names = self.cx.enum_names(tp);
                let v = self.expr(v, ind)?;
                let t = self.temp(CType::Enum);
                self.line(ind, &format!("{t} = {};", as_enum(v)?));
                Ok(E::new(
                    format!(
                        "({t} == 0 || {t} == 255 ? loft_null_text() : loft_lit({names}[{t}], strlen({names}[{t}])))"
                    ),
                    K::View,
                ))
            }
            _ => self.float_operator(name, args, ind),
        }
    }

    /// The float and single operators: `Float` names a double, `Single` a float.
    fn float_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        /// `(operator, single?, C spelling)` for the binary arithmetic.
        const INFIX: &[(&str, bool, &str)] = &[
            ("OpAddFloat", false, "+"),
            ("OpMinFloat", false, "-"),
            ("OpMulFloat", false, "*"),
            ("OpDivFloat", false, "/"),
            ("OpDivFloatNullable", false, "/"),
            ("OpAddSingle", true, "+"),
            ("OpMinSingle", true, "-"),
            ("OpMulSingle", true, "*"),
            ("OpDivSingle", true, "/"),
            ("OpDivSingleNullable", true, "/"),
        ];
        /// `(operator, single?, C function)` for the calls of two operands.
        const CALL2: &[(&str, bool, &str)] = &[
            ("OpRemFloat", false, "fmod"),
            ("OpRemFloatNullable", false, "fmod"),
            ("OpPowFloat", false, "pow"),
            ("OpRemSingle", true, "fmodf"),
            ("OpRemSingleNullable", true, "fmodf"),
            ("OpPowSingle", true, "powf"),
        ];
        /// `(operator, single?, C comparison, negated?)`.
        const CMP: &[(&str, bool, &str, bool)] = &[
            ("OpEqFloat", false, "loft_feq", false),
            ("OpNeFloat", false, "loft_feq", true),
            ("OpLtFloat", false, "loft_flt", false),
            ("OpLeFloat", false, "loft_fle", false),
            ("OpEqSingle", true, "loft_feq", false),
            ("OpNeSingle", true, "loft_feq", true),
            ("OpLtSingle", true, "loft_flt", false),
            ("OpLeSingle", true, "loft_fle", false),
        ];
        let shape = |single: bool| -> (K, fn(E) -> Result<String, String>) {
            if single {
                (K::Single, as_single)
            } else {
                (K::Float, as_float)
            }
        };
        if let Some((_, single, op)) = INFIX.iter().find(|(n, _, _)| *n == name) {
            let (k, conv) = shape(*single);
            let (a, b) = self.two(name, args, ind)?;
            return Ok(E::new(format!("({} {op} {})", conv(a)?, conv(b)?), k));
        }
        if let Some((_, single, f)) = CALL2.iter().find(|(n, _, _)| *n == name) {
            let (k, conv) = shape(*single);
            let (a, b) = self.two(name, args, ind)?;
            return Ok(E::new(format!("{f}({}, {})", conv(a)?, conv(b)?), k));
        }
        if let Some((_, single, f, neg)) = CMP.iter().find(|(n, _, _, _)| *n == name) {
            let (_, conv) = shape(*single);
            let (a, b) = self.two(name, args, ind)?;
            let neg = if *neg { "!" } else { "" };
            return Ok(E::new(
                format!("{neg}{f}({}, {})", conv(a)?, conv(b)?),
                K::Cond,
            ));
        }
        let unary = match name {
            "OpMinSingleFloat" => Some((false, "-")),
            "OpMinSingleSingle" => Some((true, "-")),
            "OpAbsFloat" => Some((false, "fabs")),
            "OpAbsSingle" => Some((true, "fabsf")),
            _ => None,
        };
        if let Some((single, f)) = unary {
            let (k, conv) = shape(single);
            let a = self.one(name, args, ind)?;
            return Ok(E::new(format!("({f}({}))", conv(a)?), k));
        }
        self.float_conversion(name, args, ind)
    }

    /// The conversions to and from floats, their math functions and their fields.
    fn float_conversion(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpConvFloatFromNull", []) => Ok(E::new("(double) NAN", K::Float)),
            ("OpConvSingleFromNull", []) => Ok(E::new("(float) NAN", K::Single)),
            ("OpConvFloatFromInt" | "OpConvFloatFromLong", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("loft_i2f({})", as_int(a)?), K::Float))
            }
            ("OpConvSingleFromInt", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("loft_i2s({})", as_int(a)?), K::Single))
            }
            ("OpCastIntFromFloat", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("loft_f2i({})", as_float(a)?), K::Int))
            }
            ("OpCastIntFromSingle", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(
                    format!("loft_f2i((double) {})", as_single(a)?),
                    K::Int,
                ))
            }
            ("OpConvBoolFromFloat", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("(!isnan({}))", as_float(a)?), K::Cond))
            }
            ("OpConvBoolFromSingle", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("(!isnan({}))", as_single(a)?), K::Cond))
            }
            ("OpCastSingleFromFloat", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("((float) {})", as_float(a)?), K::Single))
            }
            ("OpConvFloatFromSingle", [_]) => {
                let a = self.one(name, args, ind)?;
                Ok(E::new(format!("((double) {})", as_single(a)?), K::Float))
            }
            ("OpMathPiFloat", []) => Ok(E::new("M_PI", K::Float)),
            ("OpMathEFloat", []) => Ok(E::new("M_E", K::Float)),
            ("OpMathFuncFloat" | "OpMathFuncSingle", [id, v]) => {
                let id = Self::literal(id, "math function")?;
                let a = self.expr(v, ind)?;
                Ok(if name.ends_with("Single") {
                    E::new(format!("loft_math1s({id}, {})", as_single(a)?), K::Single)
                } else {
                    E::new(format!("loft_math1({id}, {})", as_float(a)?), K::Float)
                })
            }
            ("OpMathFunc2Float" | "OpMathFunc2Single", [id, a, b]) => {
                let id = Self::literal(id, "math function")?;
                let (a, b) = self.two(name, &[(*a).clone(), (*b).clone()], ind)?;
                Ok(if name.ends_with("Single") {
                    E::new(
                        format!("loft_math2s({id}, {}, {})", as_single(a)?, as_single(b)?),
                        K::Single,
                    )
                } else {
                    E::new(
                        format!("loft_math2({id}, {}, {})", as_float(a)?, as_float(b)?),
                        K::Float,
                    )
                })
            }
            ("OpFormatFloat" | "OpFormatSingle", [dest, val, width, prec, token, plus, dir]) => {
                let dest = self.text_dest(dest)?;
                let (Value::Int(token), Value::Boolean(plus)) = (token.unspan(), plus.unspan())
                else {
                    return Err(format!("{name} with a computed format"));
                };
                let es = self.args(
                    &[
                        (*val).clone(),
                        (*width).clone(),
                        (*prec).clone(),
                        (*dir).clone(),
                    ],
                    ind,
                )?;
                let [val, width, prec, dir] =
                    <[E; 4]>::try_from(es).map_err(|_| "format operands".to_string())?;
                let (val, single) = if name == "OpFormatSingle" {
                    (format!("(double) {}", as_single(val)?), 1)
                } else {
                    (as_float(val)?, 0)
                };
                Ok(E::new(
                    format!(
                        "loft_format_float(&{dest}, {val}, {}, {}, {token}, {}, (int) {}, {single})",
                        as_int(width)?,
                        as_int(prec)?,
                        u8::from(*plus),
                        as_int(dir)?
                    ),
                    K::Void,
                ))
            }
            _ => self.store_operator(name, args, ind),
        }
    }

    /// An integer literal operand of an operator: a field offset, a size, a type number.
    fn literal(v: &Value, what: &str) -> Result<i64, String> {
        match v.unspan() {
            Value::Int(i) => Ok(i64::from(*i)),
            Value::Long(l) => Ok(*l),
            _ => Err(format!("a computed {what}")),
        }
    }

    /// The operators over stores: records and their fields, vectors.
    fn store_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpDatabase", [Value::Var(v), tp]) => {
                let tp = Self::literal(tp, "type")?;
                let tp = u16::try_from(tp).map_err(|_| "a type number out of range".to_string())?;
                let size = self.cx.stores.enum_parent_size(tp);
                let var = self.var(*v);
                Ok(E::new(format!("loft_database(&{var}, {size})"), K::Void))
            }
            ("OpFreeRef", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(E::new(format!("loft_free_ref({})", as_ref(r)?), K::Void))
            }
            ("OpRefIsNull", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("({}.store_nr == 0xFFFF)", as_ref(r)?),
                    K::Cond,
                ))
            }
            ("OpNullRefSentinel", []) => Ok(E::new("loft_ref_null()", K::Ref)),
            ("OpConvBoolFromRef", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(E::new(format!("({}.rec != 0)", as_ref(r)?), K::Cond))
            }
            ("OpRefAlias", [r]) => self.expr(r, ind),
            ("OpFreeRefIfDistinct", [_, _]) => {
                let (p, w) = self.two(name, args, ind)?;
                Ok(E::new(
                    format!("loft_free_displaced({}, {})", as_ref(p)?, as_ref(w)?),
                    K::Void,
                ))
            }
            ("OpGetInt", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_int({}, {fld})", as_ref(r)?),
                    K::Int,
                ))
            }
            ("OpSetInt" | "OpSetInt4", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                let f = if name == "OpSetInt" {
                    "loft_set_int"
                } else {
                    "loft_set_int4"
                };
                Ok(E::new(
                    format!("{f}({}, {fld}, {})", as_ref(r)?, as_int(val)?),
                    K::Void,
                ))
            }
            ("OpCopyRecord", [src, dst, tp]) => {
                let raw = Self::literal(tp, "type")?;
                let raw =
                    u16::try_from(raw).map_err(|_| "a type number out of range".to_string())?;
                let tp = self.copyable(raw & crate::keys::COPY_TP_MASK)?;
                let free = u8::from(raw & crate::keys::COPY_FREE_SOURCE != 0);
                let (src, dst) = self.two(name, &[(*src).clone(), (*dst).clone()], ind)?;
                Ok(E::new(
                    format!(
                        "loft_copy_record(loft_types, {tp}, {}, {}, {free})",
                        as_ref(src)?,
                        as_ref(dst)?
                    ),
                    K::Void,
                ))
            }
            ("OpGetField", [r, fld, _tp]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(format!("loft_field({}, {fld})", as_ref(r)?), K::Ref))
            }
            _ => self.field_operator(name, args, ind),
        }
    }

    /// The enum and text fields of a record.
    fn field_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpGetEnum", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_enum({}, {fld})", as_ref(r)?),
                    K::Enum,
                ))
            }
            ("OpSetEnum", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_set_byte({}, {fld}, {})", as_ref(r)?, as_enum(val)?),
                    K::Void,
                ))
            }
            ("OpGetText", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                let t = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_free(&{t});"));
                self.line(
                    ind,
                    &format!("{t} = loft_text_own(loft_get_text({}, {fld}));", as_ref(r)?),
                );
                Ok(self.held(t, CType::Text))
            }
            ("OpSetText" | "OpSetTextReplace", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_set_text({}, {fld}, {})", as_ref(r)?, as_view(val)?),
                    K::Void,
                ))
            }
            ("OpGetFloat" | "OpGetSingle", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                let (f, k) = if name == "OpGetFloat" {
                    ("loft_get_float", K::Float)
                } else {
                    ("loft_get_single", K::Single)
                };
                Ok(E::new(format!("{f}({}, {fld})", as_ref(r)?), k))
            }
            ("OpSetFloat" | "OpSetSingle", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                let (f, v) = if name == "OpSetFloat" {
                    ("loft_set_float", as_float(val)?)
                } else {
                    ("loft_set_single", as_single(val)?)
                };
                Ok(E::new(format!("{f}({}, {fld}, {v})", as_ref(r)?), K::Void))
            }
            ("OpPushBoolean", [v, val]) => {
                let (v, val) = self.two(name, &[(*v).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_byte({}, {})", as_ref(v)?, as_bool(val)?),
                    K::Void,
                ))
            }
            ("OpPushFloat" | "OpPushSingle", [v, val]) => {
                let (v, val) = self.two(name, &[(*v).clone(), (*val).clone()], ind)?;
                let (f, x) = if name == "OpPushFloat" {
                    ("loft_vec_push_float", as_float(val)?)
                } else {
                    ("loft_vec_push_single", as_single(val)?)
                };
                Ok(E::new(format!("{f}({}, {x})", as_ref(v)?), K::Void))
            }
            _ => self.narrow_operator(name, args, ind),
        }
    }

    /// The narrow integer fields: bytes, shorts and 4-byte integers, each in its encoding.
    fn narrow_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpGetByte" | "OpGetByteNullable" | "OpGetShort" | "OpGetShortRaw", [r, fld, min]) => {
                let fld = Self::literal(fld, "field")?;
                let min = Self::literal(min, "minimum")?;
                let f = match name {
                    "OpGetByte" => "loft_get_byte",
                    "OpGetByteNullable" => "loft_get_byte_nullable",
                    "OpGetShort" => "loft_get_short",
                    _ => "loft_get_short_raw",
                };
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("{f}({}, {fld}, {})", as_ref(r)?, c_long(min)),
                    K::Int,
                ))
            }
            (
                "OpSetByte" | "OpSetByteNullable" | "OpSetShort" | "OpSetShortRaw",
                [r, fld, min, val],
            ) => {
                let fld = Self::literal(fld, "field")?;
                let min = Self::literal(min, "minimum")?;
                let f = match name {
                    "OpSetByte" => "loft_set_byte_v",
                    "OpSetByteNullable" => "loft_set_byte_nullable",
                    "OpSetShort" => "loft_set_short",
                    _ => "loft_set_short_raw",
                };
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!(
                        "{f}({}, {fld}, {}, {})",
                        as_ref(r)?,
                        c_long(min),
                        as_int(val)?
                    ),
                    K::Void,
                ))
            }
            ("OpGetInt4", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_int4({}, {fld})", as_ref(r)?),
                    K::Int,
                ))
            }
            _ => self.vector_operator(name, args, ind),
        }
    }

    /// The vector `OpNewRecord`/`OpFinishRecord` add to, and its element type: the parent
    /// itself when `fld` is `u16::MAX`, else the parent's field — a plain vector in both cases
    /// (`Stores::record_new`'s two short paths); any other collection is refused.
    fn record_collection(
        &mut self,
        data: &Value,
        ptp: &Value,
        fld: &Value,
        ind: usize,
    ) -> Result<(String, u16), String> {
        use crate::database::Parts;
        let stores = self.cx.stores;
        let ptp = u16::try_from(Self::literal(ptp, "type")?)
            .map_err(|_| "a type out of range".to_string())?;
        let fld = u16::try_from(Self::literal(fld, "field")?)
            .map_err(|_| "a field out of range".to_string())?;
        let data = self.expr(data, ind)?;
        let data = as_ref(data)?;
        if fld == u16::MAX {
            let Parts::Vector(c) = stores.types[ptp as usize].parts else {
                return Err("a new record in a collection other than a vector".into());
            };
            return Ok((data, c));
        }
        if !matches!(stores.types[ptp as usize].parts, Parts::Struct(_)) {
            return Err("a new record in a field of a non-struct".into());
        }
        let vtp = stores.field_type(ptp, fld);
        let Some(Parts::Vector(c)) = stores.types.get(vtp as usize).map(|t| &t.parts) else {
            return Err("a new record in a field that is not a vector".into());
        };
        let pos = stores.field_position(ptp, fld);
        Ok((format!("loft_field({data}, {pos})"), *c))
    }

    /// The operators over vectors.
    fn vector_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpNewRecord", [data, ptp, fld]) => {
                let (v, elem) = self.record_collection(data, ptp, fld, ind)?;
                let size = self.cx.stores.size(elem);
                let (image, n) = self.cx.default_image(elem);
                let t = self.temp(CType::Ref);
                self.line(ind, &format!("{t} = loft_vec_append({v}, {size});"));
                self.line(ind, &format!("loft_fill({t}, {image}, {n});"));
                Ok(E::new(t, K::Ref))
            }
            ("OpFinishRecord", [data, _rec, ptp, fld]) => {
                let (v, _) = self.record_collection(data, ptp, fld, ind)?;
                Ok(E::new(format!("loft_vec_finish({v})"), K::Void))
            }
            ("OpClearVector", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_vec_clear({})", as_ref(v)?), K::Void))
            }
            ("OpAppendVector", [v, other, tp]) => {
                let tp = u16::try_from(Self::literal(tp, "element type")?)
                    .map_err(|_| "an element type out of range")?;
                if self.cx.stores.is_linked(tp) {
                    return Err("an append of a linked element type".into());
                }
                self.cx.copy_type(tp)?;
                let (v, other) = self.two(name, &[(*v).clone(), (*other).clone()], ind)?;
                Ok(E::new(
                    format!(
                        "loft_vec_add(loft_types, {tp}, {}, {})",
                        as_ref(v)?,
                        as_ref(other)?
                    ),
                    K::Void,
                ))
            }
            ("OpLengthVector", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_vec_len({})", as_ref(v)?), K::Int))
            }
            ("OpPreAllocVector", [v, count, size]) => {
                let count = Self::literal(count, "count")?;
                let size = Self::literal(size, "element size")?;
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!("loft_vec_prealloc({}, {count}, {size})", as_ref(v)?),
                    K::Void,
                ))
            }
            ("OpPushEnum", [v, val]) => {
                let (v, val) = self.two(name, &[(*v).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_byte({}, {})", as_ref(v)?, as_enum(val)?),
                    K::Void,
                ))
            }
            ("OpPushInt", [v, val]) => {
                let (v, val) = self.two(name, &[(*v).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_int({}, {})", as_ref(v)?, as_int(val)?),
                    K::Void,
                ))
            }
            ("OpGetVector" | "OpGetVectorNullable", [v, size, idx]) => {
                let size = Self::literal(size, "element size")?;
                let (v, idx) = self.two(name, &[(*v).clone(), (*idx).clone()], ind)?;
                Ok(E::new(
                    format!("loft_vec_get({}, {size}, {})", as_ref(v)?, as_int(idx)?),
                    K::Ref,
                ))
            }
            _ => self.slot_operator(name, args, ind),
        }
    }

    /// The character operators: a code point, 0 the null character.
    fn char_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpConvCharacterFromInt", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_char_of({})", as_int(v)?), K::Char))
            }
            ("OpConvIntFromCharacter", [c]) => {
                let c = self.expr(c, ind)?;
                Ok(E::new(format!("loft_char_int({})", as_char(c)?), K::Int))
            }
            ("OpConvBoolFromCharacter", [c]) => {
                let c = self.expr(c, ind)?;
                Ok(E::new(format!("({} != 0)", as_char(c)?), K::Cond))
            }
            ("OpLengthCharacter", [c]) => {
                let c = self.expr(c, ind)?;
                Ok(E::new(format!("loft_char_len({})", as_char(c)?), K::Int))
            }
            ("OpTextCharacter" | "OpTextCharacterNullable", [_, _]) => {
                let (t, i) = self.two(name, args, ind)?;
                Ok(E::new(
                    format!("loft_text_char({}, {})", as_view(t)?, as_int(i)?),
                    K::Char,
                ))
            }
            ("OpAppendCharacter" | "OpAppendStackCharacter", [dest, c]) => {
                let dest = self.text_dest(dest)?;
                let c = self.expr(c, ind)?;
                Ok(E::new(
                    format!("loft_push_code(&{dest}, {})", as_char(c)?),
                    K::Void,
                ))
            }
            ("OpGetCharacter", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_char({}, {fld})", as_ref(r)?),
                    K::Char,
                ))
            }
            ("OpSetCharacter", [r, fld, c]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, c) = self.two(name, &[(*r).clone(), (*c).clone()], ind)?;
                Ok(E::new(
                    format!("loft_set_char({}, {fld}, {})", as_ref(r)?, as_char(c)?),
                    K::Void,
                ))
            }
            ("OpPushCharacter", [v, c]) => {
                let (v, c) = self.two(name, &[(*v).clone(), (*c).clone()], ind)?;
                Ok(E::new(
                    format!("loft_vec_push_char({}, {})", as_ref(v)?, as_char(c)?),
                    K::Void,
                ))
            }
            _ => Err(format!("operator {name}")),
        }
    }

    /// Booleans and bytes in records and vectors, absent vectors, store identity, text
    /// clears and the text null.
    fn slot_operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        match (name, args) {
            ("OpGetBoolean", [r, fld]) => {
                let fld = Self::literal(fld, "field")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_get_bool({}, {fld})", as_ref(r)?),
                    K::Bool,
                ))
            }
            ("OpSetBoolean", [r, fld, val]) => {
                let fld = Self::literal(fld, "field")?;
                let (r, val) = self.two(name, &[(*r).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!("loft_set_byte({}, {fld}, {})", as_ref(r)?, as_bool(val)?),
                    K::Void,
                ))
            }
            ("OpPushByte", [v, min, val]) => {
                let min = Self::literal(min, "byte minimum")?;
                let (v, val) = self.two(name, &[(*v).clone(), (*val).clone()], ind)?;
                Ok(E::new(
                    format!(
                        "loft_vec_push_byte({}, loft_byte_raw({min}, {}))",
                        as_ref(v)?,
                        as_int(val)?
                    ),
                    K::Void,
                ))
            }
            ("OpVectorIsNull", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_vec_absent({})", as_ref(v)?), K::Cond))
            }
            ("OpLengthSorted", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_vec_len({})", as_ref(v)?), K::Int))
            }
            ("OpDistinctStore", [_, _]) => {
                let (a, b) = self.two(name, args, ind)?;
                Ok(E::new(
                    format!("({}.store_nr != {}.store_nr)", as_ref(a)?, as_ref(b)?),
                    K::Cond,
                ))
            }
            ("OpClear", [r, tp]) => {
                let tp =
                    u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
                self.cx.copy_type(tp)?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_remove_claims(loft_types, {tp}, {})", as_ref(r)?),
                    K::Void,
                ))
            }
            ("OpClearText", [dest]) => {
                let dest = self.text_dest(dest)?;
                Ok(E::new(format!("{dest}.len = 0"), K::Void))
            }
            ("OpConvTextFromNull", []) => Ok(E::new("loft_null_text()", K::View)),
            ("OpConvCharacterFromNull", []) => Ok(E::new("((uint32_t) 0)", K::Char)),
            _ if name.contains("Character") => self.char_operator(name, args, ind),
            ("OpCastTextFromBool", [b]) => {
                let b = self.expr(b, ind)?;
                Ok(E::new(format!("loft_bool_text({})", as_bool(b)?), K::View))
            }
            ("OpConvBoolFromText", [t]) => {
                let t = self.expr(t, ind)?;
                Ok(E::new(
                    format!("(!loft_str_is_null({}))", as_view(t)?),
                    K::Cond,
                ))
            }
            _ => Err(format!("operator {name}")),
        }
    }
}

/// `n` as a C `int64_t` constant: the null as `LOFT_NULL`, since `INT64_C` cannot spell it.
fn c_long(n: i64) -> String {
    if n == i64::MIN {
        "LOFT_NULL".into()
    } else {
        format!("INT64_C({n})")
    }
}

fn as_int(e: E) -> Result<String, String> {
    match e.kind {
        K::Int => Ok(e.code),
        k => Err(format!("an integer expected, a {k:?} found")),
    }
}

fn as_cond(e: E) -> Result<String, String> {
    match e.kind {
        K::Cond => Ok(e.code),
        K::Bool => Ok(format!("({} == 1)", e.code)),
        k => Err(format!("a condition expected, a {k:?} found")),
    }
}

fn as_bool(e: E) -> Result<String, String> {
    match e.kind {
        K::Bool => Ok(e.code),
        K::Cond => Ok(format!("(uint8_t) ({})", bare(&e.code))),
        k => Err(format!("a boolean expected, a {k:?} found")),
    }
}

fn as_ref(e: E) -> Result<String, String> {
    match e.kind {
        K::Ref => Ok(e.code),
        k => Err(format!("a reference expected, a {k:?} found")),
    }
}

fn as_tuple(e: E, id: u32) -> Result<String, String> {
    match e.kind {
        K::Tup(i) if i == id => Ok(e.code),
        k => Err(format!("a tuple expected, a {k:?} found")),
    }
}

fn as_enum(e: E) -> Result<String, String> {
    match e.kind {
        K::Enum => Ok(e.code),
        k => Err(format!("an enum value expected, a {k:?} found")),
    }
}

fn as_float(e: E) -> Result<String, String> {
    match e.kind {
        K::Float => Ok(e.code),
        k => Err(format!("a float expected, a {k:?} found")),
    }
}

fn as_single(e: E) -> Result<String, String> {
    match e.kind {
        K::Single => Ok(e.code),
        k => Err(format!("a single expected, a {k:?} found")),
    }
}

fn as_char(e: E) -> Result<String, String> {
    match e.kind {
        K::Char => Ok(e.code),
        k => Err(format!("a character expected, a {k:?} found")),
    }
}

fn as_view(e: E) -> Result<String, String> {
    match e.kind {
        K::View => Ok(e.code),
        k => Err(format!("a text expected, a {k:?} found")),
    }
}

/// The value of `e` converted to shape `c`, for a parameter or a store.
fn as_shape(e: E, c: CType) -> Result<String, String> {
    match c {
        CType::Int => as_int(e),
        CType::Bool => as_bool(e),
        CType::Text => as_view(e),
        CType::Ref => as_ref(e),
        CType::Tuple(id) => as_tuple(e, id),
        CType::Enum => as_enum(e),
        CType::Float => as_float(e),
        CType::Single => as_single(e),
        CType::Char => as_char(e),
        CType::Void => Err("a void value".into()),
    }
}

/// The operators of each C shape, for the operators this backend writes.
const OP_INT: &[&str] = &[
    "OpAddInt",
    "OpAddLong",
    "OpMinInt",
    "OpMinLong",
    "OpMulInt",
    "OpMulLong",
    "OpDivInt",
    "OpDivLong",
    "OpRemInt",
    "OpRemLong",
    "OpGetInt",
    "OpLengthVector",
    "OpLengthText",
    "OpSizeText",
    "OpMinSingleInt",
    "OpMinSingleLong",
    "OpConvIntFromNull",
    "OpConvLongFromNull",
    "OpConvIntFromEnum",
    "OpLengthSorted",
    "OpRangeDefault",
    "OpGetByte",
    "OpGetByteNullable",
    "OpGetShort",
    "OpGetShortRaw",
    "OpGetInt4",
    "OpLandInt",
    "OpLandLong",
    "OpLorInt",
    "OpLorLong",
    "OpEorInt",
    "OpEorLong",
    "OpSLeftInt",
    "OpSLeftLong",
    "OpSRightInt",
    "OpSRightLong",
    "OpConvIntFromCharacter",
    "OpLengthCharacter",
];
const OP_BOOL: &[&str] = &[
    "OpEqInt",
    "OpEqLong",
    "OpNeInt",
    "OpNeLong",
    "OpLtInt",
    "OpLtLong",
    "OpLeInt",
    "OpLeLong",
    "OpEqBool",
    "OpNeBool",
    "OpNot",
    "OpEqText",
    "OpNeText",
    "OpRefIsNull",
    "OpConvBoolFromInt",
    "OpConvBoolFromLong",
    "OpConvBoolFromEnum",
    "OpConvBoolFromRef",
    "OpGetBoolean",
    "OpVectorIsNull",
    "OpDistinctStore",
    "OpConvBoolFromText",
    "OpConvBoolFromCharacter",
];
const OP_REF: &[&str] = &[
    "OpGetField",
    "OpGetVector",
    "OpGetVectorNullable",
    "OpNullRefSentinel",
];
const OP_ENUM: &[&str] = &[
    "OpGetEnum",
    "OpCastEnumFromInt",
    "OpConvEnumFromNull",
    "OpConstEnum",
];
const OP_TEXT: &[&str] = &[
    "OpGetText",
    "OpCastTextFromEnum",
    "OpConvTextFromNull",
    "OpCastTextFromBool",
];
const OP_FLOAT: &[&str] = &[
    "OpAddFloat",
    "OpMinFloat",
    "OpMulFloat",
    "OpDivFloat",
    "OpDivFloatNullable",
    "OpRemFloat",
    "OpRemFloatNullable",
    "OpPowFloat",
    "OpMinSingleFloat",
    "OpAbsFloat",
    "OpConvFloatFromNull",
    "OpConvFloatFromInt",
    "OpConvFloatFromLong",
    "OpConvFloatFromSingle",
    "OpMathPiFloat",
    "OpMathEFloat",
    "OpMathFuncFloat",
    "OpMathFunc2Float",
    "OpGetFloat",
];
const OP_SINGLE: &[&str] = &[
    "OpAddSingle",
    "OpMinSingle",
    "OpMulSingle",
    "OpDivSingle",
    "OpDivSingleNullable",
    "OpRemSingle",
    "OpRemSingleNullable",
    "OpPowSingle",
    "OpMinSingleSingle",
    "OpAbsSingle",
    "OpConvSingleFromNull",
    "OpConvSingleFromInt",
    "OpCastSingleFromFloat",
    "OpMathFuncSingle",
    "OpMathFunc2Single",
    "OpGetSingle",
];
const OP_FCMP: &[&str] = &[
    "OpEqFloat",
    "OpNeFloat",
    "OpLtFloat",
    "OpLeFloat",
    "OpEqSingle",
    "OpNeSingle",
    "OpLtSingle",
    "OpLeSingle",
    "OpConvBoolFromFloat",
    "OpConvBoolFromSingle",
];
const OP_FINT: &[&str] = &["OpCastIntFromFloat", "OpCastIntFromSingle"];
const OP_CHAR: &[&str] = &[
    "OpConvCharacterFromInt",
    "OpConvCharacterFromNull",
    "OpTextCharacter",
    "OpTextCharacterNullable",
    "OpGetCharacter",
];

/// The C shape an operator's value is held in, for the operators this backend writes.
fn op_shape(name: &str) -> Option<CType> {
    let shapes = [
        (OP_INT, CType::Int),
        (OP_BOOL, CType::Bool),
        (OP_REF, CType::Ref),
        (OP_ENUM, CType::Enum),
        (OP_TEXT, CType::Text),
        (OP_FLOAT, CType::Float),
        (OP_SINGLE, CType::Single),
        (OP_FCMP, CType::Bool),
        (OP_FINT, CType::Int),
        (OP_CHAR, CType::Char),
    ];
    shapes
        .iter()
        .find(|(names, _)| names.contains(&name))
        .map(|(_, c)| *c)
}

/// `s` as a C string literal: printable ASCII as itself, every other byte as an octal escape
/// (three digits, so the next character can never extend it), `?` escaped against trigraphs.
pub(crate) fn c_string(s: &str) -> String {
    let mut out = String::from("\"");
    for b in s.bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'?' => out.push_str("\\?"),
            0x20..=0x7e => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out.push('"');
    out
}

/// `c` without one pair of outer parentheses that enclose all of it: `if ((a == b))` is a
/// warning in every C compiler that reads it as a mistyped assignment.
fn bare(c: &str) -> &str {
    let Some(inner) = c.strip_prefix('(').and_then(|s| s.strip_suffix(')')) else {
        return c;
    };
    let mut depth = 0i32;
    for ch in inner.chars() {
        depth += match ch {
            '(' => 1,
            ')' => -1,
            _ => 0,
        };
        if depth < 0 {
            return c;
        }
    }
    if depth == 0 { inner } else { c }
}

fn unsupported(v: &Value) -> String {
    let kind = format!("{v:?}");
    let kind = kind.split(['(', ' ', '{']).next().unwrap_or("value");
    format!("the construct {kind}")
}
