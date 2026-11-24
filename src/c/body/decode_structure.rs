// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `@FR-R-DecodeView`'s structure clause (`@C141`) in the C: the emission of the plan
//! `lower::decode_structure` decides ([`crate::c::Cx::ps_plan`]).  Every twin is the ordinary
//! emission of the function's body with this module's hooks in front of it (`ps_stmt` in the
//! statement writer, `ps_operator` in the expression writer):
//!
//! - the producer's SHAPE TWIN `P__ps(cx, B, position…)` answers the struct `(tag, x0, x1,
//!   scalar fields…)`: a dropped statement writes nothing; a recursive call answers its shape
//!   into the call variable and its arguments into `loft_pa<call>`; a scalar field of a call's
//!   result is a struct member; a `return` answers the variant's tag, its slot — a number, a
//!   text run's span `(lo, hi)` when its guard holds, `INT64_MIN` for a field only the built
//!   value answers — and the scalar fields;
//! - the producer's WALK TWIN `P__psw<k>(cx, B, position…, f, env, out)` runs the same body,
//!   where an append to the walked field's local hands the element's positions to `f` and stops
//!   at its first answer, and a `return` ends the walk;
//! - a CONSUMER twin `G__ps(cx, loft_psb, …)` holds positions in structs: `(0, position…)`, or
//!   `(2, 0…, tag)` for a unit literal; a decoded record is `(position, scalar fields…)`, an
//!   element a struct of positions.  The tag of a value is its shape's, computed once per arm;
//!   a text field is the frame's bytes over the span, or the built value's text for a field
//!   only it answers (`P__pst`); a walk is the walk twin with the loop's body as a CLOSURE — a
//!   function of its own written by this writer, which copies the locals it names in from the
//!   site's address array at its start and back at each of its exits.  A root keeps its name
//!   and signature.
//!
//! Why a mention no hook here rewrites is sound: the plan admits a root only when every mention
//! of a position is one of the reads above, so the ordinary emission sees no position; one it
//! does see is a C type error the writer reports, and the unit is written without the clause.

use super::{E, EPILOGUE_MARK, FnWriter, K, as_cond, as_shape, var_name};
use crate::c::{CType, Cx, held, passed};
use crate::data::{Block, Value};
use crate::lower::decode_structure::{
    self as ds, Producer, PsPlan, Slot, addr, op_name, unit_literal, unit_return,
};
use crate::lower::decode_structure_scan::{Consumer, Rep, Walk, tag_test};
use crate::lower::decode_view::int_of;
use std::fmt::Write as _;
use std::rc::Rc;

/// The positional function being written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PsMode {
    /// A producer's shape twin.
    Shape(u32),
    /// A producer's walk twin of the vector field `(offset, vector type)`.
    Walk(u32, (i64, u16)),
    /// A consumer's twin, or a root written in place.
    Consumer(u32),
}

/// The marker a closure's exits carry: the copies back to the site's locals go there.
const PSC_OUT: &str = "\u{1}loft-psc-out\u{1}";

/// One walk's closure, written once the enclosing body's declarations are known.
pub(crate) struct Closure {
    name: String,
    text: String,
    /// The C names the closure copies in and back, each at its index in the site's array, and
    /// a parameter's declaration.
    copies: Vec<(String, Option<String>)>,
    /// The element's C type and the element variable.
    elem: (String, String),
}

/// What a body of the clause holds.
pub(crate) struct PsBody {
    mode: PsMode,
    plan: Rc<PsPlan>,
    /// Inside an arm that tested a value's tag: the value and the local holding its shape.
    shapes: Vec<(Value, String)>,
    /// How deep in walk closures the writer is.
    walk: usize,
    bypass: usize,
    closures: Vec<Closure>,
}

/// A position parameter's C shape: `i64` an integer, `u8` a boolean.
fn param_ctype(rust: &str) -> CType {
    if rust == "u8" {
        CType::Bool
    } else {
        CType::Int
    }
}

impl Cx<'_> {
    /// The shape struct `(tag, x0, x1, scalar fields…)`.
    pub(crate) fn ps_shape_t(&self, prod: &Producer) -> u32 {
        let mut parts = vec![CType::Enum, CType::Int, CType::Int];
        parts.extend(prod.scalars.iter().map(|s| param_ctype(s.rust)));
        self.tuple_id(parts)
    }

    /// A position struct: the kind (`0` shape unknown, `1` known, `2` a unit literal), the
    /// position's parameters, then the shape's tag and two slot words.
    pub(crate) fn ps_rep_t(&self, prod: &Producer) -> u32 {
        let mut parts = vec![CType::Enum];
        parts.extend(prod.params.iter().map(|p| param_ctype(p.1)));
        parts.extend([CType::Enum, CType::Int, CType::Int]);
        self.tuple_id(parts)
    }

    /// A recursive call's arguments: the position's parameters.
    fn ps_pa_t(&self, prod: &Producer) -> u32 {
        self.tuple_id(prod.params.iter().map(|p| param_ctype(p.1)).collect())
    }

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

    /// The C shape a consumer holds `r` in.
    pub(crate) fn ps_rep_ctype(&self, prod: &Producer, r: Rep) -> CType {
        let rep = CType::Tuple(self.ps_rep_t(prod));
        match r {
            Rep::T | Rep::V(_) => rep,
            Rep::D => {
                let mut parts = vec![rep];
                parts.extend(prod.scalars.iter().map(|s| param_ctype(s.rust)));
                CType::Tuple(self.tuple_id(parts))
            }
            Rep::E(etp) => {
                let n = self.ps_elem_fields(etp).len();
                CType::Tuple(self.tuple_id(vec![rep; n]))
            }
        }
    }

    /// The element a walk of vector store type `vtp` hands its closure.
    fn ps_elem_ctype(&self, prod: &Producer, vtp: u16) -> CType {
        match prod.elem.get(&vtp) {
            Some((_, true)) | None => CType::Tuple(self.ps_rep_t(prod)),
            Some((etp, false)) => self.ps_rep_ctype(prod, Rep::E(*etp)),
        }
    }

    /// A unit literal's position.
    fn ps_unit_rep(&self, prod: &Producer, tag: u8) -> String {
        let id = self.ps_rep_t(prod);
        let zeros = ", 0".repeat(prod.params.len());
        format!("((loft_tup{id}) {{ 2{zeros}, {tag}, 0, 0 }})")
    }

    /// The position `args` (a struct whose members `.f0…` are the parameters) with the shape
    /// `shape` known.
    fn ps_known_rep(&self, prod: &Producer, args: &str, shape: &str) -> String {
        let id = self.ps_rep_t(prod);
        let mut out = format!("((loft_tup{id}) {{ 1");
        for i in 0..prod.params.len() {
            let _ = write!(out, ", {args}.f{i}");
        }
        let _ = write!(out, ", {shape}.f0, {shape}.f1, {shape}.f2 }})");
        out
    }

    fn ps_name(&self, d: u32, suffix: &str) -> String {
        format!("{}{suffix}", crate::c::c_name(self.data, d))
    }

    /// Is `d` written as a consumer TWIN (`d__ps`)?  A root is a consumer too, but it is
    /// written in its positional form under its own name, taking its frame — so a call of one
    /// from another root stays a plain call.
    pub(crate) fn ps_twin(&self, d: u32) -> bool {
        let plan = self.ps_plan();
        plan.consumers.contains_key(&d) && !plan.roots.contains(&d)
    }
}

impl FnWriter<'_> {
    /// Write this body in the clause's `mode`.
    pub(crate) fn structure(mut self, mode: Option<PsMode>) -> Self {
        let Some(mode) = mode else {
            return self;
        };
        let plan = self.cx.ps_plan();
        match mode {
            PsMode::Shape(pd) | PsMode::Walk(pd, _) => {
                let prod = &plan.producers[&pd];
                // The buffer is a local nothing binds; each call's shape and arguments.
                self.params[prod.retbuf as usize] = false;
                self.undeclared.insert(prod.retbuf);
                self.decls.push(format!(
                    "loft_ref {} = {{ 0xFFFF, 0, 0, 0 }};",
                    self.var(prod.retbuf)
                ));
                let pa = self.cx.ps_pa_t(prod);
                let mut calls: Vec<u16> = prod.calls.iter().copied().collect();
                calls.sort_unstable();
                for cr in calls {
                    self.decls
                        .push(format!("loft_tup{pa} loft_pa{cr} = {{ 0 }};"));
                }
            }
            PsMode::Consumer(d) => {
                let cons = &plan.consumers[&d];
                let prod = &plan.producers[&cons.producer];
                if let Some(rb) = cons.retbuf {
                    self.params[rb as usize] = false;
                }
                let mut reps: Vec<(u16, Rep)> = cons.reps.iter().map(|(v, r)| (*v, *r)).collect();
                reps.sort_unstable_by_key(|x| x.0);
                for (v, r) in reps {
                    if self.params[v as usize] {
                        continue;
                    }
                    let c = self.cx.ps_rep_ctype(prod, r);
                    self.undeclared.insert(v);
                    self.decls
                        .push(format!("{} {} = {{ 0 }};", held(c), self.var(v)));
                }
            }
        }
        self.ps = Some(PsBody {
            mode,
            plan,
            shapes: Vec::new(),
            walk: 0,
            bypass: 0,
            closures: Vec::new(),
        });
        self
    }

    /// A producer or consumer twin keeps no vector hoist: its loops build nothing.
    pub(super) fn ps_lean(&self) -> bool {
        self.ps.is_some()
    }

    fn ps_plan(&self) -> Rc<PsPlan> {
        self.ps.as_ref().map(|p| p.plan.clone()).unwrap_or_default()
    }

    fn ps_mode(&self) -> Option<PsMode> {
        self.ps.as_ref().map(|p| p.mode)
    }

    /// The producer and the consumer of this body: the consumer's when it is one.
    fn ps_parts(&self) -> Option<(u32, Producer, Option<Consumer>)> {
        let plan = self.ps_plan();
        match self.ps_mode()? {
            PsMode::Shape(pd) | PsMode::Walk(pd, _) => {
                Some((pd, plan.producers[&pd].clone(), None))
            }
            PsMode::Consumer(d) => {
                let cons = plan.consumers.get(&d)?.clone();
                let prod = plan.producers.get(&cons.producer)?.clone();
                Some((cons.producer, prod, Some(cons)))
            }
        }
    }

    /// The frame expression this body hands the producer's twins.
    fn ps_frame(&self, prod: &Producer, cons: Option<&Consumer>) -> String {
        match (self.ps_mode(), cons) {
            (Some(PsMode::Consumer(d)), Some(c)) if self.ps_plan().roots.contains(&d) => {
                self.var(c.frame.unwrap_or(0))
            }
            (Some(PsMode::Consumer(_)), _) => "loft_psb".into(),
            _ => self.var(prod.frame),
        }
    }

    fn ps_hit(
        set: &std::collections::HashSet<usize, impl std::hash::BuildHasher>,
        v: &Value,
    ) -> bool {
        set.contains(&addr(v)) || set.contains(&addr(v.unspan()))
    }

    /// A statement this module writes in full.  Answers whether it wrote `v`.
    pub(super) fn ps_stmt(&mut self, v: &Value, ind: usize) -> Result<bool, String> {
        let Some(ps) = self.ps.as_ref() else {
            return Ok(false);
        };
        if ps.bypass == addr(v.unspan()) || ps.bypass == addr(v) {
            return Ok(false);
        }
        let Some((pd, prod, cons)) = self.ps_parts() else {
            return Ok(false);
        };
        match (self.ps_mode(), cons) {
            (Some(PsMode::Consumer(_)), Some(cons)) => {
                self.ps_consumer_stmt(v, pd, &prod, &cons, ind)
            }
            (Some(mode), None) => self.ps_producer_stmt(v, mode, pd, &prod, ind),
            _ => Ok(false),
        }
    }

    // ---- the producer's twins ----

    fn ps_is_yield(prod: &Producer, key: (i64, u16), v: &Value) -> Option<ds::Yield> {
        let y = prod
            .yields
            .get(&addr(v))
            .or_else(|| prod.yields.get(&addr(v.unspan())))?;
        prod.walked
            .get(&key)
            .is_some_and(|ls| ls.contains(&y.local))
            .then(|| y.clone())
    }

    fn ps_producer_stmt(
        &mut self,
        v: &Value,
        mode: PsMode,
        pd: u32,
        prod: &Producer,
        ind: usize,
    ) -> Result<bool, String> {
        let walk = match mode {
            PsMode::Walk(_, key) => Some(key),
            _ => None,
        };
        if let Some(key) = walk
            && let Some(y) = Self::ps_is_yield(prod, key, v)
        {
            let mut parts = y.parts.clone();
            parts.sort_unstable_by_key(|x| x.0);
            // Each part was decoded by the walk itself: its shape is the call variable's.
            let pos =
                |w: &Self, cr: u16| w.cx.ps_known_rep(prod, &format!("loft_pa{cr}"), &w.var(cr));
            let ec = self.cx.ps_elem_ctype(prod, key.1);
            let elem = if parts.len() == 1 && parts[0].0 < 0 {
                pos(self, parts[0].1)
            } else {
                let items: Vec<String> = parts.iter().map(|x| pos(self, x.1)).collect();
                format!("(({}) {{ {} }})", held(ec), items.join(", "))
            };
            crate::rewrite_census::fired("R-DecodeView/structure", 1);
            let e = self.temp(ec);
            self.line(
                ind,
                &format!("{e} = {elem}; /* @FR-R-DecodeView structure walk */"),
            );
            self.line(ind, &format!("if (loft_pf(cx, loft_pv, &{e}, loft_pr)) {{"));
            self.epilogue(ind + 1);
            self.line(ind + 1, "return 1;");
            self.line(ind, "}");
            return Ok(true);
        }
        if Self::ps_hit(&prod.drop, v) {
            return Ok(true);
        }
        match v.unspan() {
            Value::Block(bl) if ds::returns_buffer(bl, prod.retbuf) => {
                if walk.is_some() {
                    self.epilogue(ind);
                    self.line(ind, "return 0;");
                } else {
                    self.ps_shape_return(prod, bl, ind)?;
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
                let pa = self.ps_position_args(pd, prod, args, ind)?;
                let pa_t = self.cx.ps_pa_t(prod);
                self.line(ind, &format!("loft_pa{cr} = (loft_tup{pa_t}) {{ {pa} }};"));
                let mut call = format!(
                    "{}(cx, {}",
                    self.cx.ps_name(pd, "__ps"),
                    self.var(prod.frame)
                );
                for i in 0..prod.params.len() {
                    let _ = write!(call, ", loft_pa{cr}.f{i}");
                }
                self.line(ind, &format!("{} = {call});", self.var(*cr)));
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// The position arguments of a producer call, as a struct's members.
    fn ps_position_args(
        &mut self,
        pd: u32,
        prod: &Producer,
        args: &[Value],
        ind: usize,
    ) -> Result<String, String> {
        let def = self.cx.data.def(pd);
        let mut out = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let v = def.variables().var(&def.attributes()[i].name);
            if v == prod.frame || v == prod.retbuf {
                continue;
            }
            let rust = prod.params.iter().find(|x| x.0 == v).map_or("i64", |x| x.1);
            let e = self.expr(a, ind)?;
            out.push(as_shape(self.cx, e, param_ctype(rust))?);
        }
        Ok(out.join(", "))
    }

    fn ps_shape_return(&mut self, prod: &Producer, bl: &Block, ind: usize) -> Result<(), String> {
        let r = ds::parse_return(self.cx.data, prod, bl)
            .ok_or("decode structure: a return the plan cannot read")?;
        let id = self.cx.ps_shape_t(prod);
        let s = self.temp(CType::Tuple(id));
        let (x0, x1) = match r.slot {
            Slot::None => ("INT64_C(0)".to_string(), "INT64_C(0)".to_string()),
            Slot::Num(v) => {
                let e = self.expr(v, ind)?;
                let x = match e.kind {
                    K::Bool | K::Cond => format!("(int64_t) {}", super::as_bool(e)?),
                    _ => as_shape(self.cx, e, CType::Int)?,
                };
                (x, "INT64_C(0)".to_string())
            }
            Slot::Text(guard, lo, hi) => {
                let g = self.expr(guard, ind)?;
                let g = as_cond(g)?;
                let (a, b) = (self.temp(CType::Int), self.temp(CType::Int));
                self.line(ind, &format!("if ({g}) {{"));
                let lo = self.expr(lo, ind + 1)?;
                let lo = as_shape(self.cx, lo, CType::Int)?;
                let hi = self.expr(hi, ind + 1)?;
                let hi = as_shape(self.cx, hi, CType::Int)?;
                self.line(ind + 1, &format!("{a} = {lo};"));
                self.line(ind + 1, &format!("{b} = {hi};"));
                self.line(ind, "} else {");
                self.line(ind + 1, &format!("{a} = INT64_MIN;"));
                self.line(ind + 1, &format!("{b} = 0;"));
                self.line(ind, "}");
                (a, b)
            }
            Slot::Built => ("INT64_MIN".to_string(), "INT64_C(0)".to_string()),
        };
        let mut members = vec![format!("{}", r.tag), x0, x1];
        for sc in &prod.scalars {
            match r.scalars.iter().find(|x| x.0 == sc.off) {
                Some((_, v)) => {
                    let e = self.expr(v, ind)?;
                    members.push(as_shape(self.cx, e, param_ctype(sc.rust))?);
                }
                None => members.push("0".into()),
            }
        }
        self.line(
            ind,
            &format!("{s} = (loft_tup{id}) {{ {} }};", members.join(", ")),
        );
        self.epilogue(ind);
        self.line(ind, &format!("return {s};"));
        Ok(())
    }

    // ---- the consumers ----

    fn ps_consumer_stmt(
        &mut self,
        v: &Value,
        pd: u32,
        prod: &Producer,
        cons: &Consumer,
        ind: usize,
    ) -> Result<bool, String> {
        if Self::ps_hit(&cons.skip, v) {
            return Ok(true);
        }
        let walk = cons
            .walks
            .get(&addr(v))
            .or_else(|| cons.walks.get(&addr(v.unspan())))
            .cloned();
        if let Some(walk) = walk {
            let Value::Block(bl) = v.unspan() else {
                return Ok(false);
            };
            self.ps_walk_block(pd, prod, cons, bl, &walk, ind)?;
            return Ok(true);
        }
        match v.unspan() {
            Value::Set(x, rhs) if cons.reps.contains_key(x) => {
                self.ps_set(pd, prod, cons, *x, rhs, ind)?;
                Ok(true)
            }
            Value::Return(r) if cons.returns.is_some() => {
                let code = self.ps_rep_value(prod, cons, r)?;
                self.ps_return(prod, cons, &code, ind)?;
                Ok(true)
            }
            Value::Block(bl)
                if cons.returns.is_some() && unit_return(self.cx.data, bl).is_some() =>
            {
                let tag = unit_return(self.cx.data, bl).unwrap_or(1);
                let code = self.cx.ps_unit_rep(prod, tag);
                self.ps_return(prod, cons, &code, ind)?;
                Ok(true)
            }
            Value::Block(bl) if bl.name == "materialized_view_return" => {
                let src = bl.operators.iter().find_map(|o| match o.unspan() {
                    Value::Call(callee, args)
                        if op_name(self.cx.data, *callee) == "OpCopyRecord" =>
                    {
                        Some(&args[0])
                    }
                    _ => None,
                });
                let Some(src) = src else {
                    return Ok(false);
                };
                let code = self.ps_rep_value(prod, cons, src)?;
                self.ps_return(prod, cons, &code, ind)?;
                Ok(true)
            }
            Value::If(test, _, _) => {
                let Some((x, _)) = tag_test(self.cx.data, test) else {
                    return Ok(false);
                };
                if !self.ps_is_rep_expr(cons, x) {
                    return Ok(false);
                }
                // The value's shape, once for the test and the arm.
                let s = self.ps_shape_of(pd, prod, cons, x, ind)?;
                let key = addr(v.unspan());
                let ps = self.ps.as_mut().ok_or("no plan")?;
                ps.shapes.push((x.clone(), s));
                let saved = std::mem::replace(&mut ps.bypass, key);
                let r = self.stmt(v, ind);
                let ps = self.ps.as_mut().ok_or("no plan")?;
                ps.bypass = saved;
                ps.shapes.pop();
                r?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// `return code`: from the function, or — inside a walk's closure — into the site's
    /// answer, ending the walk.
    fn ps_return(
        &mut self,
        prod: &Producer,
        cons: &Consumer,
        code: &str,
        ind: usize,
    ) -> Result<(), String> {
        let c = cons
            .returns
            .map(|r| self.cx.ps_rep_ctype(prod, r))
            .ok_or("a return of no position")?;
        let t = self.temp(c);
        self.line(ind, &format!("{t} = {code};"));
        if self.ps.as_ref().is_some_and(|p| p.walk > 0) {
            self.line(ind, &format!("*({} *) loft_pr = {t};", held(c)));
            self.line(ind, PSC_OUT);
            self.epilogue(ind);
            self.line(ind, "return 1;");
        } else {
            self.epilogue(ind);
            self.line(ind, &format!("return {t};"));
        }
        Ok(())
    }

    fn ps_is_rep_expr(&self, cons: &Consumer, x: &Value) -> bool {
        match x.unspan() {
            Value::Var(v) => cons.reps.contains_key(v),
            Value::Call(callee, args) if op_name(self.cx.data, *callee) == "OpGetField" => {
                matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if cons.reps.contains_key(v))
            }
            _ => false,
        }
    }

    /// A position-valued expression: a position local, a field of a decoded record or of an
    /// element, a unit literal.
    fn ps_rep_value(&self, prod: &Producer, cons: &Consumer, x: &Value) -> Result<String, String> {
        if let Some(tag) = unit_literal(self.cx.data, x) {
            return Ok(self.cx.ps_unit_rep(prod, tag));
        }
        match x.unspan() {
            Value::Var(v) => Ok(self.var(*v)),
            Value::Call(callee, args) if op_name(self.cx.data, *callee) == "OpGetField" => {
                let Some(Value::Var(v)) = args.first().map(Value::unspan) else {
                    return Err("decode structure: a field of no position".into());
                };
                let off = int_of(&args[1]).unwrap_or(0);
                Ok(match cons.reps.get(v) {
                    Some(Rep::D) => format!("{}.f0", self.var(*v)),
                    Some(Rep::E(etp)) => {
                        let k = self
                            .cx
                            .ps_elem_fields(*etp)
                            .iter()
                            .position(|o| *o == off)
                            .unwrap_or(0);
                        format!("{}.f{k}", self.var(*v))
                    }
                    _ => self.var(*v),
                })
            }
            _ => Err("decode structure: no position".into()),
        }
    }

    /// The shape of the value `x` names: the cached one inside its arm, else computed.
    fn ps_shape_of(
        &mut self,
        pd: u32,
        prod: &Producer,
        cons: &Consumer,
        x: &Value,
        ind: usize,
    ) -> Result<String, String> {
        if let Some((_, name)) = self.ps.as_ref().and_then(|p| {
            p.shapes
                .iter()
                .rev()
                .find(|(e, _)| e.unspan() == x.unspan())
        }) {
            return Ok(name.clone());
        }
        let rep_t = self.cx.ps_rep_t(prod);
        let shape_t = self.cx.ps_shape_t(prod);
        let r = self.temp(CType::Tuple(rep_t));
        let s = self.temp(CType::Tuple(shape_t));
        let code = self.ps_rep_value(prod, cons, x)?;
        self.line(ind, &format!("{r} = {code};"));
        let k = prod.params.len();
        let mut call = format!(
            "{}(cx, {}",
            self.cx.ps_name(pd, "__ps"),
            self.ps_frame(prod, Some(cons))
        );
        for i in 0..k {
            let _ = write!(call, ", {r}.f{}", i + 1);
        }
        let zeros = ", 0".repeat(prod.scalars.len());
        self.line(
            ind,
            &format!(
                "{s} = {r}.f0 == 0 ? {call}) : (loft_tup{shape_t}) {{ {r}.f{}, {r}.f{}, {r}.f{}{zeros} }};",
                k + 1,
                k + 2,
                k + 3
            ),
        );
        Ok(s)
    }

    fn ps_set(
        &mut self,
        pd: u32,
        prod: &Producer,
        cons: &Consumer,
        x: u16,
        rhs: &Value,
        ind: usize,
    ) -> Result<(), String> {
        let xv = self.var(x);
        match rhs.unspan() {
            Value::Null => Ok(()),
            Value::Call(callee, args) if *callee == pd => {
                // The producer's record, held as the call's position.
                let pa = self.ps_position_args(pd, prod, args, ind)?;
                let pa_t = self.cx.ps_pa_t(prod);
                let a = self.temp(CType::Tuple(pa_t));
                let s = self.temp(CType::Tuple(self.cx.ps_shape_t(prod)));
                self.line(ind, &format!("{a} = (loft_tup{pa_t}) {{ {pa} }};"));
                let mut call = format!(
                    "{}(cx, {}",
                    self.cx.ps_name(pd, "__ps"),
                    self.ps_frame(prod, Some(cons))
                );
                for i in 0..prod.params.len() {
                    let _ = write!(call, ", {a}.f{i}");
                }
                self.line(ind, &format!("{s} = {call});"));
                let d_t = self.cx.ps_rep_ctype(prod, Rep::D);
                let mut members = vec![self.cx.ps_known_rep(prod, &a, &s)];
                for i in 0..prod.scalars.len() {
                    members.push(format!("{s}.f{}", 3 + i));
                }
                self.line(
                    ind,
                    &format!("{xv} = ({}) {{ {} }};", held(d_t), members.join(", ")),
                );
                Ok(())
            }
            Value::Call(callee, args) if self.cx.ps_twin(*callee) => {
                let code = self.ps_twin_call(prod, cons, *callee, args, ind)?;
                self.line(ind, &format!("{xv} = {code};"));
                Ok(())
            }
            Value::Block(bl) if bl.name == "Object" => {
                // The decoder's own record literal: a unit value and its scalar fields.
                let mut tag = 1u8;
                let mut vals: Vec<(i64, &Value)> = Vec::new();
                for o in &bl.operators {
                    if let Value::Call(callee, args) = o.unspan() {
                        match (op_name(self.cx.data, *callee), args.as_slice()) {
                            ("OpCopyRecord", [src, _, _]) => {
                                tag = unit_literal(self.cx.data, src).unwrap_or(1);
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
                let d_t = self.cx.ps_rep_ctype(prod, Rep::D);
                let mut members = vec![self.cx.ps_unit_rep(prod, tag)];
                for sc in &prod.scalars {
                    match vals.iter().find(|v| v.0 == sc.off) {
                        Some((_, v)) => {
                            let e = self.expr(v, ind)?;
                            members.push(as_shape(self.cx, e, param_ctype(sc.rust))?);
                        }
                        None => members.push("0".into()),
                    }
                }
                self.line(
                    ind,
                    &format!("{xv} = ({}) {{ {} }};", held(d_t), members.join(", ")),
                );
                Ok(())
            }
            _ => {
                let code = self.ps_rep_value(prod, cons, rhs)?;
                self.line(ind, &format!("{xv} = {code};"));
                Ok(())
            }
        }
    }

    /// A call of consumer twin `g`: its frame first, a position argument as its struct, the
    /// rest as the ordinary call writes them; no return buffer.
    fn ps_twin_call(
        &mut self,
        prod: &Producer,
        cons: &Consumer,
        g: u32,
        args: &[Value],
        ind: usize,
    ) -> Result<String, String> {
        let plan = self.ps_plan();
        let gc = plan
            .consumers
            .get(&g)
            .ok_or("a consumer twin without its plan")?
            .clone();
        let def = self.cx.data.def(g);
        let vars = def.variables();
        if args.len() != def.attributes().len() {
            return Err("a consumer call whose arguments do not match".into());
        }
        let shapes: Vec<CType> = def
            .attributes()
            .iter()
            .map(|a| self.cx.ctype(a.typedef.peel_link()).unwrap_or(CType::Void))
            .collect();
        let mut xs = vec![String::new(); args.len()];
        let (values, _) = self.link_args(g, args, &shapes, &mut xs, ind)?;
        let mut keep = vec![true; args.len()];
        for i in values {
            let v = vars.var(&def.attributes()[i].name);
            if gc.retbuf == Some(v) {
                keep[i] = false;
            } else if gc.reps.contains_key(&v) {
                xs[i] = self.ps_rep_value(prod, cons, &args[i])?;
            } else {
                let e = self.expr(&args[i], ind)?;
                xs[i] = as_shape(self.cx, e, shapes[i])?;
            }
        }
        let mut call = format!(
            "{}(cx, {}",
            self.cx.ps_name(g, "__ps"),
            self.ps_frame(prod, Some(cons))
        );
        for (x, k) in xs.iter().zip(keep) {
            if k {
                let _ = write!(call, ", {x}");
            }
        }
        call.push(')');
        Ok(call)
    }

    /// The reads of a position this module answers, and the calls of consumer twins: `None`
    /// for every other operator.
    pub(super) fn ps_operator(
        &mut self,
        d: u32,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        if self.ps.is_none() {
            return Ok(None);
        }
        let Some((pd, prod, cons)) = self.ps_parts() else {
            return Ok(None);
        };
        let name = op_name(self.cx.data, d).to_string();
        let Some(cons) = cons else {
            // The producer's twins: a scalar field of a call's result is its shape's member.
            if matches!(name.as_str(), "OpGetBoolean" | "OpGetInt")
                && let [Value::Var(cr), off] = args
                && prod.calls.contains(cr)
                && let Some(i) = int_of(off).and_then(|o| prod.scalar_index(o))
            {
                let k = if name == "OpGetInt" { K::Int } else { K::Bool };
                return Ok(Some(E::new(format!("{}.f{}", self.var(*cr), 3 + i), k)));
            }
            return Ok(None);
        };
        match (name.as_str(), args) {
            ("OpGetBoolean" | "OpGetInt", [Value::Var(x), off])
                if cons.reps.get(x) == Some(&Rep::D) =>
            {
                let i = int_of(off).and_then(|o| prod.scalar_index(o)).unwrap_or(0);
                let k = if name == "OpGetInt" { K::Int } else { K::Bool };
                return Ok(Some(E::new(format!("{}.f{}", self.var(*x), 1 + i), k)));
            }
            ("OpGetInt" | "OpGetBoolean", [x, _]) if self.ps_is_rep_expr(&cons, x) => {
                // The variant's number or boolean: the shape's first slot.
                let s = self.ps_shape_of(pd, &prod, &cons, x, ind)?;
                return Ok(Some(if name == "OpGetInt" {
                    E::new(format!("{s}.f1"), K::Int)
                } else {
                    E::new(format!("((uint8_t) {s}.f1)"), K::Bool)
                }));
            }
            ("OpGetEnum", [x, _]) if self.ps_is_rep_expr(&cons, x) => {
                let s = self.ps_shape_of(pd, &prod, &cons, x, ind)?;
                return Ok(Some(E::new(format!("{s}.f0"), K::Enum)));
            }
            ("OpGetText", [x, off]) if self.ps_is_rep_expr(&cons, x) => {
                return self.ps_text_read(pd, &prod, &cons, x, off, ind).map(Some);
            }
            _ => {}
        }
        if self.cx.ps_twin(d) {
            let code = self.ps_twin_call(&prod, &cons, d, args, ind)?;
            let plan = self.ps_plan();
            let gc = &plan.consumers[&d];
            if let Some(r) = gc.returns {
                return Ok(Some(match self.cx.ps_rep_ctype(&prod, r) {
                    CType::Tuple(id) => E::new(code, K::Tup(id)),
                    _ => return Err("a position that is no struct".into()),
                }));
            }
            let c = self
                .cx
                .ctype(self.cx.data.def(d).returned())
                .ok_or("a consumer twin of an unknown result")?;
            if c == CType::Text {
                let t = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_free(cx, &{t});"));
                self.line(ind, &format!("{t} = {code};"));
                return Ok(Some(self.held(t, CType::Text)));
            }
            return Ok(Some(E::new(code, super::kind_of(c))));
        }
        Ok(None)
    }

    /// A text leaf of a position: the frame's bytes over the span, or — for a field only the
    /// built value holds — that value's text, built at the position by the plain decoder.
    fn ps_text_read(
        &mut self,
        pd: u32,
        prod: &Producer,
        cons: &Consumer,
        x: &Value,
        off: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let off = int_of(off).unwrap_or(0);
        let field_view = std::mem::take(&mut self.field_view);
        let s = self.ps_shape_of(pd, prod, cons, x, ind)?;
        let q = self.temp(CType::Tuple(self.cx.ps_rep_t(prod)));
        let code = self.ps_rep_value(prod, cons, x)?;
        self.line(ind, &format!("{q} = {code};"));
        let frame = self.ps_frame(prod, Some(cons));
        let t = self.temp(CType::Text);
        let mut slow = format!("{}(cx, {frame}", self.cx.ps_name(pd, "__pst"));
        for i in 0..prod.params.len() {
            let _ = write!(slow, ", {q}.f{}", i + 1);
        }
        let _ = write!(slow, ", INT64_C({}))", prod.vo + off);
        self.line(ind, &format!("loft_text_free(cx, &{t});"));
        if field_view {
            // The slow text stays in its temporary until the site writes it again or the
            // function ends: a borrowed bind views it for longer than the statement.
            self.line(ind, &format!("if ({s}.f1 == INT64_MIN) {t} = {slow};"));
            return Ok(E::new(
                format!(
                    "({s}.f1 != INT64_MIN ? loft_ps_text(cx, {frame}, {s}.f1, {s}.f2) : loft_view(cx, &{t}))"
                ),
                K::View,
            ));
        }
        self.line(
            ind,
            &format!(
                "{t} = {s}.f1 != INT64_MIN ? loft_text_own(cx, loft_ps_text(cx, {frame}, {s}.f1, {s}.f2)) : {slow};"
            ),
        );
        Ok(self.held(t, CType::Text))
    }

    /// `for e in v.items { body }` as the walk twin with the body as its closure.
    fn ps_walk_block(
        &mut self,
        pd: u32,
        prod: &Producer,
        cons: &Consumer,
        bl: &Block,
        walk: &Walk,
        ind: usize,
    ) -> Result<(), String> {
        if self.ps.as_ref().is_some_and(|p| p.walk > 0) {
            return Err("decode structure: a walk inside a walk".into());
        }
        let Some(Value::Loop(lb)) = bl
            .operators
            .iter()
            .map(Value::unspan)
            .find(|o| matches!(o, Value::Loop(_)))
        else {
            return Err("decode structure: a walk without its loop".into());
        };
        let plan = self.ps_plan();
        let k = plan
            .walks
            .get(&(pd, walk.field.0, walk.field.1))
            .copied()
            .unwrap_or(0);
        // The owner: the vector local's source, a position.
        let owner = bl.operators.iter().find_map(|o| match o.unspan() {
            Value::Set(v, src) if *v == walk.vec => Some(src),
            _ => None,
        });
        let owner = match owner {
            Some(src) => self.ps_rep_value(prod, cons, src)?,
            None => self.var(walk.vec),
        };
        let ec = self.cx.ps_elem_ctype(prod, walk.field.1);
        // The closure's statements, written by this writer into a body of their own.
        let temps_before = self.temps.len();
        let decls_before = self.decls.len();
        let saved_body = std::mem::take(&mut self.body);
        let saved_pending = std::mem::take(&mut self.pending);
        if let Some(p) = self.ps.as_mut() {
            p.walk += 1;
        }
        let elem = self.var(walk.elem);
        let mut r = Ok(());
        for o in &lb.operators[walk.body_from..] {
            if matches!(o.unspan(), Value::Line(_)) {
                continue;
            }
            r = self.stmt(o, 1);
            if r.is_err() {
                break;
            }
        }
        self.flush(1);
        if let Some(p) = self.ps.as_mut() {
            p.walk -= 1;
        }
        let text = std::mem::replace(&mut self.body, saved_body);
        self.pending = saved_pending;
        r?;
        // What the closure copies in and back: every local, parameter, temporary and
        // declaration of the enclosing body it names, and the frame this body was handed.
        let names = super::parallel::identifiers(&text);
        let mut copies = Vec::new();
        for v in 0..self.shapes.len() {
            let v16 = v as u16;
            let name = var_name(self.vars.name(v16), v16);
            if !names.contains(name.as_str()) || v16 == walk.elem {
                continue;
            }
            if self.params[v] {
                let decl = self
                    .param_decls
                    .get(&v16)
                    .cloned()
                    .ok_or_else(|| format!("a walk reads parameter {}", self.vars.name(v16)))?;
                copies.push((name, Some(decl)));
            } else {
                copies.push((name, None));
            }
        }
        for (t, _) in &self.temps[..temps_before] {
            if names.contains(t.as_str()) {
                copies.push((t.clone(), None));
            }
        }
        for d in &self.decls[..decls_before] {
            if let Some(n) = super::parallel::declared_name(d)
                && names.contains(n)
                && !copies.iter().any(|(c, _)| c == n)
            {
                copies.push((n.to_string(), None));
            }
        }
        if names.contains("loft_psb") {
            copies.push(("loft_psb".into(), Some("loft_ref loft_psb".into())));
        }
        let id = self.ps.as_ref().map_or(0, |p| p.closures.len());
        let name = format!("{}__psc{id}", crate::c::c_name(self.cx.data, self.d));
        let pv = if copies.is_empty() {
            "0".to_string()
        } else {
            copies
                .iter()
                .map(|(c, _)| format!("(void *) &{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if let Some(p) = self.ps.as_mut() {
            p.closures.push(Closure {
                name: name.clone(),
                text,
                copies,
                elem: (held(ec), elem),
            });
        }
        crate::rewrite_census::fired("R-DecodeView/structure", 1);
        let rep_t = self.cx.ps_rep_t(prod);
        let o = self.temp(CType::Tuple(rep_t));
        self.line(
            ind,
            &format!("{o} = {owner}; /* @FR-R-DecodeView structure walk */"),
        );
        let ret = cons.returns.map(|r| self.cx.ps_rep_ctype(prod, r));
        let answer = match ret {
            Some(c) => self.temp(c),
            None => {
                let t = self.temp(CType::Int);
                t
            }
        };
        let frame = self.ps_frame(prod, Some(cons));
        let mut call = format!("{}(cx, {frame}", self.cx.ps_name(pd, &format!("__psw{k}")));
        for i in 0..prod.params.len() {
            let _ = write!(call, ", {o}.f{}", i + 1);
        }
        let _ = write!(call, ", {name}, loft_pv, (void *) &{answer})");
        self.line(ind, "{");
        self.line(ind + 1, &format!("void *const loft_pv[] = {{ {pv} }};"));
        self.line(ind + 1, &format!("if ({o}.f0 <= 1 && {call}) {{"));
        if ret.is_some() {
            if self.ps.as_ref().is_some_and(|p| p.walk > 0) {
                return Err("decode structure: a walk inside a walk".into());
            }
            self.epilogue(ind + 2);
            self.line(ind + 2, &format!("return {answer};"));
        }
        self.line(ind + 1, "}");
        self.line(ind, "}");
        Ok(())
    }

    /// The walks' closures, each declaring what the enclosing function declares
    /// (`prologue`), copying in what it reads and back at each of its exits, and freeing at
    /// its end only what it made.  Answers their declarations and their bodies.
    pub(super) fn ps_closure_functions(&self, prologue: &str) -> (String, String) {
        let mut decls = String::new();
        let mut bodies = String::new();
        let Some(ps) = self.ps.as_ref() else {
            return (decls, bodies);
        };
        for c in &ps.closures {
            let sig = format!(
                "static int {}(loft_cx *cx, void *const *loft_pv, const void *loft_pe, void *loft_pr)",
                c.name
            );
            let _ = writeln!(decls, "{sig};");
            let _ = writeln!(bodies, "\n{sig} {{");
            bodies.push_str(prologue);
            for (at, (name, decl)) in c.copies.iter().enumerate() {
                if let Some(d) = decl {
                    let _ = writeln!(bodies, "    {d};");
                }
                let _ = writeln!(bodies, "    memcpy(&{name}, loft_pv[{at}], sizeof {name});");
            }
            let _ = writeln!(bodies, "    (void) loft_pv;\n    (void) loft_pr;");
            let _ = writeln!(
                bodies,
                "    {} = *(const {} *) loft_pe;",
                c.elem.1, c.elem.0
            );
            let mut out_lines = String::new();
            for (at, (name, _)) in c.copies.iter().enumerate() {
                let _ = write!(
                    out_lines,
                    "memcpy(loft_pv[{at}], &{name}, sizeof {name}); memset(&{name}, 0, sizeof {name}); "
                );
            }
            let mut inner = c.text.clone();
            let _ = writeln!(inner, "    {PSC_OUT}");
            let _ = writeln!(inner, "    {EPILOGUE_MARK}");
            let _ = writeln!(inner, "    return 0;");
            let inner = inner.replace(PSC_OUT, &out_lines);
            bodies.push_str(&self.expand_epilogues_in(&inner));
            bodies.push_str("}\n");
        }
        (decls, bodies)
    }
}

/// The positional functions of the plan, written before the program's: the producers' shape,
/// walk and slow-text twins and the consumers' twins.  Answers their prototypes and bodies.
///
/// # Errors
/// A body this writer cannot write: the caller writes the unit without the clause.
pub(crate) fn ps_functions(cx: &Cx) -> Result<(String, String), String> {
    let plan = cx.ps_plan();
    let mut protos = String::new();
    let mut bodies = String::new();
    let mut producers: Vec<u32> = plan.producers.keys().copied().collect();
    producers.sort_unstable();
    for pd in producers {
        let (sig, body) = ps_function(cx, pd, PsMode::Shape(pd))?;
        let _ = writeln!(protos, "{sig};");
        let _ = write!(bodies, "\n{sig} {{\n{body}}}\n");
        let mut walks: Vec<((u32, i64, u16), usize)> = plan
            .walks
            .iter()
            .filter(|(k, _)| k.0 == pd)
            .map(|(k, v)| (*k, *v))
            .collect();
        walks.sort_unstable_by_key(|x| x.1);
        for ((_, off, vtp), _) in walks {
            let (sig, body) = ps_function(cx, pd, PsMode::Walk(pd, (off, vtp)))?;
            let _ = writeln!(protos, "{sig};");
            let _ = write!(bodies, "\n{sig} {{\n{body}}}\n");
        }
        let (sig, body) = ps_slow_text(cx, pd, &plan.producers[&pd])?;
        let _ = writeln!(protos, "{sig};");
        let _ = write!(bodies, "\n{sig} {{\n{body}}}\n");
    }
    let mut consumers: Vec<u32> = plan
        .consumers
        .keys()
        .copied()
        .filter(|d| cx.ps_twin(*d))
        .collect();
    consumers.sort_unstable();
    for d in consumers {
        let (sig, body) = ps_function(cx, d, PsMode::Consumer(d))?;
        let _ = writeln!(protos, "{sig};");
        let _ = write!(bodies, "\n{sig} {{\n{body}}}\n");
    }
    Ok((protos, bodies))
}

/// One positional twin: its signature and its framed body.
fn ps_function(cx: &Cx, d: u32, mode: PsMode) -> Result<(String, String), String> {
    let plan = cx.ps_plan();
    let data = cx.data;
    let def = data.def(d);
    let vars = def.variables();
    let args = vars.arguments();
    let mut shapes = crate::c::variable_shapes(cx, vars, def.code(), &args, data.body_width32(d))?;
    let (suffix, ret, dropped, lead, trail) = match mode {
        PsMode::Shape(pd) => {
            let prod = &plan.producers[&pd];
            for cr in &prod.calls {
                shapes[*cr as usize] = CType::Tuple(cx.ps_shape_t(prod));
            }
            (
                "__ps".to_string(),
                CType::Tuple(cx.ps_shape_t(prod)),
                Some(prod.retbuf),
                None,
                Vec::new(),
            )
        }
        PsMode::Walk(pd, key) => {
            let prod = &plan.producers[&pd];
            for cr in &prod.calls {
                shapes[*cr as usize] = CType::Tuple(cx.ps_shape_t(prod));
            }
            let k = plan.walks.get(&(pd, key.0, key.1)).copied().unwrap_or(0);
            (
                format!("__psw{k}"),
                CType::Bool,
                Some(prod.retbuf),
                None,
                vec![
                    "loft_ps_fn loft_pf".to_string(),
                    "void *const *loft_pv".to_string(),
                    "void *loft_pr".to_string(),
                ],
            )
        }
        PsMode::Consumer(g) => {
            let cons = &plan.consumers[&g];
            let prod = &plan.producers[&cons.producer];
            for (v, r) in &cons.reps {
                shapes[*v as usize] = cx.ps_rep_ctype(prod, *r);
            }
            let ret = match cons.returns {
                Some(r) => cx.ps_rep_ctype(prod, r),
                None => cx
                    .ctype(def.returned())
                    .ok_or("a consumer of an unknown result")?,
            };
            (
                "__ps".to_string(),
                ret,
                cons.retbuf,
                Some("loft_ref loft_psb".to_string()),
                Vec::new(),
            )
        }
    };
    let mut param_decls = std::collections::HashMap::new();
    let mut params = vec!["loft_cx *cx".to_string()];
    params.extend(lead);
    for &a in &args {
        if Some(a) == dropped {
            continue;
        }
        let c = shapes[a as usize];
        let name = var_name(vars.name(a), a);
        let decl = if matches!(vars.tp(a).base(), crate::data::Type::RefVar(_)) {
            format!("{} *{name}", held(c))
        } else {
            format!("{} {name}", passed(c))
        };
        param_decls.insert(a, decl.clone());
        params.push(decl);
    }
    params.extend(trail);
    let params = params.join(", ");
    let name = format!("{}{suffix}", crate::c::c_name(data, d));
    let (body, _) = FnWriter::new(cx, d, shapes, ret)
        .param_decls(param_decls)
        .structure(Some(mode))
        .write()
        .map_err(|why| format!("{name}: {why}"))?;
    let sig = format!("static {} {name}({params})", held(ret));
    Ok((
        sig,
        crate::c::framed_plain(cx, d, &name, &params, ret, &body),
    ))
}

/// `P__pst(cx, B, position…, off)`: the text at `off` of the value the plain decoder builds at
/// the position — the answer for a text field only the built value holds.
fn ps_slow_text(cx: &Cx, pd: u32, prod: &Producer) -> Result<(String, String), String> {
    let def = cx.data.def(pd);
    let vars = def.variables();
    let mut params = vec!["loft_cx *cx".to_string(), "loft_ref b".to_string()];
    for (i, (_, r)) in prod.params.iter().enumerate() {
        params.push(format!("{} p{i}", held(param_ctype(r))));
    }
    params.push("int64_t off".to_string());
    let sig = format!(
        "static loft_text {}({})",
        cx.ps_name(pd, "__pst"),
        params.join(", ")
    );
    let mut call = Vec::new();
    let mut k = 0;
    for a in def.attributes() {
        let v = vars.var(&a.name);
        if v == prod.frame {
            call.push("b".to_string());
        } else if v == prod.retbuf {
            call.push("loft_ref_null(cx)".to_string());
        } else {
            call.push(format!("p{k}"));
            k += 1;
        }
    }
    let body = format!(
        "    loft_ref d = {}(cx, {});\n    loft_text t = loft_text_own(cx, loft_get_text(cx, d, (uint32_t) off));\n    loft_free_ref(cx, d);\n    return t;\n",
        crate::c::c_name(cx.data, pd),
        call.join(", ")
    );
    Ok((sig, body))
}

/// A positional root's variable shapes: each position local the struct it is held in.
pub(crate) fn consumer_shapes(cx: &Cx, d: u32, shapes: &mut [CType]) {
    let plan = cx.ps_plan();
    let Some(cons) = plan.consumers.get(&d) else {
        return;
    };
    let prod = &plan.producers[&cons.producer];
    for (v, r) in &cons.reps {
        if let Some(s) = shapes.get_mut(*v as usize) {
            *s = cx.ps_rep_ctype(prod, *r);
        }
    }
}

/// `LOFT_TRACE_DECODE_STRUCTURE=1`: a positional body this writer declined, with the reason.
pub(crate) fn declined(cx: &Cx, d: u32, why: &str) {
    if std::env::var("LOFT_TRACE_DECODE_STRUCTURE").is_ok_and(|v| v != "0") {
        let name = if d == u32::MAX {
            "the unit".to_string()
        } else {
            cx.data.def(d).name().to_string()
        };
        eprintln!("[decode-structure] C {name}: written without the clause: {why}");
    }
}
