// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)

//! @PLAN28 Design A — JSON codec for the database **schema** (`Stores.types`
//! + `Stores.names`).
//!
//! The startup cache stores the parsed `Data` AND the database schema it
//! produced, then restores both verbatim — instead of recomputing the schema
//! on load.  Recomputation (re-running `typedef::fill_all` over an
//! already-resolved `Data`) does not work: those passes assume raw parser
//! output and double-process.  The schema is derived from `Data`, but — like
//! slots and the `Data` lookup indices — "derived" means "must stay consistent
//! with `Data`", which we guarantee **by construction** by snapshotting it from
//! the same parse.
//!
//! This lives in the `database` module because `Type`'s layout fields
//! (`parents`/`complex`/`linked`/`size`/`align`) are `pub(super)` and
//! `Field.other_indexes` is module-private — only code inside `database` can
//! read and reconstruct them.
//!
//! Conventions match `src/ir_schema.rs`: a tagged object `{ "k": "<variant>",
//! … }`, absolute type indices stored verbatim (whole-bundle image, no
//! relocation), strings via a local JSON escaper, decode via
//! `crate::json::parse`.  No serde.

use crate::data::{LinkedFieldGroup, LinkedFieldKind};
use crate::database::{Field, Parts, Stores, Type};
use crate::json::Parsed;
use crate::keys::{Content, Key, Str};
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// Errors from decoding the schema out of a `Parsed` tree.
#[derive(Debug, PartialEq, Eq)]
pub enum SchemaDecodeError {
    /// Top-level JSON did not parse, or a sub-node had the wrong JSON shape.
    Shape(String),
    /// The `"k"` tag named a `Parts` / `Content` variant we don't know.
    UnknownTag(String),
}

// ─── shared small helpers (kept local; ir_schema's are private) ───────────────

fn esc(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn field<'a>(p: &'a Parsed, name: &str) -> Result<&'a Parsed, SchemaDecodeError> {
    if let Parsed::Object(entries) = p {
        entries
            .iter()
            .find(|(k, _, _)| k == name)
            .map(|(_, _, v)| v)
            .ok_or_else(|| SchemaDecodeError::Shape(format!("missing field '{name}'")))
    } else {
        Err(SchemaDecodeError::Shape(format!(
            "expected object for '{name}'"
        )))
    }
}

fn as_i64(p: &Parsed) -> Result<i64, SchemaDecodeError> {
    // @PLN109 — `Parsed::as_i64` accepts both `Int` (exact i64) and `Number`.
    p.as_i64()
        .ok_or_else(|| SchemaDecodeError::Shape("expected number".into()))
}

fn as_f64(p: &Parsed) -> Result<f64, SchemaDecodeError> {
    p.as_f64()
        .ok_or_else(|| SchemaDecodeError::Shape("expected number".into()))
}

fn as_u32(p: &Parsed) -> Result<u32, SchemaDecodeError> {
    Ok(as_i64(p)? as u32)
}
fn as_u16(p: &Parsed) -> Result<u16, SchemaDecodeError> {
    Ok(as_i64(p)? as u16)
}
fn as_u8(p: &Parsed) -> Result<u8, SchemaDecodeError> {
    Ok(as_i64(p)? as u8)
}
fn as_i32(p: &Parsed) -> Result<i32, SchemaDecodeError> {
    Ok(as_i64(p)? as i32)
}
fn as_i8(p: &Parsed) -> Result<i8, SchemaDecodeError> {
    Ok(as_i64(p)? as i8)
}

fn as_bool(p: &Parsed) -> Result<bool, SchemaDecodeError> {
    if let Parsed::Bool(b) = p {
        Ok(*b)
    } else {
        Err(SchemaDecodeError::Shape("expected bool".into()))
    }
}

fn as_str(p: &Parsed) -> Result<String, SchemaDecodeError> {
    match p {
        Parsed::Str(s) | Parsed::Ident(s) => Ok(s.clone()),
        _ => Err(SchemaDecodeError::Shape("expected string".into())),
    }
}

fn arr(p: &Parsed) -> Result<&[Parsed], SchemaDecodeError> {
    if let Parsed::Array(items) = p {
        Ok(items)
    } else {
        Err(SchemaDecodeError::Shape("expected array".into()))
    }
}

fn u16_list(out: &mut String, list: &[u16]) {
    out.push('[');
    for (i, n) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{n}");
    }
    out.push(']');
}

fn u16_vec(p: &Parsed) -> Result<Vec<u16>, SchemaDecodeError> {
    arr(p)?.iter().map(as_u16).collect()
}

// (u16, bool) key lists — used by Sorted/Ordered/Index.
fn key_pairs(out: &mut String, pairs: &[(u16, bool)]) {
    out.push('[');
    for (i, (n, asc)) in pairs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "[{n},{asc}]");
    }
    out.push(']');
}

fn key_pair_vec(p: &Parsed) -> Result<Vec<(u16, bool)>, SchemaDecodeError> {
    arr(p)?
        .iter()
        .map(|it| {
            let pair = arr(it)?;
            if pair.len() != 2 {
                return Err(SchemaDecodeError::Shape("expected [u16,bool] pair".into()));
            }
            Ok((as_u16(&pair[0])?, as_bool(&pair[1])?))
        })
        .collect()
}

// ─── LinkedFieldGroup (Type.field_groups) — self-contained, mirrors ir_schema ──

fn write_field_groups(out: &mut String, groups: &[LinkedFieldGroup]) {
    out.push('[');
    for (i, g) in groups.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let kind = match g.kind {
            LinkedFieldKind::Tuple => "Tuple",
            LinkedFieldKind::Index => "Index",
        };
        let _ = write!(out, "{{\"kind\":\"{kind}\",\"instance\":{}", g.instance);
        out.push_str(",\"field_indices\":");
        u16_list(out, &g.field_indices);
        let _ = write!(out, ",\"alignment\":{},\"size\":{}}}", g.alignment, g.size);
    }
    out.push(']');
}

fn field_groups_from(p: &Parsed) -> Result<Vec<LinkedFieldGroup>, SchemaDecodeError> {
    arr(p)?
        .iter()
        .map(|it| {
            let kind = match as_str(field(it, "kind")?)?.as_str() {
                "Tuple" => LinkedFieldKind::Tuple,
                "Index" => LinkedFieldKind::Index,
                other => {
                    return Err(SchemaDecodeError::UnknownTag(format!(
                        "LinkedFieldKind::{other}"
                    )));
                }
            };
            Ok(LinkedFieldGroup {
                kind,
                instance: as_u16(field(it, "instance")?)?,
                field_indices: u16_vec(field(it, "field_indices")?)?,
                alignment: as_u8(field(it, "alignment")?)?,
                size: as_u16(field(it, "size")?)?,
            })
        })
        .collect()
}

// ─── Content (Field.default) ──────────────────────────────────────────────────

fn write_content(out: &mut String, c: &Content) {
    match c {
        Content::Long(n) => {
            let _ = write!(out, "{{\"k\":\"Long\",\"n\":{n}}}");
        }
        Content::Float(f) => {
            let _ = write!(out, "{{\"k\":\"Float\",\"f\":{f}}}");
        }
        Content::Single(f) => {
            let _ = write!(out, "{{\"k\":\"Single\",\"f\":{f}}}");
        }
        Content::Str(s) => {
            out.push_str("{\"k\":\"Str\",\"s\":");
            esc(out, s.str());
            out.push('}');
        }
    }
}

fn content_from(p: &Parsed) -> Result<Content, SchemaDecodeError> {
    let tag = as_str(field(p, "k")?)?;
    Ok(match tag.as_str() {
        "Long" => Content::Long(as_i64(field(p, "n")?)?),
        "Float" => Content::Float(as_f64(field(p, "f")?)?),
        "Single" => Content::Single(as_f64(field(p, "f")?)? as f32),
        // `Str::new` stores a raw pointer into the &str's backing memory, so a
        // non-empty default must outlive this call.  Empty strings use a
        // dangling pointer that reads "" — the common stdlib case needs no
        // leak.  Non-empty defaults are leaked (bounded: a snapshot holds a
        // small fixed set, alive for the whole process anyway).
        "Str" => {
            let s = as_str(field(p, "s")?)?;
            if s.is_empty() {
                Content::Str(Str::new(""))
            } else {
                Content::Str(Str::new(Box::leak(s.into_boxed_str())))
            }
        }
        other => return Err(SchemaDecodeError::UnknownTag(format!("Content::{other}"))),
    })
}

// ─── Field ────────────────────────────────────────────────────────────────────

fn write_field(out: &mut String, f: &Field) {
    out.push_str("{\"name\":");
    esc(out, &f.name);
    let _ = write!(
        out,
        ",\"content\":{},\"position\":{},\"default\":",
        f.content, f.position
    );
    write_content(out, &Field::default_to_wire(f.default.as_ref()));
    out.push_str(",\"nullable\":");
    out.push_str(if f.nullable { "true" } else { "false" });
    out.push_str(",\"other_indexes\":");
    u16_list(out, &f.other_indexes);
    out.push('}');
}

fn field_from(p: &Parsed) -> Result<Field, SchemaDecodeError> {
    Ok(Field {
        name: as_str(field(p, "name")?)?,
        content: as_u16(field(p, "content")?)?,
        position: as_u16(field(p, "position")?)?,
        default: Field::default_from_wire(content_from(field(p, "default")?)?),
        // An older snapshot has no `nullable` key; absent reads as non-null,
        // which is what every field in one of those was.
        nullable: field(p, "nullable")
            .ok()
            .and_then(|v| as_bool(v).ok())
            .unwrap_or(false),
        other_indexes: u16_vec(field(p, "other_indexes")?)?,
    })
}

fn write_field_list(out: &mut String, fields: &[Field]) {
    out.push('[');
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_field(out, f);
    }
    out.push(']');
}

fn field_vec(p: &Parsed) -> Result<Vec<Field>, SchemaDecodeError> {
    arr(p)?.iter().map(field_from).collect()
}

// ─── Parts ──────────────────────────────────────────────────────────────────

fn write_parts(out: &mut String, parts: &Parts) {
    match parts {
        Parts::Base => out.push_str("{\"k\":\"Base\"}"),
        Parts::DbRef => out.push_str("{\"k\":\"DbRef\"}"),
        Parts::Struct(fields) => {
            out.push_str("{\"k\":\"Struct\",\"fields\":");
            write_field_list(out, fields);
            out.push('}');
        }
        Parts::EnumValue(disc, fields) => {
            let _ = write!(out, "{{\"k\":\"EnumValue\",\"disc\":{disc},\"fields\":");
            write_field_list(out, fields);
            out.push('}');
        }
        Parts::Enum(values) => {
            out.push_str("{\"k\":\"Enum\",\"values\":[");
            for (i, (n, name)) in values.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "[{n},");
                esc(out, name);
                out.push(']');
            }
            out.push_str("]}");
        }
        Parts::Byte(n, b) => {
            let _ = write!(out, "{{\"k\":\"Byte\",\"n\":{n},\"b\":{b}}}");
        }
        Parts::Short(n, b) => {
            let _ = write!(out, "{{\"k\":\"Short\",\"n\":{n},\"b\":{b}}}");
        }
        Parts::Int(n, b) => {
            let _ = write!(out, "{{\"k\":\"Int\",\"n\":{n},\"b\":{b}}}");
        }
        Parts::IntRaw(n, b) => {
            let _ = write!(out, "{{\"k\":\"IntRaw\",\"n\":{n},\"b\":{b}}}");
        }
        Parts::ShortRaw(n, b) => {
            let _ = write!(out, "{{\"k\":\"ShortRaw\",\"n\":{n},\"b\":{b}}}");
        }
        Parts::Vector(c) => {
            let _ = write!(out, "{{\"k\":\"Vector\",\"c\":{c}}}");
        }
        Parts::Array(c) => {
            let _ = write!(out, "{{\"k\":\"Array\",\"c\":{c}}}");
        }
        Parts::ChildRec(c) => {
            let _ = write!(out, "{{\"k\":\"ChildRec\",\"c\":{c}}}");
        }
        Parts::Sorted(c, keys) => {
            let _ = write!(out, "{{\"k\":\"Sorted\",\"c\":{c},\"keys\":");
            key_pairs(out, keys);
            out.push('}');
        }
        Parts::Ordered(c, keys) => {
            let _ = write!(out, "{{\"k\":\"Ordered\",\"c\":{c},\"keys\":");
            key_pairs(out, keys);
            out.push('}');
        }
        Parts::Hash(c, keys) => {
            let _ = write!(out, "{{\"k\":\"Hash\",\"c\":{c},\"keys\":");
            u16_list(out, keys);
            out.push('}');
        }
        Parts::Index(c, keys, left) => {
            let _ = write!(out, "{{\"k\":\"Index\",\"c\":{c},\"keys\":");
            key_pairs(out, keys);
            let _ = write!(out, ",\"left\":{left}}}");
        }
        Parts::Trie(c, k) => {
            let _ = write!(out, "{{\"k\":\"Trie\",\"c\":{c},\"key\":{k}}}");
        }
        Parts::Radix(c, keys) => {
            let _ = write!(out, "{{\"k\":\"Radix\",\"c\":{c},\"keys\":");
            u16_list(out, keys);
            out.push('}');
        }
    }
}

fn parts_from(p: &Parsed) -> Result<Parts, SchemaDecodeError> {
    let tag = as_str(field(p, "k")?)?;
    Ok(match tag.as_str() {
        "Base" => Parts::Base,
        "DbRef" => Parts::DbRef,
        "Struct" => Parts::Struct(field_vec(field(p, "fields")?)?),
        "EnumValue" => Parts::EnumValue(as_u8(field(p, "disc")?)?, field_vec(field(p, "fields")?)?),
        "Enum" => {
            let mut values = Vec::new();
            for it in arr(field(p, "values")?)? {
                let pair = arr(it)?;
                if pair.len() != 2 {
                    return Err(SchemaDecodeError::Shape("expected [u16,name] pair".into()));
                }
                values.push((as_u16(&pair[0])?, as_str(&pair[1])?));
            }
            Parts::Enum(values)
        }
        "Byte" => Parts::Byte(as_i32(field(p, "n")?)?, as_bool(field(p, "b")?)?),
        "Short" => Parts::Short(as_i32(field(p, "n")?)?, as_bool(field(p, "b")?)?),
        "Int" => Parts::Int(as_i32(field(p, "n")?)?, as_bool(field(p, "b")?)?),
        "IntRaw" => Parts::IntRaw(as_i32(field(p, "n")?)?, as_bool(field(p, "b")?)?),
        "ShortRaw" => Parts::ShortRaw(as_i32(field(p, "n")?)?, as_bool(field(p, "b")?)?),
        "Vector" => Parts::Vector(as_u16(field(p, "c")?)?),
        "Array" => Parts::Array(as_u16(field(p, "c")?)?),
        "ChildRec" => Parts::ChildRec(as_u16(field(p, "c")?)?),
        "Sorted" => Parts::Sorted(as_u16(field(p, "c")?)?, key_pair_vec(field(p, "keys")?)?),
        "Ordered" => Parts::Ordered(as_u16(field(p, "c")?)?, key_pair_vec(field(p, "keys")?)?),
        "Hash" => Parts::Hash(as_u16(field(p, "c")?)?, u16_vec(field(p, "keys")?)?),
        "Index" => Parts::Index(
            as_u16(field(p, "c")?)?,
            key_pair_vec(field(p, "keys")?)?,
            as_u16(field(p, "left")?)?,
        ),
        "Trie" => Parts::Trie(as_u16(field(p, "c")?)?, as_u16(field(p, "key")?)?),
        "Radix" => Parts::Radix(as_u16(field(p, "c")?)?, u16_vec(field(p, "keys")?)?),
        other => return Err(SchemaDecodeError::UnknownTag(format!("Parts::{other}"))),
    })
}

// ─── Type ──────────────────────────────────────────────────────────────────

fn write_type(out: &mut String, t: &Type) {
    out.push_str("{\"name\":");
    esc(out, &t.name);
    out.push_str(",\"parts\":");
    write_parts(out, &t.parts);
    out.push_str(",\"keys\":[");
    for (i, k) in t.keys.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"type_nr\":{},\"position\":{},\"start\":{}}}",
            k.type_nr, k.position, k.start
        );
    }
    out.push_str("],\"parents\":");
    let parents: Vec<u16> = t.parents.iter().copied().collect();
    u16_list(out, &parents);
    let _ = write!(
        out,
        ",\"complex\":{},\"linked\":{},\"size\":{},\"align\":{},\"field_groups\":",
        t.complex, t.linked, t.size, t.align
    );
    write_field_groups(out, &t.field_groups);
    out.push('}');
}

fn type_from(p: &Parsed) -> Result<Type, SchemaDecodeError> {
    let keys: Vec<Key> = arr(field(p, "keys")?)?
        .iter()
        .map(|it| {
            Ok(Key {
                type_nr: as_i8(field(it, "type_nr")?)?,
                position: as_u16(field(it, "position")?)?,
                // loft#812 added `start`; a snapshot written before it cannot contain a
                // shifted key, so absent reads as 0 instead of failing the load.
                start: match field(it, "start") {
                    Ok(v) => as_i64(v)? as i32,
                    Err(_) => 0,
                },
            })
        })
        .collect::<Result<_, SchemaDecodeError>>()?;
    let parents: BTreeSet<u16> = u16_vec(field(p, "parents")?)?.into_iter().collect();
    let field_groups = field_groups_from(field(p, "field_groups")?)?;
    Ok(Type {
        name: as_str(field(p, "name")?)?,
        parts: parts_from(field(p, "parts")?)?,
        keys,
        parents,
        complex: as_bool(field(p, "complex")?)?,
        linked: as_bool(field(p, "linked")?)?,
        size: as_u16(field(p, "size")?)?,
        align: as_u8(field(p, "align")?)?,
        field_groups,
    })
}

// ─── Stores schema (the public entry points) ──────────────────────────────────

/// Serialise the database **schema** (`types` + `names`) to JSON.  Runtime
/// state (allocations, scratch, parallel buffers, …) is NOT part of the schema
/// and is rebuilt fresh on load.
#[must_use]
pub fn schema_to_json(stores: &Stores) -> String {
    let mut out = String::new();
    out.push_str("{\"types\":[");
    for (i, t) in stores.types.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_type(&mut out, t);
    }
    out.push_str("],\"names\":[");
    // `names` (name → type-index) is derivable from `types` order, but emit it
    // explicitly + sorted for a deterministic, self-checking document.
    let mut names: Vec<(&String, u16)> = stores.names.iter().map(|(k, &v)| (k, v)).collect();
    names.sort_by_key(|&(_, v)| v);
    for (i, (name, nr)) in names.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        esc(&mut out, name);
        let _ = write!(out, ",\"nr\":{nr}}}");
    }
    out.push_str("]}");
    out
}

/// Restore the database schema (`types` + `names`) from JSON into `stores`,
/// replacing whatever was there.  Does not touch runtime state.
///
/// # Errors
/// Malformed JSON, wrong field shape, or an unknown `Parts`/`Content` tag.
pub fn schema_from_json(stores: &mut Stores, src: &str) -> Result<(), SchemaDecodeError> {
    let parsed = crate::json::parse(src)
        .map_err(|e| SchemaDecodeError::Shape(format!("json parse: {e:?}")))?;
    let types: Vec<Type> = arr(field(&parsed, "types")?)?
        .iter()
        .map(type_from)
        .collect::<Result<_, SchemaDecodeError>>()?;
    let mut names = std::collections::HashMap::new();
    for it in arr(field(&parsed, "names")?)? {
        names.insert(as_str(field(it, "name")?)?, as_u16(field(it, "nr")?)?);
    }
    stores.types = types;
    stores.names = names;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    /// Round-trip the real stdlib's database schema: parse default/, encode the
    /// resulting `Stores` schema, decode into a fresh `Stores`, and assert the
    /// re-encode is byte-identical (the schema has no `PartialEq`).
    #[test]
    fn stdlib_schema_round_trips() {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        let json = schema_to_json(&p.database);
        assert!(
            json.len() > 1000,
            "expected a substantial schema, got {} bytes",
            json.len()
        );

        let mut restored = Stores::new();
        schema_from_json(&mut restored, &json).expect("schema decode");
        assert_eq!(
            schema_to_json(&restored),
            json,
            "schema re-encode drift after round-trip"
        );
        assert_eq!(
            restored.types.len(),
            p.database.types.len(),
            "type count changed across round-trip"
        );
        assert_eq!(
            restored.names.len(),
            p.database.names.len(),
            "names map size changed across round-trip"
        );
        crate::loft_eprintln!(
            "stdlib_schema_round_trips: {} types, {} names, {} bytes",
            restored.types.len(),
            restored.names.len(),
            json.len()
        );
    }

    #[test]
    fn unknown_parts_tag_is_an_error() {
        let mut s = Stores::new();
        let bad = r#"{"types":[{"name":"X","parts":{"k":"Bogus"},"keys":[],"parents":[],"complex":false,"linked":false,"size":0,"align":0,"field_groups":[]}],"names":[]}"#;
        match schema_from_json(&mut s, bad) {
            Err(SchemaDecodeError::UnknownTag(t)) => assert_eq!(t, "Parts::Bogus"),
            other => panic!("expected UnknownTag(Parts::Bogus), got {other:?}"),
        }
    }
}
