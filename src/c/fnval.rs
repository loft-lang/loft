// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
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
    pub(crate) fn fn_sig(&self, params: Vec<CType>, ret: CType) -> usize {
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

    /// The shapes of the visible parameters and the result of `d`, when each has one a
    /// function value can pass: no `&` parameter.
    fn value_signature(&self, d: u32) -> Option<(Vec<CType>, CType)> {
        let def = self.data.def(d);
        let params = def
            .attributes()
            .iter()
            .filter(|a| !a.hidden && a.name != "__closure")
            .map(|a| match a.typedef {
                Type::RefVar(_) => None,
                ref tp => self.ctype(tp).filter(|c| *c != CType::Void),
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
                .chain(
                    params
                        .iter()
                        .enumerate()
                        .map(|(i, c)| format!("{} a{i}", passed(*c))),
                )
                .collect();
            let _ = writeln!(
                out,
                "\nstatic {} loft_call{id}({}) {{\n    switch (f.d) {{",
                held(*ret),
                list.join(", ")
            );
            for &d in &values {
                if self.value_signature(d).as_ref() != Some(&(params.clone(), *ret)) {
                    continue;
                }
                if !fns.contains(&d) {
                    return Err(format!(
                        "a function value {} the unit does not write",
                        self.data.def(d).original_name()
                    ));
                }
                self.dispatch_arm(&mut out, d, *ret)?;
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

    /// The arm calling `d`: its visible parameters in order, the value's closure, and a text
    /// result's work buffer made for the call and released after it.
    fn dispatch_arm(&self, out: &mut String, d: u32, ret: CType) -> Result<(), String> {
        let def = self.data.def(d);
        let mut args = Vec::new();
        let mut buffers = Vec::new();
        let mut visible = 0;
        for a in def.attributes() {
            if a.name == "__closure" {
                args.push("f.clos".to_string());
            } else if !a.hidden {
                args.push(format!("a{visible}"));
                visible += 1;
            } else if matches!(&a.typedef, Type::RefVar(t) if matches!(**t, Type::Text(_))) {
                let w = format!("w{}", buffers.len());
                args.push(format!("&{w}"));
                buffers.push(w);
            } else {
                return Err(format!(
                    "a function value {} with a hidden {} buffer",
                    def.original_name(),
                    a.name
                ));
            }
        }
        let call = format!("{}({})", c_name(self.data, d), args.join(", "));
        let _ = write!(out, "    case INT64_C({d}): {{");
        for w in &buffers {
            let _ = write!(out, " loft_text {w} = {{ 0, 0, 0 }};");
        }
        if ret == CType::Void {
            let _ = write!(out, " {call};");
        } else {
            let _ = write!(out, " {} r = {call};", held(ret));
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
