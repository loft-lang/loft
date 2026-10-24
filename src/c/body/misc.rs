// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! The operators no family above claims: integer `abs` and `**`, a vector reversed, sorted or
//! fed a text's bytes, the null reference, a scalar's size, and a closure detached from a
//! displaced function value.

use super::{CType, E, FnWriter, K, as_int, as_ref, as_view};
use crate::data::Value;

impl FnWriter<'_> {
    /// The remaining operators, or the refusal that names one.
    pub(super) fn misc_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        match (name, args) {
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
