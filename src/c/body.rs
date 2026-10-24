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
    nparams: usize,
    ret: CType,
    body: String,
    /// Every temporary: its name and shape, declared at the top.
    temps: Vec<(String, CType)>,
    /// The owned text temporaries the current statement made.
    pending: Vec<String>,
    /// The label number of each enclosing loop, innermost last.
    loops: Vec<u32>,
    labels: u32,
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
            nparams: def.attributes().len(),
            ret,
            body: String::new(),
            temps: Vec::new(),
            pending: Vec::new(),
            loops: Vec::new(),
            labels: 0,
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
        for v in self.nparams..self.shapes.len() {
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
        var_name(self.vars.name(v), v)
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
        let mut texts: Vec<String> = (self.nparams..self.shapes.len())
            .filter(|&v| self.shapes[v] == CType::Text)
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
                if c == CType::Text {
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
        }
        Ok(())
    }

    fn null_into(&mut self, target: &str, c: CType, ind: usize) {
        let s = match c {
            CType::Int => format!("{target} = LOFT_NULL;"),
            CType::Bool => format!("{target} = LOFT_BOOL_NULL;"),
            CType::Text => format!("loft_text_set(&{target}, loft_null_text());"),
            CType::Ref => format!("{target} = loft_ref_null();"),
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

    /// The byte size of record type `tp`, when copying its bytes copies all of it.
    fn copyable(&self, tp: u16) -> Result<u32, String> {
        if let Some(kind) = self.cx.record_owns(tp) {
            return Err(format!("a copy of a record that owns a {kind}"));
        }
        Ok(u32::from(self.cx.stores.size(tp)))
    }

    /// `var = x`: a copy where loft copies a record, else the plain assignment.
    fn set_stmt(&mut self, var: u16, x: &Value, ind: usize) -> Result<(), String> {
        let c = self.shapes[var as usize];
        if (var as usize) < self.nparams && c == CType::Text {
            return Err("assigns a text parameter".into());
        }
        let name = self.var(var);
        if let Some((tp, frees)) = self.call_copy(var, x)? {
            let bytes = self.copyable(tp)?;
            let e = self.expr(x, ind)?;
            let r = self.temp(CType::Ref);
            self.line(ind, &format!("{r} = {};", as_ref(e)?));
            self.line(ind, &format!("loft_bind_copy(&{name}, {r}, {bytes});"));
            if frees {
                self.line(
                    ind,
                    &format!("if ({r}.store_nr != {name}.store_nr) loft_free_ref({r});"),
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
            self.line(ind, &format!("loft_bind_copy(&{name}, {r}, {bytes});"));
            self.flush(ind);
            return Ok(());
        }
        if let Some((src, tp)) = self.whole_value_bind(var, x) {
            let bytes = self.copyable(tp)?;
            let src = self.var(src);
            self.line(ind, &format!("loft_bind_copy(&{name}, {src}, {bytes});"));
            return Ok(());
        }
        self.assign(x, &name, c, ind)?;
        self.flush(ind);
        Ok(())
    }

    fn block_stmt(&mut self, b: &Block, ind: usize) -> Result<(), String> {
        b.operators.iter().try_for_each(|o| self.stmt(o, ind))
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
            Value::Long(l) => Ok(E::new(format!("INT64_C({l})"), K::Int)),
            Value::Boolean(b) => Ok(E::new(if *b { "1" } else { "0" }, K::Cond)),
            Value::Text(s) => Ok(E::new(
                format!("loft_lit({}, {})", c_string(s), s.len()),
                K::View,
            )),
            Value::Var(var) => {
                let name = self.var(*var);
                Ok(match self.shapes[*var as usize] {
                    CType::Text if (*var as usize) >= self.nparams => {
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
                if c == CType::Void {
                    self.stmt(compound, ind)?;
                    return Ok(E::new("(void) 0", K::Void));
                }
                let t = self.temp(c);
                self.assign(compound, &t, c, ind)?;
                Ok(self.held(t, c))
            }
            Value::Null => Err("a null without a type".into()),
            other => Err(unsupported(other)),
        }
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
            Value::Var(var)
                if self.shapes[*var as usize] == CType::Text && (*var as usize) >= self.nparams =>
            {
                Ok(self.var(*var))
            }
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
                cx.ctype(&a.typedef).ok_or_else(|| {
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
        let es = self.args(args, ind)?;
        let mut xs = Vec::with_capacity(es.len());
        for (e, c) in es.into_iter().zip(shapes) {
            xs.push(as_shape(e, c)?);
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

    fn two(&mut self, name: &str, args: &[Value], ind: usize) -> Result<(E, E), String> {
        let es = self.args(args, ind)?;
        let [a, b] = <[E; 2]>::try_from(es).map_err(|_| format!("{name} without two operands"))?;
        Ok((a, b))
    }

    fn operator(&mut self, name: &str, args: &[Value], ind: usize) -> Result<E, String> {
        let arith = match name {
            "OpAddInt" | "OpAddLong" => Some("loft_add"),
            "OpMinInt" | "OpMinLong" => Some("loft_sub"),
            "OpMulInt" | "OpMulLong" => Some("loft_mul"),
            "OpDivInt" | "OpDivLong" => Some("loft_div"),
            "OpRemInt" | "OpRemLong" => Some("loft_rem"),
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
                    format!("loft_text_append(&{dest}, {})", as_view(val)?),
                    K::Void,
                ))
            }
            "OpFormatInt" | "OpFormatLong" => {
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
                let bytes = self.copyable(raw & crate::keys::COPY_TP_MASK)?;
                let free = u8::from(raw & crate::keys::COPY_FREE_SOURCE != 0);
                let (src, dst) = self.two(name, &[(*src).clone(), (*dst).clone()], ind)?;
                Ok(E::new(
                    format!(
                        "loft_copy_record({}, {}, {bytes}, {free})",
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
            _ => Err(format!("operator {name}")),
        }
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
        CType::Void => Err("a void value".into()),
    }
}

/// The C shape an operator's value is held in, for the operators this backend writes.
fn op_shape(name: &str) -> Option<CType> {
    const INT: &[&str] = &[
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
    ];
    const BOOL: &[&str] = &[
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
    ];
    const REF: &[&str] = &[
        "OpGetField",
        "OpGetVector",
        "OpGetVectorNullable",
        "OpNullRefSentinel",
    ];
    if INT.contains(&name) {
        Some(CType::Int)
    } else if BOOL.contains(&name) {
        Some(CType::Bool)
    } else if REF.contains(&name) {
        Some(CType::Ref)
    } else {
        None
    }
}

/// `s` as a C string literal: printable ASCII as itself, every other byte as an octal escape
/// (three digits, so the next character can never extend it), `?` escaped against trigraphs.
fn c_string(s: &str) -> String {
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
