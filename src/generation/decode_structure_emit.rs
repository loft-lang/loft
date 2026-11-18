// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! `@FR-R-DecodeView`'s structure clause — the emission of the plan `decode_structure`
//! decides.  Every twin is the ordinary emission of the function's body with this module's
//! hooks in front of it (`ps_skip` in the statement loop, `ps_expr` in the dispatch,
//! `ps_operands` in pre-evaluation):
//!
//! - the producer's SHAPE TWIN `P__ps(B, position…) -> (tag, x0, x1, scalar fields…)`: a
//!   dropped statement emits nothing; a recursive call answers its shape into the call
//!   variable and its arguments into `__pa_<call>`; a scalar field of a call's result is a
//!   tuple element; a `return` answers the variant's tag, its slot — a number, a text run's
//!   span `(lo, hi)` when its guard holds, `i64::MIN` for a field only the built value
//!   answers — and the scalar fields;
//! - the producer's WALK TWIN `P__psw<k>(B, position…, f) -> Option<R>`: the same body, where
//!   an append to the walked field's local hands the element's positions to `f` and stops at
//!   its first answer, and a `return` ends the walk;
//! - a CONSUMER twin `G__ps(__psb, …)` holds positions in tuples: `(0, position…)`, or
//!   `(tag, 0…)` for a unit literal; a decoded record is `(position, scalar fields…)`, an
//!   element `(position, position)`.  The tag of a value is its shape's, computed once per
//!   arm; a text field is the frame's bytes over the span, or the built value's text for a
//!   field only it answers (`P__pst`); a walk is the walk twin with the loop's body as its
//!   closure.  A root keeps its name and signature.
use std::io::Write;

use super::decode_structure::{self, Producer, Slot, addr, op_name, unit_literal, unit_return};
use super::decode_structure_scan::{Consumer, Rep, tag_test};
use super::decode_view::int_of;
use super::{Output, disambiguated_fn_ident, sanitize};
use crate::data::{Block, Value};

/// The positional twin being emitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsMode {
    /// A producer's shape twin.
    Shape(u32),
    /// A producer's walk twin of the vector field `(offset, vector type)`.
    Walk(u32, (i64, u16)),
    /// A consumer's twin, or a root emitted in place.
    Consumer(u32),
}

/// How a parameter of the function being emitted appears in its positional twin.
pub enum PsParam {
    Same,
    Dropped,
    Typed(String),
}

impl Output<'_> {
    fn ps_producer_of(&self, pd: u32) -> &Producer {
        &self.plan.ps.producers[&pd]
    }

    /// The producer and consumer of the function being emitted, when it is a positional one.
    fn ps_consumer(&self) -> Option<&Consumer> {
        match self.ps_mode {
            Some(PsMode::Consumer(d)) => self.plan.ps.consumers.get(&d),
            _ => None,
        }
    }

    fn ps_cproducer(&self) -> Option<(u32, &Producer)> {
        let cons = self.ps_consumer()?;
        Some((cons.producer, self.ps_producer_of(cons.producer)))
    }

    fn ps_pproducer(&self) -> Option<(u32, &Producer)> {
        match self.ps_mode {
            Some(PsMode::Shape(d) | PsMode::Walk(d, _)) => Some((d, self.ps_producer_of(d))),
            _ => None,
        }
    }

    /// Is `d` emitted as a consumer TWIN (`d__ps`)?  A root is a consumer too, but it is
    /// emitted in its positional form under its own name, taking the frame — so a call of one
    /// from another root's twin stays a plain call.
    fn ps_twin(&self, d: u32) -> bool {
        self.plan.ps.consumers.contains_key(&d) && !self.plan.ps.roots.contains(&d)
    }

    fn ps_name(&self, d: u32, suffix: &str) -> String {
        format!(
            "{}{suffix}",
            disambiguated_fn_ident(&self.dup_fn_names, self.data.def(d))
        )
    }

    fn ps_var(&self, v: u16) -> String {
        format!(
            "var_{}",
            sanitize(self.data.def(self.def_nr).variables().name(v))
        )
    }

    /// The function's name suffix and generics, when it is a positional twin.
    pub(super) fn ps_signature_name(&self) -> Option<String> {
        match self.ps_mode? {
            PsMode::Shape(_) => Some("__ps".into()),
            PsMode::Walk(pd, key) => {
                let k = self
                    .plan
                    .ps
                    .walks
                    .get(&(pd, key.0, key.1))
                    .copied()
                    .unwrap_or(0);
                let prod = self.ps_producer_of(pd);
                Some(format!(
                    "__psw{k}<R, F: FnMut(&std::cell::UnsafeCell<Stores>, {}) -> Option<R>>",
                    self.ps_elem_type(prod, key.1)
                ))
            }
            PsMode::Consumer(d) if !self.plan.ps.roots.contains(&d) => Some("__ps".into()),
            PsMode::Consumer(_) => None,
        }
    }

    /// Parameters written right after `cell`.
    pub(super) fn ps_leading_params(&self) -> &'static str {
        match self.ps_mode {
            Some(PsMode::Consumer(d)) if !self.plan.ps.roots.contains(&d) => ", __psb: DbRef",
            _ => "",
        }
    }

    /// Parameters written after the function's own.
    pub(super) fn ps_trailing_params(&self) -> &'static str {
        match self.ps_mode {
            Some(PsMode::Walk(..)) => ", __walk: &mut F",
            _ => "",
        }
    }

    pub(super) fn ps_param(&self, def_nr: u32, i: usize) -> PsParam {
        let def = self.data.def(def_nr);
        let Some(args) = def.attributes().get(i) else {
            return PsParam::Same;
        };
        let v = def.variables().var(&args.name);
        match self.ps_mode {
            Some(PsMode::Shape(pd) | PsMode::Walk(pd, _)) => {
                if self.ps_producer_of(pd).retbuf == v {
                    PsParam::Dropped
                } else {
                    PsParam::Same
                }
            }
            Some(PsMode::Consumer(d)) => {
                let Some(cons) = self.plan.ps.consumers.get(&d) else {
                    return PsParam::Same;
                };
                if cons.retbuf == Some(v) {
                    return PsParam::Dropped;
                }
                match cons.reps.get(&v) {
                    Some(r) => {
                        PsParam::Typed(self.ps_rep_type(self.ps_producer_of(cons.producer), *r))
                    }
                    None => PsParam::Same,
                }
            }
            None => PsParam::Same,
        }
    }

    /// The return type of a positional twin.
    pub(super) fn ps_result(&self) -> Option<String> {
        match self.ps_mode? {
            PsMode::Shape(pd) => Some(self.ps_producer_of(pd).shape_tuple()),
            PsMode::Walk(..) => Some("Option<R>".into()),
            PsMode::Consumer(d) => {
                let cons = self.plan.ps.consumers.get(&d)?;
                let r = cons.returns?;
                Some(self.ps_rep_type(self.ps_producer_of(cons.producer), r))
            }
        }
    }

    fn ps_rep_type(&self, prod: &Producer, r: Rep) -> String {
        match r {
            Rep::T | Rep::V(_) => prod.rep_tuple(),
            Rep::D => {
                let mut parts = vec![prod.rep_tuple()];
                parts.extend(prod.scalars.iter().map(|s| s.rust.to_string()));
                format!("({})", parts.join(", "))
            }
            Rep::E(etp) => {
                let n = self.ps_elem_fields(etp).len();
                format!("({},)", vec![prod.rep_tuple(); n].join(", "))
            }
        }
    }

    fn ps_rep_zero(&self, prod: &Producer, r: Rep) -> String {
        let t = format!(
            "(0u8{}, 0u8, 0_i64, 0_i64)",
            ", Default::default()".repeat(prod.params.len())
        );
        match r {
            Rep::T | Rep::V(_) => t,
            Rep::D => {
                let mut parts = vec![t];
                parts.extend(
                    prod.scalars
                        .iter()
                        .map(|_| "Default::default()".to_string()),
                );
                format!("({})", parts.join(", "))
            }
            Rep::E(etp) => {
                let n = self.ps_elem_fields(etp).len();
                format!("({},)", vec![t; n].join(", "))
            }
        }
    }

    /// The element record's field offsets, in order.
    fn ps_elem_fields(&self, etp: u16) -> Vec<i64> {
        let mut offs: Vec<i64> = match self.stores.types.get(etp as usize).map(|t| &t.parts) {
            Some(crate::database::Parts::Struct(f)) => {
                f.iter().map(|x| i64::from(x.position)).collect()
            }
            _ => Vec::new(),
        };
        offs.sort_unstable();
        offs
    }

    fn ps_elem_type(&self, prod: &Producer, vtp: u16) -> String {
        match prod.elem.get(&vtp) {
            Some((_, true)) | None => prod.rep_tuple(),
            Some((etp, false)) => self.ps_rep_type(prod, Rep::E(*etp)),
        }
    }

    /// The prologue lines of a positional twin: its position locals, declared up front.
    pub(super) fn ps_prologue(&mut self, def_nr: u32) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        match self.ps_mode {
            Some(PsMode::Shape(pd) | PsMode::Walk(pd, _)) => {
                let prod = self.ps_producer_of(pd).clone();
                let _ = write!(
                    out,
                    "\n  let mut {}: DbRef = DbRef::NULL;",
                    self.ps_var(prod.retbuf)
                );
                self.declared.insert(prod.retbuf);
                let pos: Vec<&str> = prod.params.iter().map(|x| x.1).collect();
                let mut calls: Vec<u16> = prod.calls.iter().copied().collect();
                calls.sort_unstable();
                for cr in calls {
                    let _ = write!(
                        out,
                        "\n  let mut {}: {} = Default::default(); let mut __pa_{cr}: ({},) = Default::default();",
                        self.ps_var(cr),
                        prod.shape_tuple(),
                        pos.join(", ")
                    );
                    self.declared.insert(cr);
                }
            }
            Some(PsMode::Consumer(d)) => {
                let Some(cons) = self.plan.ps.consumers.get(&d).cloned() else {
                    return out;
                };
                let prod = self.ps_producer_of(cons.producer).clone();
                let vars = self.data.def(def_nr).variables();
                let mut reps: Vec<(u16, Rep)> = cons.reps.iter().map(|(v, r)| (*v, *r)).collect();
                reps.sort_unstable_by_key(|x| x.0);
                for (v, r) in reps {
                    if vars.is_argument(v) && cons.retbuf != Some(v) {
                        continue;
                    }
                    let _ = write!(
                        out,
                        "\n  let mut {}: {} = {};",
                        self.ps_var(v),
                        self.ps_rep_type(&prod, r),
                        self.ps_rep_zero(&prod, r)
                    );
                    self.declared.insert(v);
                }
                let _ = write!(out, "\n  let mut __pso: String = String::new();");
            }
            None => {}
        }
        out
    }

    /// The frame expression a positional consumer hands the producer.
    fn ps_frame(&self) -> String {
        match self.ps_mode {
            Some(PsMode::Shape(pd) | PsMode::Walk(pd, _)) => {
                self.ps_var(self.ps_producer_of(pd).frame)
            }
            Some(PsMode::Consumer(d)) if self.plan.ps.roots.contains(&d) => {
                let f = self
                    .plan
                    .ps
                    .consumers
                    .get(&d)
                    .and_then(|cons| cons.frame)
                    .unwrap_or(0);
                self.ps_var(f)
            }
            _ => "__psb".into(),
        }
    }

    /// A statement the positional twin drops.
    pub(super) fn ps_skip(&self, v: &Value) -> bool {
        match self.ps_mode {
            Some(PsMode::Shape(pd)) => self.ps_producer_of(pd).drop.contains(&addr(v)),
            Some(PsMode::Walk(pd, key)) => {
                let prod = self.ps_producer_of(pd);
                prod.drop.contains(&addr(v)) && !Self::ps_is_yield(prod, key, v)
            }
            Some(PsMode::Consumer(_)) => self
                .ps_consumer()
                .is_some_and(|cons| cons.skip.contains(&addr(v))),
            None => false,
        }
    }

    fn ps_is_yield(prod: &Producer, key: (i64, u16), v: &Value) -> bool {
        prod.yields.get(&addr(v)).is_some_and(|y| {
            prod.walked
                .get(&key)
                .is_some_and(|ls| ls.contains(&y.local))
        })
    }

    /// Operands pre-evaluation may search in a node the hook rewrites.
    pub(super) fn ps_operands<'v>(&self, x: &'v Value) -> Option<Vec<&'v Value>> {
        self.ps_mode?;
        if self.ps_skip(x) {
            return Some(Vec::new());
        }
        match x.unspan() {
            Value::Set(_, rhs) | Value::Return(rhs) => {
                if let Value::Call(callee, args) = rhs.unspan() {
                    self.ps_call_operands(*callee, args, x)
                } else if self.ps_owns(x) {
                    Some(Vec::new())
                } else {
                    None
                }
            }
            Value::Call(callee, args) => self.ps_call_operands(*callee, args, x),
            Value::If(cons, _, _) if tag_test(self.data, cons).is_some() => Some(Vec::new()),
            _ => None,
        }
    }

    /// A node this module writes in full.
    fn ps_owns(&self, x: &Value) -> bool {
        match x.unspan() {
            Value::Set(v, _) => {
                if let Some((_, prod)) = self.ps_pproducer() {
                    prod.calls.contains(v)
                } else {
                    self.ps_consumer()
                        .is_some_and(|cons| cons.reps.contains_key(v))
                }
            }
            Value::Return(_) => self
                .ps_consumer()
                .is_some_and(|cons| cons.returns.is_some()),
            _ => false,
        }
    }

    fn ps_call_operands<'v>(
        &self,
        callee: u32,
        args: &'v [Value],
        x: &'v Value,
    ) -> Option<Vec<&'v Value>> {
        if let Some((pd, prod)) = self.ps_pproducer() {
            if callee != pd {
                return None;
            }
            let def = self.data.def(pd);
            return Some(
                args.iter()
                    .enumerate()
                    .filter(|(i, _)| {
                        let v = def.variables().var(&def.attributes()[*i].name);
                        v != prod.frame && v != prod.retbuf
                    })
                    .map(|(_, y)| y)
                    .collect(),
            );
        }
        let (pd, prod) = self.ps_cproducer()?;
        let cons = self.ps_consumer()?;
        if callee == pd {
            let def = self.data.def(pd);
            return Some(
                args.iter()
                    .enumerate()
                    .filter(|(i, _)| {
                        let v = def.variables().var(&def.attributes()[*i].name);
                        v != prod.frame && v != prod.retbuf
                    })
                    .map(|(_, y)| y)
                    .collect(),
            );
        }
        if self.ps_twin(callee) {
            let gc = &self.plan.ps.consumers[&callee];
            let def = self.data.def(callee);
            return Some(
                args.iter()
                    .enumerate()
                    .filter(|(i, _)| {
                        let v = def.variables().var(&def.attributes()[*i].name);
                        gc.retbuf != Some(v) && !gc.reps.contains_key(&v)
                    })
                    .map(|(_, y)| y)
                    .collect(),
            );
        }
        let name = op_name(self.data, callee);
        if matches!(
            name,
            "OpGetEnum" | "OpGetText" | "OpGetBoolean" | "OpGetInt" | "OpConvIntFromEnum"
        ) && x.any_node(&mut |n| matches!(n, Value::Var(v) if cons.reps.contains_key(v)))
        {
            return Some(Vec::new());
        }
        if matches!(x.unspan(), Value::Set(..) | Value::Return(_)) && self.ps_owns(x) {
            return Some(Vec::new());
        }
        None
    }

    /// The hook: writes `node` when the positional twin rewrites it.  Answers whether it did.
    pub(super) fn ps_expr(&mut self, w: &mut dyn Write, node: &Value) -> std::io::Result<bool> {
        if self.ps_mode.is_none() || self.ps_bypass == addr(node) {
            return Ok(false);
        }
        if self.ps_pproducer().is_some() {
            return self.ps_producer_expr(w, node);
        }
        if self.ps_consumer().is_some() {
            return self.ps_consumer_expr(w, node);
        }
        Ok(false)
    }

    // ---- the producer's twins ----

    fn ps_producer_expr(&mut self, w: &mut dyn Write, node: &Value) -> std::io::Result<bool> {
        let (pd, prod) = self.ps_pproducer().expect("producer mode");
        let prod = prod.clone();
        let walk = match self.ps_mode {
            Some(PsMode::Walk(_, key)) => Some(key),
            _ => None,
        };
        if let Some(key) = walk
            && Self::ps_is_yield(&prod, key, node)
        {
            let y = &prod.yields[&addr(node)];
            let mut parts = y.parts.clone();
            parts.sort_unstable_by_key(|x| x.0);
            // Each part was decoded by the walk itself: its shape is the call variable's.
            let pos = |cr: u16| prod.known_rep(&format!("__pa_{cr}"), &self.ps_var(cr));
            let elem = if parts.len() == 1 && parts[0].0 < 0 {
                pos(parts[0].1)
            } else {
                format!(
                    "({},)",
                    parts
                        .iter()
                        .map(|x| pos(x.1))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            crate::rewrite_census::fired("R-DecodeView/structure", 1);
            write!(
                w,
                "/* @FR-R-DecodeView structure walk */ if let Some(__r) = __walk(cell, {elem}) {{ return Some(__r); }}"
            )?;
            return Ok(true);
        }
        if prod.drop.contains(&addr(node)) {
            write!(w, "()")?;
            return Ok(true);
        }
        match node.unspan() {
            Value::Block(bl) if decode_structure::returns_buffer(bl, prod.retbuf) => {
                if walk.is_some() {
                    write!(w, "{{ return None; }}")?;
                } else {
                    self.ps_shape_return(w, &prod, bl)?;
                }
                Ok(true)
            }
            Value::Set(cr, rhs) if prod.calls.contains(cr) => {
                let Value::Call(callee, args) = rhs.unspan() else {
                    return Ok(false);
                };
                if *callee != pd {
                    return Ok(false);
                }
                let cv = self.ps_var(*cr);
                write!(w, "{{ __pa_{cr} = (")?;
                self.ps_position_args(w, pd, &prod, args)?;
                write!(
                    w,
                    "); {cv} = {}(cell, {}",
                    self.ps_name(pd, "__ps"),
                    self.ps_var(prod.frame)
                )?;
                for i in 0..prod.params.len() {
                    write!(w, ", __pa_{cr}.{i}")?;
                }
                write!(w, "); }}")?;
                Ok(true)
            }
            Value::Call(callee, args)
                if matches!(op_name(self.data, *callee), "OpGetBoolean" | "OpGetInt") =>
            {
                if let [Value::Var(cr), off] = args.as_slice()
                    && prod.calls.contains(cr)
                    && let Some(i) = int_of(off).and_then(|o| prod.scalar_index(o))
                {
                    write!(w, "{}.{}", self.ps_var(*cr), 3 + i)?;
                    return Ok(true);
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    /// The position arguments of a producer call, as a tuple's elements.
    fn ps_position_args(
        &mut self,
        w: &mut dyn Write,
        pd: u32,
        prod: &Producer,
        args: &[Value],
    ) -> std::io::Result<()> {
        let def = self.data.def(pd);
        let mut first = true;
        for (i, args) in args.iter().enumerate() {
            let v = def.variables().var(&def.attributes()[i].name);
            if v == prod.frame || v == prod.retbuf {
                continue;
            }
            let rust = prod.params.iter().find(|x| x.0 == v).map_or("i64", |x| x.1);
            if !first {
                write!(w, ", ")?;
            }
            first = false;
            write!(w, "((")?;
            self.output_code_inner(w, args)?;
            write!(w, ") as {rust})")?;
        }
        write!(w, ",")
    }

    fn ps_shape_return(
        &mut self,
        w: &mut dyn Write,
        prod: &Producer,
        bl: &Block,
    ) -> std::io::Result<()> {
        let Some(r) = decode_structure::parse_return(self.data, prod, bl) else {
            // The plan admitted every return; a block it cannot read is a planner defect.
            write!(
                w,
                "{{ unreachable!(\"decode structure: an unread return\") }}"
            )?;
            return Ok(());
        };
        write!(w, "{{ ")?;
        match r.slot {
            Slot::None => write!(w, "let (__x0, __x1) = (0_i64, 0_i64); ")?,
            Slot::Num(v) => {
                write!(w, "let (__x0, __x1) = (((")?;
                self.output_code_inner(w, v)?;
                write!(w, ") as i64), 0_i64); ")?;
            }
            Slot::Text(callee, lo, hi) => {
                write!(w, "let (__x0, __x1) = if (((")?;
                self.output_code_inner(w, callee)?;
                write!(w, ") as u8) == 1) {{ ((")?;
                self.output_code_inner(w, lo)?;
                write!(w, ") as i64, (")?;
                self.output_code_inner(w, hi)?;
                write!(w, ") as i64) }} else {{ (i64::MIN, 0_i64) }}; ")?;
            }
            Slot::Built => write!(w, "let (__x0, __x1) = (i64::MIN, 0_i64); ")?,
        }
        write!(w, "return ({}u8, __x0, __x1", r.tag)?;
        for s in &prod.scalars {
            let Some((_, v)) = r.scalars.iter().find(|x| x.0 == s.off) else {
                write!(w, ", Default::default()")?;
                continue;
            };
            write!(w, ", ((")?;
            self.output_code_inner(w, v)?;
            write!(w, ") as {})", s.rust)?;
        }
        write!(w, "); }}")
    }

    // ---- the consumers ----

    fn ps_consumer_expr(&mut self, w: &mut dyn Write, node: &Value) -> std::io::Result<bool> {
        let cons = self.ps_consumer().expect("consumer mode").clone();
        let (pd, prod) = self.ps_cproducer().expect("consumer mode");
        let prod = prod.clone();
        if cons.skip.contains(&addr(node)) {
            write!(w, "()")?;
            return Ok(true);
        }
        if let Some(walk) = cons.walks.get(&addr(node)) {
            let Value::Block(bl) = node.unspan() else {
                return Ok(false);
            };
            self.ps_walk_block(w, &cons, pd, &prod, bl, walk)?;
            return Ok(true);
        }
        match node.unspan() {
            Value::Set(x, rhs) if cons.reps.contains_key(x) => {
                self.ps_set(w, &cons, pd, &prod, *x, rhs)?;
                Ok(true)
            }
            Value::Return(r) if cons.returns.is_some() => {
                write!(w, "return ")?;
                self.ps_wrap_open(w)?;
                self.ps_rep_value(w, &cons, &prod, r)?;
                self.ps_wrap_close(w)?;
                Ok(true)
            }
            Value::Block(bl) if cons.returns.is_some() && unit_return(self.data, bl).is_some() => {
                let tag = unit_return(self.data, bl).unwrap_or(1);
                write!(w, "{{ return ")?;
                self.ps_wrap_open(w)?;
                write!(w, "{}", prod.unit_rep(tag))?;
                self.ps_wrap_close(w)?;
                write!(w, "; }}")?;
                Ok(true)
            }
            Value::Block(bl) if bl.name == "materialized_view_return" => {
                let src = bl.operators.iter().find_map(|o| match o.unspan() {
                    Value::Call(callee, args) if op_name(self.data, *callee) == "OpCopyRecord" => {
                        Some(&args[0])
                    }
                    _ => None,
                });
                let Some(src) = src else {
                    return Ok(false);
                };
                write!(w, "{{ return ")?;
                self.ps_wrap_open(w)?;
                self.ps_rep_value(w, &cons, &prod, src)?;
                self.ps_wrap_close(w)?;
                write!(w, "; }}")?;
                Ok(true)
            }
            Value::If(test, _, _) => {
                let Some((x, _)) = tag_test(self.data, test) else {
                    return Ok(false);
                };
                if !self.ps_is_rep_expr(&cons, x) {
                    return Ok(false);
                }
                // The value's shape, once for the test and the arm.
                self.ps_serial += 1;
                let name = format!("__sh{}", self.ps_serial);
                write!(w, "{{ let {name} = ")?;
                self.ps_shape_of(w, &cons, pd, &prod, x)?;
                write!(w, "; ")?;
                self.ps_shapes.push((x.clone(), name));
                let saved = self.ps_bypass;
                self.ps_bypass = addr(node);
                let r = self.output_code_inner(w, node);
                self.ps_bypass = saved;
                self.ps_shapes.pop();
                r?;
                write!(w, " }}")?;
                Ok(true)
            }
            Value::Call(callee, args) => self.ps_consumer_call(w, &cons, pd, &prod, *callee, args),
            _ => Ok(false),
        }
    }

    fn ps_wrap_open(&self, w: &mut dyn Write) -> std::io::Result<()> {
        if self.ps_walk > 0 {
            write!(w, "Some(")?;
        }
        Ok(())
    }

    fn ps_wrap_close(&self, w: &mut dyn Write) -> std::io::Result<()> {
        if self.ps_walk > 0 {
            write!(w, ")")?;
        }
        Ok(())
    }

    fn ps_is_rep_expr(&self, cons: &Consumer, x: &Value) -> bool {
        match x.unspan() {
            Value::Var(v) => cons.reps.contains_key(v),
            Value::Call(callee, args) if op_name(self.data, *callee) == "OpGetField" => {
                matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if cons.reps.contains_key(v))
            }
            _ => false,
        }
    }

    /// A position-valued expression: a position local, a field of a decoded record or of an
    /// element, a vector field (its owner), a unit literal.
    fn ps_rep_value(
        &mut self,
        w: &mut dyn Write,
        cons: &Consumer,
        prod: &Producer,
        x: &Value,
    ) -> std::io::Result<()> {
        if let Some(tag) = unit_literal(self.data, x) {
            write!(w, "{}", prod.unit_rep(tag))?;
            return Ok(());
        }
        match x.unspan() {
            Value::Var(v) => write!(w, "{}", self.ps_var(*v)),
            Value::Call(callee, args) if op_name(self.data, *callee) == "OpGetField" => {
                let Some(Value::Var(v)) = args.first().map(Value::unspan) else {
                    return write!(w, "unreachable!()");
                };
                let off = int_of(&args[1]).unwrap_or(0);
                match cons.reps.get(v) {
                    Some(Rep::D) => write!(w, "{}.0", self.ps_var(*v)),
                    Some(Rep::E(etp)) => {
                        let k = self
                            .ps_elem_fields(*etp)
                            .iter()
                            .position(|o| *o == off)
                            .unwrap_or(0);
                        write!(w, "{}.{k}", self.ps_var(*v))
                    }
                    _ => write!(w, "{}", self.ps_var(*v)),
                }
            }
            _ => write!(w, "unreachable!()"),
        }
    }

    /// The shape of the value `x` names: the cached one inside its arm, else computed.
    fn ps_shape_of(
        &mut self,
        w: &mut dyn Write,
        cons: &Consumer,
        pd: u32,
        prod: &Producer,
        x: &Value,
    ) -> std::io::Result<()> {
        if let Some((_, name)) = self
            .ps_shapes
            .iter()
            .rev()
            .find(|(e, _)| e.unspan() == x.unspan())
        {
            return write!(w, "{name}");
        }
        write!(w, "{{ let __r = ")?;
        self.ps_rep_value(w, cons, prod, x)?;
        write!(
            w,
            "; if __r.0 == 0 {{ {}(cell, {}",
            self.ps_name(pd, "__ps"),
            self.ps_frame()
        )?;
        for i in 0..prod.params.len() {
            write!(w, ", __r.{}", i + 1)?;
        }
        let k = prod.params.len();
        write!(
            w,
            ") }} else {{ (__r.{}, __r.{}, __r.{}",
            k + 1,
            k + 2,
            k + 3
        )?;
        for _ in &prod.scalars {
            write!(w, ", Default::default()")?;
        }
        write!(w, ") }} }}")
    }

    fn ps_set(
        &mut self,
        w: &mut dyn Write,
        cons: &Consumer,
        pd: u32,
        prod: &Producer,
        x: u16,
        rhs: &Value,
    ) -> std::io::Result<()> {
        let xv = self.ps_var(x);
        match rhs.unspan() {
            Value::Null => write!(w, "()"),
            Value::Call(callee, args) if *callee == pd => {
                // The producer's record, held as the call's position.
                write!(w, "{xv} = {{ let __a = (")?;
                self.ps_position_args(w, pd, prod, args)?;
                write!(
                    w,
                    "); let __s = {}(cell, {}",
                    self.ps_name(pd, "__ps"),
                    self.ps_frame()
                )?;
                for i in 0..prod.params.len() {
                    write!(w, ", __a.{i}")?;
                }
                write!(w, "); ({}", prod.known_rep("__a", "__s"))?;
                for i in 0..prod.scalars.len() {
                    write!(w, ", __s.{}", 3 + i)?;
                }
                write!(w, ") }}")
            }
            Value::Call(callee, args) if self.ps_twin(*callee) => {
                write!(w, "{xv} = ")?;
                self.ps_consumer_call(w, cons, pd, prod, *callee, args)
                    .map(|_| ())
            }
            Value::Block(bl) if bl.name == "Object" => {
                // The decoder's own record literal: a unit value and its scalar fields.
                let mut tag = 1u8;
                let mut vals: Vec<(i64, &Value)> = Vec::new();
                for o in &bl.operators {
                    if let Value::Call(callee, args) = o.unspan() {
                        match (op_name(self.data, *callee), args.as_slice()) {
                            ("OpCopyRecord", [src, _, _]) => {
                                tag = unit_literal(self.data, src).unwrap_or(1);
                            }
                            ("OpSetInt" | "OpSetBoolean", [_, off, val]) => {
                                if let Some(o) = int_of(off) {
                                    vals.push((o, val));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                write!(w, "{xv} = ({}", prod.unit_rep(tag))?;
                for s in &prod.scalars {
                    write!(w, ", ")?;
                    match vals.iter().find(|v| v.0 == s.off) {
                        Some((_, v)) => {
                            write!(w, "((")?;
                            self.output_code_inner(w, v)?;
                            write!(w, ") as {})", s.rust)?;
                        }
                        None => write!(w, "Default::default()")?,
                    }
                }
                write!(w, ")")
            }
            _ => {
                write!(w, "{xv} = ")?;
                self.ps_rep_value(w, cons, prod, rhs)
            }
        }
    }

    fn ps_consumer_call(
        &mut self,
        w: &mut dyn Write,
        cons: &Consumer,
        pd: u32,
        prod: &Producer,
        callee: u32,
        args: &[Value],
    ) -> std::io::Result<bool> {
        let name = op_name(self.data, callee);
        match (name, args) {
            ("OpGetBoolean" | "OpGetInt", [Value::Var(d), off])
                if cons.reps.get(d) == Some(&Rep::D) =>
            {
                let i = int_of(off).and_then(|o| prod.scalar_index(o)).unwrap_or(0);
                write!(w, "{}.{}", self.ps_var(*d), 1 + i)?;
                return Ok(true);
            }
            ("OpGetInt" | "OpGetBoolean", [x, _]) if self.ps_is_rep_expr(cons, x) => {
                // The variant's number or boolean: the shape's first slot.
                let rust = if name == "OpGetInt" { "i64" } else { "u8" };
                write!(w, "((")?;
                self.ps_shape_of(w, cons, pd, prod, x)?;
                write!(w, ").1 as {rust})")?;
                return Ok(true);
            }
            ("OpGetEnum", [x, _]) if self.ps_is_rep_expr(cons, x) => {
                write!(w, "(")?;
                self.ps_shape_of(w, cons, pd, prod, x)?;
                write!(w, ").0")?;
                return Ok(true);
            }
            ("OpGetText", [x, off]) if self.ps_is_rep_expr(cons, x) => {
                let off = int_of(off).unwrap_or(0);
                self.ps_serial += 1;
                let k = self.ps_serial;
                write!(w, "{{ let __t{k} = ")?;
                self.ps_shape_of(w, cons, pd, prod, x)?;
                write!(w, "; let __q{k} = ")?;
                self.ps_rep_value(w, cons, prod, x)?;
                write!(
                    w,
                    "; if __t{k}.1 != i64::MIN {{ codegen_runtime::ps_text(stores, {}, __t{k}.1, __t{k}.2) }} else {{ __pso = {}(cell, {}",
                    self.ps_frame(),
                    self.ps_name(pd, "__pst"),
                    self.ps_frame()
                )?;
                for i in 0..prod.params.len() {
                    write!(w, ", __q{k}.{}", i + 1)?;
                }
                write!(w, ", {}); &*__pso }} }}", prod.vo + off)?;
                return Ok(true);
            }
            _ => {}
        }
        if !self.ps_twin(callee) {
            return Ok(false);
        }
        let Some(gc) = self.plan.ps.consumers.get(&callee).cloned() else {
            return Ok(false);
        };
        let def = self.data.def(callee);
        write!(
            w,
            "{}(cell, {}",
            self.ps_name(callee, "__ps"),
            self.ps_frame()
        )?;
        for (i, x) in args.iter().enumerate() {
            let v = def.variables().var(&def.attributes()[i].name);
            if gc.retbuf == Some(v) {
                continue;
            }
            write!(w, ", ")?;
            if gc.reps.contains_key(&v) {
                self.ps_rep_value(w, cons, prod, x)?;
            } else {
                self.output_code_inner(w, x)?;
            }
        }
        write!(w, ")")?;
        Ok(true)
    }

    /// `for e in v { body }` as the walk twin with the body as its closure.
    fn ps_walk_block(
        &mut self,
        w: &mut dyn Write,
        cons: &Consumer,
        pd: u32,
        prod: &Producer,
        bl: &Block,
        walk: &super::decode_structure_scan::Walk,
    ) -> std::io::Result<()> {
        let Some(Value::Loop(lb)) = bl
            .operators
            .iter()
            .map(Value::unspan)
            .find(|o| matches!(o, Value::Loop(_)))
        else {
            return write!(w, "unreachable!()");
        };
        let k = self
            .plan
            .ps
            .walks
            .get(&(pd, walk.field.0, walk.field.1))
            .copied()
            .unwrap_or(0);
        let ret = match cons.returns {
            Some(r) => self.ps_rep_type(prod, r),
            None => "()".into(),
        };
        // The owner: the vector local's source, a position.
        let owner = bl.operators.iter().find_map(|o| match o.unspan() {
            Value::Set(v, src) if *v == walk.vec => Some(src),
            _ => None,
        });
        crate::rewrite_census::fired("R-DecodeView/structure", 1);
        writeln!(w, "{{ //@FR-R-DecodeView structure walk")?;
        write!(w, "  let __pwo = ")?;
        match owner {
            Some(src) => self.ps_rep_value(w, cons, prod, src)?,
            None => write!(w, "{}", self.ps_var(walk.vec))?,
        }
        writeln!(w, ";")?;
        write!(
            w,
            "  let __pw: Option<{ret}> = if __pwo.0 <= 1 {{ {}(cell, {}",
            self.ps_name(pd, &format!("__psw{k}")),
            self.ps_frame()
        )?;
        for i in 0..prod.params.len() {
            write!(w, ", __pwo.{}", i + 1)?;
        }
        writeln!(
            w,
            ", &mut |cell: &std::cell::UnsafeCell<Stores>, __pe: {}| -> Option<{ret}> {{",
            self.ps_elem_type(prod, walk.field.1)
        )?;
        writeln!(
            w,
            "    let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
        writeln!(w, "    {} = __pe;", self.ps_var(walk.elem))?;
        self.ps_walk += 1;
        let mut r = Ok(());
        for o in &lb.operators[walk.body_from..] {
            if matches!(o.unspan(), Value::Line(_)) {
                continue;
            }
            write!(w, "    ")?;
            r = self.output_code_inner(w, o);
            if r.is_err() {
                break;
            }
            writeln!(w, ";")?;
        }
        self.ps_walk -= 1;
        r?;
        writeln!(w, "    None")?;
        writeln!(w, "  }}) }} else {{ None }};")?;
        if cons.returns.is_some() {
            write!(w, "  if let Some(__r) = __pw {{ return ")?;
            self.ps_wrap_open(w)?;
            write!(w, "__r")?;
            self.ps_wrap_close(w)?;
            writeln!(w, "; }}")?;
        } else {
            writeln!(w, "  let _ = __pw;")?;
        }
        write!(w, "}}")
    }

    /// The positional twins beside `dnr`: a producer's shape, walk and slow-text twins, a
    /// consumer's twin.  A root is emitted in place by `output_functions`.
    pub(super) fn output_ps_twins(
        &mut self,
        w: &mut dyn Write,
        dnr: u32,
        program_store: Option<&(crate::database::Stores, crate::keys::DbRef)>,
    ) -> std::io::Result<()> {
        if self.plan.ps.producers.contains_key(&dnr) {
            self.ps_mode = Some(PsMode::Shape(dnr));
            let r = self.output_function(w, dnr, program_store);
            self.ps_mode = None;
            r?;
            let mut walks: Vec<((u32, i64, u16), usize)> = self
                .plan
                .ps
                .walks
                .iter()
                .filter(|(k, _)| k.0 == dnr)
                .map(|(k, v)| (*k, *v))
                .collect();
            walks.sort_unstable_by_key(|x| x.1);
            for ((_, off, vtp), _) in walks {
                self.ps_mode = Some(PsMode::Walk(dnr, (off, vtp)));
                let r = self.output_function(w, dnr, program_store);
                self.ps_mode = None;
                r?;
            }
            self.output_ps_slow_text(w, dnr)?;
        }
        if self.ps_twin(dnr) {
            self.ps_mode = Some(PsMode::Consumer(dnr));
            let r = self.output_function(w, dnr, program_store);
            self.ps_mode = None;
            r?;
        }
        Ok(())
    }

    /// `P__pst(B, position…, off)`: the text at `off` of the value the plain decoder builds at
    /// the position — the answer for a text field only the built value holds.
    fn output_ps_slow_text(&mut self, w: &mut dyn Write, pd: u32) -> std::io::Result<()> {
        let prod = self.ps_producer_of(pd).clone();
        let def = self.data.def(pd);
        write!(
            w,
            "#[cold]\n#[inline(never)]\nfn {}(cell: &std::cell::UnsafeCell<Stores>, b: DbRef",
            self.ps_name(pd, "__pst")
        )?;
        for (i, (_, r)) in prod.params.iter().enumerate() {
            write!(w, ", p{i}: {r}")?;
        }
        writeln!(w, ", off: i64) -> String {{")?;
        write!(
            w,
            "  let d = {}(cell",
            disambiguated_fn_ident(&self.dup_fn_names, def)
        )?;
        let vars = def.variables();
        let mut k = 0;
        for args in def.attributes() {
            let v = vars.var(&args.name);
            if v == prod.frame {
                write!(w, ", b")?;
            } else if v == prod.retbuf {
                write!(w, ", DbRef::NULL")?;
            } else {
                write!(w, ", p{k}")?;
                k += 1;
            }
        }
        writeln!(w, ");")?;
        writeln!(
            w,
            "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
        writeln!(
            w,
            "  let t = if d.rec == 0 {{ String::new() }} else {{ let store = stores.store(&d); store.get_str(store.get_u32_raw(d.rec, d.pos + off as u32)).to_string() }};"
        )?;
        writeln!(w, "  OpFreeRef(cell, d, \"__pst\");")?;
        writeln!(w, "  t\n}}\n")
    }
}
