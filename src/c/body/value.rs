// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `@FR-R-ValueRecord` and `@FR-R-ValueLocal` in the C, from native's plan
//! (`opt::value_records`): an admitted function returns its record's fields as a `loft_tup`
//! struct and takes no return buffer; a value local and a tuple parameter are struct locals
//! whose field reads and writes are members; a copy from one materialises it into the
//! destination record field by field; a dead buffer is never minted or freed; and each exit
//! of an admitted function is a leaf — a build, a view's getters, the absent struct, a
//! dense struct widened with its presence word, the source of a materialised view.

use super::{CType, Cx, E, FnWriter, K, as_ref, as_shape, held};
use crate::data::{Block, Value};
use crate::opt::value_records::layout::{ValueTuple, layout_record};
use std::collections::{HashMap, HashSet};

/// One function's value-record facts, each native's own (`value_locals_in`, `dead_buffers`,
/// `forward_sites`, `value_leaves`), so the two backends write the same functions.
pub(super) struct VrFn {
    /// This function's own result layout, when it is admitted.
    pub(super) key: Option<u16>,
    pub(super) locals: HashMap<u16, u16>,
    pub(super) dead: HashSet<u16>,
    pub(super) forwards: HashMap<usize, u16>,
    pub(super) leaves: crate::opt::value_records::sites::ValueLeaves,
    /// The return buffer an admitted function's signature dropped.
    pub(super) phantom: Option<u16>,
}

impl VrFn {
    pub(super) fn of(cx: &Cx, d: u32) -> Option<VrFn> {
        let vr = cx.value_records();
        if vr.fns.is_empty() && vr.params.is_empty() {
            return None;
        }
        let data = cx.data;
        let locals = cx.vr_locals(d);
        let key = vr.fns.get(&d).copied();
        let def = data.def(d);
        let phantom = key
            .and_then(|_| crate::opt::attrs::ret_buffer_attr(def))
            .map(|a| def.variables().var(&def.attributes()[a].name))
            .filter(|v| *v != u16::MAX);
        Some(VrFn {
            key,
            dead: crate::opt::value_records::dead_buffers(data, d, &vr),
            forwards: crate::opt::value_records::forward_sites(data, d, &vr.fns, &locals),
            leaves: crate::opt::hoist::value_leaves(data, d, &vr, &locals),
            locals,
            phantom,
        })
    }
}

/// The address a leaf table keys a node by, spanned or not.
fn at(v: &Value) -> (usize, usize) {
    (
        std::ptr::from_ref(v) as usize,
        std::ptr::from_ref(v.unspan()) as usize,
    )
}

impl FnWriter<'_> {
    fn vr_layout(&self, key: u16) -> Result<ValueTuple, String> {
        self.cx
            .value_records()
            .types
            .get(&key)
            .cloned()
            .ok_or_else(|| "a value record without its layout".to_string())
    }

    fn vr_shape(&self, key: u16) -> Result<CType, String> {
        self.cx
            .vr_ctype(key)
            .ok_or_else(|| "a value record without its layout".to_string())
    }

    /// The layout key of value local (or tuple parameter) `v`.
    pub(super) fn vr_local(&self, v: u16) -> Option<u16> {
        self.vr.as_ref()?.locals.get(&v).copied()
    }

    /// `v` names no store: a value local, or the return buffer the signature dropped.
    fn vr_storeless(&self, v: u16) -> bool {
        self.vr
            .as_ref()
            .is_some_and(|f| f.locals.contains_key(&v) || f.phantom == Some(v))
    }

    pub(super) fn vr_dead(&self, v: u16) -> bool {
        self.vr.as_ref().is_some_and(|f| f.dead.contains(&v))
    }

    /// The struct member a field read of a value local at byte `off` names, with its shape.
    fn vr_member(&self, v: u16, off: i64) -> Option<(String, CType, u16)> {
        let key = self.vr_local(v)?;
        let idx = *self.cx.value_records().index.get(&(key, off))?;
        let CType::Tuple(id) = self.cx.vr_ctype(key)? else {
            return None;
        };
        let c = *self.cx.tuple_shapes(id).get(idx)?;
        Some((format!("{}.f{idx}", self.var(v)), c, key))
    }

    /// The presence member of a nullable layout's struct `s`, or `None` for a dense one.
    fn vr_presence(&self, key: u16, s: &str) -> Option<String> {
        let vr = self.cx.value_records();
        let at = vr.types.get(&key)?.presence()?;
        Some(format!("{s}.f{at}"))
    }

    /// The operators a value local, a phantom buffer or a dead buffer changes, or `None`.
    pub(super) fn vr_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let var_of = |a: Option<&Value>| match a.map(Value::unspan) {
            Some(Value::Var(v)) => Some(*v),
            _ => None,
        };
        let first = var_of(args.first());
        let void = || Ok(Some(E::new("((void) 0)", K::Void)));
        match name {
            // A tuple is in no store: there is nothing to release, and a dead buffer was
            // never minted.
            "OpFreeRef" | "OpFreeRefIfDistinct" | "OpClear"
                if first.is_some_and(|v| self.vr_storeless(v) || self.vr_dead(v)) =>
            {
                void()
            }
            // Guarded against a tuple: always distinct, so the free is unconditional.
            "OpFreeRefIfDistinct" if var_of(args.get(1)).is_some_and(|v| self.vr_storeless(v)) => {
                Ok(Some(self.operator("OpFreeRef", &args[..1], ind)?))
            }
            "OpDatabase" | "OpDatabaseNP" if first.is_some_and(|v| self.vr_dead(v)) => void(),
            "OpDistinctStore"
                if args
                    .iter()
                    .any(|a| var_of(Some(a)).is_some_and(|v| self.vr_storeless(v))) =>
            {
                Ok(Some(E::new("1", K::Cond)))
            }
            "OpRefAlias" if first.is_some_and(|v| self.vr_storeless(v)) => {
                Ok(Some(E::new("loft_ref_null()", K::Ref)))
            }
            // The presence test of a `τ?` value local reads its presence word; a dense one
            // the gate served this test for is surely present.
            "OpConvBoolFromRef" | "OpRefIsNull" if args.len() == 1 => {
                let Some(v) = first else { return Ok(None) };
                let Some(key) = self.vr_local(v) else {
                    return Ok(None);
                };
                let absent = name == "OpRefIsNull";
                Ok(Some(match self.vr_presence(key, &self.var(v)) {
                    Some(p) => E::new(
                        format!("({p} {} 1)", if absent { "!=" } else { "==" }),
                        K::Cond,
                    ),
                    None => E::new(if absent { "0" } else { "1" }, K::Cond),
                }))
            }
            "OpCopyRecord" => self.vr_copy(args, ind),
            // A view part of a value local is the reference to the place it views.
            "OpGetField" => {
                let [Value::Var(v), fld, ..] = args else {
                    return Ok(None);
                };
                let Value::Int(off) = fld.unspan() else {
                    return Ok(None);
                };
                let Some(key) = self.vr_local(*v) else {
                    return Ok(None);
                };
                let off = i64::from(*off);
                if !self
                    .cx
                    .value_records()
                    .view_offs
                    .get(&key)
                    .is_some_and(|o| o.contains(&off))
                {
                    return Ok(None);
                }
                Ok(self.vr_member(*v, off).map(|(m, _, _)| E::new(m, K::Ref)))
            }
            _ if crate::opt::loop_records::SCALAR_GETTERS.contains(&name) => {
                let [base, fld, ..] = args else {
                    return Ok(None);
                };
                let Some((v, off)) = crate::opt::body_writes::view_field(self.cx.data, base, fld)
                else {
                    return Ok(None);
                };
                Ok(self
                    .vr_member(v, off)
                    .map(|(m, c, _)| E::new(m, super::kind_of(c))))
            }
            "OpSetInt" | "OpSetFloat" | "OpSetSingle" | "OpSetBoolean" => {
                let [base, fld, val] = args else {
                    return Ok(None);
                };
                let Some((v, off)) = crate::opt::body_writes::view_field(self.cx.data, base, fld)
                else {
                    return Ok(None);
                };
                let Some((m, c, key)) = self.vr_member(v, off) else {
                    return Ok(None);
                };
                let val = self.expr(val, ind)?;
                let val = as_shape(val, c)?;
                // A write through an absent `τ?` struct lands nowhere, as the setter's null
                // record arm does.
                Ok(Some(match self.vr_presence(key, &self.var(v)) {
                    Some(p) => E::new(
                        format!("(({p} == 1) ? (void) ({m} = {val}) : (void) 0)"),
                        K::Void,
                    ),
                    None => E::new(format!("((void) ({m} = {val}))"), K::Void),
                }))
            }
            _ => Ok(None),
        }
    }

    /// A copy FROM a value local — or from an inline sub-record of one — is the struct
    /// materialised into the destination, one typed write per field, in field order.
    fn vr_copy(&mut self, args: &[Value], ind: usize) -> Result<Option<E>, String> {
        let [src, dst, tp_val] = args else {
            return Ok(None);
        };
        let data = self.cx.data;
        let (root, base, end) = match src.unspan() {
            Value::Var(v) if self.vr_local(*v).is_some() => (*v, 0, i64::MAX),
            _ => match crate::opt::body_writes::sub_record(data, src) {
                Some((root, base, sub)) if self.vr_local(root).is_some() => {
                    (root, base, base + i64::from(self.cx.stores.size(sub)))
                }
                _ => return Ok(None),
            },
        };
        let key = self.vr_local(root).ok_or("a copy from no value local")?;
        // `@FR-R-RecPtr`: an inline sub-record of a view whose address the block holds is written
        // at that address and the sub-record's offset (`bind_record_ptr` counts the copy).
        let held = crate::opt::body_writes::sub_record(data, dst).and_then(|(r, off, _)| {
            self.rec_ptrs
                .iter()
                .rev()
                .find(|(v, _, _)| *v == r)
                .map(|(_, pa, pl)| (pa.clone(), pl.clone(), off))
        });
        let dst = self.expr(dst, ind)?;
        let d = self.temp(CType::Ref);
        self.line(ind, &format!("{d} = {};", as_ref(dst)?));
        let s = self.var(root);
        // An absent `τ?` struct copies as the record form copies a null source: the same
        // `loft_copy_record` under the same type and flags (a fresh destination takes its
        // default, any other keeps what it held).
        match self.vr_presence(key, &s) {
            Some(p) => {
                let raw = u16::try_from(Self::literal(tp_val, "type")?)
                    .map_err(|_| "a type number out of range".to_string())?;
                let tp = self.copyable(raw & crate::keys::COPY_TP_MASK)?;
                let flags = u8::from(raw & crate::keys::COPY_FREE_SOURCE != 0)
                    | (u8::from(self.cx.swap_safe(tp)) << 1)
                    | (u8::from(raw & crate::keys::COPY_FRESH_DEST != 0) << 2);
                self.line(ind, &format!("if ({p} == 1) {{"));
                self.vr_write_fields(key, &d, &s, base, end, held.as_ref(), ind + 1)?;
                self.line(ind, "} else {");
                // A copy into a field of an element being built writes its default instead.
                let null_arm = self.fresh_null_arm(args, &d).unwrap_or_else(|| {
                    format!("loft_copy_record(loft_types, {tp}, loft_ref_null(), {d}, {flags});")
                });
                self.line(ind + 1, &null_arm);
                self.line(ind, "}");
            }
            None => self.vr_write_fields(key, &d, &s, base, end, held.as_ref(), ind)?,
        }
        Ok(Some(E::new("((void) 0)", K::Void)))
    }

    /// Write struct `s` of layout `key` into record `d`: each field at its offset less `base`,
    /// the fields from `base` up to `end`.
    /// `held` is the address, lock flag and offset of `d` where a bound view's address serves it.
    #[allow(clippy::too_many_arguments)]
    fn vr_write_fields(
        &mut self,
        key: u16,
        d: &str,
        s: &str,
        base: i64,
        end: i64,
        held: Option<&(String, String, i64)>,
        ind: usize,
    ) -> Result<(), String> {
        let layout = self.vr_layout(key)?;
        // Scalar fields only: the destination resolved once (`loft_rec_w`) and each field
        // written through it, where every setter would repeat the null test, the write check
        // and the address.  A view part claims, which may move the store, so its layout keeps
        // the setters.
        let fields: Vec<(usize, i64, &str)> = layout
            .fields
            .iter()
            .enumerate()
            .filter(|(_, (off, _))| *off >= base && *off < end)
            .map(|(i, (off, rt))| (i, *off - base, *rt))
            .collect();
        if fields.len() >= 2
            && !fields
                .iter()
                .any(|(_, _, rt)| crate::opt::value_records::layout::is_view_part(rt))
        {
            let at = match held {
                // As `loft_rec_w`: a locked store halts the run, or in production answers 0,
                // the null record, whose writes the run below skips.
                Some((pa, pl, off)) => format!(
                    "({pa} && !(LOFT_UNLIKELY({pl}) && loft_locked_write())) ? {pa} + {off} : (uint8_t *) 0"
                ),
                None => format!("loft_rec_w({d})"),
            };
            self.line(ind, &format!("{{ uint8_t *loft_w = {at}; if (loft_w) {{"));
            for (i, at, rt) in fields {
                let w = match rt {
                    "f32" => format!("memcpy(loft_w + {at}, &{s}.f{i}, 4);"),
                    "bool" => format!("loft_w[{at}] = (uint8_t) {s}.f{i};"),
                    _ => format!("memcpy(loft_w + {at}, &{s}.f{i}, 8);"),
                };
                self.line(ind + 1, &w);
            }
            self.line(ind, "} }");
            return Ok(());
        }
        for (i, (off, rt)) in layout.fields.iter().enumerate() {
            if *off < base || *off >= end {
                continue;
            }
            let at = *off - base;
            if crate::opt::value_records::layout::is_view_part(rt) {
                // A view part: the field emptied, then the deep copy of what the view names
                // (a null view copies nothing — the empty literal's value).
                if base != 0 {
                    return Err("a view part inside a copied sub-record".into());
                }
                let (_, elem) =
                    crate::opt::hoist::view_field_types(self.cx.stores, layout_record(key), *off)
                        .ok_or("a view part with no vector type")?;
                self.cx.copy_type(elem)?;
                let add = if self.cx.stores.is_linked(elem) {
                    "loft_vec_add_array"
                } else {
                    "loft_vec_add"
                };
                self.line(ind, &format!("loft_set_int4({d}, {at}, INT64_C(0));"));
                self.line(
                    ind,
                    &format!("{add}(loft_types, {elem}, loft_field({d}, {at}), {s}.f{i});"),
                );
                continue;
            }
            let set = match *rt {
                "f64" => format!("loft_set_float({d}, {at}, {s}.f{i});"),
                "f32" => format!("loft_set_single({d}, {at}, {s}.f{i});"),
                "bool" => format!("loft_set_byte({d}, {at}, {s}.f{i});"),
                _ => format!("loft_set_int({d}, {at}, {s}.f{i});"),
            };
            self.line(ind, &set);
        }
        Ok(())
    }

    /// A value local's bind: a null is the absent struct, anything else is assigned as a
    /// struct.  A dead buffer's bind is nothing.  `None` for any other variable.
    pub(super) fn vr_set(&mut self, var: u16, x: &Value, ind: usize) -> Option<Result<(), String>> {
        if self.vr_dead(var) {
            return Some(Ok(()));
        }
        let key = self.vr_local(var)?;
        Some((|| {
            let c = self.vr_shape(key)?;
            let target = self.var(var);
            if matches!(x.unspan(), Value::Null)
                || crate::opt::value_records::is_null_sentinel(self.cx.data, x)
            {
                self.line(
                    ind,
                    &format!("{target} = ({}) {};", held(c), self.cx.initial(c)),
                );
                return Ok(());
            }
            self.assign(x, &target, c, ind)?;
            self.flush(ind);
            Ok(())
        })())
    }

    /// A value LEAF at a struct position: the widened dense leaf, the absent struct, a build,
    /// a materialised view's source, a view's getters.  `Ok(false)` for any other value.
    pub(super) fn vr_assign(
        &mut self,
        value: &Value,
        target: &str,
        shape: CType,
        ind: usize,
    ) -> Result<bool, String> {
        let Some(facts) = self.vr.as_ref() else {
            return Ok(false);
        };
        if !matches!(shape, CType::Tuple(_)) {
            return Ok(false);
        }
        let (spanned, plain) = at(value);
        let keyed = |set: &HashSet<usize>| set.contains(&spanned) || set.contains(&plain);
        let widen = facts
            .leaves
            .widen
            .get(&spanned)
            .or_else(|| facts.leaves.widen.get(&plain))
            .copied();
        if let Some(key) = widen
            && self.widening != Some(spanned)
        {
            // The leaf as itself, dense, then its presence word: a dense struct is present.
            let dense = layout_record(key);
            let dense_shape = self.vr_shape(dense)?;
            let inner = self.temp(dense_shape);
            let saved = self.widening.replace(spanned);
            let done = self.assign(value, &inner, dense_shape, ind);
            self.widening = saved;
            done?;
            let count = self.vr_layout(dense)?.fields.len();
            let parts: Vec<String> = (0..count).map(|i| format!("{inner}.f{i}")).collect();
            self.line(
                ind,
                &format!(
                    "{target} = ({}) {{ {}, 1 }};",
                    held(shape),
                    parts.join(", ")
                ),
            );
            return Ok(true);
        }
        if keyed(&facts.leaves.nulls) {
            self.line(
                ind,
                &format!("{target} = ({}) {};", held(shape), self.cx.initial(shape)),
            );
            return Ok(true);
        }
        match value.unspan() {
            Value::Block(block) => {
                let addr = std::ptr::from_ref(&**block) as usize;
                if facts.leaves.objects.contains(&addr) {
                    return self.vr_object(block, target, shape, ind);
                }
                if facts.leaves.mv_returns.contains(&addr) {
                    self.vr_mv_return(block, target, shape, ind)?;
                    return Ok(true);
                }
                Ok(false)
            }
            Value::Var(var) if keyed(&facts.leaves.views) => {
                let key = facts
                    .key
                    .ok_or("a view leaf outside an admitted function")?;
                // Inside its widening the leaf is written as its dense self.
                let key = if self.cx.vr_ctype(key) == Some(shape) {
                    key
                } else {
                    layout_record(key)
                };
                let var = *var;
                self.vr_getters(key, &Value::Var(var), target, shape, ind)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// The struct of layout `key` read off the record `src` evaluates to, one getter per field
    /// — so a null record answers each field's null, as a later field read would — and, for a
    /// nullable layout, whether the record is there.
    fn vr_getters(
        &mut self,
        key: u16,
        src: &Value,
        target: &str,
        c: CType,
        ind: usize,
    ) -> Result<(), String> {
        let layout = self.vr_layout(key)?;
        // A sub-record of a value local is no record: each getter over it folds to the struct
        // member at the summed offset, as the callee's own reads would.
        if let Some((root, _, _)) = crate::opt::body_writes::sub_record(self.cx.data, src)
            && let Some(root_key) = self.vr_local(root)
        {
            let data = self.cx.data;
            let mut parts = Vec::new();
            for (off, kind) in &layout.fields {
                if crate::opt::value_records::layout::is_view_part(kind) {
                    return Err("a view part read off a value local's sub-record".into());
                }
                let getter = data.def_nr(crate::opt::hoist::value_getter(kind));
                let off = i32::try_from(*off).map_err(|_| "a field offset out of range")?;
                let e = self.expr(
                    &Value::Call(getter, vec![src.clone(), Value::Int(off)]),
                    ind,
                )?;
                let shape = match *kind {
                    "f64" => CType::Float,
                    "f32" => CType::Single,
                    "bool" => CType::Bool,
                    _ => CType::Int,
                };
                parts.push(as_shape(e, shape)?);
            }
            if layout.nullable {
                // Present exactly when the value local it is part of is.
                let root_name = self.var(root);
                parts.push(
                    self.vr_presence(root_key, &root_name)
                        .map_or_else(|| "1".to_string(), |p| format!("(uint8_t) ({p} == 1)")),
                );
            }
            self.line(
                ind,
                &format!("{target} = ({}) {{ {} }};", held(c), parts.join(", ")),
            );
            return Ok(());
        }
        let r = self.expr(src, ind)?;
        let rt = self.temp(CType::Ref);
        self.line(ind, &format!("{rt} = {};", as_ref(r)?));
        // The record's address taken once (`@FR-R-RecPtr`'s reading of a whole record): every
        // scalar field is one load through it, the null record's address null and each reader
        // answering its null, as the getter would.  A view part keeps its reference.
        // A block-local, so a generator's frame needs no slot for it.
        let pt = "loft_vp";
        let mut parts = Vec::new();
        for (off, kind) in &layout.fields {
            parts.push(match *kind {
                k if crate::opt::value_records::layout::is_view_part(k) => {
                    format!("loft_field({rt}, {off})")
                }
                "f64" => format!("loft_pa_f64({pt}, {off})"),
                "f32" => format!("loft_pa_f32({pt}, {off})"),
                "bool" => format!("loft_pa_bool({pt}, {off})"),
                _ => format!("loft_pa_i64({pt}, {off})"),
            });
        }
        if layout.nullable {
            parts.push(format!("(uint8_t) ({rt}.rec != 0)"));
        }
        self.line(
            ind,
            &format!(
                "{{ const uint8_t *{pt} = loft_rec_ptr({rt}); {target} = ({}) {{ {} }}; }}",
                held(c),
                parts.join(", ")
            ),
        );
        Ok(())
    }

    /// An `Object` leaf: the struct of the block's field writes (and of a sub-record copied
    /// into it), each value evaluated in the order the program wrote it.  An `Object` that
    /// returns what it builds returns the struct after the releases its return owes.
    ///
    /// A build whose fields are not all written by the block — a record update binds the
    /// buffer to its base first — is no leaf here: `Ok(false)`, and the block is written as
    /// it stands, over the buffer as the value local it is, as native writes it.
    fn vr_object(
        &mut self,
        bl: &Block,
        target: &str,
        c: CType,
        ind: usize,
    ) -> Result<bool, String> {
        let key = self
            .vr
            .as_ref()
            .and_then(|f| f.key)
            .ok_or("an Object leaf outside an admitted function")?;
        let layout = self.vr_layout(key)?;
        let vr = self.cx.value_records();
        let n = layout.fields.len();
        let (slots, fill) = self.vr_object_slots(bl, key, &layout)?;
        if layout.fields.iter().zip(&slots).any(|((_, rt), slot)| {
            slot.is_none() && !crate::opt::value_records::layout::is_view_part(rt)
        }) {
            return Ok(false);
        }
        // The parser's staged reads: the statements the literal reads before its writes.
        for op in &bl.operators {
            if matches!(op.unspan(), Value::Set(..)) {
                self.stmt(op, ind)?;
            }
        }
        let CType::Tuple(id) = c else {
            return Err("an Object leaf into a non-struct".into());
        };
        let shapes = self.cx.tuple_shapes(id);
        let mut parts: Vec<String> = vec![String::new(); n];
        for &i in &fill {
            let val = slots[i]
                .clone()
                .ok_or("a value record field left unwritten")?;
            let e = self.expr(&val, ind)?;
            let t = self.temp(shapes[i]);
            self.line(ind, &format!("{t} = {};", as_shape(e, shapes[i])?));
            parts[i] = t;
        }
        // A view leaf's part is the place this exit's gate proved (`ViewPlan::leaves`).
        let addr = std::ptr::from_ref(bl) as usize;
        for (i, (off, rt)) in layout.fields.iter().enumerate() {
            if !crate::opt::value_records::layout::is_view_part(rt) {
                continue;
            }
            let src = vr
                .views
                .get(&self.d)
                .and_then(|p| p.leaves.get(&(addr, *off)))
                .ok_or("a view leaf with no proven source")?
                .clone();
            parts[i] = self.vr_view_place(&src, ind)?;
        }
        if parts.iter().any(String::is_empty) {
            return Err("a value record field left unwritten".into());
        }
        if shapes.len() > n {
            parts.push("1".into());
        }
        self.line(
            ind,
            &format!("{target} = ({}) {{ {} }};", held(c), parts.join(", ")),
        );
        if let Some(p) = crate::opt::value_records::leaves::object_own_return(bl) {
            self.vr_object_releases(bl, p, ind)?;
            self.ret_held(target, ind);
        }
        Ok(true)
    }

    /// The value each field of an `Object` leaf of layout `key` is written, and the order the
    /// block writes them in: its `OpSet*` writes, and the getters of a sub-record copied in.
    /// A view field's write is the collection slot's zero-init, which the leaf replaces.
    fn vr_object_slots(
        &self,
        bl: &Block,
        key: u16,
        layout: &ValueTuple,
    ) -> Result<(Vec<Option<Value>>, Vec<usize>), String> {
        let data = self.cx.data;
        let vr = self.cx.value_records();
        let n = layout.fields.len();
        let mut slots: Vec<Option<Value>> = vec![None; n];
        let mut fill: Vec<usize> = Vec::new();
        for op in &bl.operators {
            let Value::Call(d, args) = op.unspan() else {
                continue;
            };
            let name = data.def(*d).name();
            if name == "OpCopyRecord"
                && let [src, dst, _] = args.as_slice()
                && let Value::Call(g, gargs) = dst.unspan()
                && data.def(*g).name() == "OpGetField"
                && let [_, sub_off, sub_tp] = gargs.as_slice()
                && let (Value::Int(sub_off), Value::Int(sub_tp)) =
                    (sub_off.unspan(), sub_tp.unspan())
                && let Ok(sub_tp) = u16::try_from(*sub_tp)
                && self.cx.stores.is_struct(sub_tp)
            {
                let base = i64::from(*sub_off);
                let end = base + i64::from(self.cx.stores.size(sub_tp));
                for (off, rt) in layout.fields.iter().filter(|(o, _)| *o >= base && *o < end) {
                    let idx = *vr
                        .index
                        .get(&(key, *off))
                        .ok_or("a sub-record field with no slot")?;
                    if slots[idx].is_some() {
                        return Err("a value record field written twice".into());
                    }
                    let getter = data.def_nr(crate::opt::hoist::value_getter(rt));
                    let off =
                        i32::try_from(*off - base).map_err(|_| "a field offset out of range")?;
                    slots[idx] = Some(Value::Call(getter, vec![src.clone(), Value::Int(off)]));
                    fill.push(idx);
                }
                continue;
            }
            if !name.starts_with("OpSet") {
                continue;
            }
            let (Some(base), Some(fld)) = (args.first(), args.get(1)) else {
                return Err("a field write without its place".into());
            };
            let (_, off) = crate::opt::body_writes::view_field(data, base, fld)
                .ok_or("a field write the value record cannot place")?;
            // A view field's write is the collection slot's own zero-init: the slot is
            // derived from the place the gate proved, below.
            if vr.view_offs.get(&key).is_some_and(|o| o.contains(&off)) {
                continue;
            }
            let idx = *vr
                .index
                .get(&(key, off))
                .ok_or("a field write with no slot")?;
            if slots[idx].is_some() {
                return Err("a value record field written twice".into());
            }
            slots[idx] = args.get(2).cloned();
            fill.push(idx);
        }
        Ok((slots, fill))
    }

    /// The statements of an `Object` that returns what it builds which are no part of the build,
    /// the yield or the return: the releases its return owes, run before the struct returns.
    fn vr_object_releases(&mut self, bl: &Block, p: u16, ind: usize) -> Result<(), String> {
        let data = self.cx.data;
        // Every statement that is no part of the build, the yield or the return is a
        // release the return owes, and runs; then the struct is returned.
        let at_buffer = |place: &Value| {
            crate::opt::writes::vector_path(data, place).is_some_and(|(r, _)| r == p)
        };
        let builds = |op: &Value| {
            op.any_node(&mut |n| {
                matches!(n, Value::Call(d, args)
                        if (*d as usize) < data.definitions.len()
                            && {
                                let nm = data.def(*d).name();
                                nm.starts_with("OpSet")
                                    || nm.starts_with("OpDatabase")
                                    || nm == "OpCopyRecord" && args.get(1).is_some_and(at_buffer)
                                    || nm == "OpAppendVector"
                            }
                            && (data.def(*d).name() == "OpCopyRecord"
                                || args.first().is_some_and(at_buffer)))
            })
        };
        for op in &bl.operators {
            match op.unspan() {
                Value::Return(_) | Value::Var(_) | Value::Line(_) | Value::Set(..) => {}
                o if builds(o) => {}
                _ => self.stmt(op, ind)?,
            }
        }
        Ok(())
    }

    /// The reference a view leaf delivers: none, the appended element's field, or the field
    /// of the container's last element.
    fn vr_view_place(
        &mut self,
        src: &crate::opt::value_records::layout::LeafSource,
        ind: usize,
    ) -> Result<String, String> {
        use crate::opt::value_records::layout::LeafSource;
        Ok(match src {
            LeafSource::Null => "loft_ref_null()".into(),
            LeafSource::Elem { elem, off, .. } => {
                let e = self.expr(&Value::Var(*elem), ind)?;
                format!("loft_field({}, {off})", as_ref(e)?)
            }
            LeafSource::Last {
                root,
                parent_tp,
                fld,
                off,
            } => {
                let stride = crate::opt::hoist::element_stride(self.cx.stores, *parent_tp, *fld);
                let r = self.expr(&Value::Var(root.0), ind)?;
                let r = as_ref(r)?;
                let container = if root.1 == crate::use_analysis::ANY_FIELD {
                    r
                } else {
                    format!("loft_field({r}, {})", root.1)
                };
                format!("loft_field(loft_vec_get({container}, {stride}, INT64_C(-1)), {off})")
            }
        })
    }

    /// A materialised view return at a struct position: the copy's source is the struct;
    /// the block's other statements run; the mint, the copy and the buffer's delivery go.
    pub(super) fn vr_mv_return(
        &mut self,
        bl: &Block,
        target: &str,
        c: CType,
        ind: usize,
    ) -> Result<(), String> {
        let data = self.cx.data;
        let src = crate::opt::value_records::sites::mv_return_source(bl, data)
            .ok_or("a materialised view return without its source")?;
        self.assign(src, target, c, ind)?;
        let (delivery, body) = bl.operators.split_last().ok_or("an empty view return")?;
        for op in body {
            if matches!(op.unspan(), Value::Call(d, _)
                if matches!(data.def(*d).name(), "OpCopyRecord" | "OpDatabase"))
            {
                continue;
            }
            self.stmt(op, ind)?;
        }
        if matches!(delivery.unspan(), Value::Return(_)) {
            self.ret_held(target, ind);
        }
        Ok(())
    }

    /// Does block `b` stand as a statement where it is a value leaf: a materialised view return,
    /// or a build that returns what it builds?
    pub(super) fn vr_stmt_leaf(&self, b: &Block) -> bool {
        self.vr.as_ref().is_some_and(|f| {
            let at = std::ptr::from_ref(b) as usize;
            f.leaves.mv_returns.contains(&at) || f.leaves.objects.contains(&at)
        })
    }

    /// Such a leaf as a statement returns its struct; a build the block does not write whole
    /// is the block as it stands.
    pub(super) fn vr_stmt(&mut self, v: &Value, b: &Block, ind: usize) -> Result<(), String> {
        let c = self.ret;
        let t = self.temp(c);
        if self.vr_assign(v, &t, c, ind)? {
            Ok(())
        } else {
            self.block_stmt(b, ind)
        }
    }

    /// The argument a tuple parameter of layout `key` takes for `arg`: the absent struct for a
    /// null, a struct the frame already holds (widened where the parameter is nullable), or
    /// the struct of a record's getters with, for a nullable layout, whether it is there.
    pub(super) fn vr_arg(&mut self, arg: &Value, key: u16, ind: usize) -> Result<String, String> {
        let c = self.vr_shape(key)?;
        if matches!(arg.unspan(), Value::Null)
            || crate::opt::value_records::is_null_sentinel(self.cx.data, arg)
        {
            return Ok(format!("(({}) {})", held(c), self.cx.initial(c)));
        }
        let form = {
            let vr = self.cx.value_records();
            let locals = self
                .vr
                .as_ref()
                .map(|f| f.locals.clone())
                .unwrap_or_default();
            crate::opt::hoist::arg_tuple_form(self.cx.data, self.d, &vr, &locals, arg, key)
        };
        let t = self.temp(c);
        match form {
            Some(false) => self.assign(arg, &t, c, ind)?,
            Some(true) => {
                let dense = layout_record(key);
                let dc = self.vr_shape(dense)?;
                let dt = self.temp(dc);
                self.assign(arg, &dt, dc, ind)?;
                let n = self.vr_layout(dense)?.fields.len();
                let parts: Vec<String> = (0..n).map(|i| format!("{dt}.f{i}")).collect();
                self.line(
                    ind,
                    &format!("{t} = ({}) {{ {}, 1 }};", held(c), parts.join(", ")),
                );
            }
            None => self.vr_getters(key, arg, &t, c, ind)?,
        }
        Ok(t)
    }

    /// What a call to `d` changes under the plan: its result layout, the return buffer its
    /// signature dropped, and its tuple parameters with their layouts.
    pub(super) fn vr_callee(&self, d: u32) -> (Option<u16>, Option<usize>, Vec<(usize, u16)>) {
        let vr = self.cx.value_records();
        let key = vr.fns.get(&d).copied();
        let dropped = key.and_then(|_| crate::opt::attrs::ret_buffer_attr(self.cx.data.def(d)));
        // In argument order: the call writes each tuple argument's struct in this order, and a
        // map's own order differed from run to run, so one program was emitted several ways.
        let mut params: Vec<(usize, u16)> = vr
            .params
            .get(&d)
            .map(|m| m.iter().map(|(i, k)| (*i, *k)).collect())
            .unwrap_or_default();
        params.sort_unstable();
        (key, dropped, params)
    }

    /// The arguments a value-record call writes itself: each tuple parameter's struct into
    /// `xs`.  Answers the argument positions left to the ordinary values, without the dropped
    /// buffer.
    pub(super) fn vr_args(
        &mut self,
        args: &[Value],
        values: Vec<usize>,
        dropped: Option<usize>,
        tuple_params: &[(usize, u16, bool)],
        xs: &mut [String],
        ind: usize,
    ) -> Result<Vec<usize>, String> {
        for &(i, key, ptr) in tuple_params {
            if let Some(a) = args.get(i) {
                let x = self.vr_arg(a, key, ind)?;
                xs[i] = if ptr {
                    self.vr_arg_address(args, i, &x, key, ind)?
                } else {
                    x
                };
            }
        }
        Ok(values
            .into_iter()
            .filter(|i| dropped != Some(*i) && !tuple_params.iter().any(|(t, _, _)| t == i))
            .collect())
    }

    /// The address a pointer-passed value record goes to its callee as (`Cx::vr_ptr_params`):
    /// the caller's own variable where the argument is one no other argument of the call names
    /// (the callee only reads it, and nothing the call runs can reach a C local but through an
    /// argument), else a temporary holding the struct `x`.
    fn vr_arg_address(
        &mut self,
        args: &[Value],
        i: usize,
        x: &str,
        key: u16,
        ind: usize,
    ) -> Result<String, String> {
        if let Value::Var(v) = args[i].unspan()
            && x == self.var(*v)
            && !args
                .iter()
                .enumerate()
                .any(|(j, o)| j != i && o.any_node(&mut |n| matches!(n, Value::Var(w) if w == v)))
        {
            return Ok(format!("&{x}"));
        }
        let c = self.vr_shape(key)?;
        let t = self.temp(c);
        self.line(ind, &format!("{t} = {x};"));
        Ok(format!("&{t}"))
    }

    /// The buffer of a FORWARD call: the site `forward_sites` names — by its node, or, where
    /// the C rewrote the node (a join's arm), by the forward's buffer handed as this call's
    /// return buffer.  Either is the record form's own effect: the callee writes the buffer it
    /// is handed and answers it.
    pub(super) fn vr_forward_target(&self, args: &[Value], dropped: Option<usize>) -> Option<u16> {
        let f = self.vr.as_ref()?;
        if let Some(b) = f.forwards.get(&(args.as_ptr() as usize)) {
            return Some(*b);
        }
        let Value::Var(b) = args.get(dropped?)?.unspan() else {
            return None;
        };
        f.forwards.values().any(|t| t == b).then_some(*b)
    }

    /// A FORWARD (`forward_sites`): the call handed buffer `b` whose answer binds back into
    /// `b` writes the callee's struct into `b` — minted where it is absent, as the callee's
    /// exit mints it — and evaluates to `b`.
    pub(super) fn vr_forward(
        &mut self,
        call: &str,
        callee_key: u16,
        buf: u16,
        ind: usize,
    ) -> Result<E, String> {
        let c = self.vr_shape(callee_key)?;
        let t = self.temp(c);
        self.line(ind, &format!("{t} = {call};"));
        let b = self.var(buf);
        let tp = layout_record(callee_key);
        let size = self.cx.stores.enum_parent_size(tp);
        self.cx.describe_type(tp);
        self.line(
            ind,
            &format!(
                "if (!({b}.store_nr != 0xFFFF && {b}.rec != 0)) loft_database_typed(&{b}, {size}, {tp});"
            ),
        );
        self.vr_write_fields(callee_key, &b, &t, 0, i64::MAX, None, ind)?;
        Ok(E::new(b, K::Ref))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_leaf_is_keyed_by_both_its_addresses() {
        let v = crate::data::Value::Var(3);
        let (a, b) = super::at(&v);
        assert_eq!(a, b, "an unspanned node is its own unspanned self");
    }
}
