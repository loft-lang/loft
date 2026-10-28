// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `@FR-R-Apart` instance 2 in the C: a record of fixed-length `vector<float>` fields travels as
//! a C struct (`loft_ap_<T>`, one `double[N]` per field) between the functions that only build
//! it, read and write its elements and hand it on, and meets the store once, at a BOUNDARY: a
//! `return` through the function's buffer, which writes it there with its constructor's steps
//! (`loft_ap_store`).  The plan is native's (`lower::apart_value::plan`); this is its emission.
//!
//! A TWIN (`<f>__apv`) answers the value through `loft_apo` and takes its record parameters as
//! `const loft_ap_<T> *loft_avp<v>`; a REGION is the apart variables of an ordinary body that
//! calls a twin.  An apart variable is the local `loft_av<v>`, a DEAD one is never declared, and
//! `expr(Var(v))` answers the struct — so a mention no clause here rewrote is a C type error,
//! never a wrong value.  A body holding either is emitted with its vector hoists off, as native's.

use super::{E, FnWriter, K, as_float, as_int, as_shape};
use crate::data::Value;
use crate::lower::apart_value::{self, ApLayout, Construction, Elem};

impl FnWriter<'_> {
    /// Take the plan's roles for this body: its twin's, or its region's when the region calls
    /// a twin (`profitable`, native's condition).
    pub(crate) fn apart_value(mut self, twin: bool) -> Self {
        let plan = self.cx.ap2_plan();
        let roles = if twin {
            plan.twin_roles.get(&self.d).cloned()
        } else {
            plan.regions.get(&self.d).filter(|r| r.profitable).cloned()
        };
        if let Some(roles) = roles.filter(|r| !r.is_empty() || twin) {
            let def = self.cx.data.def(self.d);
            self.av_rb = def
                .hidden_return_buffer_attr()
                .map(|i| def.variables().var(&def.attributes()[i].name));
            if std::env::var("LOFT_TRACE_APART").is_ok_and(|v| v != "0") {
                eprintln!(
                    "[apart] C {}{}: {} apart, {} dead",
                    def.name(),
                    if twin { "__apv" } else { "" },
                    roles.apart.len(),
                    roles.dead.len()
                );
            }
            self.av = roles;
            self.av_twin = twin;
        }
        self.av_plan = Some(plan);
        self
    }

    /// Does this body hold an instance-2 value (a twin, or a region)?
    pub(super) fn av_any(&self) -> bool {
        self.av_twin || !self.av.is_empty()
    }

    /// Is `v` an apart or a dead variable of instance 2?
    pub(super) fn av_holds(&self, v: u16) -> bool {
        self.av.apart.contains_key(&v) || self.av.dead.contains(&v)
    }

    fn av_layout(&self, v: u16) -> Option<&ApLayout> {
        let rec = self.av.apart.get(&v)?;
        self.av_plan.as_ref()?.types.get(rec)
    }

    /// Is `v` a twin's record parameter, which arrives as a pointer to its value?
    fn av_param(&self, v: u16) -> bool {
        self.av_twin && self.vars.is_argument(v) && self.av_rb != Some(v) && self.av.apart.contains_key(&v)
    }

    /// The value apart variable `v` holds, as a C lvalue (a parameter's is read-only).
    pub(super) fn av_val(&self, v: u16) -> Option<String> {
        self.av.apart.get(&v)?;
        Some(if self.av_param(v) {
            format!("(*loft_avp{v})")
        } else {
            format!("loft_av{v}")
        })
    }

    /// A pointer to the value `v` holds, as a twin's record argument takes it.
    fn av_ptr(&self, v: u16) -> String {
        if self.av_param(v) {
            format!("loft_avp{v}")
        } else {
            format!("&loft_av{v}")
        }
    }

    /// The apart locals as declarations, and — in a twin — its return buffer as the null local
    /// its other mentions name.
    pub(super) fn av_decls(&mut self) {
        let mut vs: Vec<u16> = self.av.apart.keys().copied().filter(|v| !self.av_param(*v)).collect();
        vs.sort_unstable();
        for v in vs {
            let rec = self.av.apart[&v];
            if let Some(t) = self.cx.ap2_ctype(rec) {
                self.decls.push(format!("{t} loft_av{v};"));
                self.line(1, &format!("memset(&loft_av{v}, 0, sizeof loft_av{v});"));
            }
        }
        if self.av_twin
            && let Some(rb) = self.av_rb
            && !self.av_holds(rb)
        {
            self.decls
                .push(format!("loft_ref {} = {{ 0xFFFF, 0, 0 }};", self.var(rb)));
        }
    }

    /// A construction run of an apart variable starting at `ops[at]`: the value built in a
    /// temporary, each element written in the order the program wrote it, then assigned.
    /// Answers the index of the first statement after the run.
    pub(super) fn av_construction(&mut self, ops: &[Value], at: usize, ind: usize) -> Result<Option<usize>, String> {
        if self.av.is_empty() {
            return Ok(None);
        }
        let Some(plan) = self.av_plan.clone() else {
            return Ok(None);
        };
        let mut found = None;
        for (&v, rec) in &self.av.apart {
            let layout = &plan.types[rec];
            if let Some(c) = apart_value::construction(self.cx.data, layout, ops, at, &|u| u == v) {
                found = Some(c);
                break;
            }
        }
        let Some(c) = found else {
            return Ok(None);
        };
        let target = self.av_val(c.v).ok_or("a construction of no apart value")?;
        self.av_construct(&c, &target, ind)?;
        Ok(Some(c.end))
    }

    /// Build construction `c` in a temporary and assign it to `target`.
    fn av_construct(&mut self, c: &Construction<'_>, target: &str, ind: usize) -> Result<(), String> {
        let rec = *self.av.apart.get(&c.v).ok_or("a construction of no apart value")?;
        let t = self.cx.ap2_ctype(rec).ok_or("no apart type")?;
        let n = self.decls.len();
        let tmp = format!("loft_apc{n}");
        self.decls.push(format!("{t} {tmp};"));
        self.line(ind, &format!("memset(&{tmp}, 0, sizeof {tmp});"));
        let fields = self.av_layout(c.v).map_or(0, |l| l.fields.len());
        let mut next = vec![0usize; fields];
        for (f, e) in &c.order {
            match e {
                Elem::Val(x) => {
                    let y = self.expr(x, ind)?;
                    self.line(ind, &format!("{tmp}.f{f}[{}] = {};", next[*f], as_float(y)?));
                    next[*f] += 1;
                }
                Elem::Copies(k) => {
                    for _ in 0..*k {
                        self.line(ind, &format!("{tmp}.f{f}[{}] = {tmp}.f{f}[{}];", next[*f], next[*f] - 1));
                        next[*f] += 1;
                    }
                }
            }
        }
        self.line(ind, &format!("{target} = {tmp};"));
        Ok(())
    }

    /// The construction a value block `{ <run of v>; v }` holds, if it is one.
    fn av_construction_block<'v>(&self, x: &'v Value) -> Option<Construction<'v>> {
        let Value::Block(bl) = x.unspan() else {
            return None;
        };
        let ops = &bl.operators;
        let plan = self.av_plan.as_ref()?;
        for (&v, rec) in &self.av.apart {
            if let Some(c) = apart_value::construction(self.cx.data, &plan.types[rec], ops, 0, &|u| u == v)
                && c.end + 1 == ops.len()
                && matches!(ops[c.end].unspan(), Value::Var(u) if *u == c.v)
            {
                return Some(c);
            }
        }
        None
    }

    /// A twin's call `<g>__apv(…, dest)`: a record argument as a pointer to its value, the
    /// buffer left out, every other argument (a number) as itself.
    fn av_twin_call(&mut self, g: u32, args: &[Value], dest: &str, ind: usize) -> Result<(), String> {
        let gdef = self.cx.data.def(g);
        let rbi = gdef.hidden_return_buffer_attr();
        let mut xs = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            if Some(i) == rbi {
                continue;
            }
            if let Value::Var(u) = arg.unspan()
                && self.av.apart.contains_key(u)
            {
                xs.push(self.av_ptr(*u));
                continue;
            }
            let shape = self
                .cx
                .ctype(gdef.attributes()[i].typedef.peel_link())
                .ok_or("a twin's argument of no C shape")?;
            let e = self.arg_with(arg, false, ind)?;
            xs.push(as_shape(e, shape)?);
        }
        xs.push(dest.to_string());
        self.cx.request_twin(g, super::super::APV_BIT);
        let name = super::super::twin_name(self.cx.data, g, super::super::APV_BIT);
        crate::rewrite_census::fired("R-Apart", 1);
        self.line(ind, &format!("{name}({});", xs.join(", ")));
        Ok(())
    }

    /// Instance 2's operators: an element read or write, a length, a free of apart or dead
    /// variables.  `None` for any other.
    pub(super) fn av_operator(&mut self, x: &Value, ind: usize) -> Result<Option<E>, String> {
        if self.av.is_empty() {
            return Ok(None);
        }
        let data = self.cx.data;
        if let Some((v, off, idx, _)) = apart_value::element_read(data, x)
            && let Some(l) = self.av_layout(v)
            && let Some(f) = l.field(off)
        {
            let n = l.fields[f].len;
            let val = self.av_val(v).ok_or("no apart value")?;
            let i = self.expr(idx, ind)?;
            return Ok(Some(E::new(format!("loft_ap_get({val}.f{f}, {n}, {})", as_int(i)?), K::Float)));
        }
        if let Some((v, off, idx, val, _)) = apart_value::element_write(data, x)
            && let Some(l) = self.av_layout(v)
            && let Some(f) = l.field(off)
        {
            let n = l.fields[f].len;
            let target = self.av_val(v).ok_or("no apart value")?;
            let (i, y) = self.two("OpSetFloat", &[idx, val], ind)?;
            self.line(ind, &format!("loft_ap_set({target}.f{f}, {n}, {}, {});", as_int(i)?, as_float(y)?));
            return Ok(Some(E::new("((void) 0)", K::Void)));
        }
        if let Some((v, off)) = apart_value::length_of(data, x)
            && let Some(l) = self.av_layout(v)
            && let Some(f) = l.field(off)
        {
            return Ok(Some(E::new(format!("INT64_C({})", l.fields[f].len), K::Int)));
        }
        if let Value::Call(d, args) = x.unspan()
            && matches!(data.def(*d).name(), "OpFreeRef" | "OpFreeRefIfDistinct")
            && !args.is_empty()
            && args
                .iter()
                .all(|a| matches!(a.unspan(), Value::Var(u) if self.av_holds(*u)))
        {
            return Ok(Some(E::new("((void) 0)", K::Void)));
        }
        Ok(None)
    }

    /// Instance 2's binds: a dead variable's is nothing; an apart variable's null init zeroes
    /// it, a twin's call fills it, a construction block builds it.  Answers whether it wrote
    /// `Set(var, x)`.
    pub(super) fn av_set(&mut self, var: u16, x: &Value, ind: usize) -> Result<bool, String> {
        if self.av.is_empty() {
            return Ok(false);
        }
        if self.av.dead.contains(&var) {
            return Ok(true);
        }
        let Some(val) = self.av_val(var) else {
            return Ok(false);
        };
        match x.unspan() {
            Value::Null => {
                self.line(ind, &format!("memset(&{val}, 0, sizeof {val});"));
            }
            Value::Call(g, args) if self.av_plan.as_ref().is_some_and(|p| p.twins.contains_key(g)) => {
                let dest = self.av_ptr(var);
                self.av_twin_call(*g, args, &dest, ind)?;
            }
            other => {
                let c = self
                    .av_construction_block(other)
                    .ok_or("an apart value bound to something the value form does not serve")?;
                self.av_construct(&c, &val, ind)?;
            }
        }
        Ok(true)
    }

    /// A `return` of an apart value: a twin writes it through `loft_apo` and returns; a region
    /// writes it into the function's buffer (`loft_ap_store`) and returns the buffer.  Answers
    /// whether it wrote `Return(inner)`.
    pub(super) fn av_return(&mut self, inner: &Value, ind: usize) -> Result<bool, String> {
        if !self.av_any() {
            return Ok(false);
        }
        let plan = self.av_plan.clone().ok_or("no apart plan")?;
        // The record the return answers, and how its value is produced.
        let (rec, kind) = match inner.unspan() {
            Value::Var(v) if self.av.apart.contains_key(v) => (self.av.apart[v], 0),
            Value::Call(g, _) if plan.twins.contains_key(g) => (plan.twins[g], 1),
            other => match self.av_construction_block(other) {
                Some(c) => (self.av.apart[&c.v], 2),
                None => return Ok(false),
            },
        };
        let t = self.cx.ap2_ctype(rec).ok_or("no apart type")?;
        // Where the value lands: through `loft_apo` in a twin, in a temporary in a region.
        let dest = if self.av_twin {
            "(*loft_apo)".to_string()
        } else {
            let n = self.decls.len();
            let tmp = format!("loft_avr{n}");
            self.decls.push(format!("{t} {tmp};"));
            tmp
        };
        match (kind, inner.unspan()) {
            (0, Value::Var(v)) => {
                let val = self.av_val(*v).ok_or("no apart value")?;
                self.line(ind, &format!("{dest} = {val};"));
            }
            (1, Value::Call(g, args)) => {
                let ptr = if self.av_twin { "loft_apo".to_string() } else { format!("&{dest}") };
                self.av_twin_call(*g, args, &ptr, ind)?;
            }
            _ => {
                let c = self.av_construction_block(inner).ok_or("no construction")?;
                self.av_construct(&c, &dest, ind)?;
            }
        }
        if self.av_twin {
            self.ret_value(None, ind)?;
            return Ok(true);
        }
        // The region's boundary: the value written into the buffer with its constructor's steps.
        let rb = self.av_rb.ok_or("a region returning through no buffer")?;
        let layout = plan.types[&rec].clone();
        self.cx.describe_type(layout.rec_tp);
        let bytes = self.cx.stores.enum_parent_size(layout.rec_tp);
        let offs: Vec<String> = layout.fields.iter().map(|f| format!("{}u", f.off)).collect();
        let vals: Vec<String> = (0..layout.fields.len()).map(|k| format!("{dest}.f{k}")).collect();
        let lens: Vec<String> = layout.fields.iter().map(|f| format!("{}u", f.len)).collect();
        let b = self.var(rb);
        self.line(
            ind,
            &format!(
                "{b} = loft_ap_store({b}, {bytes}, {}, {}, (const uint32_t[]){{ {} }}, (const double *const[]){{ {} }}, (const uint32_t[]){{ {} }}); /* @FR-R-Apart boundary */",
                layout.rec_tp,
                layout.fields.len(),
                offs.join(", "),
                vals.join(", "),
                lens.join(", ")
            ),
        );
        // The buffer by its own C name: `expr` answers an apart variable's value, and a local
        // promoted onto the buffer is one.
        self.raw.insert(b.clone(), K::Ref);
        self.ret_value(Some(&Value::RawExpr(b)), ind)?;
        Ok(true)
    }
}
