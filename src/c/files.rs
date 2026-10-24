// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The binary form of a value in C — what `f += v` writes and `f#read(n) as T` reads into a
//! value, which the interpreter writes with `Stores::read_data` and reads with
//! `Stores::write_data` (`src/database/io.rs`).  The emitter writes one `loft_ser{tp}` /
//! `loft_de{tp}` pair per type a program writes or reads; the builtin types' pairs are in
//! `loft_io.h`.
use super::Cx;
use crate::database::{Field, Parts};
use std::fmt::Write as _;

impl Cx<'_> {
    /// Register type `tp` and every type its binary form reaches, or refuse a kind that has
    /// none (a keyed collection, a stored reference: store-internal, as the interpreter says).
    pub(crate) fn ser_type(&self, tp: u16) -> Result<(), String> {
        if tp <= 6 || self.ser_types.borrow().contains(&tp) {
            return Ok(());
        }
        let t = self
            .stores
            .types
            .get(tp as usize)
            .ok_or("a binary form of an unknown type")?;
        self.ser_types.borrow_mut().insert(tp);
        let r = match &t.parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => fields
                .iter()
                .filter(|f| Self::serialised(f))
                .try_for_each(|f| self.ser_type(f.content)),
            Parts::Vector(elem) => self.ser_type(*elem),
            Parts::Enum(_)
            | Parts::Byte(..)
            | Parts::Short(..)
            | Parts::ShortRaw(..)
            | Parts::Int(..)
            | Parts::IntRaw(..) => Ok(()),
            other => {
                let kind = format!("{other:?}");
                Err(format!(
                    "a binary form of a {}",
                    kind.split(['(', ' '])
                        .next()
                        .unwrap_or("value")
                        .to_lowercase()
                ))
            }
        };
        if r.is_err() {
            self.ser_types.borrow_mut().remove(&tp);
        }
        r
    }

    /// A field that is part of a record's binary form: not a variant's tag, and placed.
    fn serialised(f: &Field) -> bool {
        f.name != "enum" && f.position != u16::MAX
    }

    /// Every registered type's writer and reader.
    pub(crate) fn ser_code(&self) -> String {
        let types = self.ser_types.borrow();
        let mut out = String::new();
        for &tp in types.iter() {
            let _ = writeln!(
                out,
                "static void loft_ser{tp}(loft_text *b, loft_ref r, int le);\nstatic void loft_de{tp}(loft_ref r, int le, const uint8_t *d, size_t n);"
            );
        }
        for &tp in types.iter() {
            let _ = writeln!(
                out,
                "static void loft_ser{tp}(loft_text *b, loft_ref r, int le) {{"
            );
            self.ser_body(&mut out, tp);
            out.push_str("}\n");
            let _ = writeln!(
                out,
                "static void loft_de{tp}(loft_ref r, int le, const uint8_t *d, size_t n) {{"
            );
            self.de_body(&mut out, tp);
            out.push_str("}\n");
        }
        out
    }

    /// `read_data`: the value at `r` appended to `b`.
    fn ser_body(&self, out: &mut String, tp: u16) {
        out.push_str("    loft_store_t *s = loft_store_of(r);\n    (void) s; (void) le;\n");
        match &self.stores.types[tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                for f in fields.iter().filter(|f| Self::serialised(f)) {
                    let _ = writeln!(
                        out,
                        "    loft_ser{}(b, loft_field(r, {}), le);",
                        f.content, f.position
                    );
                }
            }
            Parts::Short(..) => out.push_str(
                "    uint16_t raw;\n    memcpy(&raw, loft_at(s, r.rec, r.pos), 2);\n    loft_put(b, raw ? (uint16_t) (raw - 1) : 0, 2, le);\n",
            ),
            Parts::ShortRaw(from, _) => {
                let _ = writeln!(
                    out,
                    "    uint16_t raw;\n    memcpy(&raw, loft_at(s, r.rec, r.pos), 2);\n    loft_put(b, (uint16_t) (uint32_t) ((int64_t) raw + ({from})), 2, le);"
                );
            }
            Parts::Int(..) | Parts::IntRaw(..) => {
                out.push_str("    loft_put(b, loft_u32(s, r.rec, r.pos), 4, le);\n");
            }
            Parts::Vector(elem) => {
                let _ = writeln!(
                    out,
                    "    uint32_t v = loft_collection_rec(s, r.rec, r.pos);\n    uint32_t n = v == 0 ? 0 : loft_u32(s, v, 4);\n    for (uint32_t i = 0; i < n; i++) {{\n        loft_ref e = {{ r.store_nr, v, 8 + i * {} }};\n        loft_ser{elem}(b, e, le);\n    }}",
                    self.stores.size(*elem)
                );
            }
            // A plain enum and a byte: the stored byte.
            _ => out.push_str("    loft_push_char(b, (char) *loft_at(s, r.rec, r.pos));\n"),
        }
    }

    /// `write_data`: the value at `r` from the `n` bytes at `d`.
    fn de_body(&self, out: &mut String, tp: u16) {
        out.push_str(
            "    loft_store_t *s = loft_store_of(r);\n    (void) s; (void) le; (void) n;\n",
        );
        match &self.stores.types[tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                let mut off = 0usize;
                for f in fields.iter().filter(|f| Self::serialised(f)) {
                    let _ = writeln!(
                        out,
                        "    loft_de{}(loft_field(r, {}), le, d + {off}, n > {off} ? n - {off} : 0);",
                        f.content, f.position
                    );
                    off += self.stores.binary_size(f.content);
                }
            }
            Parts::Short(..) => out.push_str(
                "    uint16_t u = (uint16_t) loft_take(d, 2, le);\n    loft_set_short(r, 0, 0, u >= 0x8000 ? (int64_t) u - 0x10000 : (int64_t) u);\n",
            ),
            Parts::ShortRaw(from, _) => {
                let _ = writeln!(
                    out,
                    "    uint16_t u = (uint16_t) loft_take(d, 2, le);\n    loft_set_short_raw(r, 0, {from}, u >= 0x8000 ? (int64_t) u - 0x10000 : (int64_t) u);"
                );
            }
            Parts::Int(..) | Parts::IntRaw(..) => out.push_str(
                "    uint32_t u = (uint32_t) loft_take(d, 4, le);\n    memcpy(loft_at(s, r.rec, r.pos), &u, 4);\n",
            ),
            Parts::Vector(elem) => {
                let size = self.stores.size(*elem);
                let wire = match self.stores.binary_size(*elem) {
                    0 => usize::from(size),
                    w => w,
                };
                if size != 0 {
                    let _ = writeln!(
                        out,
                        "    for (size_t i = 0; i < n / {wire}; i++) {{\n        loft_ref e = loft_vec_append(r, {size});\n        loft_de{elem}(e, le, d + i * {wire}, {wire});\n        loft_vec_finish(r);\n    }}"
                    );
                }
            }
            _ => out.push_str("    *loft_at(s, r.rec, r.pos) = d[0];\n"),
        }
    }
}
