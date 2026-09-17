// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @F48 — the `loft` CLI: engine behind the `loft api-surface` subcommand (@PLN102 C1).

//! @PLN102 C1 — the library's OBSERVABLE public surface: the input to the compat-verdict
//! engine. Commits 1–3 (this file): membership + visibility TIER (commit 1) + each member's
//! resolved SIGNATURE (commit 2) + canonical determinism (commit 3) — members sorted, and
//! struct fields / enum variants sorted by name (named construction → their order is not API,
//! only layout), while fn params keep declaration order (positional → their order IS API).
//! Byte layout is the separate layout axis (commit 5).
//!
//! The surface is not the `pub`-marked set but the OBSERVABLE CLOSURE: the `pub` roots
//! plus every library type reachable through a `pub` signature, INCLUDING non-`pub` ones —
//! because loft leaks a returned non-`pub` type's readable field-shape to the consumer
//! (verified: a consumer can read `make().x` on a private `Widget`), and never-break makes
//! that leak permanent, so a `pub`-only set would silently miss field-level breaks on
//! leaked types. Each member carries loft's implicit three-tier visibility:
//! - **Public** — a `pub`-marked root.
//! - **Sealed** — a value-exposed non-`pub` type: consumers read it but cannot name or
//!   construct it (the sealed/factory granularity, kept on purpose).
//!
//! A non-`pub` type unreachable through any `pub` signature is *private* and excluded.

use crate::data::{Data, DefType, Type};
use std::collections::HashSet;

/// The visibility tier of a surface member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Public,
    Sealed,
}

impl Tier {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Public => "public",
            Tier::Sealed => "sealed",
        }
    }
}

/// One member of the public surface: identity (`name · kind · tier`) + its resolved
/// SIGNATURE — a fn's `(params) -> return`, a struct's `{ fields }`, an enum's
/// `{ variants }`, a typedef's `= target`. Type spellings are the clean user-facing form
/// (`Data::type_name_str`: nullability as `?`, no internal ownership deps). Byte layout is
/// NOT here — that is the separate layout axis (commit 5).
#[derive(Debug, Clone)]
pub struct Member {
    pub name: String,
    pub kind: &'static str,
    pub tier: Tier,
    pub signature: String,
}

impl Member {
    /// The stable one-line form: `name · kind · tier · signature`. This IS the descriptor
    /// output and the checked-in baseline line (commit 7); [`Member::from_line`] round-trips it.
    #[must_use]
    pub fn to_line(&self) -> String {
        format!(
            "{} · {} · {} · {}",
            self.name,
            self.kind,
            self.tier.as_str(),
            self.signature
        )
    }

    /// Parse a `name · kind · tier · signature` line (a baseline member), or `None` if it is not
    /// one (a comment / blank / malformed line). The signature never contains ` · `, so a
    /// 4-way split is exact.
    #[must_use]
    pub fn from_line(line: &str) -> Option<Member> {
        let mut it = line.splitn(4, " · ");
        let name = it.next()?.to_string();
        let kind = kind_from_str(it.next()?)?;
        let tier = match it.next()? {
            "public" => Tier::Public,
            "sealed" => Tier::Sealed,
            _ => return None,
        };
        let signature = it.next()?.to_string();
        Some(Member {
            name,
            kind,
            tier,
            signature,
        })
    }
}

/// Map a printed kind back to its `&'static str` (the inverse of [`classify`]'s labels).
fn kind_from_str(s: &str) -> Option<&'static str> {
    Some(match s {
        "fn" => "fn",
        "method" => "method",
        "operator" => "operator",
        "struct" => "struct",
        "enum" => "enum",
        "typedef" => "typedef",
        "constant" => "constant",
        "interface" => "interface",
        _ => return None,
    })
}

/// The observable public surface of the library whose definitions were parsed from
/// `lib_file` (matched on `Definition.position.file`), as the tier-tagged closure. Stdlib
/// and dependency defs come from other files and are not this library's to break, so they
/// are excluded even when a `pub` signature mentions them.
#[must_use]
pub fn surface(data: &Data, lib_file: &str) -> Vec<Member> {
    let in_lib = |d: u32| d < data.definitions() && data.def(d).position.file == lib_file;

    let mut members: Vec<Member> = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    let mut work: Vec<u32> = Vec::new();

    // 1. Roots — the `pub`-marked, surface-kind top-level defs of this library.
    for d in 0..data.definitions() {
        if in_lib(d)
            && data.def(d).pub_visible
            && classify(data, d).is_some()
            && !is_receiver_alias(data, d)
            && seen.insert(d)
        {
            work.push(d);
        }
    }

    // 2. Closure — follow each member's signature into the library types it mentions; a
    //    reachable non-`pub` one is Sealed. Recurse until the reachable set is closed.
    while let Some(d) = work.pop() {
        let Some((kind, name)) = classify(data, d) else {
            continue;
        };
        let tier = if data.def(d).pub_visible {
            Tier::Public
        } else {
            Tier::Sealed
        };
        let signature = signature_of(data, d, kind);
        members.push(Member {
            name,
            kind,
            tier,
            signature,
        });
        for r in referenced_defs_of(data, d) {
            if in_lib(r)
                && classify(data, r).is_some()
                && !is_receiver_alias(data, r)
                && seen.insert(r)
            {
                work.push(r);
            }
        }
    }

    members.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.kind.cmp(b.kind)));
    members
}

/// Is this the bare-name alias a `both` receiver registers — a member no caller can observe?
///
/// `data.rs`'s `add_fn` gives ONE `Dynamic` dispatcher two jobs (`if is_both ||
/// overload_label.is_some()`), and only the targets tell them apart:
///
/// * an **overload set** targets free functions (`f_…`), and its dispatcher is the only member
///   carrying the name a consumer writes — `pick`, `scale`. Recording it is REQUIRED; the real
///   overloads live under mangled keys (`f_15integer#integer_pick`) nobody can call.
/// * a **`both` receiver** targets the method (`t_<LEN><Type>_<name>`) it is a second spelling
///   of. C123 measured `self` and `both` identical across every call spelling — import lists
///   included, once methods were brought in by name — so this alias is not an independent
///   member, and recording it makes the C123 migration it prescribes read as `removed fn`.
///
/// That is the whole defect: `regex`'s published surface carried `matches · fn · (text: fn) ->
/// unknown` beside `text.matches`, so renaming its receiver — the migration C123 names this
/// library for — reported an API BREAK against a caller-invisible phantom.
///
/// Filtered HERE and not in [`classify`], which is the canonical def→(kind, name) mapper the LSP
/// outline reuses: this is an api-surface policy, not a naming fact.
fn is_receiver_alias(data: &Data, d: u32) -> bool {
    let def = data.def(d);
    if def.def_type != DefType::Dynamic {
        return false;
    }
    let mut targets = def
        .attributes
        .iter()
        .filter_map(|a| match a.typedef.base() {
            Type::Routine(r) => Some(*r),
            _ => None,
        });
    // Every target a method, and at least one: a dispatcher with no routine target is not a
    // spelling of anything, so it keeps whatever `classify` makes of it.
    let mut any = false;
    for r in &mut targets {
        any = true;
        if r >= data.definitions() || !data.def(r).name.starts_with("t_") {
            return false;
        }
    }
    any
}

/// The surface `(kind, user-facing name)` for a def, or `None` if it is not a standalone
/// surface member (an enum VARIANT — part of its enum's shape — or an internal / unresolved
/// def). Strips loft's internal name encodings — a global fn is `n_<name>`, a method is
/// `t_<LEN><Type>_<method>`, an operator is `Op<Name>` — to the name a reader would write.
///
/// Public because it is the canonical "definition → (display kind, user name)" mapper: the
/// LSP outline (`loft::lsp::outline`) reuses it so the name-decoding lives in exactly one place.
#[must_use]
pub fn classify(data: &Data, d: u32) -> Option<(&'static str, String)> {
    let def = data.def(d);
    let name = def.name.as_str();
    match def.def_type {
        DefType::Function | DefType::Dynamic | DefType::Generic => {
            if let Some(f) = name.strip_prefix("n_") {
                Some(("fn", f.to_string()))
            } else if name.starts_with("t_") {
                Some(("method", method_name(name)))
            } else if name.starts_with("Op") {
                Some(("operator", name.to_string()))
            } else {
                Some(("fn", name.to_string()))
            }
        }
        DefType::Struct => Some(("struct", name.to_string())),
        DefType::Enum => Some(("enum", name.to_string())),
        DefType::Type => Some(("typedef", name.to_string())),
        DefType::Constant => Some(("constant", name.to_string())),
        DefType::Interface => Some(("interface", name.to_string())),
        DefType::EnumValue | DefType::Vector | DefType::Unknown => None,
    }
}

/// `t_<LEN><Type>_<method>` → `<Type>.<method>` (best-effort; falls back to the raw name).
fn method_name(raw: &str) -> String {
    let body = raw.strip_prefix("t_").unwrap_or(raw);
    let digits: String = body.chars().take_while(char::is_ascii_digit).collect();
    if let Ok(len) = digits.parse::<usize>() {
        let rest = &body[digits.len()..];
        if rest.len() >= len {
            let ty = &rest[..len];
            let method = rest[len..].strip_prefix('_').unwrap_or(&rest[len..]);
            return format!("{ty}.{method}");
        }
    }
    raw.to_string()
}

/// The resolved signature of a member, in the clean user-facing type spelling
/// (`Data::type_name_str` — nullability as `?`, no internal deps). Hidden
/// return-mechanism params are skipped; order follows declaration order (canonical
/// ordering is commit 3).
///
/// Public alongside [`classify`] because the LSP hover (`loft::lsp::symbol_at`)
/// renders the same clean signature the API surface does — one spelling, one home.
#[must_use]
pub fn signature_of(data: &Data, d: u32, kind: &str) -> String {
    let def = data.def(d);
    let ty = |t: &Type| data.type_name_str(t);
    // Render a field/parameter list. Skip hidden return-mechanism params and the synthetic
    // `enum` discriminant tag (`enum` is a reserved keyword, never a user field). `sort`
    // CANONICALISES by name: struct/enum fields use NAMED construction, so their order is
    // not part of the API — a reorder is a layout change (commit 5's axis), not an API break,
    // so sorting kills that false diff. Function PARAMS are positional (calls bind by
    // position), so their order IS the API and is preserved (`sort = false`).
    // `defaults` marks a parameter that carries one, as ` = default` — the FACT that it is
    // optional, not the value. That fact is what makes appending one additive
    // (COMPATIBILITY.md § Per-surface: "a new optional parameter"), and without it in the
    // surface the diff cannot tell an optional addition from a required one, which is how a
    // library adopting the idiom loft recommends came to be told it had broken its consumers
    // (loft#1191). Only FN params carry it: a struct's members are parsed back out by
    // `api_diff::members_of` splitting on `:`, and a value after one would read as part of
    // the type. The value itself is deliberately not rendered — see the note on
    // `api_diff::signature_break`.
    let render = |atts: &[crate::data::Attribute], sort: bool, defaults: bool| -> Vec<String> {
        let mut v: Vec<(&str, String)> = atts
            .iter()
            .filter(|a| !a.hidden && a.name != "enum")
            .map(|a| {
                let opt = if defaults && !matches!(a.value, crate::data::Value::Null) {
                    " = default"
                } else {
                    ""
                };
                (
                    a.name.as_str(),
                    format!("{}: {}{}", a.name, ty(&a.typedef), opt),
                )
            })
            .collect();
        if sort {
            v.sort_by(|a, b| a.0.cmp(b.0));
        }
        v.into_iter().map(|(_, s)| s).collect()
    };
    match kind {
        "fn" | "method" | "operator" => {
            format!(
                "({}) -> {}",
                render(def.attributes(), false, true).join(", "),
                ty(&def.returned)
            )
        }
        "struct" => format!("{{ {} }}", render(def.attributes(), true, false).join(", ")),
        "enum" => {
            // Variants are matched/constructed by name, so variant ORDER is not API either —
            // sort the variants, and each variant's fields, by name (the discriminant value a
            // reorder would shift is a layout concern, commit 5).
            let mut variants: Vec<(String, String)> = Vec::new();
            for v in 0..data.definitions() {
                let vd = data.def(v);
                if vd.parent == d && vd.def_type == DefType::EnumValue {
                    let fields = render(vd.attributes(), true, false);
                    let rendered = if fields.is_empty() {
                        vd.name.clone()
                    } else {
                        format!("{} {{ {} }}", vd.name, fields.join(", "))
                    };
                    variants.push((vd.name.clone(), rendered));
                }
            }
            variants.sort_by(|a, b| a.0.cmp(&b.0));
            let rendered: Vec<String> = variants.into_iter().map(|(_, s)| s).collect();
            format!("{{ {} }}", rendered.join(", "))
        }
        "typedef" => format!("= {}", ty(&def.returned)),
        "constant" => format!(": {}", ty(&def.returned)),
        _ => String::new(),
    }
}

/// Every library def referenced through `d`'s SIGNATURE — the observable edges: a fn's
/// return + params, a struct's field types, an enum's variant field types, a typedef's
/// target. The body is never followed (it is not observable).
fn referenced_defs_of(data: &Data, d: u32) -> Vec<u32> {
    let def = data.def(d);
    let mut out = Vec::new();
    collect_type_defs(&def.returned, &mut out); // return / typedef target
    for a in def.attributes() {
        if !a.hidden {
            collect_type_defs(&a.typedef, &mut out); // fields / parameters
        }
    }
    // An enum's variants are child defs; their fields are part of the enum's shape.
    if def.def_type == DefType::Enum {
        for v in 0..data.definitions() {
            let vd = data.def(v);
            if vd.parent == d && vd.def_type == DefType::EnumValue {
                for a in vd.attributes() {
                    if !a.hidden {
                        collect_type_defs(&a.typedef, &mut out);
                    }
                }
            }
        }
    }
    out
}

/// Collect the user-def numbers a `Type` references, recursing through wrappers. Base /
/// scalar / routine / unknown types reference no user def (the `_` arm).
fn collect_type_defs(t: &Type, out: &mut Vec<u32>) {
    match t {
        Type::Reference(n, _) | Type::Enum(n, _, _) => out.push(*n),
        Type::Sorted(n, _, _)
        | Type::Index(n, _, _)
        | Type::Radix(n, _, _)
        | Type::Trie(n, _, _)
        | Type::Hash(n, _, _) => out.push(*n),
        Type::Vector(b, _) | Type::Optional(b) | Type::RefVar(b) | Type::Rewritten(b) => {
            collect_type_defs(b, out);
        }
        Type::Iterator(a, b) => {
            collect_type_defs(a, out);
            collect_type_defs(b, out);
        }
        Type::Function(ps, r, ..) => {
            for p in ps {
                collect_type_defs(p, out);
            }
            collect_type_defs(r, out);
        }
        Type::Tuple(es) => {
            for e in es {
                collect_type_defs(e, out);
            }
        }
        _ => {}
    }
}
