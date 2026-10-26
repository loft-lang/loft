// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C
//! Function values in C.  A value is a `loft_fn` — the definition it names and the closure
//! record a capturing lambda reads — and a call through one goes to the dispatcher of its
//! signature, `loft_call{n}`, which switches on the definition number as the interpreter's
//! `State::fn_call_ref` does.  A dispatcher's arms are every function the unit uses as a value
//! whose visible parameters and result have the signature's shapes.
use super::{CType, Cx, c_name, held, passed};
use crate::data::Type;
use std::fmt::Write as _;

impl Cx<'_> {
    /// The number of the dispatcher for these parameter and result shapes, made on first use.
    pub(crate) fn fn_sig(&self, params: Vec<(CType, u8)>, ret: CType) -> usize {
        let mut sigs = self.fn_sigs.borrow_mut();
        if let Some(i) = sigs.iter().position(|(p, r)| *p == params && *r == ret) {
            return i;
        }
        sigs.push((params, ret));
        sigs.len() - 1
    }

    /// `d` used as a value: a dispatcher arm for it, and the C value that names it.
    pub(crate) fn fn_value(&self, d: u32) -> String {
        self.fn_values.borrow_mut().insert(d);
        format!("loft_fn_of(INT64_C({d}))")
    }

    /// Is `d` a function a value can name (rather than an integer that happens to be one)?
    pub(crate) fn is_function(&self, d: i32) -> bool {
        u32::try_from(d).is_ok_and(|d| {
            d < self.data.definitions()
                && matches!(self.data.def(d).def_type(), crate::data::DefType::Function)
        })
    }

    /// The shapes of the visible parameters — each with whether it is a `&` link, passed as
    /// the address of the caller's variable — and the result of `d`, when each has one.
    fn value_signature(&self, d: u32) -> Option<(Vec<(CType, bool)>, CType)> {
        let def = self.data.def(d);
        let params = def
            .attributes()
            .iter()
            .filter(|a| !a.hidden && a.name != "__closure")
            .map(|a| match a.typedef.base() {
                Type::RefVar(inner) => self
                    .ctype(inner)
                    .filter(|c| *c != CType::Void)
                    .map(|c| (c, true)),
                tp => self
                    .ctype(tp)
                    .filter(|c| *c != CType::Void)
                    .map(|c| (c, false)),
            })
            .collect::<Option<Vec<_>>>()?;
        Some((params, self.ctype(def.returned())?))
    }

    /// Every dispatcher, after the prototypes of `fns`, the functions the unit writes.
    pub(crate) fn dispatch_code(&self, fns: &[u32]) -> Result<String, String> {
        let sigs = self.fn_sigs.borrow().clone();
        let values = self.fn_values.borrow().clone();
        let mut out = String::new();
        for (id, (params, ret)) in sigs.iter().enumerate() {
            let list: Vec<String> = std::iter::once("loft_fn f".to_string())
                .chain(params.iter().enumerate().map(|(i, (c, mode))| match mode {
                    2 => format!("loft_ref a{i}"),
                    1 => format!("{} *a{i}", held(*c)),
                    _ => format!("{} a{i}", passed(*c)),
                }))
                .collect();
            let _ = writeln!(
                out,
                "\nstatic {} loft_call{id}({}) {{\n    switch (f.d) {{",
                held(*ret),
                list.join(", ")
            );
            // A place is a link the arm hands to the place twin.
            let linked: Vec<(CType, bool)> = params.iter().map(|(c, m)| (*c, *m > 0)).collect();
            let places: Vec<bool> = params.iter().map(|(_, m)| *m == 2).collect();
            for &d in &values {
                if self.value_signature(d).as_ref() != Some(&(linked.clone(), *ret)) {
                    continue;
                }
                if places.contains(&true) && !fns.contains(&d) {
                    return Err(format!(
                        "a store place handed to a & parameter of {}",
                        self.data.def(d).original_name()
                    ));
                }
                // A native named as a value: its C function, called with the visible arguments.
                if !fns.contains(&d)
                    && let Some(c) = super::plain_native(self.data.def(d).name())
                {
                    let args: Vec<String> = (0..params.len()).map(|i| format!("a{i}")).collect();
                    let call = format!("{c}({})", args.join(", "));
                    if *ret == CType::Void {
                        let _ = writeln!(out, "    case INT64_C({d}): {call}; return;");
                    } else {
                        let _ = writeln!(out, "    case INT64_C({d}): return {call};");
                    }
                    continue;
                }
                if !fns.contains(&d) {
                    return Err(format!(
                        "a function value {} the unit does not write",
                        self.data.def(d).original_name()
                    ));
                }
                self.dispatch_arm(&mut out, d, *ret, &places)?;
            }
            out.push_str("    default:\n        break;\n    }\n");
            // An absent value, or one naming no function of this signature: the result's null.
            match ret {
                CType::Void => out.push_str("}\n"),
                CType::Text => out.push_str("    return loft_text_own(loft_null_text());\n}\n"),
                c => {
                    let _ = writeln!(
                        out,
                        "    {{ {} r = {}; return r; }}\n}}",
                        held(*c),
                        self.initial(*c)
                    );
                }
            }
        }
        Ok(out)
    }

    /// `loft_drop_closure` (`State::drop_fn_ref`): a function value's closure record releases
    /// what it captured through its type's drop cascade — the hooks of the captured values run
    /// there.  A value with no record, or one in a freed store, releases nothing.
    pub(crate) fn drop_code(&self, fns: &[u32]) -> String {
        let mut out = String::from(
            "\nstatic void loft_drop_closure(loft_fn f) {\n    if (!loft_store_live(f.clos)) return;\n    switch (f.d) {\n",
        );
        for &d in self.fn_values.borrow().iter() {
            let cascade = self.data.drop_cascade_nr(self.data.def(d).closure_record());
            if cascade != u32::MAX && fns.contains(&cascade) {
                let _ = writeln!(
                    out,
                    "    case INT64_C({d}): {}(f.clos); return;",
                    c_name(self.data, cascade)
                );
            }
        }
        out.push_str("    default:\n        return;\n    }\n}\n");
        out
    }

    /// The function an arm calls for `d` handed store places at `mask`, and the mask of its
    /// place twin: a text place goes to the IR's own store instance (`n_app@st1`,
    /// `parser/store_text.rs`), which reads and writes the field; any other place to the place
    /// twin of the function the arm calls.
    fn arm_callee(&self, d: u32, mask: u64) -> Result<(u32, u64), String> {
        let def = self.data.def(d);
        let text_bits = def
            .attributes()
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                mask & (1 << i) != 0
                    && matches!(a.typedef.base(),
                        Type::RefVar(t) if matches!(t.base(), Type::Text(_)))
            })
            .fold(0u64, |m, (i, _)| m | (1 << i));
        let mut callee = d;
        if text_bits != 0 {
            callee = self.data.store_text_instance(d, text_bits);
            if callee == u32::MAX {
                return Err(format!(
                    "a text place handed to {} through a function value",
                    def.original_name()
                ));
            }
        }
        let mask = mask & !text_bits;
        if mask != 0 {
            self.request_twin(callee, mask);
        }
        Ok((callee, mask))
    }

    /// The arm calling `d`: its visible parameters in order, the value's closure, a text
    /// result's work buffer made for the call and released after it, and a return buffer as
    /// `--native`'s dispatch hands one (`generation::emit`, PLAN51 V-c): the null sentinel for a
    /// record or a work buffer — the callee mints its own store — and, for a vector return, a
    /// vector store made for the call, kept when the result IS it and freed otherwise
    /// (`State::fn_return`'s rule, loft#1179).
    fn dispatch_arm(
        &self,
        out: &mut String,
        d: u32,
        ret: CType,
        places: &[bool],
    ) -> Result<(), String> {
        let def = self.data.def(d);
        let mut args = Vec::new();
        let mut mask = 0u64;
        let mut buffers = Vec::new();
        let mut visible = 0;
        let vec_buf = match def.returned().ret_promo_base() {
            Type::Vector(elm, _) => {
                let tp = self
                    .data
                    .name_type(&format!("main_vector<{}>", elm.name(self.data)), 0);
                (tp != u16::MAX).then_some(tp)
            }
            _ => None,
        };
        let mut uses_vec_buf = false;
        for (i, a) in def.attributes().iter().enumerate() {
            if a.name == "__closure" {
                args.push("f.clos".to_string());
            } else if !a.hidden {
                if places.get(visible) == Some(&true) {
                    mask |= 1 << i;
                }
                args.push(format!("a{visible}"));
                visible += 1;
            } else if matches!(a.typedef.base(), Type::RefVar(t) if matches!(t.base(), Type::Text(_)))
            {
                let w = format!("w{}", buffers.len());
                args.push(format!("&{w}"));
                buffers.push(w);
            } else if a.work_buffer {
                args.push("loft_ref_null()".to_string());
            } else if !matches!(a.typedef, Type::Optional(_))
                && matches!(
                    a.typedef.base(),
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                )
            {
                if vec_buf.is_some() && ret == CType::Ref {
                    uses_vec_buf = true;
                    args.push("hb".to_string());
                } else {
                    args.push("loft_ref_null()".to_string());
                }
            } else {
                return Err(format!(
                    "a function value {} with a hidden {} buffer",
                    def.original_name(),
                    a.name
                ));
            }
        }
        let (callee, mask) = self.arm_callee(d, mask)?;
        let call = format!(
            "{}({})",
            super::twin_name(self.data, callee, mask),
            args.join(", ")
        );
        let _ = write!(out, "    case INT64_C({d}): {{");
        for w in &buffers {
            let _ = write!(out, " loft_text {w} = {{ 0, 0, 0 }};");
        }
        let vec_tp = vec_buf.filter(|_| uses_vec_buf && ret == CType::Ref);
        if let Some(tp) = vec_tp {
            let _ = write!(
                out,
                " loft_ref hb = loft_ref_null(); loft_database_wrapper(&hb, {}, {tp});",
                self.stores.size(tp)
            );
        }
        if ret == CType::Void {
            let _ = write!(out, " {call};");
        } else {
            let _ = write!(out, " {} r = {call};", held(ret));
        }
        if vec_tp.is_some() {
            out.push_str(" if (r.store_nr != hb.store_nr) loft_free_ref(hb);");
        }
        for w in &buffers {
            let _ = write!(out, " loft_text_free(&{w});");
        }
        if ret == CType::Void {
            out.push_str(" return; }\n");
        } else {
            out.push_str(" return r; }\n");
        }
        Ok(())
    }
}
