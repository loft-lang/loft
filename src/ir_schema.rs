// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I63 — Store-resident IR (reader / handle / materializer)

//! Interim native-IR ↔ JSON codec — @PLAN28 startup-cache bootstrap.
//!
//! A hand-written `Data` / `Definition` / `Value` / `Type` ↔ JSON round-trip
//! (`*_to_json` / `*_from_json`) plus [`compare_data`] (native-vs-native
//! equivalence) and [`load_snapshot`] (file load).  Decode reuses loft's own
//! JSON parser — never serde.  Absolute `def_nr` / `known_type` indices are
//! stored verbatim: this is the whole-bundle image (internally consistent),
//! not the deferred per-library form ([`DESIGN_DECISIONS.md` § C69]).
//!
//! ## Interim — slated for retirement (@PLN11)
//!
//! This codec walks the **native** Rust IR graph.  @PLN11 (Data-as-store)
//! moves the IR into `Stores` records, after which `Stores::show_json`
//! serialises it directly and this hand-rolled native walk is retired.  Until
//! then it earns its keep two ways: as the **traversal skeleton** @PLN11
//! arc B's native→store materializer mirrors, and as arc B's **equivalence
//! oracle** ([`compare_data`]).  See `plans/11-data-as-store`
//! § Standalone upside decision.  Nothing in the live compile path calls into
//! this module yet — only the `LOFT_DUMP_SNAPSHOT` debug hook in `main.rs`.
//!
//! [`DESIGN_DECISIONS.md` § C69]: ../../doc/claude/DESIGN_DECISIONS.md

use crate::data::{
    Attribute, Block, Data, DefType, Definition, ImpureCategory, IntegerSpec, LinkedFieldGroup,
    LinkedFieldKind, Purity, Type, Value,
};
use crate::json::Parsed;
use crate::keys::Key;
use crate::lexer::Position;
use std::fmt::Write as _;
use std::num::NonZeroU8;

// ─── Type JSON round-trip (Step 2 first slice — Value-free) ──────────────────
//
// `Type` is the Value-free half of the IR: every variant carries indices
// (`u32` def_nr / `u16` dep entries), `Box<Type>` / `Vec<Type>` recursion, or
// `(String, bool)` / `String` key lists — never a `Value`.  Encoding is a
// tagged object `{ "k": "<variant>", … }`; decode dispatches on the `"k"` tag.
// Indices are stored verbatim (whole-bundle image).

/// JSON-escape `s` into `out` as a quoted string (mirrors the database
/// generator's `write_json_escaped`; kept local for this Value-free slice).
fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
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

/// Encode a `Vec<u16>` dep list as a JSON array of numbers.
fn write_u16_list(out: &mut String, list: &[u16]) {
    out.push('[');
    for (i, n) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{n}");
    }
    out.push(']');
}

/// Encode a `Vec<(String, bool)>` sort-key list (`Sorted` / `Index`).
fn write_sort_keys(out: &mut String, keys: &[(String, bool)]) {
    out.push('[');
    for (i, (name, asc)) in keys.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        write_str(out, name);
        let _ = write!(out, ",\"asc\":{asc}}}");
    }
    out.push(']');
}

/// Encode a `Vec<String>` name list (`Radix` / `Hash`).
fn write_str_list(out: &mut String, names: &[String]) {
    out.push('[');
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_str(out, name);
    }
    out.push(']');
}

/// Encode an `IntegerSpec` inline (`forced` is 0 when `forced_size` is None).
fn write_integer_spec(out: &mut String, spec: &IntegerSpec) {
    let forced = spec.forced_size.map_or(0u8, NonZeroU8::get);
    let _ = write!(
        out,
        "{{\"min\":{},\"max\":{},\"not_null\":{},\"forced\":{}}}",
        spec.min, spec.max, spec.not_null, forced
    );
}

/// Serialise a `Type` to a JSON string.  Value-free; recurses through
/// `Box<Type>` / `Vec<Type>`.
#[must_use]
pub fn type_to_json(ty: &Type) -> String {
    let mut out = String::new();
    write_type(&mut out, ty);
    out
}

fn write_type(out: &mut String, ty: &Type) {
    match ty {
        // @PLN25 — `Optional(τ)` shares its base's LAYOUT but carries the nullability that
        // DN1 type-checking depends on, so the schema PRESERVES it (a peel would round-trip
        // `integer?` back to a non-null `integer`). The `inner` is never itself `Optional`
        // (`Type::optional` is idempotent, N-Idem).
        Type::Optional(inner) => {
            out.push_str("{\"k\":\"Optional\",\"inner\":");
            write_type(out, inner);
            out.push('}');
        }
        Type::Unknown(n) => {
            let _ = write!(out, "{{\"k\":\"Unknown\",\"n\":{n}}}");
        }
        Type::Null => out.push_str("{\"k\":\"Null\"}"),
        Type::Void => out.push_str("{\"k\":\"Void\"}"),
        Type::Never => out.push_str("{\"k\":\"Never\"}"),
        Type::Boolean => out.push_str("{\"k\":\"Boolean\"}"),
        Type::Float => out.push_str("{\"k\":\"Float\"}"),
        Type::Single => out.push_str("{\"k\":\"Single\"}"),
        Type::Character => out.push_str("{\"k\":\"Character\"}"),
        Type::Keys => out.push_str("{\"k\":\"Keys\"}"),
        Type::Integer(spec) => {
            out.push_str("{\"k\":\"Integer\",\"spec\":");
            write_integer_spec(out, spec);
            out.push('}');
        }
        Type::Text(dep) => {
            out.push_str("{\"k\":\"Text\",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Enum(n, is_ref, dep) => {
            let _ = write!(out, "{{\"k\":\"Enum\",\"n\":{n},\"ref\":{is_ref},\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Reference(n, dep) => {
            let _ = write!(out, "{{\"k\":\"Reference\",\"n\":{n},\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::RefVar(inner) => {
            out.push_str("{\"k\":\"RefVar\",\"t\":");
            write_type(out, inner);
            out.push('}');
        }
        Type::Vector(inner, dep) => {
            out.push_str("{\"k\":\"Vector\",\"t\":");
            write_type(out, inner);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Routine(n) => {
            let _ = write!(out, "{{\"k\":\"Routine\",\"n\":{n}}}");
        }
        Type::Iterator(step, inner) => {
            out.push_str("{\"k\":\"Iterator\",\"step\":");
            write_type(out, step);
            out.push_str(",\"inner\":");
            write_type(out, inner);
            out.push('}');
        }
        Type::Sorted(n, keys, dep) => {
            let _ = write!(out, "{{\"k\":\"Sorted\",\"n\":{n},\"keys\":");
            write_sort_keys(out, keys);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Index(n, keys, dep) => {
            let _ = write!(out, "{{\"k\":\"Index\",\"n\":{n},\"keys\":");
            write_sort_keys(out, keys);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Trie(n, key, dep) => {
            let _ = write!(out, "{{\"k\":\"Trie\",\"n\":{n},\"key\":");
            write_str(out, key);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Radix(n, names, dep) => {
            let _ = write!(out, "{{\"k\":\"Radix\",\"n\":{n},\"names\":");
            write_str_list(out, names);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Hash(n, names, dep) => {
            let _ = write!(out, "{{\"k\":\"Hash\",\"n\":{n},\"names\":");
            write_str_list(out, names);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Function(args, result, dep) => {
            out.push_str("{\"k\":\"Function\",\"args\":[");
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_type(out, a);
            }
            out.push_str("],\"result\":");
            write_type(out, result);
            out.push_str(",\"dep\":");
            write_u16_list(out, dep);
            out.push('}');
        }
        Type::Rewritten(inner) => {
            out.push_str("{\"k\":\"Rewritten\",\"t\":");
            write_type(out, inner);
            out.push('}');
        }
        Type::Tuple(elems) => {
            out.push_str("{\"k\":\"Tuple\",\"elems\":[");
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_type(out, e);
            }
            out.push_str("]}");
        }
    }
}

/// Errors from decoding a `Type` / `Value` out of a `Parsed` tree.
#[derive(Debug, PartialEq, Eq)]
pub enum TypeDecodeError {
    /// Top-level JSON did not parse, or a sub-node had the wrong JSON shape.
    Shape(String),
    /// The `"k"` tag named a variant we don't know.
    UnknownTag(String),
}

/// Parse a JSON string (via `crate::json::parse`) back into a `Type`.
///
/// # Errors
/// Returns [`TypeDecodeError`] when the JSON is malformed, a field is missing
/// or has the wrong shape, or the `"k"` tag is unrecognised.
pub fn type_from_json(src: &str) -> Result<Type, TypeDecodeError> {
    let parsed = crate::json::parse(src)
        .map_err(|e| TypeDecodeError::Shape(format!("json parse: {e:?}")))?;
    type_from_parsed(&parsed)
}

/// Look up a field in a `Parsed::Object` by name.
fn field<'a>(obj: &'a Parsed, name: &str) -> Result<&'a Parsed, TypeDecodeError> {
    if let Parsed::Object(entries) = obj {
        entries
            .iter()
            .find(|(k, _, _)| k == name)
            .map(|(_, _, v)| v)
            .ok_or_else(|| TypeDecodeError::Shape(format!("missing field '{name}'")))
    } else {
        Err(TypeDecodeError::Shape(format!(
            "expected object for '{name}'"
        )))
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn as_u32(p: &Parsed) -> Result<u32, TypeDecodeError> {
    // @PLN109 — `as_i64` accepts an integer-shaped `Parsed::Int` (exact) or a
    // legacy `Number` (truncated).
    p.as_i64()
        .map(|n| n as u32)
        .ok_or_else(|| TypeDecodeError::Shape("expected number".into()))
}

/// Read a signed `i32` (e.g. `IntegerSpec.min`, which can be negative —
/// `i32::MIN + 1` for plain-integer templates).  `as_u32` would clamp a
/// negative `f64` to 0, so decode the float directly.
#[allow(clippy::cast_possible_truncation)]
fn as_i32(p: &Parsed) -> Result<i32, TypeDecodeError> {
    p.as_i64()
        .map(|n| n as i32)
        .ok_or_else(|| TypeDecodeError::Shape("expected number".into()))
}

fn as_u16(p: &Parsed) -> Result<u16, TypeDecodeError> {
    Ok(as_u32(p)? as u16)
}

fn as_bool(p: &Parsed) -> Result<bool, TypeDecodeError> {
    if let Parsed::Bool(b) = p {
        Ok(*b)
    } else {
        Err(TypeDecodeError::Shape("expected bool".into()))
    }
}

fn as_str(p: &Parsed) -> Result<String, TypeDecodeError> {
    match p {
        Parsed::Str(s) | Parsed::Ident(s) => Ok(s.clone()),
        _ => Err(TypeDecodeError::Shape("expected string".into())),
    }
}

/// Type-dep wrapper: tagged space-`Unknown` (the JSON snapshot does not
/// record the address space — H2 DEPS_INVENTORY).
fn deps(p: &Parsed) -> Result<crate::data::Deps, TypeDecodeError> {
    dep_list(p).map(crate::data::Deps::unknown)
}

fn dep_list(p: &Parsed) -> Result<Vec<u16>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items.iter().map(as_u16).collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

fn sort_keys(p: &Parsed) -> Result<Vec<(String, bool)>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items
            .iter()
            .map(|it| Ok((as_str(field(it, "name")?)?, as_bool(field(it, "asc")?)?)))
            .collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

fn str_list(p: &Parsed) -> Result<Vec<String>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items.iter().map(as_str).collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

fn type_list(p: &Parsed) -> Result<Vec<Type>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items.iter().map(type_from_parsed).collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

fn integer_spec(p: &Parsed) -> Result<IntegerSpec, TypeDecodeError> {
    let min = as_i32(field(p, "min")?)?;
    let max = as_u32(field(p, "max")?)?;
    let not_null = as_bool(field(p, "not_null")?)?;
    let forced = as_u32(field(p, "forced")?)? as u8;
    Ok(IntegerSpec {
        min,
        max,
        not_null,
        forced_size: NonZeroU8::new(forced),
    })
}

fn type_from_parsed(p: &Parsed) -> Result<Type, TypeDecodeError> {
    let tag = as_str(field(p, "k")?)?;
    Ok(match tag.as_str() {
        "Unknown" => Type::Unknown(as_u32(field(p, "n")?)?),
        "Null" => Type::Null,
        "Void" => Type::Void,
        "Never" => Type::Never,
        "Boolean" => Type::Boolean,
        "Float" => Type::Float,
        "Single" => Type::Single,
        "Character" => Type::Character,
        "Keys" => Type::Keys,
        "Integer" => Type::Integer(integer_spec(field(p, "spec")?)?),
        "Text" => Type::Text(deps(field(p, "dep")?)?),
        "Enum" => Type::Enum(
            as_u32(field(p, "n")?)?,
            as_bool(field(p, "ref")?)?,
            deps(field(p, "dep")?)?,
        ),
        "Reference" => Type::Reference(as_u32(field(p, "n")?)?, deps(field(p, "dep")?)?),
        "RefVar" => Type::RefVar(Box::new(type_from_parsed(field(p, "t")?)?)),
        // @PLN25 — preserve the nullable marker (write side pairs it under "inner").
        "Optional" => Type::Optional(Box::new(type_from_parsed(field(p, "inner")?)?)),
        "Vector" => Type::Vector(
            Box::new(type_from_parsed(field(p, "t")?)?),
            deps(field(p, "dep")?)?,
        ),
        "Routine" => Type::Routine(as_u32(field(p, "n")?)?),
        "Iterator" => Type::Iterator(
            Box::new(type_from_parsed(field(p, "step")?)?),
            Box::new(type_from_parsed(field(p, "inner")?)?),
        ),
        "Sorted" => Type::Sorted(
            as_u32(field(p, "n")?)?,
            sort_keys(field(p, "keys")?)?,
            deps(field(p, "dep")?)?,
        ),
        "Index" => Type::Index(
            as_u32(field(p, "n")?)?,
            sort_keys(field(p, "keys")?)?,
            deps(field(p, "dep")?)?,
        ),
        "Radix" => Type::Radix(
            as_u32(field(p, "n")?)?,
            str_list(field(p, "names")?)?,
            deps(field(p, "dep")?)?,
        ),
        "Trie" => Type::Trie(
            as_u32(field(p, "n")?)?,
            as_str(field(p, "key")?)?,
            deps(field(p, "dep")?)?,
        ),
        "Hash" => Type::Hash(
            as_u32(field(p, "n")?)?,
            str_list(field(p, "names")?)?,
            deps(field(p, "dep")?)?,
        ),
        "Function" => Type::Function(
            type_list(field(p, "args")?)?,
            Box::new(type_from_parsed(field(p, "result")?)?),
            deps(field(p, "dep")?)?,
        ),
        "Rewritten" => Type::Rewritten(Box::new(type_from_parsed(field(p, "t")?)?)),
        "Tuple" => Type::Tuple(type_list(field(p, "elems")?)?),
        other => return Err(TypeDecodeError::UnknownTag(other.to_string())),
    })
}

// ─── Value JSON (Step 2: full encoder + decoder for all 32 variants) ──────────
//
// `Value` is the recursive IR-body enum.  The encoder is exhaustive (a `match`
// cannot be partial); the decoder dispatches on the `"k"` tag and covers every
// variant.  It was built in reviewable slices (V1 leaf → V2 recursive payloads
// → V3 Block/Loop/Span → V4 FnRef), now all landed.
//
// Same conventions as `Type`: tagged object `{ "k": "<variant>", … }`,
// absolute indices verbatim, strings via `write_str`.

/// Serialise a `Value` to JSON.  Exhaustive over all 32 variants.
#[must_use]
pub fn value_to_json(v: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, v);
    out
}

fn write_value(out: &mut String, v: &Value) {
    match v {
        Value::Null => out.push_str("{\"k\":\"Null\"}"),
        Value::Line(n) => {
            let _ = write!(out, "{{\"k\":\"Line\",\"n\":{n}}}");
        }
        Value::Int(n) => {
            let _ = write!(out, "{{\"k\":\"Int\",\"n\":{n}}}");
        }
        Value::Long(n) => {
            // i64 as a STRING, not a JSON number: a JSON number decodes through
            // f64 (53-bit mantissa) and silently loses precision beyond 2^53 — a
            // `Long` of 2^53+1 came back as 2^53 (caught by the tests/scripts IR
            // round-trip).  A quoted integer preserves every bit.
            let _ = write!(out, "{{\"k\":\"Long\",\"n\":\"{n}\"}}");
        }
        Value::Float(f) => {
            let _ = write!(out, "{{\"k\":\"Float\",\"f\":{f}}}");
        }
        Value::Single(f) => {
            let _ = write!(out, "{{\"k\":\"Single\",\"f\":{f}}}");
        }
        Value::Boolean(b) => {
            let _ = write!(out, "{{\"k\":\"Boolean\",\"b\":{b}}}");
        }
        Value::Enum(ord, tp) => {
            let _ = write!(out, "{{\"k\":\"Enum\",\"ord\":{ord},\"tp\":{tp}}}");
        }
        Value::Text(s) => {
            out.push_str("{\"k\":\"Text\",\"s\":");
            write_str(out, s);
            out.push('}');
        }
        Value::RawExpr(s) => {
            out.push_str("{\"k\":\"RawExpr\",\"s\":");
            write_str(out, s);
            out.push('}');
        }
        Value::Var(n) => {
            let _ = write!(out, "{{\"k\":\"Var\",\"n\":{n}}}");
        }
        Value::Break(n) => {
            let _ = write!(out, "{{\"k\":\"Break\",\"n\":{n}}}");
        }
        Value::Continue(n) => {
            let _ = write!(out, "{{\"k\":\"Continue\",\"n\":{n}}}");
        }
        Value::FnRefDnr(n) => {
            let _ = write!(out, "{{\"k\":\"FnRefDnr\",\"n\":{n}}}");
        }
        Value::TupleGet(var, idx) => {
            let _ = write!(out, "{{\"k\":\"TupleGet\",\"var\":{var},\"idx\":{idx}}}");
        }
        Value::Keys(keys) => {
            out.push_str("{\"k\":\"Keys\",\"keys\":");
            write_key_list(out, keys);
            out.push('}');
        }
        // ── recursive variants (decoded in V2+) ──
        Value::Call(d, args) => {
            let _ = write!(out, "{{\"k\":\"Call\",\"d\":{d},\"args\":");
            write_value_list(out, args);
            out.push('}');
        }
        Value::CallRef(var, args) => {
            let _ = write!(out, "{{\"k\":\"CallRef\",\"var\":{var},\"args\":");
            write_value_list(out, args);
            out.push('}');
        }
        Value::Insert(items) => {
            out.push_str("{\"k\":\"Insert\",\"items\":");
            write_value_list(out, items);
            out.push('}');
        }
        Value::Tuple(items) => {
            out.push_str("{\"k\":\"Tuple\",\"items\":");
            write_value_list(out, items);
            out.push('}');
        }
        Value::Parallel(arms) => {
            out.push_str("{\"k\":\"Parallel\",\"arms\":");
            write_value_list(out, arms);
            out.push('}');
        }
        Value::Set(var, inner) => {
            let _ = write!(out, "{{\"k\":\"Set\",\"var\":{var},\"v\":");
            write_value(out, inner);
            out.push('}');
        }
        Value::Return(inner) => {
            out.push_str("{\"k\":\"Return\",\"v\":");
            write_value(out, inner);
            out.push('}');
        }
        Value::Drop(inner) => {
            out.push_str("{\"k\":\"Drop\",\"v\":");
            write_value(out, inner);
            out.push('}');
        }
        Value::Yield(inner) => {
            out.push_str("{\"k\":\"Yield\",\"v\":");
            write_value(out, inner);
            out.push('}');
        }
        Value::TuplePut(var, idx, inner) => {
            let _ = write!(
                out,
                "{{\"k\":\"TuplePut\",\"var\":{var},\"idx\":{idx},\"v\":"
            );
            write_value(out, inner);
            out.push('}');
        }
        Value::If(cond, t, f) => {
            out.push_str("{\"k\":\"If\",\"cond\":");
            write_value(out, cond);
            out.push_str(",\"t\":");
            write_value(out, t);
            out.push_str(",\"f\":");
            write_value(out, f);
            out.push('}');
        }
        Value::Iter(var, create, next, init) => {
            let _ = write!(out, "{{\"k\":\"Iter\",\"var\":{var},\"create\":");
            write_value(out, create);
            out.push_str(",\"next\":");
            write_value(out, next);
            out.push_str(",\"init\":");
            write_value(out, init);
            out.push('}');
        }
        Value::Span(boxed) => {
            let (pos, inner) = boxed.as_ref();
            out.push_str("{\"k\":\"Span\",\"pos\":");
            write_position(out, pos);
            out.push_str(",\"v\":");
            write_value(out, inner);
            out.push('}');
        }
        Value::Block(b) => {
            out.push_str("{\"k\":\"Block\",\"block\":");
            write_block(out, b);
            out.push('}');
        }
        Value::Loop(b) => {
            out.push_str("{\"k\":\"Loop\",\"block\":");
            write_block(out, b);
            out.push('}');
        }
        Value::FnRef(d, var, ty) => {
            let _ = write!(out, "{{\"k\":\"FnRef\",\"d\":{d},\"var\":{var},\"t\":");
            write_type(out, ty);
            out.push('}');
        }
    }
}

fn write_value_list(out: &mut String, list: &[Value]) {
    out.push('[');
    for (i, v) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_value(out, v);
    }
    out.push(']');
}

fn write_key_list(out: &mut String, keys: &[Key]) {
    out.push('[');
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"type_nr\":{},\"position\":{},\"start\":{}}}",
            k.type_nr, k.position, k.start
        );
    }
    out.push(']');
}

fn write_position(out: &mut String, pos: &Position) {
    out.push_str("{\"file\":");
    write_str(out, &pos.file);
    let _ = write!(out, ",\"line\":{},\"pos\":{}}}", pos.line, pos.pos);
}

fn write_block(out: &mut String, b: &Block) {
    out.push_str("{\"name\":");
    write_str(out, b.name);
    out.push_str(",\"operators\":");
    write_value_list(out, &b.operators);
    out.push_str(",\"result\":");
    write_type(out, &b.result);
    let _ = write!(out, ",\"scope\":{},\"var_size\":{}}}", b.scope, b.var_size);
}

/// Parse a JSON string into a `Value`.  Decodes all 32 variants: leaf (V1),
/// recursive `Value`/`Vec<Value>`/scalar payloads (V2), `Block`/`Loop`/`Span`
/// (V3), and `FnRef` (V4 — slice complete).
///
/// # Errors
/// Malformed JSON, wrong field shape, or an unknown variant tag.
pub fn value_from_json(src: &str) -> Result<Value, TypeDecodeError> {
    let parsed = crate::json::parse(src)
        .map_err(|e| TypeDecodeError::Shape(format!("json parse: {e:?}")))?;
    value_from_parsed(&parsed)
}

fn as_i64(p: &Parsed) -> Result<i64, TypeDecodeError> {
    match p {
        // i64 is now encoded as a quoted string to survive >2^53 (see the
        // `Value::Long` encoder).  Accept the legacy bare-number form too, so
        // snapshots written before this change still decode (small i64 fields
        // like `type_nr` also reach here as bare numbers and stay exact).
        Parsed::Str(s) => s
            .parse::<i64>()
            .map_err(|_| TypeDecodeError::Shape("expected an i64 string".into())),
        // @PLN109 — an integer-shaped bare number now decodes exactly via `Int`.
        Parsed::Int(n) => Ok(*n),
        Parsed::Number(n) => Ok(*n as i64),
        _ => Err(TypeDecodeError::Shape("expected number".into())),
    }
}

fn as_f64(p: &Parsed) -> Result<f64, TypeDecodeError> {
    p.as_f64()
        .ok_or_else(|| TypeDecodeError::Shape("expected number".into()))
}

fn as_u8(p: &Parsed) -> Result<u8, TypeDecodeError> {
    Ok(as_u32(p)? as u8)
}

fn key_list(p: &Parsed) -> Result<Vec<Key>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items
            .iter()
            .map(|it| {
                Ok(Key {
                    type_nr: as_i64(field(it, "type_nr")?)? as i8,
                    position: as_u16(field(it, "position")?)?,
                    // loft#812 added `start`; a document written before it simply has
                    // no shifted key, so absent means 0 rather than malformed.
                    start: match field(it, "start") {
                        Ok(v) => as_i64(v)? as i32,
                        Err(_) => 0,
                    },
                })
            })
            .collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

/// Decode a JSON array of `Value` nodes (V2 — the recursive payload helper).
fn value_list(p: &Parsed) -> Result<Vec<Value>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items.iter().map(value_from_parsed).collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

/// Decode the `Value` at object field `name`, boxed (V2 convenience).
fn boxed(p: &Parsed, name: &str) -> Result<Box<Value>, TypeDecodeError> {
    Ok(Box::new(value_from_parsed(field(p, name)?)?))
}

/// Decode a [`Block`] (V3).  `Block.name` is a `&'static str` — at parse time
/// it is always a string literal, so on load from a snapshot we materialise
/// one via [`Box::leak`].  This is a *deliberate, bounded* leak: a loaded
/// image holds a small fixed set of distinct block names, and those names
/// live for the whole process anyway (the cache loads once at startup).
fn block_from_parsed(p: &Parsed) -> Result<Block, TypeDecodeError> {
    let name: &'static str = Box::leak(as_str(field(p, "name")?)?.into_boxed_str());
    Ok(Block {
        name,
        operators: value_list(field(p, "operators")?)?,
        result: type_from_parsed(field(p, "result")?)?,
        scope: as_u16(field(p, "scope")?)?,
        var_size: as_u16(field(p, "var_size")?)?,
    })
}

/// Decode a [`Position`] (V3 — carried by `Value::Span`).
fn position_from_parsed(p: &Parsed) -> Result<Position, TypeDecodeError> {
    Ok(Position {
        file: as_str(field(p, "file")?)?,
        line: as_u32(field(p, "line")?)?,
        pos: as_u32(field(p, "pos")?)?,
    })
}

fn value_from_parsed(p: &Parsed) -> Result<Value, TypeDecodeError> {
    let tag = as_str(field(p, "k")?)?;
    Ok(match tag.as_str() {
        // ── leaf variants (V1) ──
        "Null" => Value::Null,
        "Line" => Value::Line(as_u32(field(p, "n")?)?),
        "Int" => Value::Int(as_i32(field(p, "n")?)?),
        "Long" => Value::Long(as_i64(field(p, "n")?)?),
        "Float" => Value::Float(as_f64(field(p, "f")?)?),
        "Single" => Value::Single(as_f64(field(p, "f")?)? as f32),
        "Boolean" => Value::Boolean(as_bool(field(p, "b")?)?),
        "Enum" => Value::Enum(as_u8(field(p, "ord")?)?, as_u16(field(p, "tp")?)?),
        "Text" => Value::Text(as_str(field(p, "s")?)?),
        "RawExpr" => Value::RawExpr(as_str(field(p, "s")?)?),
        "Var" => Value::Var(as_u16(field(p, "n")?)?),
        "Break" => Value::Break(as_u16(field(p, "n")?)?),
        "Continue" => Value::Continue(as_u16(field(p, "n")?)?),
        "FnRefDnr" => Value::FnRefDnr(as_u16(field(p, "n")?)?),
        "TupleGet" => Value::TupleGet(as_u16(field(p, "var")?)?, as_u16(field(p, "idx")?)?),
        "Keys" => Value::Keys(key_list(field(p, "keys")?)?),
        // ── recursive variants (V2): Value / Vec<Value> / scalar payloads ──
        "Call" => Value::Call(as_u32(field(p, "d")?)?, value_list(field(p, "args")?)?),
        "CallRef" => Value::CallRef(as_u16(field(p, "var")?)?, value_list(field(p, "args")?)?),
        "Insert" => Value::Insert(value_list(field(p, "items")?)?),
        "Tuple" => Value::Tuple(value_list(field(p, "items")?)?),
        "Parallel" => Value::Parallel(value_list(field(p, "arms")?)?),
        "Set" => Value::Set(as_u16(field(p, "var")?)?, boxed(p, "v")?),
        "Return" => Value::Return(boxed(p, "v")?),
        "Drop" => Value::Drop(boxed(p, "v")?),
        "Yield" => Value::Yield(boxed(p, "v")?),
        "TuplePut" => Value::TuplePut(
            as_u16(field(p, "var")?)?,
            as_u16(field(p, "idx")?)?,
            boxed(p, "v")?,
        ),
        "If" => Value::If(boxed(p, "cond")?, boxed(p, "t")?, boxed(p, "f")?),
        "Iter" => Value::Iter(
            as_u16(field(p, "var")?)?,
            boxed(p, "create")?,
            boxed(p, "next")?,
            boxed(p, "init")?,
        ),
        // ── V3: Block / Loop (&'static str name) + Span (Position) ──
        "Block" => Value::Block(Box::new(block_from_parsed(field(p, "block")?)?)),
        "Loop" => Value::Loop(Box::new(block_from_parsed(field(p, "block")?)?)),
        "Span" => Value::Span(Box::new((
            position_from_parsed(field(p, "pos")?)?,
            value_from_parsed(field(p, "v")?)?,
        ))),
        "FnRef" => Value::FnRef(
            as_i32(field(p, "d")?)?,
            as_u16(field(p, "var")?)?,
            Box::new(type_from_parsed(field(p, "t")?)?),
        ),
        other => return Err(TypeDecodeError::UnknownTag(other.to_string())),
    })
}

// ─── Attribute JSON (Step 2 container slice C1) ───────────────────────────────
//
// `Attribute` is a struct field / parameter descriptor on a `Definition`.  It
// is a plain data bag (no cross-field invariant), so the codec is a direct
// field walk: scalars + `Type` (typedef) + three `Value` fields (value / check
// / check_message).  The `primary` field is `pub(crate)` (widened seam in
// data.rs) so this out-of-module codec can read and reconstruct it.
//
// Same conventions as `Type`/`Value`: a flat JSON object; `Type` via
// write_type/type_from_parsed, `Value` via write_value/value_from_parsed.

/// Serialise an [`Attribute`] to JSON.
#[must_use]
pub fn attribute_to_json(a: &Attribute) -> String {
    let mut out = String::new();
    write_attribute(&mut out, a);
    out
}

fn write_attribute(out: &mut String, a: &Attribute) {
    out.push_str("{\"name\":");
    write_str(out, &a.name);
    out.push_str(",\"typedef\":");
    write_type(out, &a.typedef);
    let _ = write!(
        out,
        ",\"mutable\":{},\"constant\":{},\"init\":{},\"nullable\":{},\"primary\":{},\"hidden\":{},\"const_field\":{}",
        a.mutable, a.constant, a.init, a.nullable, a.primary, a.hidden, a.const_field
    );
    out.push_str(",\"value\":");
    write_value(out, &a.value);
    out.push_str(",\"check\":");
    write_value(out, &a.check);
    out.push_str(",\"check_message\":");
    write_value(out, &a.check_message);
    let _ = write!(
        out,
        ",\"alias_d_nr\":{},\"assigned_lambda_d_nr\":{}",
        a.alias_d_nr, a.assigned_lambda_d_nr
    );
    out.push_str(",\"links\":");
    write_str(out, &a.links.join(" "));
    out.push('}');
}

/// Parse a JSON string into an [`Attribute`].
///
/// # Errors
/// Malformed JSON, wrong field shape, or an unknown variant tag in a nested
/// `Type` / `Value`.
pub fn attribute_from_json(src: &str) -> Result<Attribute, TypeDecodeError> {
    let parsed = crate::json::parse(src)
        .map_err(|e| TypeDecodeError::Shape(format!("json parse: {e:?}")))?;
    attribute_from_parsed(&parsed)
}

fn attribute_from_parsed(p: &Parsed) -> Result<Attribute, TypeDecodeError> {
    Ok(Attribute {
        name: as_str(field(p, "name")?)?,
        typedef: type_from_parsed(field(p, "typedef")?)?,
        mutable: as_bool(field(p, "mutable")?)?,
        constant: as_bool(field(p, "constant")?)?,
        // Tolerant of older JSON written before this field existed (defaults false),
        // mirroring the `links` handling below.
        const_field: match field(p, "const_field") {
            Ok(f) => as_bool(f)?,
            Err(_) => false,
        },
        // @PLN40 Phase 2 — value-const flag; serialization deferred to Phase 2b,
        // so tolerate JSON written without it (defaults false), like const_field.
        value_const: match field(p, "value_const") {
            Ok(f) => as_bool(f)?,
            Err(_) => false,
        },
        init: as_bool(field(p, "init")?)?,
        nullable: as_bool(field(p, "nullable")?)?,
        primary: as_bool(field(p, "primary")?)?,
        hidden: as_bool(field(p, "hidden")?)?,
        value: value_from_parsed(field(p, "value")?)?,
        check: value_from_parsed(field(p, "check")?)?,
        check_message: value_from_parsed(field(p, "check_message")?)?,
        alias_d_nr: as_u32(field(p, "alias_d_nr")?)?,
        assigned_lambda_d_nr: as_u32(field(p, "assigned_lambda_d_nr")?)?,
        // @PLN86 F8b — group#right member links; tolerant of older JSON without the field.
        links: match field(p, "links") {
            Ok(f) => as_str(f)?.split_whitespace().map(str::to_string).collect(),
            Err(_) => Vec::new(),
        },
        // @PLN35 — `#lexeme` is a parse-time-only marker, not stored; defaults to false.
        lexeme: false,
    })
}

// ─── Definition JSON (Step 2 container slice C2) ──────────────────────────────
//
// `Definition` is a top-level named entity (a type / struct / enum / function /
// constant).  This slice serialises the **portable IR** of a definition and
// recomputes everything codegen-derived on load (per the recompute-slots
// decision in STARTUP_CACHE_PLAN.md):
//
//   stored  : name, source, def_type, parent, position, attributes, code,
//             returned[_not_null], rust, native, op_code, known_type,
//             pub_visible, closure_record, mutated_captures, scalars_to_box,
//             bounds, forced_size, purity, field_groups, synthetic
//   derived : attr_names (rebuilt from attributes on load)
//   omitted : code_position / code_length / const_ref (codegen output → 0/None;
//             byte_code_from recomputes them)
//
// **Scope (C2): type-level definitions.**  `Definition.variables` (the
// per-function `Function` debug-symbol table) is reconstructed *empty* via
// `Function::new(name, file)` — correct for type / struct / enum / constant
// defs, whose variable table is empty.  Function *bodies* carry a populated
// variable table (debug symbols, kept per the user decision); serialising that
// table is the **next slice** (C-functions).  A round-trip of a function def
// through C2 would drop its variable table — tests scope to type-level defs.
//
// `Definition` derives only `Clone` (no `PartialEq`), so round-trip tests use
// re-encode equality (encode → decode → encode is stable), as for `Attribute`.

/// Serialise a [`Definition`] to JSON (portable IR; see module note on scope).
#[must_use]
pub fn definition_to_json(d: &Definition) -> String {
    let mut out = String::new();
    write_definition(&mut out, d);
    out
}

fn def_type_str(t: &DefType) -> &'static str {
    match t {
        DefType::Unknown => "Unknown",
        DefType::Function => "Function",
        DefType::Dynamic => "Dynamic",
        DefType::Enum => "Enum",
        DefType::EnumValue => "EnumValue",
        DefType::Struct => "Struct",
        DefType::Vector => "Vector",
        DefType::Type => "Type",
        DefType::Constant => "Constant",
        DefType::Generic => "Generic",
        DefType::Interface => "Interface",
    }
}

fn impure_category_str(c: ImpureCategory) -> &'static str {
    match c {
        ImpureCategory::HostIo => "HostIo",
        ImpureCategory::Prng => "Prng",
        ImpureCategory::Io => "Io",
        ImpureCategory::ParentWrite => "ParentWrite",
        ImpureCategory::ParCall => "ParCall",
    }
}

fn write_u32_list(out: &mut String, list: &[u32]) {
    out.push('[');
    for (i, n) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{n}");
    }
    out.push(']');
}

fn write_purity(out: &mut String, p: Purity) {
    match p {
        Purity::Unknown => out.push_str("{\"k\":\"Unknown\"}"),
        Purity::Pure => out.push_str("{\"k\":\"Pure\"}"),
        Purity::Impure(c) => {
            let _ = write!(
                out,
                "{{\"k\":\"Impure\",\"cat\":\"{}\"}}",
                impure_category_str(c)
            );
        }
    }
}

fn write_linked_field_group(out: &mut String, g: &LinkedFieldGroup) {
    let kind = match g.kind {
        LinkedFieldKind::Tuple => "Tuple",
        LinkedFieldKind::Index => "Index",
    };
    let _ = write!(out, "{{\"kind\":\"{kind}\",\"instance\":{}", g.instance);
    out.push_str(",\"field_indices\":");
    write_u16_list(out, &g.field_indices);
    let _ = write!(out, ",\"alignment\":{},\"size\":{}}}", g.alignment, g.size);
}

fn write_attribute_list(out: &mut String, list: &[Attribute]) {
    out.push('[');
    for (i, a) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_attribute(out, a);
    }
    out.push(']');
}

fn write_field_group_list(out: &mut String, list: &[LinkedFieldGroup]) {
    out.push('[');
    for (i, g) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_linked_field_group(out, g);
    }
    out.push(']');
}

fn write_definition(out: &mut String, d: &Definition) {
    out.push_str("{\"name\":");
    write_str(out, &d.name);
    let _ = write!(
        out,
        ",\"source\":{},\"def_type\":\"{}\"",
        d.source,
        def_type_str(&d.def_type)
    );
    let _ = write!(out, ",\"parent\":{}", d.parent);
    out.push_str(",\"position\":");
    write_position(out, &d.position);
    out.push_str(",\"attributes\":");
    write_attribute_list(out, &d.attributes);
    out.push_str(",\"code\":");
    write_value(out, &d.code);
    out.push_str(",\"returned\":");
    write_type(out, &d.returned);
    let _ = write!(out, ",\"returned_not_null\":{}", d.returned_not_null);
    out.push_str(",\"rust\":");
    write_str(out, &d.rust);
    out.push_str(",\"native\":");
    write_str(out, &d.native);
    out.push_str(",\"cap\":"); // @PLN86
    write_str(out, &d.cap);
    out.push_str(",\"superseded\":"); // @PLN102 arc C
    write_str(out, &d.superseded);
    out.push_str(",\"c_symbol\":"); // @PLN24 arc A
    write_str(out, &d.c_symbol);
    out.push_str(",\"c_sig\":");
    write_str(out, &d.c_sig);
    let _ = write!(
        out,
        ",\"op_code\":{},\"known_type\":{},\"pub_visible\":{},\"null_safe\":{},\"closure_record\":{}",
        d.op_code, d.known_type, d.pub_visible, d.null_safe, d.closure_record
    );
    out.push_str(",\"mutated_captures\":");
    write_str_list(out, &d.mutated_captures);
    out.push_str(",\"scalars_to_box\":");
    write_str_list(out, &d.scalars_to_box);
    out.push_str(",\"bounds\":");
    write_u32_list(out, &d.bounds);
    // forced_size: Option<u8>, n ∈ {1,2,4,8}; 0 is never valid → encodes None.
    let _ = write!(out, ",\"forced_size\":{}", d.forced_size.unwrap_or(0));
    out.push_str(",\"purity\":");
    write_purity(out, d.purity);
    out.push_str(",\"field_groups\":");
    write_field_group_list(out, &d.field_groups);
    // synthetic: Option<&'static str>; reasons are non-empty idents → "" = None.
    out.push_str(",\"synthetic\":");
    write_str(out, d.synthetic.unwrap_or(""));
    // variables (C4): the per-function variable table as DEBUG SYMBOLS + the
    // final stack_pos.  Only the fields codegen reads are stored (see the
    // snapshot seam in src/variables/mod.rs).  Empty for type-level defs.
    out.push_str(",\"variables\":");
    write_variables(out, &d.variables);
    out.push('}');
}

/// Encode the per-function variable table (C4): the codegen-read fields of
/// each `Variable`, plus the `names` lookup map and `inline_ref_vars` set.
/// The function's own `name`/`file` are NOT stored — they equal the owning
/// `Definition.name` / `position.file` (set at `add_def`) and are reconstructed
/// on decode; codegen uses them only for diagnostics.
fn write_variables(out: &mut String, f: &crate::variables::Function) {
    out.push_str("{\"vars\":[");
    for i in 0..f.snapshot_len() {
        if i > 0 {
            out.push(',');
        }
        let v = f.snapshot_var(i);
        out.push_str("{\"name\":");
        write_str(out, v.name);
        out.push_str(",\"type_def\":");
        write_type(out, v.type_def);
        let _ = write!(
            out,
            ",\"stack_pos\":{},\"uses\":{},\"argument\":{},\"stack_allocated\":{},\"skip_free\":{},\"captured\":{},\"caller_hidden_buf\":{},\"view_elided\":{},\"owner_witness\":{}}}",
            v.stack_pos,
            v.uses,
            v.argument,
            v.stack_allocated,
            v.skip_free,
            v.captured,
            v.caller_hidden_buf,
            v.view_elided,
            v.owner_witness
        );
    }
    out.push_str("],\"names\":[");
    // Sort the names map by var_nr for a deterministic emission (HashMap
    // iteration order is otherwise unstable → would break golden / re-encode).
    let mut names = f.snapshot_names();
    names.sort_by_key(|&(_, nr)| nr);
    for (i, (name, nr)) in names.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        write_str(out, name);
        let _ = write!(out, ",\"nr\":{nr}}}");
    }
    out.push_str("],\"inline_refs\":");
    let mut refs = f.snapshot_inline_refs();
    refs.sort_unstable();
    write_u16_list(out, &refs);
    out.push('}');
}

/// Parse a JSON string into a [`Definition`] (portable IR; see module note).
///
/// # Errors
/// Malformed JSON, wrong field shape, an unknown `def_type` / `purity` /
/// `field_group` tag, or an unknown variant tag in a nested `Type` / `Value`.
pub fn definition_from_json(src: &str) -> Result<Definition, TypeDecodeError> {
    let parsed = crate::json::parse(src)
        .map_err(|e| TypeDecodeError::Shape(format!("json parse: {e:?}")))?;
    definition_from_parsed(&parsed)
}

fn def_type_from_str(s: &str) -> Result<DefType, TypeDecodeError> {
    Ok(match s {
        "Unknown" => DefType::Unknown,
        "Function" => DefType::Function,
        "Dynamic" => DefType::Dynamic,
        "Enum" => DefType::Enum,
        "EnumValue" => DefType::EnumValue,
        "Struct" => DefType::Struct,
        "Vector" => DefType::Vector,
        "Type" => DefType::Type,
        "Constant" => DefType::Constant,
        "Generic" => DefType::Generic,
        "Interface" => DefType::Interface,
        other => return Err(TypeDecodeError::UnknownTag(format!("DefType::{other}"))),
    })
}

fn impure_category_from_str(s: &str) -> Result<ImpureCategory, TypeDecodeError> {
    Ok(match s {
        "HostIo" => ImpureCategory::HostIo,
        "Prng" => ImpureCategory::Prng,
        "Io" => ImpureCategory::Io,
        "ParentWrite" => ImpureCategory::ParentWrite,
        "ParCall" => ImpureCategory::ParCall,
        other => {
            return Err(TypeDecodeError::UnknownTag(format!(
                "ImpureCategory::{other}"
            )));
        }
    })
}

fn u32_list(p: &Parsed) -> Result<Vec<u32>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items.iter().map(as_u32).collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

fn purity_from_parsed(p: &Parsed) -> Result<Purity, TypeDecodeError> {
    let tag = as_str(field(p, "k")?)?;
    Ok(match tag.as_str() {
        "Unknown" => Purity::Unknown,
        "Pure" => Purity::Pure,
        "Impure" => Purity::Impure(impure_category_from_str(&as_str(field(p, "cat")?)?)?),
        other => return Err(TypeDecodeError::UnknownTag(format!("Purity::{other}"))),
    })
}

fn linked_field_group_from_parsed(p: &Parsed) -> Result<LinkedFieldGroup, TypeDecodeError> {
    let kind = match as_str(field(p, "kind")?)?.as_str() {
        "Tuple" => LinkedFieldKind::Tuple,
        "Index" => LinkedFieldKind::Index,
        other => {
            return Err(TypeDecodeError::UnknownTag(format!(
                "LinkedFieldKind::{other}"
            )));
        }
    };
    Ok(LinkedFieldGroup {
        kind,
        instance: as_u16(field(p, "instance")?)?,
        field_indices: dep_list(field(p, "field_indices")?)?,
        alignment: as_u32(field(p, "alignment")?)? as u8,
        size: as_u16(field(p, "size")?)?,
    })
}

fn attribute_list(p: &Parsed) -> Result<Vec<Attribute>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items.iter().map(attribute_from_parsed).collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

fn field_group_list(p: &Parsed) -> Result<Vec<LinkedFieldGroup>, TypeDecodeError> {
    if let Parsed::Array(items) = p {
        items.iter().map(linked_field_group_from_parsed).collect()
    } else {
        Err(TypeDecodeError::Shape("expected array".into()))
    }
}

/// Decode a per-function variable table (C4).  `fn_name` / `fn_file` are the
/// owning definition's name / source file — the `Function`'s own name/file are
/// not stored (they equal these).
fn variables_from_parsed(
    p: &Parsed,
    fn_name: &str,
    fn_file: &str,
) -> Result<crate::variables::Function, TypeDecodeError> {
    let vars_arr = field(p, "vars")?;
    let Parsed::Array(items) = vars_arr else {
        return Err(TypeDecodeError::Shape(
            "variables.vars: expected array".into(),
        ));
    };
    let mut vars = Vec::with_capacity(items.len());
    for it in items {
        vars.push(crate::variables::RestoredVar {
            name: as_str(field(it, "name")?)?,
            type_def: type_from_parsed(field(it, "type_def")?)?,
            stack_pos: as_u16(field(it, "stack_pos")?)?,
            uses: as_u16(field(it, "uses")?)?,
            argument: as_bool(field(it, "argument")?)?,
            stack_allocated: as_bool(field(it, "stack_allocated")?)?,
            skip_free: as_bool(field(it, "skip_free")?)?,
            captured: as_bool(field(it, "captured")?)?,
            caller_hidden_buf: as_bool(field(it, "caller_hidden_buf")?)?,
            view_elided: as_bool(field(it, "view_elided")?)?,
            owner_witness: as_u16(field(it, "owner_witness")?)?,
        });
    }
    let names_arr = field(p, "names")?;
    let Parsed::Array(name_items) = names_arr else {
        return Err(TypeDecodeError::Shape(
            "variables.names: expected array".into(),
        ));
    };
    let mut names = Vec::with_capacity(name_items.len());
    for it in name_items {
        names.push((as_str(field(it, "name")?)?, as_u16(field(it, "nr")?)?));
    }
    let inline_refs = dep_list(field(p, "inline_refs")?)?;
    Ok(crate::variables::Function::from_snapshot(
        fn_name,
        fn_file,
        vars,
        names,
        inline_refs,
    ))
}

fn definition_from_parsed(p: &Parsed) -> Result<Definition, TypeDecodeError> {
    let name = as_str(field(p, "name")?)?;
    let position = position_from_parsed(field(p, "position")?)?;
    let attributes = attribute_list(field(p, "attributes")?)?;
    // attr_names is a derived index (name → attribute slot); rebuild from the
    // restored attribute list rather than storing it.
    let attr_names = attributes
        .iter()
        .enumerate()
        .map(|(i, a)| (a.name.clone(), i))
        .collect();
    let forced = as_u32(field(p, "forced_size")?)? as u8;
    let synthetic_s = as_str(field(p, "synthetic")?)?;
    // variables (C4): the per-function debug-symbol table.  The function's
    // name/file equal this def's name/position.file (set at add_def), so they
    // are reconstructed here rather than stored.
    let variables = variables_from_parsed(field(p, "variables")?, &name, &position.file)?;
    Ok(Definition {
        bound_holder: false,
        variables,
        attr_names,
        // codegen-derived: recomputed by byte_code_from on load.
        code_position: 0,
        code_length: 0,
        const_ref: None,
        source: as_u16(field(p, "source")?)?,
        def_type: def_type_from_str(&as_str(field(p, "def_type")?)?)?,
        parent: as_u32(field(p, "parent")?)?,
        attributes,
        code: value_from_parsed(field(p, "code")?)?,
        returned: type_from_parsed(field(p, "returned")?)?,
        returned_not_null: as_bool(field(p, "returned_not_null")?)?,
        rust: as_str(field(p, "rust")?)?,
        native: as_str(field(p, "native")?)?,
        cap: as_str(field(p, "cap")?)?,               // @PLN86
        superseded: as_str(field(p, "superseded")?)?, // @PLN102 arc C
        c_symbol: as_str(field(p, "c_symbol")?)?,     // @PLN24
        c_sig: as_str(field(p, "c_sig")?)?,           // @PLN24
        op_code: as_u16(field(p, "op_code")?)?,
        known_type: as_u16(field(p, "known_type")?)?,
        pub_visible: as_bool(field(p, "pub_visible")?)?,
        null_safe: as_bool(field(p, "null_safe")?)?, // @PLN46 W2
        closure_record: as_u32(field(p, "closure_record")?)?,
        mutated_captures: str_list(field(p, "mutated_captures")?)?,
        scalars_to_box: str_list(field(p, "scalars_to_box")?)?,
        bounds: u32_list(field(p, "bounds")?)?,
        forced_size: if forced == 0 { None } else { Some(forced) },
        purity: purity_from_parsed(field(p, "purity")?)?,
        field_groups: field_group_list(field(p, "field_groups")?)?,
        synthetic: if synthetic_s.is_empty() {
            None
        } else {
            Some(Box::leak(synthetic_s.into_boxed_str()))
        },
        // moved last so the `&name` / `&position.file` borrows above are done.
        name,
        position,
    })
}

// ─── Data JSON (Step 2 container slice C3) ────────────────────────────────────
//
// The whole-`Data` document — the reviewable `stdlib.json` artifact.  It ties
// the per-`Definition` codec (C2) into one array plus the small amount of
// `Data`-level state the snapshot needs.
//
//   stored  : definitions[] (each via the C2 codec), source
//   rebuilt : def_names / operators / op_codes / possible / use_names — all
//             re-derived by `Data::rebuild_indices` from the definitions
//             (pure functions of `definitions`; never stored)
//   omitted : transient parse state (used_definitions / used_attributes /
//             referenced / statics) and package/wasm metadata (native_* /
//             wasm_bridge_*), which are empty for a stdlib/bundle snapshot;
//             caller_index stays a lazy OnceLock
//
// This is the whole-bundle image: absolute `def_nr` / `known_type` indices are
// stored verbatim (mutually consistent by construction — no relocation).
//
// `Data` has no `PartialEq`, so round-trip is proven by re-encode equality
// (encode → decode → encode is stable), as for `Attribute` / `Definition`.

/// Serialise a whole [`Data`] to JSON — the snapshot document.
#[must_use]
pub fn data_to_json(data: &Data) -> String {
    let mut out = String::new();
    let _ = write!(out, "{{\"source\":{},\"definitions\":[", data.source);
    for (i, d_nr) in (0..data.definitions()).enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_definition(&mut out, data.def(d_nr));
    }
    // The import tables (loft#1359): a load replays `imports` into `def_names`
    // and restores `use_names` verbatim — neither derives from the definitions.
    out.push_str("],\"imports\":[");
    for (i, imp) in data.applied_imports().iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let (name, bind) = imp
            .name
            .as_ref()
            .map_or(("", ""), |(n, b)| (n.as_str(), b.as_str()));
        let _ = write!(
            out,
            "{{\"lib\":{},\"into\":{},\"name\":",
            imp.lib_source, imp.into_source
        );
        write_str(&mut out, name);
        out.push_str(",\"bind\":");
        write_str(&mut out, bind);
        out.push('}');
    }
    out.push_str("],\"use_names\":[");
    for (i, (name, source)) in data.use_name_pairs().iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        write_str(&mut out, name);
        let _ = write!(out, ",\"source\":{source}}}");
    }
    out.push_str("]}");
    out
}

/// One retained import from its snapshot object; an empty `name` is a
/// wildcard `use lib::*`.
fn import_from_parsed(p: &Parsed) -> Result<crate::data::AppliedImport, TypeDecodeError> {
    let name = as_str(field(p, "name")?)?;
    Ok(crate::data::AppliedImport {
        lib_source: as_u16(field(p, "lib")?)?,
        into_source: as_u16(field(p, "into")?)?,
        name: if name.is_empty() {
            None
        } else {
            Some((name, as_str(field(p, "bind")?)?))
        },
    })
}

/// Parse a whole [`Data`] from JSON, rebuilding all derived indices.
///
/// The returned `Data` starts from [`Data::new`], has its `definitions`,
/// `source` and the two import tables restored, then [`Data::rebuild_indices`]
/// re-derives `def_names` / `operators` / `op_codes` / `possible` and replays
/// the imports into `def_names`.  Codegen-derived state
/// (slots, bytecode) is recomputed by the normal compile pass, not here.
///
/// # Errors
/// Malformed JSON, wrong field shape, or an unknown tag in a nested
/// `Type` / `Value` / `Definition`.
pub fn data_from_json(src: &str) -> Result<Data, TypeDecodeError> {
    let parsed = crate::json::parse(src)
        .map_err(|e| TypeDecodeError::Shape(format!("json parse: {e:?}")))?;
    let mut data = Data::new();
    data.source = as_u16(field(&parsed, "source")?)?;
    let defs = field(&parsed, "definitions")?;
    let Parsed::Array(items) = defs else {
        return Err(TypeDecodeError::Shape("definitions: expected array".into()));
    };
    for it in items {
        let def = definition_from_parsed(it)?;
        data.definitions.push(def);
    }
    // Both tables are optional in the document so a snapshot written before
    // they existed still loads (with no imports, as it had none to give).
    if let Ok(Parsed::Array(items)) = field(&parsed, "imports") {
        let mut applied = Vec::with_capacity(items.len());
        for it in items {
            applied.push(import_from_parsed(it)?);
        }
        data.set_applied_imports(applied);
    }
    if let Ok(Parsed::Array(items)) = field(&parsed, "use_names") {
        let mut pairs = Vec::with_capacity(items.len());
        for it in items {
            pairs.push((as_str(field(it, "name")?)?, as_u16(field(it, "source")?)?));
        }
        data.set_use_names(pairs);
    }
    data.rebuild_indices();
    Ok(data)
}

// ─── Snapshot load + compare (Step 2 — validation path) ───────────────────────
//
// The load path at the `Data` level is [`data_from_json`].  [`load_snapshot`]
// wraps it with file I/O.  [`compare_data`] is the validator the quick-loading
// work leans on: it diffs a loaded `Data` against a freshly-parsed reference
// `Data` WITHOUT mutating either — re-encoding each definition to JSON and
// reporting the *first* divergence (which definition, which byte offset) so a
// mismatch is debuggable rather than a bare boolean.
//
// Re-encode equality is the comparison key because `Data` / `Definition` don't
// derive `PartialEq`, and because it is exactly the property the snapshot must
// hold: encode(load(snapshot)) must equal encode(fresh).  A field the decoder
// drops or reconstructs wrongly shows up as a per-definition JSON difference.

/// Load a [`Data`] snapshot from a file written by [`data_to_json`].
///
/// # Errors
/// File read error, or any [`data_from_json`] decode error.
pub fn load_snapshot(path: &str) -> Result<Data, TypeDecodeError> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| TypeDecodeError::Shape(format!("read {path}: {e}")))?;
    data_from_json(&src)
}

/// A localized description of the first place two `Data` values diverge.
#[derive(Debug, PartialEq, Eq)]
pub enum DataDiff {
    /// The two `Data` have different definition counts.
    Count { reference: u32, loaded: u32 },
    /// Definition `index` (`name`) re-encodes differently; `at` is the byte
    /// offset of the first differing character, with short context snippets.
    Definition {
        index: u32,
        name: String,
        at: usize,
        reference: String,
        loaded: String,
    },
    /// The whole-`Data` re-encode differs but every definition matched — a
    /// `Data`-level field (e.g. `source`) diverged.
    Header { reference: String, loaded: String },
}

/// Compare a `loaded` `Data` against a `reference` (freshly-parsed) `Data` by
/// re-encoding both, returning the first divergence or `Ok(())` if identical.
///
/// Neither argument is mutated — this is the read-only validator for the
/// snapshot load path (does NOT swap the snapshot into the running `Data`).
///
/// # Errors
/// [`DataDiff`] describing the first definition (or header) that differs.
pub fn compare_data(reference: &Data, loaded: &Data) -> Result<(), DataDiff> {
    let rn = reference.definitions();
    let ln = loaded.definitions();
    if rn != ln {
        return Err(DataDiff::Count {
            reference: rn,
            loaded: ln,
        });
    }
    for d_nr in 0..rn {
        let r = definition_to_json(reference.def(d_nr));
        let l = definition_to_json(loaded.def(d_nr));
        if r != l {
            let at = r
                .char_indices()
                .zip(l.char_indices())
                .find(|((_, rc), (_, lc))| rc != lc)
                .map_or_else(|| r.len().min(l.len()), |((i, _), _)| i);
            let snippet = |s: &str| {
                let start = at.saturating_sub(20);
                let end = (at + 40).min(s.len());
                s.get(start..end).unwrap_or(s).to_string()
            };
            return Err(DataDiff::Definition {
                index: d_nr,
                name: reference.def(d_nr).name.clone(),
                at,
                reference: snippet(&r),
                loaded: snippet(&l),
            });
        }
    }
    // Every definition matched; catch any Data-level (header) difference.
    let r = data_to_json(reference);
    let l = data_to_json(loaded);
    if r != l {
        return Err(DataDiff::Header {
            reference: r,
            loaded: l,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Deps;

    // ── Type JSON round-trip (Step 2 first slice) ───────────────────────────

    /// Assert `Type → JSON → Type` is lossless for one value.
    fn round_trip(ty: &Type) {
        let json = type_to_json(ty);
        let back =
            type_from_json(&json).unwrap_or_else(|e| panic!("decode {ty:?} from {json}: {e:?}"));
        assert_eq!(*ty, back, "round-trip mismatch via {json}");
    }

    #[test]
    fn type_round_trip_all_variants() {
        // One representative of every Type variant, including the recursive
        // and key-carrying ones.  Value-free by construction.
        let cases = vec![
            Type::Unknown(7),
            Type::Null,
            Type::Void,
            Type::Never,
            Type::Boolean,
            Type::Float,
            Type::Single,
            Type::Character,
            Type::Keys,
            Type::Integer(IntegerSpec::wide()),
            Type::Integer(IntegerSpec::u8()),
            Type::Integer(IntegerSpec::signed32()),
            Type::Text(Deps::none()),
            Type::Text(Deps::unknown(vec![1, 2, 3])),
            Type::Enum(4, true, Deps::unknown(vec![5])),
            Type::Enum(4, false, Deps::none()),
            Type::Reference(9, Deps::unknown(vec![0, 1])),
            Type::RefVar(Box::new(Type::Boolean)),
            Type::Vector(Box::new(Type::Text(Deps::none())), Deps::unknown(vec![2])),
            Type::Routine(11),
            Type::Iterator(
                Box::new(Type::Integer(IntegerSpec::wide())),
                Box::new(Type::Null),
            ),
            Type::Sorted(
                3,
                vec![("name".into(), true), ("age".into(), false)],
                Deps::unknown(vec![1]),
            ),
            Type::Index(3, vec![("key".into(), true)], Deps::none()),
            Type::Radix(3, vec!["x".into(), "y".into()], Deps::unknown(vec![1])),
            Type::Trie(3, "word".into(), Deps::unknown(vec![1])),
            Type::Hash(3, vec!["id".into()], Deps::none()),
            Type::Function(
                vec![Type::Integer(IntegerSpec::wide()), Type::Text(Deps::none())],
                Box::new(Type::Boolean),
                Deps::unknown(vec![0]),
            ),
            Type::Rewritten(Box::new(Type::Text(Deps::none()))),
            Type::Tuple(vec![
                Type::Integer(IntegerSpec::wide()),
                Type::Text(Deps::none()),
            ]),
            // Nested recursion: vector<vector<reference>>.
            Type::Vector(
                Box::new(Type::Vector(
                    Box::new(Type::Reference(2, Deps::none())),
                    Deps::none(),
                )),
                Deps::none(),
            ),
        ];
        for ty in &cases {
            round_trip(ty);
        }
    }

    #[test]
    fn string_fields_escape_round_trip() {
        // Key names with characters that need JSON escaping must survive.
        let ty = Type::Sorted(
            1,
            vec![("a\"b\\c\nd".into(), true), ("tab\there".into(), false)],
            Deps::none(),
        );
        round_trip(&ty);
    }

    #[test]
    fn unknown_tag_is_an_error() {
        let err = type_from_json("{\"k\":\"Bogus\"}");
        assert_eq!(err, Err(TypeDecodeError::UnknownTag("Bogus".into())));
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(matches!(
            type_from_json("{not json"),
            Err(TypeDecodeError::Shape(_))
        ));
    }

    // ── Golden output tests ─────────────────────────────────────────────────
    //
    // Pin the EXACT JSON string for representative variants.  The round-trip
    // tests prove losslessness; these lock the on-disk *format* so it can't
    // silently change shape (a format drift would invalidate every cached
    // snapshot, so the format is a contract).  They also document what the
    // emitted JSON looks like.

    #[test]
    fn golden_bare_variant() {
        // Payload-free variant → just the tag.
        assert_eq!(type_to_json(&Type::Boolean), r#"{"k":"Boolean"}"#);
        assert_eq!(type_to_json(&Type::Null), r#"{"k":"Null"}"#);
    }

    #[test]
    fn golden_integer_spec() {
        // `forced` is 0 when forced_size is None.
        assert_eq!(
            type_to_json(&Type::Integer(IntegerSpec::wide())),
            r#"{"k":"Integer","spec":{"min":-2147483647,"max":4294967295,"not_null":false,"forced":0}}"#
        );
        // u8 alias carries a forced 1-byte width.
        assert_eq!(
            type_to_json(&Type::Integer(IntegerSpec::u8())),
            r#"{"k":"Integer","spec":{"min":0,"max":255,"not_null":false,"forced":1}}"#
        );
    }

    #[test]
    fn golden_index_carrying_variant() {
        // def_nr + dep list stored verbatim (absolute, whole-bundle image).
        assert_eq!(
            type_to_json(&Type::Reference(9, Deps::unknown(vec![0, 1]))),
            r#"{"k":"Reference","n":9,"dep":[0,1]}"#
        );
    }

    #[test]
    fn golden_recursive_variant() {
        // Box<Type> recursion nests the child object inline under "t".
        assert_eq!(
            type_to_json(&Type::Vector(
                Box::new(Type::Text(Deps::none())),
                Deps::unknown(vec![2])
            )),
            r#"{"k":"Vector","t":{"k":"Text","dep":[]},"dep":[2]}"#
        );
    }

    #[test]
    fn golden_sort_keys() {
        assert_eq!(
            type_to_json(&Type::Sorted(
                3,
                vec![("name".into(), true), ("age".into(), false)],
                Deps::unknown(vec![1]),
            )),
            r#"{"k":"Sorted","n":3,"keys":[{"name":"name","asc":true},{"name":"age","asc":false}],"dep":[1]}"#
        );
    }

    #[test]
    fn golden_function_variant() {
        assert_eq!(
            type_to_json(&Type::Function(
                vec![Type::Boolean, Type::Float],
                Box::new(Type::Null),
                Deps::none(),
            )),
            r#"{"k":"Function","args":[{"k":"Boolean"},{"k":"Float"}],"result":{"k":"Null"},"dep":[]}"#
        );
    }

    #[test]
    fn golden_output_is_valid_json() {
        // Every emitted string must parse back through loft's own JSON parser
        // (strict dialect) — guards against a malformed-emitter regression
        // independent of our own decoder.
        let samples = [
            Type::Boolean,
            Type::Integer(IntegerSpec::u8()),
            Type::Vector(
                Box::new(Type::Reference(2, Deps::unknown(vec![3]))),
                Deps::none(),
            ),
            Type::Sorted(1, vec![("k".into(), true)], Deps::none()),
        ];
        for ty in &samples {
            let json = type_to_json(ty);
            assert!(
                crate::json::parse(&json).is_ok(),
                "emitter produced invalid JSON: {json}"
            );
        }
    }

    // ─── Value JSON (slice V1: leaf variants) ───────────────────────────

    /// Every leaf (non-Value-recursive) variant survives encode→decode.
    #[test]
    fn value_leaf_round_trip() {
        let samples = [
            Value::Null,
            Value::Line(7),
            Value::Int(-5),
            Value::Int(2_000_000_000),
            Value::Long(9_999_999_999),
            Value::Float(3.5),
            Value::Single(2.5),
            Value::Boolean(true),
            Value::Boolean(false),
            Value::Enum(3, 12),
            Value::Text("hi\n\"x\"\t\\z".to_string()),
            Value::RawExpr("a + b".to_string()),
            Value::Var(4),
            Value::Break(1),
            Value::Continue(2),
            Value::FnRefDnr(8),
            Value::TupleGet(1, 2),
            Value::Keys(vec![
                Key {
                    type_nr: -1,
                    position: 5,
                    start: 100,
                },
                Key {
                    type_nr: 3,
                    position: 0,
                    start: 0,
                },
            ]),
        ];
        for v in &samples {
            let json = value_to_json(v);
            let back = value_from_json(&json)
                .unwrap_or_else(|e| panic!("decode failed for {json}: {e:?}"));
            assert_eq!(&back, v, "round trip mismatch via {json}");
        }
    }

    /// Pin the exact emitted JSON for representative leaf variants — the
    /// on-disk format is a contract (a drift invalidates every snapshot).
    #[test]
    fn value_golden_leaf_format() {
        assert_eq!(value_to_json(&Value::Null), r#"{"k":"Null"}"#);
        assert_eq!(value_to_json(&Value::Int(-5)), r#"{"k":"Int","n":-5}"#);
        assert_eq!(
            value_to_json(&Value::Boolean(true)),
            r#"{"k":"Boolean","b":true}"#
        );
        assert_eq!(
            value_to_json(&Value::Enum(3, 12)),
            r#"{"k":"Enum","ord":3,"tp":12}"#
        );
        assert_eq!(value_to_json(&Value::Var(4)), r#"{"k":"Var","n":4}"#);
        assert_eq!(
            value_to_json(&Value::TupleGet(1, 2)),
            r#"{"k":"TupleGet","var":1,"idx":2}"#
        );
        assert_eq!(
            value_to_json(&Value::Text("a\"b".to_string())),
            r#"{"k":"Text","s":"a\"b"}"#
        );
        assert_eq!(
            value_to_json(&Value::Keys(vec![Key {
                type_nr: -1,
                position: 5,
                start: -32768
            }])),
            r#"{"k":"Keys","keys":[{"type_nr":-1,"position":5,"start":-32768}]}"#
        );
    }

    /// The full encoder is exhaustive (it must compile over all 32 variants);
    /// every emit — leaf AND recursive — must be valid JSON even before the
    /// recursive decoders land.
    #[test]
    fn value_encoder_emits_valid_json_for_recursive() {
        let block = Block {
            name: "blk",
            operators: vec![Value::Int(1), Value::Null],
            result: Type::Boolean,
            scope: 2,
            var_size: 8,
        };
        let samples = [
            Value::Call(5, vec![Value::Int(1), Value::Var(0)]),
            Value::Set(3, Box::new(Value::Int(9))),
            Value::If(
                Box::new(Value::Boolean(true)),
                Box::new(Value::Int(1)),
                Box::new(Value::Int(0)),
            ),
            Value::Block(Box::new(block.clone())),
            Value::Loop(Box::new(block)),
            Value::FnRef(2, 1, Box::new(Type::Boolean)),
            Value::Span(Box::new((
                Position {
                    file: "f.loft".to_string(),
                    line: 3,
                    pos: 7,
                },
                Value::Int(42),
            ))),
        ];
        for v in &samples {
            let json = value_to_json(v);
            assert!(
                crate::json::parse(&json).is_ok(),
                "encoder produced invalid JSON: {json}"
            );
        }
    }

    /// V2 round-trips the recursive variants whose payload is only
    /// `Value` / `Vec<Value>` / scalars (Block/Loop/Span/FnRef wait
    /// for V3–V4).  Nesting is exercised so the recursion is real.
    #[test]
    fn value_recursive_round_trip() {
        let samples = [
            Value::Call(5, vec![Value::Int(1), Value::Var(0)]),
            Value::Call(0, vec![]),
            Value::CallRef(2, vec![Value::Null]),
            Value::Insert(vec![Value::Int(1), Value::Int(2)]),
            Value::Tuple(vec![Value::Boolean(true), Value::Text("x".into())]),
            Value::Parallel(vec![Value::Var(1), Value::Var(2)]),
            Value::Set(3, Box::new(Value::Int(9))),
            Value::Return(Box::new(Value::Null)),
            Value::Drop(Box::new(Value::Var(0))),
            Value::Yield(Box::new(Value::Int(7))),
            Value::TuplePut(1, 4, Box::new(Value::Int(3))),
            Value::If(
                Box::new(Value::Boolean(true)),
                Box::new(Value::Call(1, vec![Value::Int(0)])),
                Box::new(Value::Set(2, Box::new(Value::Int(5)))),
            ),
            Value::Iter(
                3,
                Box::new(Value::Var(0)),
                Box::new(Value::Call(9, vec![])),
                Box::new(Value::Null),
            ),
        ];
        for v in &samples {
            let json = value_to_json(v);
            let back = value_from_json(&json)
                .unwrap_or_else(|e| panic!("decode failed for {json}: {e:?}"));
            assert_eq!(&back, v, "round trip mismatch via {json}");
        }
    }

    /// Pin the exact emitted JSON for a representative recursive variant.
    #[test]
    fn value_golden_recursive_format() {
        assert_eq!(
            value_to_json(&Value::Call(5, vec![Value::Int(1), Value::Var(0)])),
            r#"{"k":"Call","d":5,"args":[{"k":"Int","n":1},{"k":"Var","n":0}]}"#
        );
        assert_eq!(
            value_to_json(&Value::Set(3, Box::new(Value::Int(9)))),
            r#"{"k":"Set","var":3,"v":{"k":"Int","n":9}}"#
        );
        assert_eq!(
            value_to_json(&Value::If(
                Box::new(Value::Boolean(true)),
                Box::new(Value::Int(1)),
                Box::new(Value::Int(0)),
            )),
            r#"{"k":"If","cond":{"k":"Boolean","b":true},"t":{"k":"Int","n":1},"f":{"k":"Int","n":0}}"#
        );
    }

    /// V3 round-trips Block / Loop (with the `&'static str` name) and Span
    /// (with a Position), including nested Values inside the block body.
    #[test]
    fn value_block_span_round_trip() {
        let block = Block {
            name: "Iter range",
            operators: vec![
                Value::Set(0, Box::new(Value::Int(1))),
                Value::Call(7, vec![Value::Var(0)]),
                Value::Null,
            ],
            result: Type::Boolean,
            scope: 2,
            var_size: 8,
        };
        let samples = [
            Value::Block(Box::new(block.clone())),
            Value::Loop(Box::new(block)),
            Value::Span(Box::new((
                Position {
                    file: "src/x.loft".to_string(),
                    line: 12,
                    pos: 4,
                },
                Value::Call(3, vec![Value::Int(9)]),
            ))),
            // empty-body block — the degenerate operators=[] case
            Value::Block(Box::new(Block {
                name: "",
                operators: vec![],
                result: Type::Null,
                scope: 0,
                var_size: 0,
            })),
        ];
        for v in &samples {
            let json = value_to_json(v);
            let back = value_from_json(&json)
                .unwrap_or_else(|e| panic!("decode failed for {json}: {e:?}"));
            assert_eq!(&back, v, "round trip mismatch via {json}");
        }
    }

    /// Pin the exact emitted JSON for a Block and a Span.
    #[test]
    fn value_golden_block_span_format() {
        assert_eq!(
            value_to_json(&Value::Block(Box::new(Block {
                name: "blk",
                operators: vec![Value::Null],
                result: Type::Boolean,
                scope: 2,
                var_size: 8,
            }))),
            r#"{"k":"Block","block":{"name":"blk","operators":[{"k":"Null"}],"result":{"k":"Boolean"},"scope":2,"var_size":8}}"#
        );
        assert_eq!(
            value_to_json(&Value::Span(Box::new((
                Position {
                    file: "f.loft".to_string(),
                    line: 3,
                    pos: 7
                },
                Value::Int(42),
            )))),
            r#"{"k":"Span","pos":{"file":"f.loft","line":3,"pos":7},"v":{"k":"Int","n":42}}"#
        );
    }

    /// V4 round-trips FnRef (Type payload), closing the Value slice.
    #[test]
    fn value_fnref_round_trip() {
        let samples = [
            Value::FnRef(2, 1, Box::new(Type::Boolean)),
            // negative dnr + sentinel var (the to_default shape, data.rs:536)
            Value::FnRef(
                -1,
                u16::MAX,
                Box::new(Type::Function(vec![], Box::new(Type::Null), Deps::none())),
            ),
        ];
        for v in &samples {
            let json = value_to_json(v);
            let back = value_from_json(&json)
                .unwrap_or_else(|e| panic!("decode failed for {json}: {e:?}"));
            assert_eq!(&back, v, "round trip mismatch via {json}");
        }
    }

    /// Pin the exact emitted JSON for FnRef.
    #[test]
    fn value_golden_fnref_format() {
        assert_eq!(
            value_to_json(&Value::FnRef(2, 1, Box::new(Type::Boolean))),
            r#"{"k":"FnRef","d":2,"var":1,"t":{"k":"Boolean"}}"#
        );
    }

    /// One representative of every payload shape decodes — the decoder is
    /// exhaustive against the (compiler-enforced exhaustive) encoder.
    #[test]
    fn value_decoder_covers_every_shape() {
        let block = Block {
            name: "b",
            operators: vec![Value::Null],
            result: Type::Null,
            scope: 0,
            var_size: 0,
        };
        let all = [
            Value::Null,
            Value::Call(0, vec![]),
            Value::Block(Box::new(block.clone())),
            Value::Loop(Box::new(block)),
            Value::Span(Box::new((
                Position {
                    file: "f".into(),
                    line: 1,
                    pos: 1,
                },
                Value::Null,
            ))),
            Value::FnRef(0, 0, Box::new(Type::Null)),
        ];
        for v in &all {
            let json = value_to_json(v);
            let back =
                value_from_json(&json).unwrap_or_else(|e| panic!("decode error for {json}: {e:?}"));
            assert_eq!(&back, v, "round trip mismatch via {json}");
        }
    }

    /// A genuinely unknown tag is a `UnknownTag` error.
    #[test]
    fn value_unknown_tag_is_an_error() {
        assert_eq!(
            value_from_json(r#"{"k":"Bogus"}"#),
            Err(TypeDecodeError::UnknownTag("Bogus".to_string()))
        );
    }

    // ─── Attribute JSON (container slice C1) ────────────────────────────

    fn sample_attribute() -> Attribute {
        Attribute {
            name: "weight".to_string(),
            typedef: Type::Integer(IntegerSpec::u8()),
            mutable: true,
            constant: false,
            const_field: false,
            value_const: false,
            init: true,
            nullable: false,
            primary: true,
            hidden: false,
            value: Value::Int(5),
            check: Value::Call(9, vec![Value::Var(0), Value::Int(100)]),
            check_message: Value::Text("too big".to_string()),
            alias_d_nr: 42,
            assigned_lambda_d_nr: u32::MAX,
            links: vec!["stats#read".to_string(), "stats#update".to_string()],
            lexeme: false,
        }
    }

    /// `Attribute` derives only `Clone` (no `PartialEq`), so losslessness is
    /// proven by re-encode equality: encode → decode → encode must be stable.
    #[test]
    fn attribute_round_trip_via_reencode() {
        let a = sample_attribute();
        let json = attribute_to_json(&a);
        let back = attribute_from_json(&json)
            .unwrap_or_else(|e| panic!("decode failed for {json}: {e:?}"));
        assert_eq!(attribute_to_json(&back), json, "re-encode drift");
    }

    /// The all-defaults attribute (as `add_attribute` builds it) round-trips,
    /// including the three `Value::Null` fields and the `u32::MAX` sentinels.
    #[test]
    fn attribute_default_shape_round_trips() {
        let a = Attribute {
            name: String::new(),
            typedef: Type::Unknown(0),
            mutable: true,
            constant: false,
            const_field: false,
            value_const: false,
            init: false,
            nullable: true,
            primary: false,
            hidden: false,
            value: Value::Null,
            check: Value::Null,
            check_message: Value::Null,
            alias_d_nr: u32::MAX,
            assigned_lambda_d_nr: u32::MAX,
            links: Vec::new(),
            lexeme: false,
        };
        let json = attribute_to_json(&a);
        let back = attribute_from_json(&json).expect("decode");
        assert_eq!(attribute_to_json(&back), json);
    }

    /// Pin the exact emitted JSON — the on-disk format is a contract.
    #[test]
    fn attribute_golden_format() {
        let a = Attribute {
            name: "x".to_string(),
            typedef: Type::Boolean,
            mutable: false,
            constant: true,
            const_field: false,
            value_const: false,
            init: false,
            nullable: false,
            primary: false,
            hidden: false,
            value: Value::Null,
            check: Value::Null,
            check_message: Value::Null,
            alias_d_nr: 0,
            assigned_lambda_d_nr: 0,
            links: Vec::new(),
            lexeme: false,
        };
        assert_eq!(
            attribute_to_json(&a),
            r#"{"name":"x","typedef":{"k":"Boolean"},"mutable":false,"constant":true,"init":false,"nullable":false,"primary":false,"hidden":false,"const_field":false,"value":{"k":"Null"},"check":{"k":"Null"},"check_message":{"k":"Null"},"alias_d_nr":0,"assigned_lambda_d_nr":0,"links":""}"#
        );
    }

    /// Every emitted attribute is valid JSON under loft's own parser.
    #[test]
    fn attribute_emit_is_valid_json() {
        let json = attribute_to_json(&sample_attribute());
        assert!(
            crate::json::parse(&json).is_ok(),
            "attribute emitter produced invalid JSON: {json}"
        );
    }

    // ─── Definition JSON (container slice C2) ───────────────────────────

    /// A type-level `Definition` (a struct) with attributes + a field group,
    /// matching what `Data::add` / the parser produce for a type def.
    fn sample_struct_def() -> Definition {
        Definition {
            bound_holder: false,
            name: "Point".to_string(),
            source: 1,
            def_type: DefType::Struct,
            parent: u32::MAX,
            position: Position {
                file: "geo.loft".to_string(),
                line: 4,
                pos: 0,
            },
            attributes: vec![
                Attribute {
                    name: "x".to_string(),
                    typedef: Type::Integer(IntegerSpec::u8()),
                    mutable: true,
                    constant: false,
                    const_field: false,
                    value_const: false,
                    init: false,
                    nullable: true,
                    primary: false,
                    hidden: false,
                    value: Value::Null,
                    check: Value::Null,
                    check_message: Value::Null,
                    alias_d_nr: u32::MAX,
                    assigned_lambda_d_nr: u32::MAX,
                    links: Vec::new(),
                    lexeme: false,
                },
                Attribute {
                    name: "y".to_string(),
                    typedef: Type::Integer(IntegerSpec::u8()),
                    mutable: true,
                    constant: false,
                    const_field: false,
                    value_const: false,
                    init: false,
                    nullable: true,
                    primary: false,
                    hidden: false,
                    value: Value::Int(0),
                    check: Value::Null,
                    check_message: Value::Null,
                    alias_d_nr: u32::MAX,
                    assigned_lambda_d_nr: u32::MAX,
                    links: Vec::new(),
                    lexeme: false,
                },
            ],
            attr_names: std::collections::HashMap::new(),
            code: Value::Null,
            returned: Type::Reference(7, Deps::none()),
            returned_not_null: false,
            rust: String::new(),
            native: String::new(),
            cap: String::new(),
            superseded: String::new(),
            c_symbol: String::new(),
            c_sig: String::new(),
            op_code: u16::MAX,
            code_position: 0,
            code_length: 0,
            known_type: 3,
            variables: crate::variables::Function::new("Point", "geo.loft"),
            pub_visible: true,
            null_safe: false,
            closure_record: u32::MAX,
            mutated_captures: Vec::new(),
            scalars_to_box: Vec::new(),
            bounds: vec![5, 9],
            const_ref: None,
            forced_size: Some(4),
            purity: Purity::Impure(ImpureCategory::HostIo),
            field_groups: vec![LinkedFieldGroup {
                kind: LinkedFieldKind::Tuple,
                instance: 0,
                field_indices: vec![0, 1],
                alignment: 8,
                size: 16,
            }],
            synthetic: Some("enum_dispatcher"),
        }
    }

    /// `Definition` derives only `Clone`, so losslessness is proven by
    /// re-encode equality — and we additionally assert the recomputed /
    /// derived fields on the decoded value.
    #[test]
    fn definition_round_trip_via_reencode() {
        let d = sample_struct_def();
        let json = definition_to_json(&d);
        let back = definition_from_json(&json)
            .unwrap_or_else(|e| panic!("decode failed for {json}: {e:?}"));
        assert_eq!(definition_to_json(&back), json, "re-encode drift");

        // attr_names is rebuilt from attributes (not stored).
        assert_eq!(back.attr_names.get("x"), Some(&0));
        assert_eq!(back.attr_names.get("y"), Some(&1));
        // codegen-derived fields decode to their recompute-me sentinels.
        assert_eq!(back.code_position, 0);
        assert_eq!(back.code_length, 0);
        assert!(back.const_ref.is_none());
        // the &'static synthetic reason survives via Box::leak.
        assert_eq!(back.synthetic, Some("enum_dispatcher"));
        // variables reconstructed empty (type-level def).
        assert_eq!(back.variables.name, "Point");
    }

    /// The all-defaults type def (as `Data::add` builds it) round-trips,
    /// including `synthetic: None` (emitted as "") and `forced_size: None`.
    #[test]
    fn definition_default_shape_round_trips() {
        let d = Definition {
            bound_holder: false,
            name: "T".to_string(),
            source: 0,
            def_type: DefType::Type,
            parent: u32::MAX,
            position: Position {
                file: String::new(),
                line: 0,
                pos: 0,
            },
            attributes: Vec::new(),
            attr_names: std::collections::HashMap::new(),
            code: Value::Null,
            returned: Type::Unknown(0),
            returned_not_null: false,
            rust: String::new(),
            native: String::new(),
            cap: String::new(),
            superseded: String::new(),
            c_symbol: String::new(),
            c_sig: String::new(),
            op_code: u16::MAX,
            code_position: 0,
            code_length: 0,
            known_type: u16::MAX,
            variables: crate::variables::Function::new("T", ""),
            pub_visible: false,
            null_safe: false,
            closure_record: u32::MAX,
            mutated_captures: Vec::new(),
            scalars_to_box: Vec::new(),
            bounds: Vec::new(),
            const_ref: None,
            forced_size: None,
            purity: Purity::Unknown,
            field_groups: Vec::new(),
            synthetic: None,
        };
        let json = definition_to_json(&d);
        let back = definition_from_json(&json).expect("decode");
        assert_eq!(definition_to_json(&back), json);
        assert!(back.synthetic.is_none());
        assert!(back.forced_size.is_none());
    }

    /// Every emitted definition is valid JSON under loft's own parser.
    #[test]
    fn definition_emit_is_valid_json() {
        let json = definition_to_json(&sample_struct_def());
        assert!(
            crate::json::parse(&json).is_ok(),
            "definition emitter produced invalid JSON: {json}"
        );
    }

    /// An unknown def_type tag is a decode error (not a silent misparse).
    /// `Definition` has no `Debug`/`PartialEq`, so match the error directly
    /// rather than `assert_eq!` on the whole `Result`.
    #[test]
    fn definition_unknown_def_type_errors() {
        let mut json = definition_to_json(&sample_struct_def());
        json = json.replace("\"def_type\":\"Struct\"", "\"def_type\":\"Bogus\"");
        match definition_from_json(&json) {
            Err(TypeDecodeError::UnknownTag(t)) => assert_eq!(t, "DefType::Bogus"),
            Err(e) => panic!("expected UnknownTag(DefType::Bogus), got {e:?}"),
            Ok(_) => panic!("expected an error, got Ok"),
        }
    }
}
