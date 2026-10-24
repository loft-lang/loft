// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! Content equality in C — `OpEqContent`, which the interpreter answers with
//! `Stores::eq_content` (`src/database/equality.rs`, `@FR-E-Eq`).  The emitter writes one
//! `loft_eq{tp}` per type a program compares; two absent values are equal, an absent one
//! differs from every present one, a collection compares element by element, and a stored
//! reference compares the records it names, a pair already under comparison counting as equal
//! (where a cycle ends).
use super::Cx;
use crate::database::{Field, Parts};
use std::fmt::Write as _;

impl Cx<'_> {
    /// Register type `tp` and every type its comparison reaches, or refuse a kind the C does
    /// not compare.
    pub(crate) fn eq_type(&self, tp: u16) -> Result<(), String> {
        if tp <= 6 || self.eq_types.borrow().contains(&tp) {
            return Ok(());
        }
        let t = self
            .stores
            .types
            .get(tp as usize)
            .ok_or("a comparison of an unknown type")?;
        self.eq_types.borrow_mut().insert(tp);
        let r = match &t.parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => fields
                .iter()
                .filter(|f| Self::compared(f))
                .try_for_each(|f| {
                    if f.target == u16::MAX {
                        self.eq_type(f.content)
                    } else {
                        self.eq_type(f.target)
                    }
                }),
            Parts::Enum(values) => values
                .iter()
                .filter(|(v, _)| *v != u16::MAX)
                .try_for_each(|(v, _)| self.eq_type(*v)),
            Parts::Vector(elem) | Parts::ChildRec(elem) => self.eq_type(*elem),
            Parts::Sorted(elem, _)
            | Parts::Hash(elem, _)
            | Parts::Index(elem, _)
            | Parts::Ordered(elem, _)
            | Parts::Array(elem) => {
                self.keys_of(tp);
                self.eq_type(*elem)
            }
            Parts::Byte(..)
            | Parts::Short(..)
            | Parts::ShortRaw(..)
            | Parts::Int(..)
            | Parts::IntRaw(..)
            | Parts::DbRef => Ok(()),
            other => {
                let kind = format!("{other:?}");
                Err(format!(
                    "a comparison of a {}",
                    kind.split(['(', ' '])
                        .next()
                        .unwrap_or("value")
                        .to_lowercase()
                ))
            }
        };
        if r.is_err() {
            self.eq_types.borrow_mut().remove(&tp);
        }
        r
    }

    /// The C function comparing two values of type `tp`.
    pub(crate) fn eq_fn(tp: u16) -> String {
        match tp {
            0 | 1 => "loft_eq_long".into(),
            2 => "loft_eq_single".into(),
            3 => "loft_eq_float".into(),
            4 => "loft_eq_byte".into(),
            5 => "loft_eq_text".into(),
            6 => "loft_eq_u32".into(),
            _ => format!("loft_eq{tp}"),
        }
    }

    /// A field that is content: not a collection's bookkeeping (`#…`) nor a secondary view.
    fn compared(f: &Field) -> bool {
        !f.name.starts_with('#') && f.other_indexes().first() != Some(&u16::MAX)
    }

    /// Every registered type's comparison.
    pub(crate) fn eq_code(&self) -> String {
        let types = self.eq_types.borrow();
        let mut out = String::new();
        for &tp in types.iter() {
            let _ = writeln!(out, "static int loft_eq{tp}(loft_ref a, loft_ref b);");
        }
        for &tp in types.iter() {
            let t = &self.stores.types[tp as usize];
            let _ = writeln!(out, "static int loft_eq{tp}(loft_ref a, loft_ref b) {{");
            out.push_str("    if (loft_same_place(a, b)) return 1;\n");
            out.push_str("    if (a.rec == 0 || b.rec == 0) return a.rec == 0 && b.rec == 0;\n");
            match &t.parts {
                Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                    Self::eq_fields(&mut out, fields, "    ");
                    out.push_str("    return 1;\n");
                }
                Parts::Enum(values) => {
                    out.push_str("    uint8_t v = *loft_at(loft_store_of(a), a.rec, a.pos);\n");
                    out.push_str(
                        "    if (v != *loft_at(loft_store_of(b), b.rec, b.pos)) return 0;\n",
                    );
                    out.push_str("    switch (v) {\n");
                    for (i, (vtp, _)) in values.iter().enumerate() {
                        if let Some(Parts::EnumValue(_, fields)) =
                            self.stores.types.get(*vtp as usize).map(|t| &t.parts)
                        {
                            let _ = writeln!(out, "    case {}:", i + 1);
                            Self::eq_fields(&mut out, fields, "        ");
                            out.push_str("        return 1;\n");
                        }
                    }
                    out.push_str("    default:\n        return 1;\n    }\n");
                }
                Parts::Hash(elem, _)
                | Parts::Index(elem, _)
                | Parts::Ordered(elem, _)
                | Parts::Array(elem) => {
                    let (keys, n) = self.keys_of(tp);
                    let _ = writeln!(
                        out,
                        "    return loft_eq_keyed(a, b, {}, {keys}, {n}, {});",
                        i32::from(matches!(t.parts, Parts::Hash(..))),
                        Self::eq_fn(*elem)
                    );
                }
                Parts::Vector(elem) | Parts::Sorted(elem, _) => {
                    let _ = writeln!(
                        out,
                        "    return loft_eq_vector(a, b, {}, {});",
                        self.stores.size(*elem),
                        Self::eq_fn(*elem)
                    );
                }
                Parts::ChildRec(elem) => {
                    let _ = writeln!(
                        out,
                        "    return loft_eq_child(a, b, {});",
                        Self::eq_fn(*elem)
                    );
                }
                Parts::Byte(min, _) => {
                    let _ = writeln!(out, "    (void) {min};\n    return loft_eq_byte(a, b);");
                }
                Parts::Short(..) | Parts::ShortRaw(..) => {
                    out.push_str("    return loft_eq_u16(a, b);\n");
                }
                Parts::Int(..) | Parts::IntRaw(..) => {
                    out.push_str("    return loft_eq_u32(a, b);\n")
                }
                _ => out.push_str("    return loft_eq_dbref(a, b);\n"),
            }
            out.push_str("}\n");
        }
        out
    }

    /// `eq_fields`: each compared field by its own type, a variant's `enum` tag by its byte, a
    /// stored reference by the record it names.
    fn eq_fields(out: &mut String, fields: &[Field], ind: &str) {
        for f in fields.iter().filter(|f| Self::compared(f)) {
            let (pa, pb) = (
                format!("loft_field(a, {})", f.position),
                format!("loft_field(b, {})", f.position),
            );
            let test = if f.name == "enum" {
                format!("loft_eq_byte({pa}, {pb})")
            } else if f.target != u16::MAX {
                format!(
                    "loft_eq_reference({pa}, {pb}, {}, {})",
                    f.target,
                    Self::eq_fn(f.target)
                )
            } else {
                format!("{}({pa}, {pb})", Self::eq_fn(f.content))
            };
            let _ = writeln!(out, "{ind}if (!{test}) return 0;");
        }
    }
}
