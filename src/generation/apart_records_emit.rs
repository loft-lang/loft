// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! `@FR-R-Apart` instance 4 — the emission of what [`apart_records::plan`] admitted.  An owner
//! `v` is the Rust local `__ap_<v>` of its type's struct (`Apr<n>`, one
//! `codegen_runtime::AprVec<E>` per field); a twin's parameter is `__ap_<p>: &Apr<n>`.  A push
//! group's element is the byte offset `__apo_<elm>` its push answered.  Every mention the plan
//! admitted is rewritten here and nothing else is: an unrewritten mention names `var_<v>`,
//! which no longer exists — a `rustc` error, never a wrong value.
use std::io::Write;

use super::apart_records::{self, Enc};
use super::{Output, sanitize};
use crate::data::Value;

impl Output<'_> {
    fn apr_roles(&self) -> Option<&apart_records::RecRoles> {
        self.apart_ctx
            .as_ref()
            .map(|c| &c.recs)
            .filter(|r| !r.is_empty())
    }

    /// The record definition of an apart variable of the body.
    fn apr_rec(&self, v: u16) -> Option<u32> {
        self.apr_roles()?.vars.get(&v).copied()
    }

    /// The value as a call argument: a reference to the owner's local, or the twin's own
    /// parameter, which already is one.
    pub(super) fn apr_arg(&self, v: u16) -> Option<String> {
        let roles = self.apr_roles()?;
        roles.vars.get(&v)?;
        Some(if roles.param == Some(v) {
            self.ap_apr_name(v)
        } else {
            format!("&{}", self.ap_apr_name(v))
        })
    }

    fn ap_apr_name(&self, v: u16) -> String {
        format!(
            "__ap_{}",
            sanitize(self.data.def(self.def_nr).variables().name(v))
        )
    }

    fn apo_name(&self, elm: u16) -> String {
        format!(
            "__apo_{}",
            sanitize(self.data.def(self.def_nr).variables().name(elm))
        )
    }

    /// Is this call of `d` with `vals` a twin's call taking an apart value?  The twin's
    /// apart argument is the one the plan named.
    pub(super) fn apr_call(&self, d: u32, vals: &[Value]) -> bool {
        let Some(&(_, at, _)) = self.plan.apr.twins.get(&d) else {
            return false;
        };
        matches!(vals.get(at).map(Value::unspan), Some(Value::Var(v)) if self.apr_rec(*v).is_some())
    }

    /// The parts `[(slot, bytes, n), …]` of an apart value, for a boundary.
    pub(super) fn apr_parts(&self, v: u16) -> Option<String> {
        self.apr_rec(v)?;
        Some(format!("&{}.parts()", self.ap_apr_name(v)))
    }

    /// The struct of every type instance 4 admitted, once per program.
    pub(super) fn output_apr_types(&mut self, w: &mut dyn Write) -> std::io::Result<()> {
        let mut types: Vec<&apart_records::RecLayout> = self.plan.apr.types.values().collect();
        types.sort_by(|a, b| a.rust.cmp(&b.rust));
        for t in types {
            let r = &t.rust;
            let field = |k: usize| format!("f{k}");
            let decl: Vec<String> = t
                .fields
                .iter()
                .enumerate()
                .map(|(k, f)| format!("{}: codegen_runtime::AprVec<{}>", field(k), f.esize))
                .collect();
            let new: Vec<String> = (0..t.fields.len())
                .map(|k| format!("{}: codegen_runtime::AprVec::new()", field(k)))
                .collect();
            let clear: Vec<String> = (0..t.fields.len())
                .map(|k| format!("self.{}.clear();", field(k)))
                .collect();
            let parts: Vec<String> = t
                .fields
                .iter()
                .enumerate()
                .map(|(k, f)| {
                    format!(
                        "({}_u32, self.{f}.bytes(), self.{f}.len() as u32)",
                        f.off,
                        f = field(k)
                    )
                })
                .collect();
            writeln!(
                w,
                "/// `@FR-R-Apart` instance 4 — the value form of a record of record vectors.\n\
                 struct {r} {{ {} }}\n\
                 impl {r} {{\n  fn new() -> {r} {{ {r} {{ {} }} }}\n  fn clear_all(&mut self) {{ {} }}\n  \
                 fn parts(&self) -> [(u32, &[u8], u32); {}] {{ [{}] }}\n}}\n",
                decl.join(", "),
                new.join(", "),
                clear.join(" "),
                t.fields.len(),
                parts.join(", ")
            )?;
        }
        Ok(())
    }

    /// The field of an apart value at byte offset `off`: `(variable, field index)`.
    fn apr_field(&self, base: &Value, off: &Value) -> Option<(u16, usize)> {
        let Value::Var(v) = base.unspan() else {
            return None;
        };
        let rec = self.apr_rec(*v)?;
        let off = match off.unspan() {
            Value::Int(i) => i64::from(*i),
            _ => return None,
        };
        Some((*v, self.plan.apr.types[&rec].field(off)?))
    }

    /// A field's length `len(OpGetField(v, off, _))` of an apart value.
    fn apr_length(&self, x: &Value) -> Option<(u16, usize)> {
        let Value::Call(d, a) = x.unspan() else {
            return None;
        };
        if !matches!(self.data.def(*d).name(), "t_6vector_len" | "OpLengthVector") {
            return None;
        }
        let Value::Call(g, fa) = a.first()?.unspan() else {
            return None;
        };
        if self.data.def(*g).name() != "OpGetField" {
            return None;
        }
        self.apr_field(fa.first()?, fa.get(1)?)
    }

    /// An element write `OpSet*(elm, off, x)` of a push group: `(owner, field, element offset,
    /// encoding, x)`.
    fn apr_write<'v>(&self, x: &'v Value) -> Option<(u16, usize, i64, Enc, &'v Value, u16)> {
        let Value::Call(d, a) = x.unspan() else {
            return None;
        };
        let enc = match self.data.def(*d).name() {
            "OpSetInt" => Enc::I64,
            "OpSetFloat" => Enc::F64,
            "OpSetBoolean" => Enc::Bool,
            _ => return None,
        };
        let [base, off, val] = &a[..] else {
            return None;
        };
        let Value::Var(elm) = base.unspan() else {
            return None;
        };
        let &(owner, f) = self.apr_roles()?.elems.get(elm)?;
        let Value::Int(off) = off.unspan() else {
            return None;
        };
        Some((owner, f, i64::from(*off), enc, val, *elm))
    }

    /// The operands of a mention [`Self::apr_expr`] takes that are expressions of their own.
    pub(super) fn apr_operands<'v>(&self, x: &'v Value) -> Option<Vec<&'v Value>> {
        self.apr_roles()?;
        if let Some((_, _, _, _, val, _)) = self.apr_write(x) {
            return Some(vec![val]);
        }
        let mut probe: Vec<u8> = Vec::new();
        // Every other mention this hook takes has no operand of its own.
        (self.apr_length(x).is_some() || self.apr_statement(&mut probe, x).ok()?).then(Vec::new)
    }

    /// The expression hook: answers whether it wrote `x`.
    pub(super) fn apr_expr(&mut self, w: &mut dyn Write, x: &Value) -> std::io::Result<bool> {
        if self.apr_roles().is_none() {
            return Ok(false);
        }
        if let Some((v, f)) = self.apr_length(x) {
            write!(w, "({}.f{f}.len() as i64)", self.ap_apr_name(v))?;
            return Ok(true);
        }
        if let Some((owner, f, off, enc, val, elm)) = self.apr_write(x) {
            let (ty, set) = match enc {
                Enc::I64 => ("i64", "set_i64"),
                Enc::F64 => ("f64", "set_f64"),
                Enc::Bool => ("u8", "set_byte"),
            };
            write!(w, "{{ let __apv: {ty} = (")?;
            self.output_code_inner(w, val)?;
            write!(
                w,
                ") as {ty}; {}.f{f}.{set}({} + {off}, __apv) }}",
                self.ap_apr_name(owner),
                self.apo_name(elm)
            )?;
            return Ok(true);
        }
        self.apr_statement(w, x)
    }

    /// The statements of an owner's life and the boundary: what each becomes.
    fn apr_statement(&self, w: &mut dyn Write, stmt: &Value) -> std::io::Result<bool> {
        let Some(roles) = self.apr_roles() else {
            return Ok(false);
        };
        let owner = |var: &Value| match var.unspan() {
            Value::Var(var_nr)
                if roles.vars.contains_key(var_nr) && roles.param != Some(*var_nr) =>
            {
                Some(*var_nr)
            }
            _ => None,
        };
        match stmt.unspan() {
            Value::Set(var, rhs) if owner(&Value::Var(*var)).is_some() => {
                if !matches!(rhs.unspan(), Value::Null) {
                    return Ok(false);
                }
                write!(w, "()")?;
                Ok(true)
            }
            Value::Set(elm, rhs) if roles.elems.contains_key(elm) => {
                let (var, fld) = roles.elems[elm];
                let Value::Call(..) = rhs.unspan() else {
                    return Ok(false);
                };
                write!(
                    w,
                    "let {} = {}.f{fld}.push_elem()",
                    self.apo_name(*elm),
                    self.ap_apr_name(var)
                )?;
                Ok(true)
            }
            Value::Call(def, args) => {
                let first = args.first();
                match self.data.def(*def).name() {
                    "OpDatabase" if first.and_then(owner).is_some() => {
                        let var = first.and_then(owner).unwrap_or_default();
                        write!(w, "{}.clear_all()", self.ap_apr_name(var))?;
                        Ok(true)
                    }
                    "OpSetInt4" if first.and_then(owner).is_some() => {
                        let Some((var, fld)) =
                            args.get(1).and_then(|o| self.apr_field(&args[0], o))
                        else {
                            return Ok(false);
                        };
                        write!(w, "{}.f{fld}.clear()", self.ap_apr_name(var))?;
                        Ok(true)
                    }
                    "OpFinishRecord" | "OpFreeRef" if first.and_then(owner).is_some() => {
                        write!(w, "()")?;
                        Ok(true)
                    }
                    "OpCopyRecord" => {
                        let (Some(Value::Var(param)), Some(dst)) =
                            (first.map(Value::unspan), args.get(1))
                        else {
                            return Ok(false);
                        };
                        let Some(parts) = self.apr_parts(*param) else {
                            return Ok(false);
                        };
                        let Value::Var(elm_var) = dst.unspan() else {
                            return Ok(false);
                        };
                        let dst = sanitize(self.data.def(self.def_nr).variables().name(*elm_var));
                        write!(
                            w,
                            "stores.apart_write_fresh(&(var_{dst}), {parts}) /*@FR-R-Apart boundary*/"
                        )?;
                        Ok(true)
                    }
                    _ => Ok(false),
                }
            }
            _ => Ok(false),
        }
    }
}
