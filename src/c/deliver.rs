// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C
//! `deliver(tag, value)` in C — the loopback host of `Stores::deliver_loopback`: the value's
//! bytes as `read_via_descriptor` reads them through its layout, printed as hex.  The emitter
//! writes one `loft_dlv{tp}` per type the value reaches; a store-internal kind (a keyed
//! collection, a stored reference, a child record) answers its refusal when it is reached, as
//! the descriptor walk does.
use super::{Cx, body::c_string};
use crate::database::Parts;
use std::fmt::Write as _;

impl Cx<'_> {
    /// The C walker of type `tp` and of every type it reaches, registered for the unit.
    pub(crate) fn deliver_type(&self, tp: u16) -> String {
        if self.deliver_types.borrow_mut().insert(tp)
            && let Some(t) = self.stores.types.get(tp as usize)
        {
            match &t.parts {
                Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                    for f in fields.iter().filter(|f| Self::is_data(f)) {
                        self.deliver_type(f.content);
                    }
                }
                Parts::Vector(e) | Parts::Array(e) => {
                    self.deliver_type(*e);
                }
                _ => {}
            }
        }
        format!("loft_dlv{tp}")
    }

    /// `LayoutField::is_data`: the enum discriminant, an absent field and a `#` synthetic are
    /// not data.
    fn is_data(f: &crate::database::Field) -> bool {
        f.name != "enum" && f.position != u16::MAX && !f.name.starts_with('#')
    }

    /// Every registered walker.
    pub(crate) fn deliver_code(&self) -> String {
        let types = self.deliver_types.borrow();
        let mut out = String::new();
        for &tp in types.iter() {
            let _ = writeln!(
                out,
                "static const char *loft_dlv{tp}(loft_text *o, loft_ref r);"
            );
        }
        for &tp in types.iter() {
            let _ = writeln!(
                out,
                "static const char *loft_dlv{tp}(loft_text *o, loft_ref r) {{"
            );
            out.push_str(&self.deliver_body(tp));
            out.push_str("}\n");
        }
        out
    }

    fn deliver_body(&self, tp: u16) -> String {
        let stores = self.stores;
        let Some(t) = stores.types.get(tp as usize) else {
            return format!(
                "    (void) o; (void) r;\n    return {};\n",
                c_string(&format!("type {tp} not in descriptor"))
            );
        };
        let raw = |n: u32| format!("    loft_dlv_raw(o, r, {n});\n    return 0;\n");
        match &t.parts {
            // `BaseKind::from_type_id`: a long and an integer are 8 bytes, the rest their own.
            Parts::Base => match tp {
                2 | 6 => raw(4),
                4 => raw(1),
                5 => "    loft_dlv_text(o, r);\n    return 0;\n".into(),
                _ => raw(8),
            },
            Parts::Byte(..) | Parts::Enum(_) => raw(1),
            Parts::Short(..) => "    loft_dlv_short(o, r, 0, 1);\n    return 0;\n".into(),
            Parts::ShortRaw(start, _) => {
                format!("    loft_dlv_short(o, r, {start}, 0);\n    return 0;\n")
            }
            Parts::Int(..) | Parts::IntRaw(..) => raw(4),
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                let mut s = String::from("    const char *e;\n");
                for f in fields.iter().filter(|f| Self::is_data(f)) {
                    let _ = writeln!(
                        s,
                        "    if ((e = loft_dlv{}(o, loft_field(r, {}))) != 0) return e;",
                        f.content, f.position
                    );
                }
                s.push_str("    (void) o; (void) r;\n    return 0;\n");
                s
            }
            Parts::Vector(e) => format!(
                "    uint32_t v = loft_u32(loft_store_of(r), r.rec, r.pos), n = v ? loft_u32(loft_store_of(r), v, 4) : 0;\n    for (uint32_t i = 0; i < n; i++) {{\n        loft_ref x = {{ r.store_nr, v, 8 + {} * i }};\n        const char *e = loft_dlv{e}(o, x);\n        if (e) return e;\n    }}\n    return 0;\n",
                stores.size(*e)
            ),
            Parts::Array(e) => format!(
                "    uint32_t v = loft_u32(loft_store_of(r), r.rec, r.pos), n = v ? loft_u32(loft_store_of(r), v, 4) : 0;\n    for (uint32_t i = 0; i < n; i++) {{\n        loft_ref x = {{ r.store_nr, loft_u32(loft_store_of(r), v, 8 + 4 * i), 8 }};\n        const char *e = loft_dlv{e}(o, x);\n        if (e) return e;\n    }}\n    return 0;\n"
            ),
            _ => format!(
                "    (void) o; (void) r;\n    return {};\n",
                c_string(&format!(
                    "type {tp} ({}) is a store-internal kind — not in the serializable subset \
                     (cursor-walked in a later phase)",
                    t.name
                ))
            ),
        }
    }
}
