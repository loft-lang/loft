// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database (alloc/persistence/journal/snapshot): the layout identity's schema half

//! The half of a store's layout identity only the compiler can answer: the DEF-level
//! nullability schema and the program's root types, both read from the IR.  Classifying a
//! sidecar, reading and writing it stay in `schema_sidecar`, which a compiled program links
//! (@PLN191).

use crate::database::Stores;
use crate::schema_sidecar::{LayoutIdentity, SCHEMA_MARKER};

/// @PLN102 arc-E F9 — build the identity WITH the DEF-level nullability schema
/// (`Data` available: the CLI `loft layout` / api-surface sites). Two identical
/// byte layouts that differ only in a full-width `τ` vs `τ?` field now differ
/// in `schema`, so `schema_sidecar::classify` refuses the raw handoff.
#[must_use]
pub fn of_scoped(stores: &Stores, roots: &[u16], data: &crate::data::Data) -> LayoutIdentity {
    LayoutIdentity {
        layout_hash: stores.layout_algo_hash(roots),
        dump: stores.layout_dump(roots),
        schema: nullability_schema(data, roots),
    }
}

/// @PLN102 arc-E F9 — the DEF-level nullability signature of `roots`: for each root
/// type, the names of its `Optional` (nullable) fields. This is the fact the storage
/// layout DROPS for full-width types (`integer` vs `integer?` share a byte layout, so
/// the same `dump`), which makes a raw handoff between them unsafe. Since every
/// user-declared type is a root ([`program_roots`]), a per-root-type signature covers
/// the whole surface without a closure walk. Sorted for determinism. ALWAYS non-empty
/// (a `SCHEMA_MARKER` version header leads it) so that an all-non-null program's schema
/// is "computed, no nullable fields" — DISTINCT from the empty string, which means
/// "schema ABSENT" (a pre-F9 sidecar, or an identity built without `Data`) and is
/// grandfathered in `schema_sidecar::classify`. Narrow nullable fields already differ in the `dump`
/// (the `null=` token), so this only becomes load-bearing for the full-width case.
#[must_use]
pub fn nullability_schema(data: &crate::data::Data, roots: &[u16]) -> String {
    use crate::data::Type;
    let rootset: std::collections::HashSet<u16> = roots.iter().copied().collect();
    let mut entries: Vec<String> = Vec::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if def.known_type == u16::MAX || !rootset.contains(&def.known_type) {
            continue;
        }
        let mut nullable: Vec<&str> = def
            .attributes
            .iter()
            .filter(|a| matches!(a.typedef, Type::Optional(_)))
            .map(|a| a.name.as_str())
            .collect();
        if !nullable.is_empty() {
            nullable.sort_unstable();
            entries.push(format!("{}:{}", def.name, nullable.join(",")));
        }
    }
    entries.sort_unstable();
    // The marker leads so a NO-nullable-fields program is "computed empty" (non-empty
    // string), never confused with an ABSENT schema (the empty string).
    if entries.is_empty() {
        SCHEMA_MARKER.to_string()
    } else {
        format!("{SCHEMA_MARKER};{}", entries.join(";"))
    }
}

/// The user-defined struct / enum types of a program — the roots of its layout
/// identity. Stdlib types are excluded (the layout closure pulls in whatever
/// base / collection types they reference). Feed these to `LayoutIdentity::of`
/// to build the identity the baseline (Phase F) or the `.dschema` (Phase D) records.
#[must_use]
pub fn program_roots(data: &crate::data::Data) -> Vec<u16> {
    use crate::data::DefType;
    (0..data.definitions())
        .map(|d_nr| data.def(d_nr))
        .filter(|def| {
            matches!(def.def_type, DefType::Struct | DefType::Enum)
                // User-declared types have plain-identifier names. Compiler-
                // generated ones (`__tuple<…>`, `main_vector<…>`, closures) carry
                // `__` or a `<…>` instantiation suffix; the layout closure reaches
                // them via the user types that use them, so they aren't roots.
                && !def.name.starts_with("__")
                && !def.name.contains('<')
                && !crate::file_access::is_stdlib_source(def.position.file.as_str())
                && def.known_type != u16::MAX
        })
        .map(|def| def.known_type)
        .collect()
}
