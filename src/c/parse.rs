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
        // `loft_parse_types` names the type table, which then exists.
        self.name_types();
        if tp <= 6 {
            return Ok(());
        }
        match &t.parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                // A linked group's members are filed together (`parse_group`), each of a kind
                // a group carries.
                for (i, f) in fields.iter().enumerate() {
                    if f.other_indexes().is_empty() {
                        continue;
                    }
                    let fld = u16::try_from(i).map_err(|_| "a record too wide")?;
                    self.member_row(f.position, f.content)?;
                    for (pos, mtp) in self.group_siblings(tp, fld) {
                        self.member_row(pos, mtp)?;
                    }
                }
                if !self.stores.zero_default(tp) {
                    self.default_image(tp);
                }
                fields.iter().try_for_each(|f| self.parse_type(f.content))
            }
            Parts::Vector(c) => self.parse_type(*c),
            // `@FR-L-OutOfLine` — an out-of-line field reads its content's value.
            Parts::ChildRec(c) if self.stores.child_rec_is_value(*c) => {
                self.name_types();
                self.describe_type(*c);
                self.parse_type(*c)
            }
            Parts::Enum(values) => {
                self.enum_names(tp);
                values
                    .iter()
                    .filter(|(v, _)| *v != u16::MAX)
                    .try_for_each(|(v, _)| self.parse_type(*v))
            }
            // A keyed collection: each element is filed under its key as it is read, and a
            // displaced one is released through the type table.
            Parts::Hash(c, _)
            | Parts::Sorted(c, _)
            | Parts::Index(c, _)
            | Parts::Ordered(c, _)
            | Parts::Radix(c, _)
            | Parts::Trie(c, _)
            | Parts::Array(c) => {
                self.keys_of(tp);
                self.name_types();
                self.describe_type(*c);
                self.parse_type(*c)
            }
            Parts::Byte(..)
            | Parts::Short(..)
            | Parts::ShortRaw(..)
            | Parts::Int(..)
            | Parts::IntRaw(..)
            | Parts::DbRef
            | Parts::ChildRec(_)
            | Parts::Base => Ok(()),
        }
    }

    /// A synthetic `__nullable<S>` (`Data::nullable_enum_for`): its `Some` variant's value and
    /// the type and position of that variant's `payload` field, which a present object is
    /// read into (`walk_parsed_into`'s @PLN25 E2 arm); nothing for any other enum.
    fn nullable_some(&self, tp: u16, values: &[(u16, String)]) -> String {
        if !self.stores.types[tp as usize]
            .name
            .starts_with("__nullable<")
        {
            return String::new();
        }
        let Some((k, (vtp, _))) = values.iter().enumerate().find(|(_, (_, n))| n == "Some") else {
            return String::new();
        };
        let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
            &self.stores.types[*vtp as usize].parts
        else {
            return String::new();
        };
        fields
            .iter()
            .find(|f| f.name == "payload")
            .map_or_else(String::new, |f| {
                format!(
                    ", .some = {}, .payload_tp = {}, .payload_pos = {}",
                    k + 1,
                    f.content,
                    f.position
                )
            })
    }

    /// A collection's row: its kind, element, key parts and whether its records are linked.
    fn keyed_row(&self, tp: u16, c: u16) -> String {
        let (keys, nk) = self.keys_of(tp);
        let keyed = match self.stores.types[tp as usize].parts {
            Parts::Array(_) => "LOFT_P_ARRAY",
            Parts::Hash(..) => "LOFT_KEYED_HASH",
            Parts::Sorted(..) => "LOFT_KEYED_SORTED",
            Parts::Index(..) => "LOFT_KEYED_INDEX",
            Parts::Ordered(..) => "LOFT_KEYED_ORDERED",
            Parts::Radix(..) => "LOFT_KEYED_SPATIAL",
            _ => "LOFT_KEYED_TRIE",
        };
        format!(
            ".kind = LOFT_P_KEYED, .elem = {c}, .keyed = {keyed}, .keys = {keys}, .nk = {nk}, .linked = {}",
            u8::from(self.stores.is_linked(c))
        )
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
                    let mut field_rows = Vec::new();
                    for (i, f) in fields.iter().enumerate() {
                        field_rows.push(self.parse_field(f, &self.parse_group(&mut out, tp, i)));
                    }
                    let _ = writeln!(
                        out,
                        "static const loft_pfield loft_pf{tp}[] = {{ {} }};",
                        if field_rows.is_empty() {
                            "{ \"\", 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 }".to_string()
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
                Parts::Hash(c, _)
                | Parts::Sorted(c, _)
                | Parts::Index(c, _)
                | Parts::Ordered(c, _)
                | Parts::Radix(c, _)
                | Parts::Trie(c, _)
                | Parts::Array(c) => self.keyed_row(tp, *c),
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
                        ".kind = LOFT_P_ENUM, .n = {}, .vnames = {}, .vtypes = loft_pv{tp}, .strict_enum = {}{}",
                        values.len(),
                        self.enum_names(tp),
                        u8::from(strict),
                        self.nullable_some(tp, values)
                    )
                }
                Parts::Byte(min, null) => narrow("LOFT_P_BYTE", *min, *null),
                Parts::Short(min, null) => narrow("LOFT_P_SHORT", *min, *null),
                Parts::ShortRaw(min, null) => narrow("LOFT_P_SHORTRAW", *min, *null),
                Parts::Int(min, null) => narrow("LOFT_P_NINT", *min, *null),
                Parts::IntRaw(min, null) => narrow("LOFT_P_NINTRAW", *min, *null),
                Parts::DbRef => ".kind = LOFT_P_DBREF".to_string(),
                Parts::ChildRec(c) => format!(
                    ".kind = LOFT_P_CHILD, .elem = {}",
                    if self.stores.child_rec_is_value(*c) {
                        *c
                    } else {
                        u16::MAX
                    }
                ),
                Parts::Base => ".kind = LOFT_P_OTHER".to_string(),
            };
            let row = format!("{{ {row}, .size = {size}, .name = {} }}", c_string(&t.name));
            rows.push(format!("[{tp}] = {row}"));
        }
        let _ = writeln!(
            out,
            "static const loft_ptype loft_ptypes[{}] = {{ {} }};\nstatic const loft_tdesc *loft_parse_types(void) {{ return loft_types; }}",
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
    /// The group the `i`-th field of struct `tp` is a member of (`@FR-Col-Group`): its members
    /// — the field itself first — written as `loft_pg{tp}_{i}`, and the initializer tail a
    /// field row carries for it; the empty tail outside a group, or when a member is a kind
    /// a group does not carry (the record finish then files the field alone).
    fn parse_group(&self, out: &mut String, tp: u16, i: usize) -> String {
        let fld = u16::try_from(i).unwrap_or(u16::MAX);
        let siblings = self.group_siblings(tp, fld);
        if siblings.is_empty() {
            return String::new();
        }
        let mut members = vec![(
            self.stores.field_position(tp, fld),
            self.stores.field_type(tp, fld),
        )];
        members.extend(siblings);
        let Ok(rows) = members
            .iter()
            .map(|&(pos, mtp)| self.member_row(pos, mtp))
            .collect::<Result<Vec<_>, _>>()
        else {
            return String::new();
        };
        let Ok((elem, held, nullable)) = self.group_release(&members) else {
            return String::new();
        };
        let _ = writeln!(
            out,
            "static const loft_member loft_pg{tp}_{i}[] = {{ {} }};",
            rows.join(", ")
        );
        format!(
            ", loft_pg{tp}_{i}, {}, {elem}, {held}, {nullable}",
            members.len()
        )
    }

    fn parse_field(&self, f: &crate::database::Field, group: &str) -> String {
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
            "{{ {}, {}, {}, {}, {}, {}, {dkind}, {}, UINT64_C({dbits:#x}), {}, {}{group} }}",
            c_string(&f.name),
            f.content,
            f.position,
            u8::from(f.nullable),
            // No absent pass writes the `enum` tag, nor a view of a linked group: the records
            // its owner holds fill it (`write_absent_fields`).
            u8::from(f.name == "enum" || f.other_indexes().first() == Some(&u16::MAX)),
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
