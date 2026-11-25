// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `@FR-R-DecodeView` (`@C141`) in the C, the view clause: the emission of the plan
//! `lower::decode_view` decides ([`crate::c::Cx::dv_plan`]).
//!
//! - NAMES: a region function is written again as `<f>__dv` (the [`crate::c::DV_BIT`] twin)
//!   beside its own body, and every call a twin or a root makes to a region function takes the
//!   bit, so a twin's other variants (`__rg`, `__d`, …) are its `__dv` variants too.
//! - THE FRAME: a root names its frame `cx->dv_frame` for the length of its call, in its frame
//!   wrapper (`framed`); a span read anywhere in the region reads that frame.
//! - A SPAN WRITE: a text run of a frame parameter into a span text field writes the span when
//!   the run's guard holds and its bytes are UTF-8 (`loft_dv_span_text`), and the owned text
//!   otherwise; a byte slice of a frame parameter sets the local's span word (`loft_dvs<v>`),
//!   which the local's move into a span byte-string field writes.  Every other write of a span
//!   field clears the span word.  A span local's holder is placed only on its slice guard's
//!   fallback path.
//! - A SPAN READ: `OpGetText` of a span text field answers the frame's bytes for a span and
//!   the field's own text otherwise (`loft_dv_text`).
//!
//! Why a mention no clause here rewrites is sound: the plan admits a region only when every
//! read of a span field is an `OpGetText` and no byte-string field is read at all, so the
//! ordinary emission of everything else reads only fields that hold their own values.

use super::{E, FnWriter, K, as_cond, as_int, as_ref};
use crate::data::Value;
use crate::lower::decode_view::{self, DvPlan, DvType, int_of};

/// What a body of the region holds of the plan: whether it is a `__dv` twin, the plan, and
/// the span locals it declares.
pub(crate) struct DvBody {
    twin: bool,
    plan: std::rc::Rc<DvPlan>,
}

impl FnWriter<'_> {
    /// Take the plan for this body: a `__dv` twin (`twin`), or a root's own body.  Any other
    /// body takes nothing.
    pub(crate) fn decode_view(mut self, twin: bool) -> Self {
        let plan = self.cx.dv_plan();
        if !(twin && plan.region.contains(&self.d) || !twin && plan.roots.contains_key(&self.d)) {
            return self;
        }
        if twin && let Some(locals) = plan.span_locals.get(&self.d) {
            let mut locals: Vec<u16> = locals.iter().copied().collect();
            locals.sort_unstable();
            for v in locals {
                self.decls
                    .push(format!("loft_dvs loft_dvs{v} = {{ 0, 0 }};"));
            }
        }
        self.dv = Some(DvBody { twin, plan });
        self
    }

    /// A call of `d` from this body names its `__dv` twin: `d` is in the region, and this body
    /// is a twin, or a root calling another function than itself.
    pub(super) fn dv_call(&self, d: u32) -> bool {
        self.dv
            .as_ref()
            .is_some_and(|dv| dv.plan.region.contains(&d) && (dv.twin || d != self.d))
    }

    /// The view type record expression `rec` is, in this body.
    fn dv_view(&self, rec: &Value) -> Option<DvType> {
        let dv = self.dv.as_ref()?;
        decode_view::view_of(&dv.plan, self.cx.data, self.cx.stores, self.d, rec).cloned()
    }

    fn dv_frame_param(&self, v: u16) -> bool {
        self.dv
            .as_ref()
            .and_then(|dv| dv.plan.bparams.get(&self.d))
            .is_some_and(|s| s.contains(&v))
    }

    fn dv_span_local(&self, v: u16) -> bool {
        self.dv.as_ref().is_some_and(|dv| {
            dv.twin
                && dv
                    .plan
                    .span_locals
                    .get(&self.d)
                    .is_some_and(|s| s.contains(&v))
        })
    }

    /// `OpGetField(rec, off, vtp)` of a span byte-string field: `(rec, off, spare)`.
    fn dv_bytes_field<'v>(&self, dst: &'v Value) -> Option<(&'v Value, u32, u32)> {
        let Value::Call(g, gargs) = dst.unspan() else {
            return None;
        };
        if self.cx.data.def(*g).name() != "OpGetField" {
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

    /// A span local's holder statement: placed only on its slice guard's fallback path.
    /// Answers whether it wrote `v`.
    pub(super) fn dv_stmt(&mut self, v: &Value, ind: usize) -> Result<bool, String> {
        let key = std::ptr::from_ref(v) as usize;
        let Some(dv) = self.dv.as_ref().filter(|dv| dv.twin) else {
            return Ok(false);
        };
        if self.dv_bypass == key {
            return Ok(false);
        }
        let Some(guard) = dv.plan.lazy.get(&key).cloned() else {
            return Ok(false);
        };
        let g = self.expr(&guard, ind)?;
        self.line(ind, &format!("if (!{}) {{", as_cond(g)?));
        let saved = std::mem::replace(&mut self.dv_bypass, key);
        let r = self.stmt(v, ind + 1);
        self.dv_bypass = saved;
        r?;
        self.line(ind, "}");
        Ok(true)
    }

    /// The span field writes and the text reads of a twin or a root: `None` for every other
    /// operator.
    pub(super) fn dv_operator(
        &mut self,
        d: u32,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        if self.dv.is_none() {
            return Ok(None);
        }
        match (self.cx.data.def(d).name(), args) {
            ("OpGetText", [rec, off]) => self.dv_text_read(rec, off, ind),
            ("OpSetText", [rec, off, val]) => self.dv_text_write(d, args, (rec, off, val), ind),
            ("OpMoveVector", [dst, src, _]) => self.dv_bytes_move(d, args, (dst, src), ind),
            ("OpSliceVector", [dst, frame, lo, hi, _]) => {
                self.dv_bytes_slice(d, args, (dst, frame, lo, hi), ind)
            }
            _ => Ok(None),
        }
    }

    /// The span word's offset when `OpGetText(rec, off)` reads a span text field in this body:
    /// the one question every text field read of the region asks.
    pub(super) fn dv_text_spare(&self, rec: &Value, off: &Value) -> Option<u32> {
        let view = self.dv_view(rec)?;
        let off = int_of(off).and_then(|o| u32::try_from(o).ok())?;
        view.text.contains(&off).then_some(view.spare)
    }

    /// `OpGetText` of a span text field: the frame's bytes for a span, the field's own text
    /// otherwise — a view where the plain read gives one, else an owned copy, as it does.
    fn dv_text_read(&mut self, rec: &Value, off: &Value, ind: usize) -> Result<Option<E>, String> {
        let Some(spare) = self.dv_text_spare(rec, off) else {
            return Ok(None);
        };
        let Some(off) = int_of(off).and_then(|o| u32::try_from(o).ok()) else {
            return Ok(None);
        };
        let field_view = std::mem::take(&mut self.field_view);
        let r = self.expr(rec, ind)?;
        let code = as_ref(r)?;
        if field_view {
            let at = if code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                code
            } else {
                let t = self.temp(crate::c::CType::Ref);
                self.line(ind, &format!("{t} = {code};"));
                t
            };
            return Ok(Some(E::new(
                format!("loft_dv_text(cx, {at}, {off}, {spare})"),
                K::View,
            )));
        }
        let t = self.temp(crate::c::CType::Text);
        self.line(ind, &format!("loft_text_free(cx, &{t});"));
        self.line(
            ind,
            &format!("{t} = loft_text_own(cx, loft_dv_text(cx, {code}, {off}, {spare}));"),
        );
        Ok(Some(self.held(t, crate::c::CType::Text)))
    }

    /// The record of a span field write, bound once: its expression may compute.
    fn dv_record(&mut self, rec: &Value, ind: usize) -> Result<String, String> {
        let r = self.expr(rec, ind)?;
        let t = self.temp(crate::c::CType::Ref);
        self.line(ind, &format!("{t} = {};", as_ref(r)?));
        Ok(t)
    }

    /// The operator as the ordinary emission writes it, as a statement at `ind`.
    fn dv_plain(&mut self, d: u32, args: &[Value], ind: usize) -> Result<(), String> {
        let e = self.call(d, args, ind)?;
        if e.code != "(void) 0" {
            self.line(ind, &format!("{};", e.code));
        }
        Ok(())
    }

    /// A write of a span text field: a text run of a frame parameter writes the span when its
    /// guard holds and its bytes are UTF-8; every other write is the owned text, and clears the
    /// span word.
    fn dv_text_write(
        &mut self,
        d: u32,
        args: &[Value],
        (rec, off, val): (&Value, &Value, &Value),
        ind: usize,
    ) -> Result<Option<E>, String> {
        let Some(view) = self.dv_view(rec) else {
            return Ok(None);
        };
        let Some(off) = int_of(off).and_then(|o| u32::try_from(o).ok()) else {
            return Ok(None);
        };
        if !view.text.contains(&off) {
            return Ok(None);
        }
        let spare = view.spare;
        let run = decode_view::text_run(self.cx.data, val).filter(|r| self.dv_frame_param(r.frame));
        let r = self.dv_record(rec, ind)?;
        if let Some(run) = run {
            crate::rewrite_census::fired("R-DecodeView", 1);
            let g = self.expr(run.guard, ind)?;
            let g = as_cond(g)?;
            let frame = self.var(run.frame);
            let lo = self.expr(run.lo, ind)?;
            let lo = as_int(lo)?;
            let hi = self.expr(run.hi, ind)?;
            let hi = as_int(hi)?;
            self.line(
                ind,
                &format!(
                    "if (!({g} && loft_dv_span_text(cx, {r}, {off}, {spare}, {frame}, {lo}, {hi}))) {{ /* @FR-R-DecodeView */"
                ),
            );
        } else {
            self.line(ind, "{");
        }
        self.dv_plain(d, args, ind + 1)?;
        self.line(ind + 1, &format!("loft_dv_clear(cx, {r}, {spare});"));
        self.line(ind, "}");
        Ok(Some(E::new("(void) 0", K::Void)))
    }

    /// A byte local's move into a span byte-string field: the span its slice set, else the
    /// vector, which clears the span word.
    fn dv_bytes_move(
        &mut self,
        d: u32,
        args: &[Value],
        (dst, src): (&Value, &Value),
        ind: usize,
    ) -> Result<Option<E>, String> {
        let Some((rec, off, spare)) = self.dv_bytes_field(dst) else {
            return Ok(None);
        };
        let r = self.dv_record(rec, ind)?;
        if let Value::Var(bs) = src.unspan()
            && self.dv_span_local(*bs)
        {
            crate::rewrite_census::fired("R-DecodeView", 1);
            self.line(
                ind,
                &format!(
                    "if (loft_dvs{bs}.at != 0) {{ /* @FR-R-DecodeView */ loft_dv_put(cx, {r}, {off}, {spare}, loft_dvs{bs}); loft_dvs{bs}.at = 0; }} else {{"
                ),
            );
        } else {
            self.line(ind, "{");
        }
        self.dv_plain(d, args, ind + 1)?;
        self.line(ind + 1, &format!("loft_dv_clear(cx, {r}, {spare});"));
        self.line(ind, "}");
        Ok(Some(E::new("(void) 0", K::Void)))
    }

    /// A slice of a frame parameter into a span byte local: its span word, or the slice when
    /// the bounds fit none.
    fn dv_bytes_slice(
        &mut self,
        d: u32,
        args: &[Value],
        (dst, frame, lo, hi): (&Value, &Value, &Value, &Value),
        ind: usize,
    ) -> Result<Option<E>, String> {
        let Value::Var(bs) = dst.unspan() else {
            return Ok(None);
        };
        if !self.dv_span_local(*bs)
            || !matches!(frame.unspan(), Value::Var(v) if self.dv_frame_param(*v))
        {
            return Ok(None);
        }
        let f = self.expr(frame, ind)?;
        let f = as_ref(f)?;
        let lo = self.expr(lo, ind)?;
        let lo = as_int(lo)?;
        let hi = self.expr(hi, ind)?;
        let hi = as_int(hi)?;
        self.line(
            ind,
            &format!("loft_dvs{bs} = loft_dv_span(cx, {f}, {lo}, {hi});"),
        );
        self.line(ind, &format!("if (loft_dvs{bs}.at == 0) {{"));
        self.dv_plain(d, args, ind + 1)?;
        self.line(ind, "}");
        Ok(Some(E::new("(void) 0", K::Void)))
    }
}
