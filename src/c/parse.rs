// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C
//! `T.parse(text)` in C — `Stores::record_from_text`.  The runtime's walk (`loft_parse.h`)
//! reads what each type accepts and what an absent value leaves from `loft_ptypes`, a row per
//! type the parse reaches, written here from the program's own type table: a record's fields
//! with their names, nullability and declared defaults, an enum's variants, a narrow integer's
//! encoding, a collection's content.
use super::{Cx, body::c_string};
use crate::database::Parts;
use crate::keys::Content;
use std::fmt::Write as _;

impl Cx<'_> {
    /// Register type `tp` and every type a parse of it reaches, or refuse a kind the C walk does
    /// not read.
    pub(crate) fn parse_type(&self, tp: u16) -> Result<(), String> {
        if self.parse_types.borrow().contains(&tp) {
            return Ok(());
        }
        let t = self
            .stores
            .types
            .get(tp as usize)
            .ok_or("a parse of a type not laid out")?;
        self.parse_types.borrow_mut().insert(tp);
        if tp <= 6 {
            return Ok(());
        }
        if t.name.starts_with("__nullable<") {
            return Err("a parse of a vector of nullable records".into());
        }
        match &t.parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                if fields.iter().any(|f| !f.other_indexes().is_empty()) {
                    return Err("a parse of a record holding a linked group".into());
                }
                if !self.stores.zero_default(tp) {
                    self.default_image(tp);
                }
                fields.iter().try_for_each(|f| self.parse_type(f.content))
            }
            Parts::Vector(c) => self.parse_type(*c),
            Parts::Enum(values) => {
                self.enum_names(tp);
                values
                    .iter()
                    .filter(|(v, _)| *v != u16::MAX)
                    .try_for_each(|(v, _)| self.parse_type(*v))
            }
            Parts::Byte(..)
            | Parts::Short(..)
            | Parts::ShortRaw(..)
            | Parts::Int(..)
            | Parts::IntRaw(..)
            | Parts::DbRef
            | Parts::ChildRec(_)
            | Parts::Base => Ok(()),
            _ => Err("a parse of a keyed collection".into()),
        }
    }

    /// The parse table, `loft_ptypes`, indexed by type number; nothing when no parse is
    /// written.
    pub(crate) fn parse_table(&self) -> String {
        let types = self.parse_types.borrow();
        let Some(&last) = types.last() else {
            return String::new();
        };
        let mut out = String::new();
        let mut rows = Vec::new();
        for &tp in types.iter() {
            let t = &self.stores.types[tp as usize];
            let size = self.stores.size(tp);
            let narrow = |kind: &str, min: i32, null: bool| {
                format!(
                    ".kind = {kind}, .min = {min}, .nullable = {}",
                    u8::from(null)
                )
            };
            let row = match &t.parts {
                _ if tp <= 6 => format!(".kind = {tp}"),
                Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                    let field_rows: Vec<String> =
                        fields.iter().map(|f| self.parse_field(f)).collect();
                    let _ = writeln!(
                        out,
                        "static const loft_pfield loft_pf{tp}[] = {{ {} }};",
                        if field_rows.is_empty() {
                            "{ \"\", 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 }".to_string()
                        } else {
                            field_rows.join(", ")
                        }
                    );
                    let zero = self.stores.zero_default(tp);
                    let prefill = if zero {
                        String::new()
                    } else {
                        format!(", .prefill = {}", self.default_image(tp).0)
                    };
                    format!(
                        ".kind = LOFT_P_RECORD, .n = {}, .fields = loft_pf{tp}, .variant = {}, .zero_default = {}{prefill}",
                        fields.len(),
                        u8::from(matches!(t.parts, Parts::EnumValue(..))),
                        u8::from(zero)
                    )
                }
                Parts::Vector(c) => format!(".kind = LOFT_P_VECTOR, .elem = {c}"),
                Parts::Enum(values) => {
                    let variant_types: Vec<String> =
                        values.iter().map(|(v, _)| v.to_string()).collect();
                    let _ = writeln!(
                        out,
                        "static const uint16_t loft_pv{tp}[] = {{ {} }};",
                        if variant_types.is_empty() {
                            "0".to_string()
                        } else {
                            variant_types.join(", ")
                        }
                    );
                    let strict = !t.name.starts_with("__") && t.name != "JsonValue";
                    format!(
                        ".kind = LOFT_P_ENUM, .n = {}, .vnames = {}, .vtypes = loft_pv{tp}, .strict_enum = {}",
                        values.len(),
                        self.enum_names(tp),
                        u8::from(strict)
                    )
                }
                Parts::Byte(min, null) => narrow("LOFT_P_BYTE", *min, *null),
                Parts::Short(min, null) => narrow("LOFT_P_SHORT", *min, *null),
                Parts::ShortRaw(min, null) => narrow("LOFT_P_SHORTRAW", *min, *null),
                Parts::Int(min, null) => narrow("LOFT_P_NINT", *min, *null),
                Parts::IntRaw(min, null) => narrow("LOFT_P_NINTRAW", *min, *null),
                Parts::DbRef => ".kind = LOFT_P_DBREF".to_string(),
                Parts::ChildRec(_) => ".kind = LOFT_P_CHILD".to_string(),
                _ => ".kind = LOFT_P_OTHER".to_string(),
            };
            let row = format!("{{ {row}, .size = {size}, .name = {} }}", c_string(&t.name));
            rows.push(format!("[{tp}] = {row}"));
        }
        let _ = writeln!(
            out,
            "static const loft_ptype loft_ptypes[{}] = {{ {} }};",
            u32::from(last) + 1,
            rows.join(", ")
        );
        out
    }

    /// `loft_is_main_vector`: the types a vector variable's own store is minted as
    /// (`main_vector<T>`), which a whole-vector write delivers parse errors into.
    pub(crate) fn main_vector_test(&self) -> String {
        let cases: Vec<String> = self
            .stores
            .types
            .iter()
            .enumerate()
            .filter(|(_, t)| t.name.starts_with("main_vector<"))
            .map(|(tp, _)| format!("    case {tp}:"))
            .collect();
        if cases.is_empty() {
            return "static int loft_is_main_vector(uint16_t tp) {\n    (void) tp;\n    return 0;\n}\n"
                .to_string();
        }
        format!(
            "static int loft_is_main_vector(uint16_t tp) {{\n    switch (tp) {{\n{}\n        return 1;\n    default:\n        return 0;\n    }}\n}}\n",
            cases.join("\n")
        )
    }

    /// A field's row: its name, type, position, declared nullability, whether the absent
    /// pass passes it over (`enum`), whether it is the `type` tag a prefill stamps, and its
    /// declared default as the literal `write_declared_default` walks in.
    fn parse_field(&self, f: &crate::database::Field) -> String {
        let (dkind, di, dbits, ds) = match &f.default {
            None => ("LOFT_PD_NONE", 0, 0, String::new()),
            Some(Content::Long(n)) => match &self.stores.types[f.content as usize].parts {
                Parts::Enum(vals) if *n > 0 && (*n as usize) <= vals.len() => {
                    ("LOFT_PD_STR", 0, 0, vals[*n as usize - 1].1.clone())
                }
                _ if f.content == 4 => ("LOFT_PD_BOOL", *n, 0, String::new()),
                _ => ("LOFT_PD_INT", *n, 0, String::new()),
            },
            Some(Content::Float(v)) => ("LOFT_PD_NUMBER", 0, v.to_bits(), String::new()),
            Some(Content::Single(v)) => {
                ("LOFT_PD_NUMBER", 0, f64::from(*v).to_bits(), String::new())
            }
            Some(Content::Str(s)) => ("LOFT_PD_STR", 0, 0, s.str().to_string()),
        };
        format!(
            "{{ {}, {}, {}, {}, {}, {}, {dkind}, {}, UINT64_C({dbits:#x}), {}, {} }}",
            c_string(&f.name),
            f.content,
            f.position,
            u8::from(f.nullable),
            u8::from(f.name == "enum"),
            u8::from(f.name == "type" && f.position == 0),
            if di == i64::MIN {
                "INT64_MIN".to_string()
            } else {
                format!("INT64_C({di})")
            },
            c_string(&ds),
            ds.len()
        )
    }
}
