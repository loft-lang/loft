// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! `@FR-R-Apart` instance 2 — the emission of what [`apart_value::plan`] admitted.  An apart
//! variable `v` is the Rust local `__ap_<v>` of its type's struct (`Apart<n>`, one `[f64; N]`
//! per field); a dead one emits nothing.  Every mention the plan admitted is rewritten here,
//! and nothing else is: an unrewritten mention names `var_<v>`, which in a twin does not exist
//! and in a region is the record form's null `DbRef` — so the plan and this file must agree
//! mention for mention, and the guard cells hold them to it.
//!
//! The statement hook takes a construction run (several statements) and every statement the
//! expression hook takes; the expression hook takes an element read, write and length, a bind,
//! a free and a `return`.  A twin and a region are emitted with the vector hoists off, so no
//! loop plans a header, a base or an address on a variable that is no longer a record.
use std::io::Write;

use super::apart_value::{self, ApLayout, Construction, Elem, Roles};
use super::{Output, sanitize};
use crate::data::Value;

/// How a twin's signature takes one parameter ([`Output::apart_twin_param`]).
pub enum TwinParam {
    /// As the original function takes it.
    Same,
    /// Not at all: the return buffer.
    Dropped,
    /// This text: a record parameter as a reference to its value.
    Value(String),
}

/// The plan for the body being emitted.
#[derive(Clone, Debug)]
pub struct ApartCtx {
    pub roles: Roles,
    /// A twin (`n_<f>__apv`) rather than the function's own body.
    pub twin: bool,
    /// The function's return buffer variable.
    pub rb: Option<u16>,
}

impl Output<'_> {
    /// The context `def_nr`'s body is emitted under: its twin's while a twin is emitted,
    /// else its region when that is profitable.
    pub(super) fn apart_ctx_for(&self, def_nr: u32) -> Option<ApartCtx> {
        let def = self.data.def(def_nr);
        let rb = super::hoist::ret_buffer_attr(def)
            .map(|i| def.variables().var(&def.attributes()[i].name));
        if self.apart_twin {
            let roles = self.apart.twin_roles.get(&def_nr)?.clone();
            return Some(ApartCtx {
                roles,
                twin: true,
                rb,
            });
        }
        let roles = self.apart.regions.get(&def_nr)?;
        roles.profitable.then(|| ApartCtx {
            roles: roles.clone(),
            twin: false,
            rb,
        })
    }

    fn ap_apart(&self, v: u16) -> bool {
        self.apart_ctx
            .as_ref()
            .is_some_and(|c| c.roles.apart.contains_key(&v))
    }

    fn ap_dead(&self, v: u16) -> bool {
        self.apart_ctx
            .as_ref()
            .is_some_and(|c| c.roles.dead.contains(&v))
    }

    fn ap_name(&self, v: u16) -> String {
        format!(
            "__ap_{}",
            sanitize(self.data.def(self.def_nr).variables().name(v))
        )
    }

    fn ap_layout(&self, v: u16) -> Option<ApLayout> {
        let rec = self.apart_ctx.as_ref()?.roles.apart.get(&v)?;
        self.apart.types.get(rec).cloned()
    }

    /// The struct of every apart type, once per program.
    pub(super) fn output_apart_types(&mut self, w: &mut dyn Write) -> std::io::Result<()> {
        if self.apart_types_done {
            return Ok(());
        }
        self.apart_types_done = true;
        let mut types: Vec<&ApLayout> = self.apart.types.values().collect();
        types.sort_by(|a, b| a.rust.cmp(&b.rust));
        for t in types {
            let fields: Vec<String> = t
                .fields
                .iter()
                .enumerate()
                .map(|(k, f)| format!("f{k}: [f64; {}]", f.len))
                .collect();
            let zero: Vec<String> = t
                .fields
                .iter()
                .enumerate()
                .map(|(k, f)| format!("f{k}: [0.0; {}]", f.len))
                .collect();
            writeln!(
                w,
                "/// `@FR-R-Apart` — the value form of a record of fixed-length float vectors.\n#[derive(Clone, Copy)]\nstruct {r} {{ {} }}\nimpl {r} {{ const ZERO: {r} = {r} {{ {} }}; }}\n",
                fields.join(", "),
                zero.join(", "),
                r = t.rust
            )?;
        }
        Ok(())
    }

    /// How the twin's signature takes parameter `i`: a record parameter as a reference to its
    /// value, the return buffer not at all, every other parameter as the original has it.
    pub(super) fn apart_twin_param(&self, def_nr: u32, i: usize) -> TwinParam {
        let Some(ctx) = self.apart_ctx.as_ref().filter(|c| c.twin) else {
            return TwinParam::Same;
        };
        let def = self.data.def(def_nr);
        let a = &def.attributes()[i];
        let v = def.variables().var(&a.name);
        if ctx.rb == Some(v) {
            return TwinParam::Dropped;
        }
        let Some(rec) = ctx.roles.apart.get(&v) else {
            return TwinParam::Same;
        };
        let t = &self.apart.types[rec].rust;
        TwinParam::Value(format!(", __ap_{}: &{t}", sanitize(&a.name)))
    }

    /// The twin's result type.
    pub(super) fn apart_twin_result(&self, def_nr: u32) -> Option<String> {
        self.apart_ctx.as_ref().filter(|c| c.twin)?;
        let rec = self.apart.twins.get(&def_nr)?;
        Some(self.apart.types[rec].rust.clone())
    }

    /// The lines the body's prologue starts with: each apart local, and in a twin the return
    /// buffer as the null record its other emission names.
    pub(super) fn apart_prologue(&self) -> String {
        use std::fmt::Write as _;
        let Some(ctx) = &self.apart_ctx else {
            return String::new();
        };
        let vars = self.data.def(self.def_nr).variables();
        let mut out = String::new();
        let mut apart: Vec<(&u16, &u32)> = ctx.roles.apart.iter().collect();
        apart.sort_unstable();
        for (&v, rec) in apart {
            // A twin's record parameters are its arguments; the return buffer is a local in
            // both forms (a region's buffer parameter keeps the record it converts into).
            if vars.is_argument(v) && ctx.rb != Some(v) {
                continue;
            }
            let t = &self.apart.types[rec].rust;
            let _ = write!(out, "\n  let mut {}: {t} = {t}::ZERO;", self.ap_name(v));
        }
        if ctx.twin
            && let Some(rb) = ctx.rb
        {
            let _ = write!(
                out,
                "\n  let mut var_{}: DbRef = DbRef::NULL;",
                sanitize(vars.name(rb))
            );
        }
        out
    }

    /// The statement hook: a construction run or a statement the expression hook takes,
    /// emitted as a statement; answers the index of the first statement after it.
    pub(super) fn apart_statement(
        &mut self,
        w: &mut dyn Write,
        ops: &[Value],
        at: usize,
    ) -> std::io::Result<Option<usize>> {
        if self.apart_ctx.is_none() {
            return Ok(None);
        }
        if let Some(c) = self.apart_construction_at(ops, at) {
            self.indent(w)?;
            write!(w, "{} = ", self.ap_name(c.v))?;
            self.apart_construct(w, &c)?;
            writeln!(w, ";")?;
            return Ok(Some(c.end));
        }
        let mut probe: Vec<u8> = Vec::new();
        if self.apart_expr(&mut probe, &ops[at])? {
            self.indent(w)?;
            w.write_all(&probe)?;
            writeln!(w, ";")?;
            return Ok(Some(at + 1));
        }
        Ok(None)
    }

    fn apart_construction_at<'v>(&self, ops: &'v [Value], at: usize) -> Option<Construction<'v>> {
        let ctx = self.apart_ctx.as_ref()?;
        let data = self.data;
        for (&v, rec) in &ctx.roles.apart {
            let layout = &self.apart.types[rec];
            if let Some(c) = apart_value::construction(data, layout, ops, at, &|u| u == v) {
                return Some(c);
            }
        }
        None
    }

    /// A construction as a block expression answering the value: zeroed, then each element
    /// written in the order the program wrote it.
    fn apart_construct(&mut self, w: &mut dyn Write, c: &Construction<'_>) -> std::io::Result<()> {
        let layout = self
            .ap_layout(c.v)
            .expect("an admitted construction has a layout");
        writeln!(w, "{{ let mut __apc = {}::ZERO;", layout.rust)?;
        let mut next = vec![0usize; layout.fields.len()];
        for (f, e) in &c.order {
            match e {
                Elem::Val(x) => {
                    self.indent(w)?;
                    write!(w, "  __apc.f{f}[{}] = ", next[*f])?;
                    self.output_code_inner(w, x)?;
                    writeln!(w, ";")?;
                    next[*f] += 1;
                }
                Elem::Copies(n) => {
                    for _ in 0..*n {
                        self.indent(w)?;
                        writeln!(
                            w,
                            "  __apc.f{f}[{}] = __apc.f{f}[{}];",
                            next[*f],
                            next[*f] - 1
                        )?;
                        next[*f] += 1;
                    }
                }
            }
        }
        self.indent(w)?;
        write!(w, "  __apc }}")
    }

    /// `n_<g>__apv(cell, …)` — a twin's call: a record argument by reference to its value,
    /// the buffer left out, every other argument as itself.
    fn apart_twin_call(
        &mut self,
        w: &mut dyn Write,
        g: u32,
        args: &[Value],
    ) -> std::io::Result<()> {
        crate::rewrite_census::fired("R-Apart", 1);
        let gdef = self.data.def(g);
        let rbi = super::hoist::ret_buffer_attr(gdef);
        write!(w, "{}__apv(cell", self.fn_ident(gdef))?;
        for (i, arg) in args.iter().enumerate() {
            if Some(i) == rbi {
                continue;
            }
            write!(w, ", ")?;
            match arg.unspan() {
                Value::Var(u) if self.ap_apart(*u) => write!(w, "&{}", self.ap_name(*u))?,
                _ => self.output_code_inner(w, arg)?,
            }
        }
        write!(w, ")")
    }

    /// The value an apart variable holds, by value: a twin's record parameter is a reference.
    fn apart_value_of(&self, v: u16) -> String {
        let vars = self.data.def(self.def_nr).variables();
        let twin_param = self
            .apart_ctx
            .as_ref()
            .is_some_and(|c| c.twin && vars.is_argument(v) && c.rb != Some(v));
        if twin_param {
            format!("*{}", self.ap_name(v))
        } else {
            self.ap_name(v)
        }
    }

    /// A `return` of an apart value: the value itself from a twin; from a region, the value
    /// written into the return buffer (`ap_store_f64`), which is what the function answers.
    fn apart_return(
        &mut self,
        w: &mut dyn Write,
        value: &dyn Fn(&mut Self, &mut dyn Write) -> std::io::Result<()>,
        rec: u32,
    ) -> std::io::Result<()> {
        let ctx = self
            .apart_ctx
            .clone()
            .expect("an apart return has a context");
        if ctx.twin {
            write!(w, "return ")?;
            return value(self, w);
        }
        let layout = self.apart.types[&rec].clone();
        let rb = sanitize(
            self.data
                .def(self.def_nr)
                .variables()
                .name(ctx.rb.expect("a region returns through its buffer")),
        );
        write!(w, "{{ let __apr = ")?;
        value(self, w)?;
        write!(
            w,
            "; return codegen_runtime::ap_store_f64(cell, var_{rb}, {}_i32, &[",
            layout.rec_tp
        )?;
        for (k, f) in layout.fields.iter().enumerate() {
            write!(w, "({}_u32, &__apr.f{k}[..]), ", f.off)?;
        }
        write!(w, "]) }}")
    }

    /// The expression hook: answers whether it wrote `x`.
    #[expect(clippy::too_many_lines, reason = "one arm per admitted mention")]
    pub(super) fn apart_expr(&mut self, w: &mut dyn Write, x: &Value) -> std::io::Result<bool> {
        if self.apart_ctx.is_none() {
            return Ok(false);
        }
        let data = self.data;
        if let Some((v, off, idx, nullable)) = apart_value::element_read(data, x)
            && self.ap_apart(v)
            && let Some(f) = self.ap_layout(v).and_then(|l| l.field(off))
        {
            if nullable {
                write!(
                    w,
                    "codegen_runtime::ap_get_f64_or_null(&{}.f{f}, ",
                    self.ap_name(v)
                )?;
            } else {
                write!(
                    w,
                    "codegen_runtime::ap_get_f64(cell, &{}.f{f}, ",
                    self.ap_name(v)
                )?;
            }
            self.output_code_inner(w, idx)?;
            write!(w, ")")?;
            return Ok(true);
        }
        if let Some((v, off, idx, val, _)) = apart_value::element_write(data, x)
            && self.ap_apart(v)
            && let Some(f) = self.ap_layout(v).and_then(|l| l.field(off))
        {
            // The index and the value first, in the program's order, then the write: the
            // value may read the same array.
            write!(w, "{{ let __api: i64 = ")?;
            self.output_code_inner(w, idx)?;
            write!(w, "; let __apv: f64 = ")?;
            self.output_code_inner(w, val)?;
            write!(
                w,
                "; codegen_runtime::ap_set_f64(cell, &mut {}.f{f}, __api, __apv) }}",
                self.ap_name(v)
            )?;
            return Ok(true);
        }
        if let Some((v, off)) = apart_value::length_of(data, x)
            && self.ap_apart(v)
            && let Some(l) = self.ap_layout(v)
            && let Some(f) = l.field(off)
        {
            write!(w, "{}_i64", l.fields[f].len)?;
            return Ok(true);
        }
        match x.unspan() {
            Value::Set(v, _) if self.ap_dead(*v) => {
                write!(w, "()")?;
                Ok(true)
            }
            Value::Set(v, rhs) if self.ap_apart(*v) => {
                let name = self.ap_name(*v);
                match rhs.unspan() {
                    Value::Null => {
                        let t = self.ap_layout(*v).map(|l| l.rust).unwrap_or_default();
                        write!(w, "{name} = {t}::ZERO")?;
                    }
                    Value::Call(g, args) if self.apart.twins.contains_key(g) => {
                        write!(w, "{name} = ")?;
                        self.apart_twin_call(w, *g, args)?;
                    }
                    other => {
                        let Some(c) = self.apart_construction_block(other) else {
                            return Ok(false);
                        };
                        write!(w, "{name} = ")?;
                        self.apart_construct(w, &c)?;
                    }
                }
                Ok(true)
            }
            Value::Call(d, args)
                if matches!(data.def(*d).name(), "OpFreeRef" | "OpFreeRefIfDistinct")
                    && !args.is_empty()
                    && args.iter().all(|a| {
                        matches!(a.unspan(), Value::Var(u) if self.ap_apart(*u) || self.ap_dead(*u))
                    }) =>
            {
                write!(w, "()")?;
                Ok(true)
            }
            Value::Return(inner) => {
                let Some(ctx) = self.apart_ctx.clone() else {
                    return Ok(false);
                };
                match inner.unspan() {
                    Value::Var(v) if self.ap_apart(*v) => {
                        let rec = ctx.roles.apart[v];
                        let val = self.apart_value_of(*v);
                        self.apart_return(w, &|_, w| write!(w, "{val}"), rec)?;
                        Ok(true)
                    }
                    Value::Call(g, args)
                        if self.apart.twins.contains_key(g)
                            && ctx.rb.is_some_and(|b| self.ap_apart(b)) =>
                    {
                        let rec = self.apart.twins[g];
                        let (g, args) = (*g, args.clone());
                        self.apart_return(w, &|s, w| s.apart_twin_call(w, g, &args), rec)?;
                        Ok(true)
                    }
                    other => {
                        let Some(rec) = self
                            .apart_construction_block(other)
                            .and_then(|c| ctx.roles.apart.get(&c.v).copied())
                        else {
                            return Ok(false);
                        };
                        let other = other.clone();
                        self.apart_return(
                            w,
                            &|s, w| {
                                let c = s
                                    .apart_construction_block(&other)
                                    .expect("checked above");
                                s.apart_construct(w, &c)
                            },
                            rec,
                        )?;
                        Ok(true)
                    }
                }
            }
            _ => Ok(false),
        }
    }

    /// The operands of an apart element read, write or length that are expressions of their
    /// own — the index, and a write's value — or `None` when `x` is no such mention.
    pub(super) fn apart_operands<'v>(&self, x: &'v Value) -> Option<Vec<&'v Value>> {
        self.apart_ctx.as_ref()?;
        if let Some((v, _, idx, _)) = apart_value::element_read(self.data, x)
            && self.ap_apart(v)
        {
            return Some(vec![idx]);
        }
        if let Some((v, _, idx, val, _)) = apart_value::element_write(self.data, x)
            && self.ap_apart(v)
        {
            return Some(vec![idx, val]);
        }
        if let Some((v, _)) = apart_value::length_of(self.data, x)
            && self.ap_apart(v)
        {
            return Some(Vec::new());
        }
        None
    }

    /// An object block `{ <construction of v>; v }` of an apart variable.
    fn apart_construction_block<'v>(&self, x: &'v Value) -> Option<Construction<'v>> {
        let Value::Block(bl) = x.unspan() else {
            return None;
        };
        let ops = &bl.operators;
        let c = self.apart_construction_at(ops, 0)?;
        (c.end + 1 == ops.len() && matches!(ops[c.end].unspan(), Value::Var(u) if *u == c.v))
            .then_some(c)
    }
}
