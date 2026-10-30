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

    /// The remaining operators; the vector reshapes and sizes follow.
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
            // The place a store link names (`c = &v[1]`): its `loft_ref`.
            ("OpVarRef", [Value::Var(v)]) if self.store_links[*v as usize] => {
                Ok(E::new(super::var_name(self.vars.name(*v), *v), K::Ref))
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
                let es = self.args(&[v, t, lo, hi], ind)?;
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
            // `DbRef::NULL_REPORTED`: a null whose absence is accounted for, `pos`'s top bit set.
            ("OpNullReported", []) => Ok(E::new("loft_ref_null_reported()", K::Ref)),
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
            _ => self.vector_shape_operator(name, args, ind),
        }
    }

    /// The vector reshapes (insert, reserve, keep a range) and the size operators.
    fn vector_shape_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        match (name, args) {
            ("OpBitNotSingleInt", [v]) => {
                let v = self.expr(v, ind)?;
                Ok(E::new(format!("(~{})", as_int(v)?), K::Int))
            }
            ("OpSizeStruct", [r, sz]) => {
                let sz = Self::literal(sz, "size")?;
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("((void) {}, INT64_C({sz}))", as_ref(r)?),
                    K::Int,
                ))
            }
            ("OpSizeHash", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(E::new(
                    format!("loft_hash_table_bytes({})", as_ref(r)?),
                    K::Int,
                ))
            }
            ("OpReserveVector", [v, count, size]) => {
                let size = Self::literal(size, "element size")?;
                let (v, count) = self.two(name, &[v, count], ind)?;
                Ok(E::new(
                    format!(
                        "loft_vec_reserve({}, {}, {size})",
                        as_ref(v)?,
                        as_int(count)?
                    ),
                    K::Void,
                ))
            }
            ("OpInsertVector", [v, size, index, tp]) => {
                let size = Self::literal(size, "element size")?;
                let tp = u16::try_from(Self::literal(tp, "element type")?)
                    .map_err(|_| "an element type out of range")?;
                let (v, index) = self.two(name, &[v, index], ind)?;
                let (image, n) = self.cx.default_image(tp);
                let t = self.temp(CType::Ref);
                // A linked element is a record of its own, its slot the record's id.
                let insert = if self.cx.stores.is_linked(tp) {
                    format!(
                        "loft_vec_insert_linked({}, {}, {})",
                        as_ref(v)?,
                        self.cx.stores.size(tp),
                        as_int(index)?
                    )
                } else {
                    format!(
                        "loft_vec_insert({}, {size}, {})",
                        as_ref(v)?,
                        as_int(index)?
                    )
                };
                self.line(ind, &format!("{t} = {insert};"));
                self.line(ind, &format!("loft_fill({t}, {image}, {n});"));
                Ok(E::new(t, K::Ref))
            }
            ("OpKeepRange", [v, lo, hi, tp]) => {
                let tp = u16::try_from(Self::literal(tp, "element type")?)
                    .map_err(|_| "an element type out of range")?;
                if self.cx.stores.is_linked(tp) || self.cx.stores.type_owns_heap(tp) {
                    return Err("a kept range of elements that own records".into());
                }
                let size = self.cx.stores.size(tp);
                // `@FR-R-Refresh`'s keep-range clause: a push header on the path takes the
                // clamped length at the op's own site.
                let held = crate::opt::writes::vector_path(self.cx.data, v)
                    .and_then(|p| self.push_head(&p));
                let es = self.args(&[v, lo, hi], ind)?;
                let [v, lo, hi] = <[E; 3]>::try_from(es)
                    .map_err(|_| "a kept range without three operands".to_string())?;
                let keep = format!(
                    "loft_vec_keep_range({}, {}, {}, {size})",
                    as_ref(v)?,
                    as_int(lo)?,
                    as_int(hi)?
                );
                Ok(E::new(
                    match held {
                        Some((h, hs)) => {
                            format!("((void) ({keep}, {h} = loft_ph_of({h}.v, {hs})))")
                        }
                        None => keep,
                    },
                    K::Void,
                ))
            }
            _ => self.child_record_operator(name, args, ind),
        }
    }

    /// A struct field's child record (a function value's closure, `@FR-L-CapOwn`): the end of
    /// the operator chain, holding the refusal that names an operator not carried.
    fn child_record_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        match (name, args) {
            ("OpRefFromChildRec", [f]) => {
                let f = self.expr(f, ind)?;
                Ok(E::new(
                    format!("loft_ref_from_child_rec({})", as_ref(f)?),
                    K::Ref,
                ))
            }
            ("OpChildRec", [f, tp]) => {
                let tp =
                    u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
                self.cx.name_types();
                self.cx.describe_type(tp);
                let (image, n) = self.cx.default_image(tp);
                let f = self.expr(f, ind)?;
                let t = self.temp(CType::Ref);
                self.line(
                    ind,
                    &format!(
                        "{t} = loft_new_child_rec(loft_types, {tp}, {}, {image}, {n});",
                        as_ref(f)?
                    ),
                );
                Ok(E::new(t, K::Ref))
            }
            ("OpCopyClosure", [src, tp]) => {
                let tp =
                    u16::try_from(Self::literal(tp, "type")?).map_err(|_| "a type out of range")?;
                self.cx.name_types();
                self.cx.describe_type(tp);
                let owned = self.cx.stores.owned_capture_slots(tp);
                let list = if owned.is_empty() {
                    "0".to_string()
                } else {
                    let items: Vec<String> = owned.iter().map(u16::to_string).collect();
                    format!("(const uint16_t[]){{ {} }}", items.join(", "))
                };
                let src = self.expr(src, ind)?;
                let t = self.temp(CType::Ref);
                self.line(
                    ind,
                    &format!(
                        "{t} = loft_copy_closure(loft_types, {tp}, {}, {list}, {});",
                        as_ref(src)?,
                        owned.len()
                    ),
                );
                Ok(E::new(t, K::Ref))
            }
            // `@FR-B-OutOfLine` — a write into an out-of-line field.
            ("OpSetChildRec", [f, src, tp]) => {
                let raw = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type number out of range".to_string())?;
                let tp = self.copyable(raw & crate::keys::COPY_TP_MASK)?;
                let free = u8::from(raw & crate::keys::COPY_FREE_SOURCE != 0);
                let (f, src) = self.two(name, &[f, src], ind)?;
                Ok(E::new(
                    format!(
                        "loft_set_child_rec(loft_types, {tp}, {}, {}, {free})",
                        as_ref(f)?,
                        as_ref(src)?
                    ),
                    K::Void,
                ))
            }
            _ => Err(format!("operator {name}")),
        }
    }
}
