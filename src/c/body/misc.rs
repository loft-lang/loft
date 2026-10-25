// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! The operators no family above claims: integer `abs` and `**`, a vector reversed, sorted or
//! fed a text's bytes, the null reference, a scalar's size, and a closure detached from a
//! displaced function value.

use super::{CType, E, FnWriter, K, as_int, as_ref, as_view};
use crate::data::Value;

impl FnWriter<'_> {
    /// The Unicode natives — a text's case mapping, a character's class — or `None`.
    pub(super) fn unicode_native(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        match (name, args) {
            ("t_4text_to_lowercase" | "t_4text_to_uppercase", [t]) => {
                let f = self.cx.unicode_case(name == "t_4text_to_uppercase");
                let t = self.expr(t, ind)?;
                let r = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_free(&{r});"));
                self.line(ind, &format!("{r} = {f}({});", as_view(t)?));
                Ok(Some(self.held(r, CType::Text)))
            }
            (n, [c]) if n.starts_with("t_9character_is_") => {
                let Some(f) = self.cx.unicode_class(&n["t_9character_is_".len()..]) else {
                    return Ok(None);
                };
                let c = self.expr(c, ind)?;
                Ok(Some(E::new(
                    format!("{f}({})", super::as_char(c)?),
                    K::Bool,
                )))
            }
            _ => Ok(None),
        }
    }

    /// The remaining operators, or the refusal that names one.
    pub(super) fn misc_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        match (name, args) {
            ("OpCastFloatFromText", [t]) => {
                let t = self.expr(t, ind)?;
                Ok(E::new(
                    format!("loft_float_of_text({})", as_view(t)?),
                    K::Float,
                ))
            }
            ("OpCastSingleFromText", [t]) => {
                let t = self.expr(t, ind)?;
                Ok(E::new(
                    format!("loft_single_of_text({})", as_view(t)?),
                    K::Single,
                ))
            }
            ("OpAbsInt", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("loft_abs({})", as_int(v)?), K::Int))
            }
            ("OpPowInt", [_, _]) => {
                let (a, b) = self.two(name, args, ind)?;
                Ok(E::new(
                    format!("loft_pow({}, {})", as_int(a)?, as_int(b)?),
                    K::Int,
                ))
            }
            ("OpReverseVector", [v, size]) => {
                let size = Self::literal(size, "element size")?;
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!("loft_vec_reverse({}, {size})", as_ref(v)?),
                    K::Void,
                ))
            }
            ("OpSortVector", [v, tp]) => {
                let tp = u16::try_from(Self::literal(tp, "element type")?)
                    .map_err(|_| "an element type out of range")?;
                let text = self.cx.stores.is_text_type(tp);
                let size = self.cx.stores.size(tp);
                let float = i32::from(tp == 2 || tp == 3);
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!(
                        "loft_vec_sort({}, {size}, {float}, {})",
                        as_ref(v)?,
                        i32::from(text)
                    ),
                    K::Void,
                ))
            }
            ("OpAppendTextBytes", [v, t, lo, hi]) => {
                let es = self.args(&[v.clone(), t.clone(), lo.clone(), hi.clone()], ind)?;
                let [v, t, lo, hi] = <[E; 4]>::try_from(es)
                    .map_err(|_| "a byte append without four operands".to_string())?;
                Ok(E::new(
                    format!(
                        "loft_append_text_bytes({}, {}, {}, {})",
                        as_ref(v)?,
                        as_view(t)?,
                        as_int(lo)?,
                        as_int(hi)?
                    ),
                    K::Void,
                ))
            }
            ("OpConvRefFromNull", []) => Ok(E::new("loft_ref_null()", K::Ref)),
            // The operand runs for its effect; the answer is the type's constant size.
            ("OpSizeScalar", [v, sz]) => {
                let sz = Self::literal(sz, "size")?;
                let v = self.expr(v, ind)?;
                Ok(E::new(
                    format!("((void) ({}), INT64_C({sz}))", v.code),
                    K::Int,
                ))
            }
            // `OpFnRefDetachShared(old, new)` (State::fn_ref_detach_shared): the displaced value
            // keeps no closure it shares a store with the new one, nor one it never had.
            ("OpFnRefDetachShared", [Value::Var(old), Value::Var(new)])
                if self.shapes[*old as usize] == CType::Fn
                    && self.shapes[*new as usize] == CType::Fn =>
            {
                let (o, n) = (self.var(*old), self.var(*new));
                Ok(E::new(
                    format!(
                        "{{ if ({o}.clos.rec == 0 || {o}.clos.store_nr == {n}.clos.store_nr) {o}.clos = loft_ref_null(); }}"
                    ),
                    K::Void,
                ))
            }
            _ => Err(format!("operator {name}")),
        }
    }
}
