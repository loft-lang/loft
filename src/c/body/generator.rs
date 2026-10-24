// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! Generators as C (@F34): a function returning `iterator<T>` becomes a frame struct holding
//! every local, a creator that fills the frame from the arguments, a resume function that
//! restores the locals, jumps to the label after the last `yield` and saves them again at the
//! next one, and a release.  The body is written as any other; a `yield` and a `return` are the
//! only statements that read differently.  The runtime half is the generator table in
//! `loft_rt.h`.

use super::{E, FnWriter, K, as_ref, as_shape, as_view};
use crate::c::{CType, held, passed};
use crate::data::{Type, Value};
use std::fmt::Write as _;

/// What a generator's C is, beside its creator's body: the frame struct and the prototypes,
/// written before the unit's functions, and the resume and release functions, after them.
pub(crate) struct GenParts {
    pub(crate) decls: String,
    pub(crate) bodies: String,
}

impl FnWriter<'_> {
    /// `yield x`: the value through `out`, the locals into the frame, and the label the next
    /// resume jumps to.
    pub(super) fn gen_yield(&mut self, x: &Value, ind: usize) -> Result<(), String> {
        let c = self.gen_elem.ok_or("a yield outside a generator")?;
        let e = self.expr(x, ind)?;
        let put = match c {
            CType::Text => format!("loft_text_set((loft_text *) out, {});", as_view(e)?),
            other => format!("*({} *) out = {};", held(other), as_shape(e, other)?),
        };
        self.line(ind, &put);
        self.flush(ind);
        self.yields += 1;
        let k = self.yields;
        self.line(ind, &format!("fr->state = {k};"));
        self.line(ind, &format!("LOFT_SAVE{};", self.d));
        self.line(ind, "return;");
        self.line(0, &format!("loft_y{k}:;"));
        Ok(())
    }

    /// The generator operators, or `None` for another.
    pub(super) fn gen_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        match (name, args) {
            // The next value: the iterator's element type, its null when the generator has
            // run to its end.
            ("OpCoroutineNext", [g, _]) => {
                let Value::Var(v) = g.unspan() else {
                    return Err("a generator step on a value that is not a variable".into());
                };
                let Type::Iterator(elem, _) = self.vars.tp(*v) else {
                    return Err("a generator step on a value that is not an iterator".into());
                };
                let c = self
                    .cx
                    .ctype(elem)
                    .filter(|c| !matches!(c, CType::Void | CType::Tuple(_)))
                    .ok_or("a generator of tuples")?;
                let g = self.var(*v);
                let t = self.temp(c);
                if c == CType::Text {
                    self.line(ind, &format!("loft_text_set(&{t}, loft_null_text());"));
                    self.line(ind, &format!("loft_gen_next({g}, &{t});"));
                    return Ok(Some(self.held(t, CType::Text)));
                }
                self.null_into(&t, c, ind);
                self.line(ind, &format!("loft_gen_next({g}, &{t});"));
                Ok(Some(E::new(t, super::kind_of(c))))
            }
            ("OpCoroutineExhausted", [g]) => {
                let g = self.expr(g, ind)?;
                Ok(Some(E::new(
                    format!("loft_gen_exhausted({})", as_ref(g)?),
                    K::Bool,
                )))
            }
            _ => Ok(None),
        }
    }

    /// The generator: its creator's body, the frame, the resume and the release.
    pub(crate) fn write_gen(mut self) -> Result<(String, GenParts), String> {
        let d = self.d;
        if self.links.iter().any(|l| *l) {
            return Err("a generator with a & parameter".into());
        }
        if self.shapes.iter().any(|c| matches!(c, CType::Tuple(_))) {
            return Err("a generator with a tuple local".into());
        }
        let code = self.code;
        self.stmt(code, 1)?;
        self.line(0, "loft_gen_end:");
        self.epilogue(1);
        self.line(1, "fr->done = 1;");
        // Every variable and temporary is a field: a parameter as it is passed, the rest as
        // they are held.
        let mut fields: Vec<(String, String, CType, bool)> = (0..self.shapes.len())
            .map(|v| {
                let c = self.shapes[v];
                let ty = if self.params[v] { passed(c) } else { held(c) };
                (ty, self.var(v as u16), c, self.params[v])
            })
            .collect();
        for (t, c) in &self.temps {
            fields.push((held(*c), t.clone(), *c, false));
        }
        let mut decls = String::from("typedef struct { int32_t state; int32_t done;");
        for (ty, n, c, param) in &fields {
            let _ = write!(decls, " {ty} {n};");
            if *param && *c == CType::Text {
                let _ = write!(decls, " loft_text own_{n};");
            }
        }
        let _ = writeln!(
            decls,
            " }} loft_gen{d};\nstatic void loft_resume{d}(void *frp, void *out);\nstatic void loft_genfree{d}(void *frp);"
        );
        let saves: Vec<String> = fields
            .iter()
            .map(|(_, n, _, _)| format!("fr->{n} = {n};"))
            .collect();
        let mut bodies = format!(
            "\n#define LOFT_SAVE{d} do {{ {} }} while (0)\nstatic void loft_resume{d}(void *frp, void *out) {{\n    loft_gen{d} *fr = (loft_gen{d} *) frp;\n",
            saves.join(" ")
        );
        for (ty, n, _, _) in &fields {
            let _ = writeln!(bodies, "    {ty} {n} = fr->{n};");
        }
        bodies.push_str("    (void) out;\n    switch (fr->state) {\n");
        for k in 1..=self.yields {
            let _ = writeln!(bodies, "    case {k}: goto loft_y{k};");
        }
        bodies.push_str("    default: break;\n    }\n");
        bodies.push_str(&self.body);
        let _ = writeln!(bodies, "}}\n#undef LOFT_SAVE{d}");
        // The release: a generator stopped before its end still holds its texts.
        let _ = writeln!(
            bodies,
            "static void loft_genfree{d}(void *frp) {{\n    loft_gen{d} *fr = (loft_gen{d} *) frp;\n    if (!fr->done) {{"
        );
        for (_, n, c, param) in &fields {
            if *c == CType::Text && !param {
                let _ = writeln!(bodies, "        loft_text_free(&fr->{n});");
            }
        }
        bodies.push_str("    }\n");
        for (_, n, c, param) in &fields {
            if *c == CType::Text && *param {
                let _ = writeln!(bodies, "    loft_text_free(&fr->own_{n});");
            }
        }
        bodies.push_str("    free(fr);\n}\n");
        // The creator: the frame, its arguments — a text copied, since the generator outlives
        // the caller's view — and every other field at its initial value.
        let mut creator = format!(
            "    loft_gen{d} *fr = (loft_gen{d} *) calloc(1, sizeof *fr);\n    if (!fr) {{ fputs(\"loft: out of memory\\n\", stderr); abort(); }}\n"
        );
        for (ty, n, c, param) in &fields {
            if *param {
                if *c == CType::Text {
                    let _ = writeln!(
                        creator,
                        "    fr->own_{n} = loft_text_own({n});\n    fr->{n} = loft_view(&fr->own_{n});"
                    );
                } else {
                    let _ = writeln!(creator, "    fr->{n} = {n};");
                }
            } else {
                let _ = writeln!(creator, "    fr->{n} = ({ty}) {};", self.cx.initial(*c));
            }
        }
        let _ = writeln!(
            creator,
            "    return loft_gen_new(fr, loft_resume{d}, loft_genfree{d});"
        );
        Ok((creator, GenParts { decls, bodies }))
    }
}
