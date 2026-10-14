// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! `@FR-R-DecodeView` — the emission of the plan `decode_view` decides.
//!
//! - NAMES: a region function is re-emitted as `<f>__dv` beside its own body, and every call
//!   a twin or a root makes to a region function names the twin.  The suffix rides on
//!   `fn_ident`, so a twin's ranged, destination and view-return variants take it too: the
//!   twin queues carry it as [`DV_BIT`] on the function number.
//! - THE FRAME: a root names its frame for the length of its call (`DvFrame`); a span read
//!   anywhere in the region reads that frame.
//! - A SPAN WRITE: a text run of a frame parameter into a span text field writes the span
//!   when the run's guard holds and its bytes are UTF-8, and the owned text otherwise; a
//!   byte slice of a frame parameter sets the local's span word, which the local's move into
//!   a span field writes.  Every other write of a span field clears the span word.
//! - A SPAN READ: `OpGetText` of a span text field answers the frame's bytes for a span and
//!   the field's own text otherwise.
use std::io::Write;

use super::Output;
use super::decode_view::{self, DV_BIT, DvType, int_of};
use crate::data::{Definition, Value};

impl Output<'_> {
    /// The function being emitted rewrites span fields: a `__dv` twin, or a root.
    pub(super) fn dv_on(&self) -> bool {
        self.dv_active || self.dv.roots.contains_key(&self.def_nr)
    }

    /// `def` is named by its `__dv` twin here: a region function, in a twin's body (its own
    /// signature included) or called from a root.
    pub(super) fn dv_renames(&self, def: &Definition) -> bool {
        if self.dv.names.is_empty() || !self.dv.names.contains(def.name()) {
            return false;
        }
        self.dv_active
            || (self.dv.roots.contains_key(&self.def_nr)
                && !std::ptr::eq(def, self.data.def(self.def_nr)))
    }

    /// A twin queue's key for callee `d`: its number, with [`DV_BIT`] for a `__dv` twin.
    pub(super) fn dv_key(&self, d: u32) -> u32 {
        if (d as usize) < self.data.definitions.len() && self.dv_renames(self.data.def(d)) {
            d | DV_BIT
        } else {
            d
        }
    }

    /// The prologue lines: a root's frame guard, a twin's span words of its byte locals.
    pub(super) fn dv_prologue(&self, def_nr: u32) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let vars = self.data.def(def_nr).variables();
        if !self.dv_active
            && let Some(&b) = self.dv.roots.get(&def_nr)
        {
            let _ = write!(
                out,
                "\n  let _dvg = codegen_runtime::DvFrame::enter(cell, var_{}); //@FR-R-DecodeView",
                super::sanitize(vars.name(b))
            );
        }
        if self.dv_active
            && let Some(locals) = self.dv.span_locals.get(&def_nr)
        {
            let mut locals: Vec<u16> = locals.iter().copied().collect();
            locals.sort_unstable();
            for v in locals {
                let _ = write!(out, "\n  let mut __dvs_{v}: i64 = 0;");
            }
        }
        out
    }

    fn dv_view(&self, rec: &Value) -> Option<DvType> {
        decode_view::view_of(&self.dv, self.data, self.stores, self.def_nr, rec).cloned()
    }

    fn dv_frame_param(&self, v: u16) -> bool {
        self.dv
            .bparams
            .get(&self.def_nr)
            .is_some_and(|s| s.contains(&v))
    }

    fn dv_span_local(&self, v: u16) -> bool {
        self.dv_active
            && self
                .dv
                .span_locals
                .get(&self.def_nr)
                .is_some_and(|s| s.contains(&v))
    }

    /// The span text field `OpSetText(rec, off, val)` writes, if `x` is one.
    fn dv_text_set<'v>(&self, x: &'v Value) -> Option<(&'v Value, u32, &'v Value, u32)> {
        let Value::Call(d, args) = x.unspan() else {
            return None;
        };
        if self.data.def(*d).name() != "OpSetText" {
            return None;
        }
        let [rec, off, val] = args.as_slice() else {
            return None;
        };
        let view = self.dv_view(rec)?;
        let off = u32::try_from(int_of(off)?).ok()?;
        view.text
            .contains(&off)
            .then_some((rec, off, val, view.spare))
    }

    /// Operands pre-evaluation may search in a span write: none.  The hook writes the
    /// guard and the bounds where they stand, and the owned write after them.
    pub(super) fn dv_operands<'v>(&self, x: &'v Value) -> Option<Vec<&'v Value>> {
        if !self.dv_on() {
            return None;
        }
        if self.dv_text_set(x).is_some() {
            return Some(Vec::new());
        }
        let Value::Call(d, args) = x.unspan() else {
            return None;
        };
        match (self.data.def(*d).name(), args.as_slice()) {
            ("OpMoveVector", [dst, ..]) if self.dv_bytes_field(dst).is_some() => Some(Vec::new()),
            ("OpSliceVector", [dst, ..]) if matches!(dst.unspan(), Value::Var(v) if self.dv_span_local(*v)) => {
                Some(Vec::new())
            }
            _ => None,
        }
    }

    /// `OpGetField(rec, off, vtp)` of a span byte-string field: `(rec, off, spare)`.
    fn dv_bytes_field<'v>(&self, dst: &'v Value) -> Option<(&'v Value, u32, u32)> {
        let Value::Call(g, gargs) = dst.unspan() else {
            return None;
        };
        if self.data.def(*g).name() != "OpGetField" {
            return None;
        }
        let [rec, off, tp] = gargs.as_slice() else {
            return None;
        };
        let view = self.dv_view(rec)?;
        let off = u32::try_from(int_of(off)?).ok()?;
        let tp = u16::try_from(int_of(tp)?).ok()?;
        view.bytes
            .contains(&(off, tp))
            .then_some((rec, off, view.spare))
    }

    /// The node itself, through the ordinary path.
    fn dv_plain(&mut self, w: &mut dyn Write, x: &Value) -> std::io::Result<()> {
        let saved = self.dv_bypass;
        self.dv_bypass = std::ptr::from_ref(x) as usize;
        let r = self.output_code_inner(w, x);
        self.dv_bypass = saved;
        r
    }

    /// The span field writes and the text reads of a twin or a root.  Answers whether it
    /// wrote `x`.
    pub(super) fn dv_expr(&mut self, w: &mut dyn Write, x: &Value) -> std::io::Result<bool> {
        if !self.dv_on() || self.dv_bypass == std::ptr::from_ref(x) as usize {
            return Ok(false);
        }
        // A span local's holder: placed only on its slice guard's fallback path.
        if self.dv_active
            && let Some(guard) = self.dv.lazy.get(&(std::ptr::from_ref(x) as usize)).cloned()
        {
            write!(w, "if !(((({{ ")?;
            self.output_code_inner(w, &guard)?;
            write!(w, " }}) as u8) == 1)) {{ ")?;
            self.dv_plain(w, x)?;
            write!(w, "; }}")?;
            return Ok(true);
        }
        let Value::Call(d, args) = x.unspan() else {
            return Ok(false);
        };
        let name = self.data.def(*d).name();
        match (name, args.as_slice()) {
            ("OpGetText", [rec, off]) => {
                let Some(view) = self.dv_view(rec) else {
                    return Ok(false);
                };
                let Some(off) = int_of(off).and_then(|o| u32::try_from(o).ok()) else {
                    return Ok(false);
                };
                if !view.text.contains(&off) {
                    return Ok(false);
                }
                write!(w, "codegen_runtime::dv_text(stores, ")?;
                self.output_code_inner(w, rec)?;
                write!(w, ", {off}, {})", view.spare)?;
                Ok(true)
            }
            ("OpSetText", _) => {
                let Some((rec, off, val, spare)) = self.dv_text_set(x) else {
                    return Ok(false);
                };
                if let Some(run) = decode_view::text_run(self.data, val)
                    && self.dv_frame_param(run.frame)
                {
                    crate::rewrite_census::fired("R-DecodeView", 1);
                    write!(w, "if !(((({{ ")?;
                    self.output_code_inner(w, run.guard)?;
                    write!(
                        w,
                        " }}) as u8) == 1) && codegen_runtime::dv_span_text(stores, "
                    )?;
                    self.output_code_inner(w, rec)?;
                    write!(w, ", {off}, {spare}, ")?;
                    self.output_code_inner(w, &Value::Var(run.frame))?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, run.lo)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, run.hi)?;
                    write!(w, ")) {{ ")?;
                } else {
                    write!(w, "{{ ")?;
                }
                self.dv_plain(w, x)?;
                write!(w, "; codegen_runtime::dv_clear(stores, ")?;
                self.output_code_inner(w, rec)?;
                write!(w, ", {spare}); }}")?;
                Ok(true)
            }
            ("OpMoveVector", [dst, src, _]) => {
                let Some((rec, off, spare)) = self.dv_bytes_field(dst) else {
                    return Ok(false);
                };
                if let Value::Var(bs) = src.unspan()
                    && self.dv_span_local(*bs)
                {
                    crate::rewrite_census::fired("R-DecodeView", 1);
                    write!(
                        w,
                        "if __dvs_{bs} != 0 {{ codegen_runtime::dv_put_span(stores, "
                    )?;
                    self.output_code_inner(w, rec)?;
                    write!(
                        w,
                        ", {off}, {spare}, __dvs_{bs}); __dvs_{bs} = 0; }} else {{ "
                    )?;
                } else {
                    write!(w, "{{ ")?;
                }
                self.dv_plain(w, x)?;
                write!(w, "; codegen_runtime::dv_clear(stores, ")?;
                self.output_code_inner(w, rec)?;
                write!(w, ", {spare}); }}")?;
                Ok(true)
            }
            ("OpSliceVector", [dst, b, lo, hi, _]) => {
                let Value::Var(bs) = dst.unspan() else {
                    return Ok(false);
                };
                if !self.dv_span_local(*bs)
                    || !matches!(b.unspan(), Value::Var(v) if self.dv_frame_param(*v))
                {
                    return Ok(false);
                }
                write!(w, "{{ __dvs_{bs} = codegen_runtime::dv_span(stores, ")?;
                self.output_code_inner(w, b)?;
                write!(w, ", ")?;
                self.output_code_inner(w, lo)?;
                write!(w, ", ")?;
                self.output_code_inner(w, hi)?;
                write!(w, "); if __dvs_{bs} == 0 {{ ")?;
                self.dv_plain(w, x)?;
                write!(w, "; }} }}")?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}
