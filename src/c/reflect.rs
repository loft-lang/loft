// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C
//! Type reflection in C — `reflect_type(kt)` and `type_named(name)` (`default/07_reflect.loft`).
//! What a type reflects as is the interpreter's own answer: `native::reflect_type_into` runs
//! on a scratch copy of the stores for every type of the program, and the record it writes is
//! read back into `loft_rtypes`, a row per type.  The runtime (`loft_rt.h`, `loft_reflect`)
//! builds the `TypeInfo` from that row by the same sequence of writes, so the two cannot
//! answer differently.
use super::{Cx, body::c_string};
use crate::database::Stores;
use crate::keys::DbRef;
use std::fmt::Write as _;

/// One type's reflection, read back from the record `reflect_type_into` wrote.
struct Row {
    name: String,
    kind: i32,
    size: i64,
    element: String,
    collection: i32,
    /// `None` when the record has no field vector (a type that is no record).
    fields: Option<Vec<(String, String, i64, i32, i32)>>,
    variants: Option<Vec<String>>,
    keys: Option<Vec<(String, i64, i32)>>,
}

/// `(type, field)`'s byte position in the reflection schema.
fn at(s: &Stores, tp: &str, field: &str) -> u32 {
    u32::from(s.position(s.name(tp), field))
}

/// The vector at field `fld` of `(rec, pos)`: its record and length, or `None` for no vector.
fn vector_at(s: &Stores, r: &DbRef, rec: u32, fld: u32) -> Option<(u32, u32)> {
    let v = s.store(r).get_u32_raw(rec, fld);
    (v != 0).then(|| (v, s.store(r).get_u32_raw(v, 4)))
}

fn read_row(s: &mut Stores, kt: u16) -> Row {
    let r = crate::native::reflect_type_into(s, kt);
    let (fi, vi, ki) = (
        u32::from(s.size(s.name("FieldInfo"))),
        u32::from(s.size(s.name("VariantInfo"))),
        u32::from(s.size(s.name("KeyInfo"))),
    );
    let st = s.store(&r);
    let base = r.pos;
    let text = |rec: u32, fld: u32| st.get_str(st.get_u32_raw(rec, fld)).to_string();
    let fields = vector_at(s, &r, r.rec, base + at(s, "TypeInfo", "fields")).map(|(v, n)| {
        (0..n)
            .map(|i| {
                let e = 8 + i * fi;
                (
                    text(v, e + at(s, "FieldInfo", "name")),
                    text(v, e + at(s, "FieldInfo", "type_name")),
                    st.get_int(v, e + at(s, "FieldInfo", "position")),
                    st.get_byte(v, e + at(s, "FieldInfo", "kind"), 0),
                    st.get_byte(v, e + at(s, "FieldInfo", "nullable"), 0),
                )
            })
            .collect()
    });
    let variants = vector_at(s, &r, r.rec, base + at(s, "TypeInfo", "variants")).map(|(v, n)| {
        (0..n)
            .map(|i| text(v, 8 + i * vi + at(s, "VariantInfo", "name")))
            .collect()
    });
    let keys = vector_at(s, &r, r.rec, base + at(s, "TypeInfo", "keys")).map(|(v, n)| {
        (0..n)
            .map(|i| {
                let e = 8 + i * ki;
                (
                    text(v, e + at(s, "KeyInfo", "name")),
                    st.get_int(v, e + at(s, "KeyInfo", "position")),
                    st.get_byte(v, e + at(s, "KeyInfo", "ascending"), 0),
                )
            })
            .collect()
    });
    Row {
        name: text(r.rec, base + at(s, "TypeInfo", "name")),
        kind: st.get_byte(r.rec, base + at(s, "TypeInfo", "kind"), 0),
        size: st.get_int(r.rec, base + at(s, "TypeInfo", "size")),
        element: text(r.rec, base + at(s, "TypeInfo", "element")),
        collection: st.get_byte(r.rec, base + at(s, "TypeInfo", "collection"), 0),
        fields,
        variants,
        keys,
    }
}

/// The data fields of `kt` (`LayoutField::is_data`) as `reflect_field_at` reads them: position,
/// type, `TypeKind` code, how the scalar is read (`loft_rdata.cls`), a narrow width's bias and
/// nullability, and whether a path may descend into it (an inline record).
fn data_rows(s: &Stores, kt: u16) -> Vec<String> {
    use crate::database::{BaseKind, LayoutNode};
    let desc = s.layout_descriptor(&[kt]);
    let Some(LayoutNode::Record(fields) | LayoutNode::EnumValue(_, fields)) = desc.nodes.get(&kt)
    else {
        return Vec::new();
    };
    fields
        .iter()
        .filter(|f| f.is_data())
        .map(|f| {
            let node = desc.nodes.get(&f.content);
            let kind = crate::native::reflect_kind(node);
            let (cls, start, nullable) = match node {
                Some(LayoutNode::Base(BaseKind::Integer)) => (1, 0, false),
                Some(LayoutNode::Base(BaseKind::Long)) => (2, 0, false),
                Some(LayoutNode::Base(BaseKind::Float)) => (3, 0, false),
                Some(LayoutNode::Base(BaseKind::Single)) => (4, 0, false),
                Some(LayoutNode::Base(BaseKind::Boolean)) => (5, 0, false),
                Some(LayoutNode::Base(BaseKind::Character)) => (6, 0, false),
                Some(LayoutNode::Base(BaseKind::Text)) => (7, 0, false),
                Some(LayoutNode::Byte { start, nullable }) => (8, *start, *nullable),
                Some(LayoutNode::Short { start, .. }) => (9, *start, false),
                Some(LayoutNode::ShortRaw { start, .. }) => (10, *start, false),
                Some(LayoutNode::Int { .. }) => (11, 0, false),
                _ => (0, 0, false),
            };
            let inline = matches!(
                node,
                Some(LayoutNode::Record(_) | LayoutNode::EnumValue(..))
            );
            format!(
                "{{ {}, {}, {kind}, {cls}, {start}, {}, {} }}",
                f.position,
                f.content,
                u8::from(nullable),
                u8::from(inline)
            )
        })
        .collect()
}

/// A C array of `items`, or one placeholder element for an empty list (C99 has no empty
/// initialiser).
fn c_array(out: &mut String, decl: &str, items: &[String], empty: &str) {
    let body = if items.is_empty() {
        empty.to_string()
    } else {
        items.join(", ")
    };
    let _ = writeln!(out, "{decl} = {{ {body} }};");
}

/// Type `kt`'s field, variant, key and data arrays.
fn row_arrays(out: &mut String, kt: usize, row: &Row, data: &[String]) {
    if let Some(fields) = &row.fields {
        let items: Vec<String> = fields
            .iter()
            .map(|(name, tn, pos, kind, null)| {
                format!(
                    "{{ {}, {}, {pos}, {kind}, {null} }}",
                    c_string(name),
                    c_string(tn)
                )
            })
            .collect();
        let decl = format!("static const loft_rfield loft_rf{kt}[]");
        c_array(out, &decl, &items, "{ \"\", \"\", 0, 0, 0 }");
    }
    if let Some(variants) = &row.variants {
        let items: Vec<String> = variants.iter().map(|v| c_string(v)).collect();
        let decl = format!("static const char *const loft_rv{kt}[]");
        c_array(out, &decl, &items, "\"\"");
    }
    if let Some(keys) = &row.keys {
        let items: Vec<String> = keys
            .iter()
            .map(|(name, pos, asc)| format!("{{ {}, {pos}, {asc} }}", c_string(name)))
            .collect();
        let decl = format!("static const loft_rkey loft_rk{kt}[]");
        c_array(out, &decl, &items, "{ \"\", 0, 0 }");
    }
    if !data.is_empty() {
        let decl = format!("static const loft_rdata loft_rd{kt}[]");
        c_array(out, &decl, data, "");
    }
}

/// Type `kt`'s `loft_rtype` initialiser; `ndata` its data fields.
fn row_entry(kt: usize, row: &Row, lookup: &str, ndata: usize) -> String {
    let count = |n: Option<usize>, prefix: &str| {
        n.map_or((-1, "0".to_string()), |n| {
            (n as i64, format!("{prefix}{kt}"))
        })
    };
    let (nf, f) = count(row.fields.as_ref().map(Vec::len), "loft_rf");
    let (nv, v) = count(row.variants.as_ref().map(Vec::len), "loft_rv");
    let (nk, k) = count(row.keys.as_ref().map(Vec::len), "loft_rk");
    let d = if ndata > 0 {
        format!("loft_rd{kt}")
    } else {
        "0".to_string()
    };
    format!(
        "{{ {lookup}, {}, {}, INT64_C({}), {}, {}, {nf}, {f}, {nv}, {v}, {nk}, {k}, {ndata}, {d} }}",
        c_string(&row.name),
        row.kind,
        row.size,
        c_string(&row.element),
        row.collection,
    )
}

/// `loft_rlay`: the reflection records' type numbers, sizes and field positions.
fn layout_line(stores: &Stores) -> String {
    let size = |t: &str| stores.size(stores.name(t)).to_string();
    let pos = |t: &str, f: &str| at(stores, t, f).to_string();
    let mut parts = vec![
        stores.name("TypeInfo").to_string(),
        size("TypeInfo"),
        size("FieldInfo"),
        size("VariantInfo"),
        size("KeyInfo"),
    ];
    for (t, f) in [
        ("TypeInfo", "name"),
        ("TypeInfo", "kind"),
        ("TypeInfo", "size"),
        ("TypeInfo", "fields"),
        ("TypeInfo", "variants"),
        ("TypeInfo", "element"),
        ("TypeInfo", "collection"),
        ("TypeInfo", "keys"),
        ("FieldInfo", "name"),
        ("FieldInfo", "type_name"),
        ("FieldInfo", "position"),
        ("FieldInfo", "kind"),
        ("FieldInfo", "nullable"),
        ("VariantInfo", "name"),
        ("VariantInfo", "tag"),
        ("KeyInfo", "name"),
        ("KeyInfo", "position"),
        ("KeyInfo", "ascending"),
    ] {
        parts.push(pos(t, f));
    }
    parts.push(stores.name("ValueInfo").to_string());
    parts.push(size("ValueInfo"));
    for f in ["kind", "is_null", "i", "f", "t"] {
        parts.push(pos("ValueInfo", f));
    }
    format!(
        "static const loft_rlayout loft_rlay = {{ {} }};\n",
        parts.join(", ")
    )
}

impl Cx<'_> {
    /// `loft_rtypes`: the reflection of every type the program has, the layout of the
    /// reflection records, and the answer for a type number it has not (`#kt`).
    pub(crate) fn reflect_table(&self) -> String {
        // The clone keeps the schema and no stores (as `default_image`'s scratch); the answers
        // are left in it and dropped with it.
        let mut scratch = self.stores.clone();
        scratch.max = 0;
        let count = self.stores.types.len();
        let rows: Vec<Row> = (0..=count)
            .map(|kt| read_row(&mut scratch, kt as u16))
            .collect();
        let mut out = String::new();
        let mut entries = Vec::with_capacity(count);
        for (kt, row) in rows.iter().enumerate().take(count) {
            let data = data_rows(self.stores, kt as u16);
            row_arrays(&mut out, kt, row, &data);
            let lookup = c_string(&self.stores.types[kt].name);
            entries.push(row_entry(kt, row, &lookup, data.len()));
        }
        let _ = writeln!(
            out,
            "static const loft_rtype loft_rtypes[] = {{ {} }};\nstatic const uint32_t loft_nrtypes = {count};",
            entries.join(",\n    ")
        );
        // A number past the table: no node, so the kind and collection of no type.
        let _ = writeln!(
            out,
            "static const loft_rtype loft_rnone = {};",
            row_entry(count, &rows[count], "\"\"", 0)
        );
        out.push_str(&layout_line(self.stores));
        out
    }
}
