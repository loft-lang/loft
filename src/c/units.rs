// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native generators: one written C unit cut into the units of a split build
//! @PLN229 B1 — one written unit cut into the units of a split build.
//!
//! The writer writes the unit it always writes, with each function's body marked with the file
//! it is declared in (`SPLIT_FN`) and the start of what no file owns marked (`SPLIT_SHARED`).
//! This cuts it: every unit starts with the same head (the runtime, the types, the prototypes the
//! head itself needs) and then the prototypes its own functions call,
//! one shared unit carries what no file owns (the dispatchers, the drop cascades, the generated
//! twins, the tables, `main`) and every file gets a unit with its own functions.
//!
//! A function defined in one unit and called from another needs external linkage, so every
//! function the units define loses its `static` / `inline` / `LOFT_HOT_INLINE`, at its
//! definition and at its prototype in the head.  The runtime keeps its own: every unit carries
//! it whole (`@C169`), and its state is declared once through `LOFT_STATE` — the shared unit is
//! compiled with `LOFT_SPLIT_OWN`, every other with `LOFT_SPLIT_USE`.  A file-scope variable a
//! body moved out (a work-text slot) keeps its `static`: it belongs to its function's unit.

use super::{SPLIT_FN, SPLIT_FN_END, SPLIT_SHARED, SPLIT_UNIT};
use std::collections::{BTreeMap, HashSet};

/// The storage words a function the units define drops (`static`, `inline`, the hot-helper
/// macro, which is `static inline` with a hint).
const STORAGE: [&str; 3] = ["static ", "inline ", "LOFT_HOT_INLINE "];

/// `line` without the leading storage words of [`STORAGE`].
fn without_storage(line: &str) -> &str {
    let mut rest = line;
    loop {
        match STORAGE.iter().find(|w| rest.starts_with(**w)) {
            Some(w) => rest = &rest[w.len()..],
            None => return rest,
        }
    }
}

/// The name a column-0 function line declares or defines, when it is one: the identifier before
/// the first `(`, with nothing that assigns before it (a variable's initializer is no function).
fn function_name(line: &str) -> Option<&str> {
    if line.starts_with([' ', '\t', '#', '/', '}', '{']) || line.is_empty() {
        return None;
    }
    let paren = line.find('(')?;
    let head = &line[..paren];
    if head.contains('=') || head.contains('"') {
        return None;
    }
    let name = head
        .trim_end()
        .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .next()?;
    (!name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit())).then_some(name)
}

/// Is `line` the head of a definition (`… f(…) {`) rather than a prototype (`… f(…);`)?
fn is_definition(line: &str) -> bool {
    line.trim_end().ends_with('{')
}

/// `text` with every column-0 line naming a function of `defined` written without storage: a
/// definition head everywhere, a prototype only where `prototypes` says so.
fn give_linkage(text: &str, defined: &HashSet<String>, prototypes: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let bare = without_storage(line);
        let rewrite = bare.len() != line.len()
            && function_name(bare).is_some_and(|n| defined.contains(n))
            && (is_definition(bare) || (prototypes && bare.trim_end().ends_with(';')));
        out.push_str(if rewrite { bare } else { line });
    }
    out
}

/// Every identifier `text` holds — a token of letters, digits and `_` not starting with a digit.
/// Strings and comments are not skipped: a name only they mention brings a prototype the unit does
/// not need, which costs nothing but its line.
fn identifiers(text: &str) -> HashSet<&str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty() && !t.starts_with(|c: char| c.is_ascii_digit()))
        .collect()
}

/// The written `unit` as the units of a split build, each starting with a
/// `SPLIT_UNIT <name>*/` line: the shared unit first, then one per file in a fixed order.
/// `runtime_end` closes the runtime in the head: nothing before it is rewritten.  A unit with no
/// `SPLIT_SHARED` mark is returned as it is.
pub(crate) fn into_units(unit: &str, runtime_end: &str) -> String {
    let Some(at) = unit.find(SPLIT_SHARED) else {
        return unit.to_string();
    };
    let head = &unit[..at];
    let rest = &unit[at + SPLIT_SHARED.len()..];
    // What no file owns, and each file's functions, in the order the writer wrote them.
    let mut shared = String::new();
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    let mut cursor = rest;
    while let Some(open) = cursor.find(SPLIT_FN) {
        shared.push_str(&cursor[..open]);
        let after = &cursor[open + SPLIT_FN.len()..];
        let Some(name_end) = after.find("*/") else {
            break;
        };
        let file = after[..name_end].to_string();
        let body_start = &after[name_end + 2..];
        let Some(close) = body_start.find(SPLIT_FN_END) else {
            break;
        };
        files
            .entry(file)
            .or_default()
            .push_str(&body_start[..close]);
        cursor = &body_start[close + SPLIT_FN_END.len()..];
    }
    shared.push_str(cursor);
    // Every function a unit defines gets external linkage.
    let mut defined: HashSet<String> = HashSet::new();
    for text in std::iter::once(&shared).chain(files.values()) {
        for line in text.lines() {
            let bare = without_storage(line);
            if is_definition(bare)
                && let Some(n) = function_name(bare)
            {
                defined.insert(n.to_string());
            }
        }
    }
    let runtime_at = head.find(runtime_end).map_or(0, |a| a + runtime_end.len());
    let after = give_linkage(&head[runtime_at..], &defined, true);
    // The prototypes of the units' functions leave the head: each unit gets those its body names
    // (`own_prototypes`), so a unit's text changes only with what it calls.  One the head's own
    // code names (a table naming a function) stays in the head, for every unit.
    let mut moved: Vec<(String, String)> = Vec::new();
    let mut kept = String::with_capacity(after.len());
    for line in after.split_inclusive('\n') {
        let proto = !is_definition(line)
            && line.trim_end().ends_with(';')
            && function_name(line).is_some_and(|n| defined.contains(n));
        if proto {
            moved.push((function_name(line).unwrap_or_default().to_string(), line.to_string()));
        } else {
            kept.push_str(line);
        }
    }
    let head_names = identifiers(&kept);
    let mut stay = String::new();
    moved.retain(|(n, line)| {
        let named = head_names.contains(n.as_str());
        if named {
            stay.push_str(line);
        }
        !named
    });
    let head = format!("{}{stay}{kept}", &head[..runtime_at]);
    let mut out = String::new();
    let mut put = |name: &str, body: &str| {
        let body = give_linkage(body, &defined, false);
        let names = identifiers(&body);
        out.push_str(SPLIT_UNIT);
        out.push_str(name);
        out.push_str("*/\n");
        out.push_str(&head);
        for (n, line) in &moved {
            if names.contains(n.as_str()) {
                out.push_str(line);
            }
        }
        out.push_str(&body);
    };
    put("shared", &shared);
    for (i, body) in files.values().enumerate() {
        put(&format!("file{i}"), body);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_function_line_names_its_function_and_a_variable_none() {
        assert_eq!(function_name("int64_t n_f(int64_t v0_a) {"), Some("n_f"));
        assert_eq!(function_name("loft_text *n_g(void);"), Some("n_g"));
        assert_eq!(
            function_name("loft_text loft_wp_n_f_2 = { 0, 0, 0 };"),
            None
        );
        assert_eq!(function_name("    if (x) {"), None);
    }

    #[test]
    fn the_units_share_the_head_and_each_file_gets_its_functions() {
        let unit = format!(
            "RT\nEND\nstatic int64_t n_a(void);\nstatic inline int64_t n_b(void);\n{SPLIT_SHARED}int main(void) {{ return n_a(); }}\n{SPLIT_FN}one.loft*/\nstatic int64_t n_a(void) {{\n  return n_b();\n}}\n{SPLIT_FN_END}\n{SPLIT_FN}two.loft*/\nstatic inline int64_t n_b(void) {{\n  return 1;\n}}\n{SPLIT_FN_END}\n"
        );
        let units = into_units(&unit, "END\n");
        let parts: Vec<&str> = units.split(SPLIT_UNIT).filter(|p| !p.is_empty()).collect();
        assert_eq!(parts.len(), 3, "{units}");
        assert!(parts[0].starts_with("shared*/") && parts[0].contains("int main(void)"));
        assert!(
            parts[1].contains("int64_t n_a(void) {")
                && !parts[1].contains("static int64_t n_a(void) {")
        );
        assert!(
            parts[2].contains("int64_t n_b(void) {")
                && !parts[2].contains("inline int64_t n_b(void) {")
        );
        // a prototype goes, without its storage, to the units whose functions name it: `n_b` to
        // the file calling it and the file defining it, not to the shared unit, which calls `n_a`
        assert!(parts[1].contains("\nint64_t n_b(void);") && parts[2].contains("\nint64_t n_b(void);"));
        assert!(!parts[0].contains("n_b(void);") && parts[0].contains("\nint64_t n_a(void);"));
        assert!(parts.iter().all(|p| !p.contains("static inline int64_t n_b")));
        // the runtime before the end mark is untouched
        assert!(parts.iter().all(|p| p.contains("RT\nEND\n")));
    }
}
