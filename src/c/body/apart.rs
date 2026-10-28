// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `@FR-R-Apart` instances 3 and 4 in the C.  Instance 3: a `vector<float>` local that never
//! leaves its function (`generation::apart_vector::roles`, native's gate) is a `loft_apvec` —
//! 64 elements inline, a spill beyond — and its companions (the store it was minted into, a work
//! buffer's saved value) emit nothing.  Instance 4: a record of RECORD VECTORS built in a frame and handed to
//! the functions that copy it into a collection is held as one byte buffer per field
//! (`loft_text loft_apr<v>[K]`, freed with the frame's texts) in the element's own layout, and
//! the store meets it once, at the BOUNDARY: the copy of an append group writes the bytes into
//! the appended element's blocks (`loft_apart_write_fresh`), or `@FR-R-HandOver`'s rotate
//! clause into the oldest element's (`loft_rotate_from_parts`).  The plan is native's
//! (`lower::apart_records::plan`, `roles`); this is its emission.
//!
//! An owner `v` and a twin's parameter are never declared as records: `expr(Var(v))` answers
//! the buffer array, which is what a twin call passes, and any mention no clause here rewrote
//! hands a `loft_text *` where a `loft_ref` is wanted — a C compile error, never a wrong value.
//! A push group's element is never declared either; its byte offset is `loft_apo<elm>`.
//! `LOFT_NO_APART_RECORDS=1` keeps every such record in the store.

use super::{E, FnWriter, K, as_bool, as_float, as_int};
use crate::data::Value;
use crate::generation::apart_vector;
use crate::lower::apart_records::{Enc, RecLayout};

impl FnWriter<'_> {
    /// Take the plan's roles for this body: its owners and their push groups' elements, or —
    /// for its `__apr` twin — the apart parameter.
    pub(crate) fn apart(mut self, twin: bool) -> Self {
        let plan = self.cx.apr_plan();
        let roles = crate::lower::apart_records::roles(self.cx.data, &plan, self.d, twin);
        if !roles.is_empty() {
            if !twin {
                crate::rewrite_census::fired("R-Apart/records", roles.vars.len());
            }
            if std::env::var("LOFT_TRACE_APART").is_ok_and(|v| v != "0") {
                eprintln!(
                    "[apart-records] C {}{}: {} apart, {} element(s)",
                    self.cx.data.def(self.d).name(),
                    if twin { "__apr" } else { "" },
                    roles.vars.len(),
                    roles.elems.len()
                );
            }
        }
        self.apr = roles;
        self.apr_plan = Some(plan);
        self.apart_vectors();
        self
    }

    /// Instance 3's roles for this body: its apart `vector<float>` locals and their companions.
    pub(super) fn apart_vectors(&mut self) {
        if let Some(roles) = apart_vector::roles(self.cx.data, self.d) {
            crate::rewrite_census::fired("R-Apart/vector", roles.apart.len());
            self.apv = roles;
        }
    }

    /// Is `v` an apart vector of this body (not a companion)?
    pub(super) fn apv_vec(&self, v: u16) -> bool {
        self.apv.apart.contains(&v)
    }

    /// The inline buffers of the apart vectors, as declarations, and their spills for the
    /// epilogue to free.
    pub(super) fn apv_decls(&mut self) {
        let mut vs: Vec<u16> = self.apv.apart.iter().copied().collect();
        vs.sort_unstable();
        for v in vs {
            self.decls.push(format!(
                "loft_apvec loft_apv{v} = {{ 0, {{ 0 }}, {{ 0, 0, 0 }} }};"
            ));
        }
    }

    pub(super) fn apv_texts(&self) -> Vec<String> {
        let mut vs: Vec<u16> = self.apv.apart.iter().copied().collect();
        vs.sort_unstable();
        vs.into_iter()
            .map(|v| format!("loft_apv{v}.spill"))
            .collect()
    }

    /// A statement of instance 3 that emits nothing of the store's: a free or a save naming only
    /// apart vectors and companions, a reservation, a companion's mint or emptying.
    fn apv_quiet(&self, name: &str, args: &[Value]) -> bool {
        let held = |a: &Value| matches!(a.unspan(), Value::Var(u) if self.apv.holds(*u));
        match name {
            "OpPutRef" | "OpFreeRefIfDistinct" | "OpFreeRef" => {
                !args.is_empty()
                    && args
                        .iter()
                        .all(|a| held(a) || !matches!(a.unspan(), Value::Var(_)))
                    && args.iter().any(held)
            }
            "OpPreAllocVector" => args
                .first()
                .is_some_and(|a| matches!(a.unspan(), Value::Var(u) if self.apv_vec(*u))),
            "OpDatabase" | "OpDatabaseNP" | "OpSetInt4" => args.first().is_some_and(
                |a| matches!(a.unspan(), Value::Var(u) if self.apv.companions.contains(u)),
            ),
            _ => false,
        }
    }

    /// Instance 3's operators and calls: an element read or write, a push, a length, a clear, a
    /// quiet statement.  `None` for any other.
    pub(super) fn apv_operator(
        &mut self,
        name: &str,
        x: &Value,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        if self.apv.is_empty() {
            return Ok(None);
        }
        let data = self.cx.data;
        if let Some((v, idx, _)) = apart_vector::read(data, x)
            && self.apv_vec(v)
        {
            let i = self.expr(idx, ind)?;
            return Ok(Some(E::new(
                format!("loft_apv_get(&loft_apv{v}, {})", as_int(i)?),
                K::Float,
            )));
        }
        if let Some((v, idx, val)) = apart_vector::write(data, x)
            && self.apv_vec(v)
        {
            let (i, y) = self.two("OpSetFloat", &[idx, val], ind)?;
            self.line(
                ind,
                &format!(
                    "loft_apv_set(&loft_apv{v}, {}, {});",
                    as_int(i)?,
                    as_float(y)?
                ),
            );
            return Ok(Some(E::new("((void) 0)", K::Void)));
        }
        if let Some((v, val)) = apart_vector::push(data, x)
            && self.apv_vec(v)
        {
            let y = self.expr(val, ind)?;
            self.line(
                ind,
                &format!("loft_apv_push(&loft_apv{v}, {});", as_float(y)?),
            );
            return Ok(Some(E::new("((void) 0)", K::Void)));
        }
        if let Some((op, v)) = apart_vector::unary(data, x)
            && self.apv_vec(v)
        {
            return Ok(Some(match op {
                "len" => E::new(format!("((int64_t) loft_apv{v}.n)"), K::Int),
                "clear" => {
                    self.line(ind, &format!("loft_apv_clear(&loft_apv{v});"));
                    E::new("((void) 0)", K::Void)
                }
                _ => E::new("((void) 0)", K::Void),
            }));
        }
        if self.apv_quiet(name, args) {
            return Ok(Some(E::new("((void) 0)", K::Void)));
        }
        Ok(None)
    }

    /// Instance 3's statements: an apart vector's empty init (a bind, or a work buffer's
    /// `if OpRefIsNull(v) { … } else { clear }`) is a clear; a companion's bind is nothing.
    /// Answers whether it wrote `x`.
    pub(super) fn apv_stmt(&mut self, x: &Value, ind: usize) -> bool {
        if self.apv.is_empty() {
            return false;
        }
        if let Some((v, _)) = apart_vector::buffer_init(self.cx.data, x)
            && self.apv_vec(v)
        {
            self.line(ind, &format!("loft_apv_clear(&loft_apv{v});"));
            return true;
        }
        match x.unspan() {
            Value::Set(v, _) if self.apv_vec(*v) => {
                self.line(ind, &format!("loft_apv_clear(&loft_apv{v});"));
                true
            }
            Value::Set(v, _) if self.apv.companions.contains(v) => true,
            _ => false,
        }
    }

    /// Is `v` held apart in this body — an owner, the twin's parameter, or a push group's
    /// element — so that it is not declared as a record?
    pub(super) fn apr_holds(&self, v: u16) -> bool {
        self.apr.holds(v) || self.apv.holds(v) || self.av_holds(v)
    }

    /// Does this body hold any value apart (instance 3 or 4)?
    pub(super) fn apr_any(&self) -> bool {
        !self.apr.is_empty() || !self.apv.is_empty() || self.av_any()
    }

    fn apr_layout(&self, v: u16) -> Option<&RecLayout> {
        let rec = self.apr.vars.get(&v)?;
        self.apr_plan.as_ref()?.types.get(rec)
    }

    /// The buffer array of apart value `v`: the twin's parameter, or the owner's local.
    pub(super) fn apr_buf(&self, v: u16) -> Option<String> {
        self.apr.vars.get(&v)?;
        Some(if self.apr.param == Some(v) {
            "loft_aprp".to_string()
        } else {
            format!("loft_apr{v}")
        })
    }

    /// The owners' buffers and the elements' offsets, as declarations; the buffers' texts for the
    /// epilogue to free.
    pub(super) fn apr_decls(&mut self) {
        let mut owners: Vec<u16> = self
            .apr
            .vars
            .keys()
            .copied()
            .filter(|v| self.apr.param != Some(*v))
            .collect();
        owners.sort_unstable();
        for v in owners {
            let k = self.apr_layout(v).map_or(0, |l| l.fields.len());
            let zero = vec!["{ 0, 0, 0 }"; k].join(", ");
            self.decls
                .push(format!("loft_text loft_apr{v}[{k}] = {{ {zero} }};"));
        }
        let mut elems: Vec<u16> = self.apr.elems.keys().copied().collect();
        elems.sort_unstable();
        for e in elems {
            self.decls.push(format!("size_t loft_apo{e} = 0;"));
        }
    }

    /// The owners' field buffers, which the epilogue frees with the frame's texts.
    pub(super) fn apr_texts(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut owners: Vec<u16> = self
            .apr
            .vars
            .keys()
            .copied()
            .filter(|v| self.apr.param != Some(*v))
            .collect();
        owners.sort_unstable();
        for v in owners {
            let k = self.apr_layout(v).map_or(0, |l| l.fields.len());
            out.extend((0..k).map(|i| format!("loft_apr{v}[{i}]")));
        }
        out
    }

    /// The field of apart value `base` at byte offset `off`: `(variable, field index)`.
    fn apr_field(&self, base: &Value, off: &Value) -> Option<(u16, usize)> {
        let Value::Var(v) = base.unspan() else {
            return None;
        };
        let off = match off.unspan() {
            Value::Int(i) => i64::from(*i),
            _ => return None,
        };
        Some((*v, self.apr_layout(*v)?.field(off)?))
    }

    /// A field's length `len(OpGetField(v, off, _))` of an apart value, or `None`.
    pub(super) fn apr_length(&self, name: &str, args: &[Value]) -> Option<E> {
        if self.apr.is_empty() || !matches!(name, "t_6vector_len" | "OpLengthVector") {
            return None;
        }
        let Value::Call(g, fa) = args.first()?.unspan() else {
            return None;
        };
        if self.cx.data.def(*g).name() != "OpGetField" {
            return None;
        }
        let (v, k) = self.apr_field(fa.first()?, fa.get(1)?)?;
        let esize = self.apr_layout(v)?.fields[k].esize;
        let buf = self.apr_buf(v)?;
        Some(E::new(
            format!("((int64_t) ({buf}[{k}].len / {esize}u))"),
            K::Int,
        ))
    }

    /// Does this call of `d` hand an apart value to `d`'s `__apr` twin at the position the plan
    /// names?  (`expr` answers the value's buffers there.)
    pub(super) fn apr_twin_call(&self, d: u32, args: &[Value]) -> bool {
        let Some(plan) = self.apr_plan.as_ref() else {
            return false;
        };
        let Some(&(_, at, rec)) = plan.twins.get(&d) else {
            return false;
        };
        matches!(args.get(at).map(Value::unspan), Some(Value::Var(v)) if self.apr.vars.get(v) == Some(&rec))
    }

    /// The statements of an owner's life written as statements: its null init, and a push group's
    /// element taken in its field's buffer.  Answers whether it wrote `Set(var, x)`.
    pub(super) fn apr_set(&mut self, var: u16, x: &Value, ind: usize) -> Result<bool, String> {
        if self.apr.is_empty() {
            return Ok(false);
        }
        if self.apr.vars.contains_key(&var) {
            if matches!(x.unspan(), Value::Null) && self.apr.param != Some(var) {
                return Ok(true);
            }
            return Err("an apart record assigned".into());
        }
        let Some(&(owner, k)) = self.apr.elems.get(&var) else {
            return Ok(false);
        };
        // The element's pre-init: no element yet, and nothing to write.
        if matches!(x.unspan(), Value::Null) {
            return Ok(true);
        }
        if !matches!(x.unspan(), Value::Call(d, _) if self.cx.data.def(*d).name() == "OpNewRecord")
        {
            return Err("an apart element bound to something other than its push".into());
        }
        let esize = self
            .apr_layout(owner)
            .ok_or("an element of no apart layout")?
            .fields[k]
            .esize;
        let buf = self.apr_buf(owner).ok_or("an element of no apart value")?;
        self.line(
            ind,
            &format!("loft_apo{var} = loft_apr_push(&{buf}[{k}], {esize}u);"),
        );
        Ok(true)
    }

    /// The operators of an owner's life and the boundary: a mint (every field emptied), a field
    /// emptied, a push group's finish and the owner's free (nothing), an element's field write
    /// (a store into its field's buffer), a field's length, and a twin's copy into an appended
    /// element (`loft_apart_write_fresh`).  `None` for any other operator.
    pub(super) fn apr_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        if self.apr.is_empty() {
            return Ok(None);
        }
        if let Some(e) = self.apr_length(name, args) {
            return Ok(Some(e));
        }
        let void = || Ok(Some(E::new("((void) 0)", K::Void)));
        let owner = |w: &Self, x: Option<&Value>| match x.map(Value::unspan) {
            Some(Value::Var(v)) if w.apr.vars.contains_key(v) && w.apr.param != Some(*v) => {
                Some(*v)
            }
            _ => None,
        };
        match name {
            "OpDatabase" | "OpDatabaseNP" if owner(self, args.first()).is_some() => {
                let v = owner(self, args.first()).unwrap_or_default();
                let k = self.apr_layout(v).map_or(0, |l| l.fields.len());
                let buf = self.apr_buf(v).ok_or("no apart buffer")?;
                for i in 0..k {
                    self.line(ind, &format!("{buf}[{i}].len = 0;"));
                }
                void()
            }
            "OpSetInt4" if owner(self, args.first()).is_some() => {
                let (v, k) = args
                    .get(1)
                    .and_then(|o| self.apr_field(&args[0], o))
                    .ok_or("an apart field emptied at no field")?;
                let buf = self.apr_buf(v).ok_or("no apart buffer")?;
                self.line(ind, &format!("{buf}[{k}].len = 0;"));
                void()
            }
            "OpFinishRecord" | "OpFreeRef" if owner(self, args.first()).is_some() => void(),
            "OpSetInt" | "OpSetFloat" | "OpSetBoolean" => {
                let [base, off, val] = args else {
                    return Ok(None);
                };
                let Value::Var(elm) = base.unspan() else {
                    return Ok(None);
                };
                let Some(&(v, k)) = self.apr.elems.get(elm) else {
                    return Ok(None);
                };
                let Value::Int(off) = off.unspan() else {
                    return Err("an element field at no offset".into());
                };
                let enc = match name {
                    "OpSetInt" => Enc::I64,
                    "OpSetFloat" => Enc::F64,
                    _ => Enc::Bool,
                };
                let x = self.expr(val, ind)?;
                let (ty, x, n) = match enc {
                    Enc::I64 => ("int64_t", as_int(x)?, 8),
                    Enc::F64 => ("double", as_float(x)?, 8),
                    Enc::Bool => ("uint8_t", as_bool(x)?, 1),
                };
                let buf = self.apr_buf(v).ok_or("no apart buffer")?;
                self.line(
                    ind,
                    &format!(
                        "{{ {ty} loft_apv = ({ty}) ({x}); memcpy({buf}[{k}].p + loft_apo{elm} + {off}, &loft_apv, {n}); }}"
                    ),
                );
                void()
            }
            "OpCopyRecord" => {
                let [src, dst, _] = args else {
                    return Ok(None);
                };
                let Value::Var(p) = src.unspan() else {
                    return Ok(None);
                };
                let Some(layout) = self.apr_layout(*p).cloned() else {
                    return Ok(None);
                };
                let buf = self.apr_buf(*p).ok_or("no apart buffer")?;
                let (slots, sizes) = parts(&layout);
                let d = self.expr(dst, ind)?;
                let d = super::as_ref(d)?;
                self.line(
                    ind,
                    &format!(
                        "loft_apart_write_fresh({d}, {buf}, {slots}, {sizes}, {}); /* @FR-R-Apart boundary */",
                        layout.fields.len()
                    ),
                );
                crate::rewrite_census::fired("R-Apart/records/boundary", 1);
                void()
            }
            _ => Ok(None),
        }
    }

    /// `@FR-R-HandOver`'s rotate from an apart source: the C call `loft_rotate_from_parts`, or
    /// `None` when the source is no apart value.
    pub(super) fn apr_rotate(
        &self,
        src: &Value,
        vec: &str,
        size: u32,
        k: i64,
        plan_c: &str,
        count: usize,
    ) -> Option<String> {
        let Value::Var(p) = src.unspan() else {
            return None;
        };
        let layout = self.apr_layout(*p)?;
        let buf = self.apr_buf(*p)?;
        let (slots, _) = parts(layout);
        Some(format!(
            "loft_rotate_from_parts({vec}, {buf}, {slots}, {}, {size}u, INT64_C({k}), {plan_c}, {count})",
            layout.fields.len()
        ))
    }
}

/// The fields' slots and element sizes of `layout`, as C array literals.
fn parts(layout: &RecLayout) -> (String, String) {
    let slots: Vec<String> = layout
        .fields
        .iter()
        .map(|f| format!("{}u", f.off))
        .collect();
    let sizes: Vec<String> = layout
        .fields
        .iter()
        .map(|f| format!("{}u", f.esize))
        .collect();
    (
        format!("(const uint32_t[]){{ {} }}", slots.join(", ")),
        format!("(const uint32_t[]){{ {} }}", sizes.join(", ")),
    )
}
