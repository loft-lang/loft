// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Rendering a value of a store as text in C — the `{v}` of a record or a vector, which the
//! interpreter writes with `ShowDb` (`src/database/format.rs`).  The emitter writes one
//! `loft_show{tp}` per type a program renders, with the type's field and variant names in
//! it; the runtime's `loft_show_*` helpers are the parts every type shares.
use super::{Cx, body::c_string};
use crate::database::{Field, Parts};
use std::fmt::Write as _;

impl Cx<'_> {
    /// Register type `tp` and every type its rendering reaches, or refuse a kind the C
    /// renderer does not write.
    pub(crate) fn show_type(&self, tp: u16) -> Result<(), String> {
        if tp <= 6 || self.show_types.borrow().contains(&tp) {
            return Ok(());
        }
        let t = self
            .stores
            .types
            .get(tp as usize)
            .ok_or("a format of an unknown type")?;
        self.show_types.borrow_mut().insert(tp);
        let r = match &t.parts {
            Parts::Enum(_) if t.name == "JsonValue" => Err("a format of a JsonValue".into()),
            Parts::Enum(values) => values
                .iter()
                .filter(|(v, _)| *v != u16::MAX)
                .try_for_each(|(v, _)| self.show_type(*v)),
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                fields.iter().try_for_each(|f| self.show_type(f.content))
            }
            Parts::Vector(elem) => self.show_type(*elem),
            Parts::Byte(..)
            | Parts::Short(..)
            | Parts::ShortRaw(..)
            | Parts::Int(..)
            | Parts::IntRaw(..)
            | Parts::ChildRec(_) => Ok(()),
            other => {
                let kind = format!("{other:?}");
                Err(format!(
                    "a format of a {}",
                    kind.split(['(', ' '])
                        .next()
                        .unwrap_or("value")
                        .to_lowercase()
                ))
            }
        };
        if r.is_err() {
            self.show_types.borrow_mut().remove(&tp);
        }
        r
    }

    /// The C function that renders a value of type `tp`.
    pub(crate) fn show_fn(tp: u16) -> String {
        match tp {
            0 | 1 => "loft_show_long".into(),
            2 => "loft_show_single".into(),
            3 => "loft_show_float".into(),
            4 => "loft_show_bool".into(),
            5 => "loft_show_text".into(),
            6 => "loft_show_char".into(),
            _ => format!("loft_show{tp}"),
        }
    }

    /// Every registered type's render function, and the field writer of every struct and
    /// variant among them.
    pub(crate) fn show_code(&self) -> String {
        let types = self.show_types.borrow();
        let mut out = String::new();
        for &tp in types.iter() {
            let _ = writeln!(
                out,
                "static void loft_show{tp}(loft_text *s, loft_ref r, int fmt, int ind);"
            );
        }
        for &tp in types.iter() {
            let t = &self.stores.types[tp as usize];
            if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &t.parts {
                Self::fields_fn(&mut out, tp, fields);
            }
        }
        for &tp in types.iter() {
            let _ = writeln!(
                out,
                "static void loft_show{tp}(loft_text *s, loft_ref r, int fmt, int ind) {{"
            );
            out.push_str("    if (r.rec == 0) { loft_push(s, \"null\", 4); return; }\n");
            self.show_body(&mut out, tp);
            out.push_str("}\n");
        }
        out
    }

    /// `write_struct` over `fields`, for a type whose layout they are: `loft_showf{tp}`,
    /// framed by the `complex` flag of the type the caller renders (a variant's fields are
    /// framed by its enum's).
    fn fields_fn(out: &mut String, tp: u16, fields: &[Field]) {
        let shown: Vec<&Field> = fields
            .iter()
            .filter(|f| {
                f.name != "enum"
                    && !f.name.starts_with('#')
                    && f.other_indexes().first() != Some(&u16::MAX)
            })
            .collect();
        let _ = writeln!(
            out,
            "static void loft_showf{tp}(loft_text *s, loft_ref r, int fmt, int ind, int complex_type) {{"
        );
        let _ = writeln!(
            out,
            "    int c = loft_show_open(s, fmt, ind, complex_type, {});",
            i32::from(!shown.is_empty())
        );
        for (i, f) in shown.iter().enumerate() {
            if i > 0 {
                out.push_str("    loft_show_sep(s, fmt, ind, c);\n");
            }
            let _ = writeln!(out, "    loft_show_name(s, fmt, {});", c_string(&f.name));
            let _ = writeln!(
                out,
                "    {}(s, loft_field(r, {}), fmt, ind + 1);",
                Self::show_fn(f.content),
                f.position
            );
        }
        let _ = writeln!(
            out,
            "    loft_show_close(s, fmt, ind, c, {});\n}}",
            i32::from(!shown.is_empty())
        );
    }

    fn show_body(&self, out: &mut String, tp: u16) {
        let t = &self.stores.types[tp as usize];
        let complex = i32::from(t.is_complex());
        match &t.parts {
            Parts::Struct(_) => {
                let _ = writeln!(out, "    loft_showf{tp}(s, r, fmt, ind, {complex});");
            }
            Parts::EnumValue(..) => {
                let open = c_string(&format!("{{\"{}\":", t.name));
                let _ = writeln!(
                    out,
                    "    if (fmt & 2) {{ loft_push_cstr(s, {open}); loft_showf{tp}(s, r, fmt, ind, {complex}); loft_push_char(s, '}}'); }}\n    else loft_showf{tp}(s, r, fmt, ind, {complex});"
                );
            }
            Parts::Enum(values) => self.enum_body(out, &t.name, values, complex),
            Parts::Vector(elem) => {
                let e = &self.stores.types[*elem as usize];
                let _ = writeln!(
                    out,
                    "    if (loft_vec_absent(r)) {{ loft_push(s, \"null\", 4); return; }}\n    loft_show_list(s, r, fmt, ind, {}, {}, {}, {});",
                    self.stores.size(*elem),
                    i32::from(e.is_complex()),
                    i32::from(matches!(e.parts, Parts::Struct(_) | Parts::EnumValue(..))),
                    Self::show_fn(*elem)
                );
            }
            Parts::Byte(min, nullable) => {
                let _ = writeln!(
                    out,
                    "    (void) fmt; (void) ind;\n    loft_show_byte(s, r, {min}, {});",
                    i32::from(*nullable)
                );
            }
            Parts::Short(min, nullable) => {
                let _ = writeln!(
                    out,
                    "    (void) fmt; (void) ind;\n    loft_show_short(s, r, {min}, {});",
                    i32::from(*nullable)
                );
            }
            Parts::ShortRaw(min, nullable) => {
                let _ = writeln!(
                    out,
                    "    (void) fmt; (void) ind;\n    loft_show_short_raw(s, r, {min}, {});",
                    i32::from(*nullable)
                );
            }
            Parts::Int(_, nullable) => {
                let _ = writeln!(
                    out,
                    "    (void) fmt; (void) ind;\n    loft_show_int4(s, r, {});",
                    i32::from(*nullable)
                );
            }
            Parts::IntRaw(_, nullable) => {
                let _ = writeln!(
                    out,
                    "    (void) fmt; (void) ind;\n    loft_show_int_raw(s, r, {});",
                    i32::from(*nullable)
                );
            }
            _ => out.push_str("    (void) fmt; (void) ind;\n    loft_show_child(s, r);\n"),
        }
    }

    /// An enum: its variant's name (and fields), or for a `__nullable<S>` the dense value
    /// its `Some` variant carries.
    fn enum_body(&self, out: &mut String, name: &str, values: &[(u16, String)], complex: i32) {
        if name.starts_with("__nullable<") {
            // A present element renders as the dense value its `Some` variant carries.
            out.push_str("    switch (*loft_at(loft_store_of(r), r.rec, r.pos)) {\n");
            for (i, (vtp, _)) in values.iter().enumerate() {
                if let Some(Parts::EnumValue(_, st)) =
                    self.stores.types.get(*vtp as usize).map(|t| &t.parts)
                    && let Some(pf) = st.iter().find(|f| f.name == "payload")
                {
                    let _ = writeln!(
                        out,
                        "    case {}: {}(s, loft_field(r, {}), fmt, ind); return;",
                        i + 1,
                        Self::show_fn(pf.content),
                        pf.position
                    );
                }
            }
            out.push_str(
                "    case 0: loft_push(s, \"null\", 4); return;\n    default: return;\n    }\n",
            );
            return;
        }
        out.push_str("    uint8_t v = *loft_at(loft_store_of(r), r.rec, r.pos);\n");
        out.push_str("    switch (v) {\n");
        for (i, (vtp, name)) in values.iter().enumerate() {
            let payload = matches!(
                self.stores.types.get(*vtp as usize).map(|t| &t.parts),
                Some(Parts::EnumValue(..))
            );
            let json_open = c_string(&format!("{{\"{name}\":"));
            let body = if payload {
                format!("loft_showf{vtp}(s, r, fmt, ind, {complex});")
            } else {
                "loft_push(s, \"{}\", 2);".into()
            };
            let plain = if payload {
                format!(" loft_push_char(s, ' '); loft_showf{vtp}(s, r, fmt, ind, {complex});")
            } else {
                String::new()
            };
            let _ = writeln!(
                out,
                "    case {}:\n        if (fmt & 2) {{ loft_push_cstr(s, {json_open}); {body} loft_push_char(s, '}}'); }}\n        else {{ loft_push_cstr(s, {});{plain} }}\n        return;",
                i + 1,
                c_string(name)
            );
        }
        out.push_str("    default:\n        loft_push_cstr(s, (fmt & 2) || v == 0 || v == 255 ? \"null\" : \"?\");\n    }\n");
    }
}
