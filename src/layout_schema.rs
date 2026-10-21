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
    let (root, facts) = LayoutIdentity::root_and_facts(stores, roots);
    LayoutIdentity {
        layout_hash: stores.layout_algo_hash(roots),
        dump: stores.layout_dump(roots),
        schema: nullability_schema(data, roots),
        root,
        facts,
        slot: None,
        version: String::new(),
        sources: String::new(),
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

// ── @PLN196 step 9: where a computed field's value comes from ──────────────────────────────

/// `@FR-L-Upgrade-Source` — the sources section of a project baseline: per field of a user
/// struct whose default reads the record, the stored fields it reads and the release that first
/// recorded the line.  `previous` is the baseline being replaced: a line it already had keeps
/// its release, so the section says when a value started being computed from its sources, not
/// when the baseline was last accepted.  `version` is this release (empty: none recorded).
#[must_use]
pub fn render_sources(data: &crate::data::Data, previous: &str, version: &str) -> String {
    let had: std::collections::BTreeMap<&str, &str> = previous
        .lines()
        .filter_map(|l| l.split_once(" since "))
        .collect();
    let mut out = String::new();
    for ((tp, field), paths) in &data.default_sources {
        let d_nr = data.def_nr(tp);
        if d_nr == u32::MAX
            || crate::file_access::is_stdlib_source(data.def(d_nr).position.file.as_str())
        {
            continue;
        }
        let head = format!("{tp}.{field} <- {}", paths.join(", "));
        let since = had.get(head.as_str()).copied().unwrap_or(version);
        out.push_str(&head);
        if !since.is_empty() {
            out.push_str(" since ");
            out.push_str(since);
        }
        out.push('\n');
    }
    out
}

/// `@FR-L-Upgrade-Source` — what a build would lose of the values its baseline's computed
/// fields were made from: for each sources line whose struct and field still exist but one of
/// whose sources this build no longer declares, a sentence naming the dropped field, the
/// computed one, and the two cures — an `upgrade` routine for the struct, or a floor
/// (`data_compatible_with`) at or above the release that started computing it.  Empty when
/// nothing is lost.
#[must_use]
pub fn lost_sources(
    data: &crate::data::Data,
    sources: &str,
    release: &crate::database::Release,
) -> Vec<String> {
    use crate::data::{DefType, Type};
    let routines: Vec<String> = crate::parser::Parser::upgrade_routine_defs(data)
        .iter()
        .map(|r| data.def(r.item).name().to_string())
        .collect();
    let floor = crate::database::version_triple(&release.oldest);
    // The struct field a path stops resolving at (`Inner.label`), or `None` when it resolves.
    let broken = |mut d_nr: u32, path: &str| -> Option<String> {
        let segs: Vec<&str> = path.split('.').collect();
        for (i, seg) in segs.iter().enumerate() {
            let a = data.attr(d_nr, seg);
            if a == usize::MAX {
                return Some(format!("{}.{seg}", data.def(d_nr).name()));
            }
            if i + 1 < segs.len() {
                match data.attr_type(d_nr, a).base() {
                    Type::Reference(next, _) if data.def_type(*next) == DefType::Struct => {
                        d_nr = *next;
                    }
                    _ => return None,
                }
            }
        }
        None
    };
    let mut out = Vec::new();
    for line in sources.lines() {
        let (head, since) = line.split_once(" since ").unwrap_or((line, ""));
        let Some((target, paths)) = head.split_once(" <- ") else {
            continue;
        };
        let Some((tp, field)) = target.split_once('.') else {
            continue;
        };
        let d_nr = data.def_nr(tp);
        if d_nr == u32::MAX
            || data.def_type(d_nr) != DefType::Struct
            || data.attr(d_nr, field) == usize::MAX
            || routines.iter().any(|r| r == tp)
        {
            continue; // gone with its value, or carried by the type's routine
        }
        if let (Some(f), Some(s)) = (floor, crate::database::version_triple(since))
            && f >= s
        {
            continue; // every store this build reads already holds the computed value
        }
        for path in paths.split(", ") {
            let Some(dropped) = broken(d_nr, path) else {
                continue;
            };
            let from = if since.is_empty() {
                "the baseline".to_string()
            } else {
                since.to_string()
            };
            let floor_cure = if since.is_empty() {
                "record releases in loft.toml".to_string()
            } else {
                format!("raise `data_compatible_with` to \"{since}\"")
            };
            out.push(format!(
                "{dropped} is dropped, and {from} computes {target} from it: a store older than \
                 that holds no {target} and would lose the value — write \
                 `fn upgrade(from: Version, old: <the old fields>, item: {tp})`, or {floor_cure}"
            ));
        }
    }
    out
}
