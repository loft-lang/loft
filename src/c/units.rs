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
//! A function defined in one unit and named from another (or from the head) needs external
//! linkage, so it loses its `static` / `inline` / `LOFT_HOT_INLINE`, at its definition and at
//! its prototype.  Every other function keeps its storage: the C compiler inlines it and drops
//! what nothing calls, as in the one-unit build.  The runtime keeps its own: every unit carries
//! the runtime's hot part (`split_runtime`; the cold part is one unit of its own, the split
//! build's exception to `@C169`), and its state is declared once through `LOFT_STATE` — the shared unit is
//! compiled with `LOFT_SPLIT_OWN`, every other with `LOFT_SPLIT_USE`.  A file-scope variable a
//! body moved out (a work-text slot) keeps its `static`: it belongs to its function's unit.

use super::{SPLIT_FN, SPLIT_FN_END, SPLIT_SHARED, SPLIT_UNIT};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

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
    // `T (*name)(…)` is a variable holding a function, not a function
    if line[paren + 1..].starts_with('*') {
        return None;
    }
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

/// The runtime cut in two (@C169's split-build exception): every unit carries the HOT runtime —
/// its types, its macros and its inline-hinted functions whole — and one unit, compiled once,
/// carries the COLD functions, the plain `static` ones.  At the check tier gcc inlines a plain
/// `static` function only where it is called once, so a unit loses almost nothing by calling it
/// instead; the inline-hinted helpers (store access, references, the frame) stay where the
/// generated code can inline them.
///
/// Returns `(hot, cold, callbacks)`: `hot` is `runtime` with each cold function a prototype,
/// `cold` is `runtime` with each cold function defined with external linkage, and `callbacks`
/// the functions the runtime declares and the program defines (`loft_init`): the cold unit calls
/// them, so they are declared external in both halves and the program's definition is too.  A function head is one
/// column-0 line ending in `{` and its body ends at the next column-0 `}`; a definition the writer
/// spells otherwise keeps its `static` in every unit, which costs time, never correctness.
fn split_runtime(runtime: &str) -> (String, String, HashSet<String>) {
    let lines: Vec<&str> = runtime.split_inclusive('\n').collect();
    let plain = |line: &str| -> Option<String> {
        let rest = line.strip_prefix("static ")?;
        if rest.starts_with(|c: char| c.is_ascii_uppercase())
            || rest.starts_with("inline ")
            || rest.starts_with("const ")
            || rest.starts_with("struct ")
            || rest.starts_with("union ")
            || rest.starts_with("enum ")
        {
            return None;
        }
        function_name(rest).map(str::to_string)
    };
    let mut cold_names: HashSet<String> = HashSet::new();
    let mut defined: HashSet<&str> = HashSet::new();
    for line in &lines {
        // a definition of any form: a body opened on its line (`{` at its end, or a whole body
        // on one line) — everything but a prototype
        if !line.trim_end().ends_with(';')
            && let Some(n) = function_name(without_storage(line))
        {
            defined.insert(n);
        }
        if is_definition(line)
            && let Some(n) = plain(line)
        {
            cold_names.insert(n);
        }
    }
    let callbacks: HashSet<String> = lines
        .iter()
        .filter(|l| !is_definition(l) && l.trim_end().ends_with(';'))
        .filter_map(|l| plain(l))
        .filter(|n| !defined.contains(n.as_str()))
        .collect();
    let mut hot = String::with_capacity(runtime.len());
    let mut cold = String::with_capacity(runtime.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let name = plain(line).filter(|n| cold_names.contains(n) || callbacks.contains(n));
        match name {
            Some(_) if is_definition(line) => {
                let bare = &line["static ".len()..];
                let head = bare.trim_end().trim_end_matches('{').trim_end();
                hot.push_str(head);
                hot.push_str(";\n");
                cold.push_str(bare);
                i += 1;
                while i < lines.len() {
                    cold.push_str(lines[i]);
                    let end = lines[i].starts_with('}');
                    i += 1;
                    if end {
                        break;
                    }
                }
                continue;
            }
            Some(_) if line.trim_end().ends_with(';') => {
                // its prototype: external, as its definition now is
                hot.push_str(&line["static ".len()..]);
                cold.push_str(&line["static ".len()..]);
            }
            _ => {
                hot.push_str(line);
                cold.push_str(line);
            }
        }
        i += 1;
    }
    (hot, cold, callbacks)
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
    // The functions each unit defines, and which unit that is.
    let bodies: Vec<&String> = std::iter::once(&shared).chain(files.values()).collect();
    let mut definer: HashMap<String, usize> = HashMap::new();
    let mut heads: HashMap<String, String> = HashMap::new();
    for (u, text) in bodies.iter().enumerate() {
        for line in text.lines() {
            let bare = without_storage(line);
            if is_definition(bare)
                && let Some(n) = function_name(bare)
            {
                definer.insert(n.to_string(), u);
                heads
                    .entry(n.to_string())
                    .or_insert_with(|| line.to_string());
            }
        }
    }
    let runtime_at = head.find(runtime_end).map_or(0, |a| a + runtime_end.len());
    let (hot, cold, callbacks) = split_runtime(&head[..runtime_at]);
    // The prototypes of the units' functions leave the head: each unit gets those its body names,
    // so a unit's text changes only with what it calls.  One the head's own code names (a table
    // naming a function) stays in the head, for every unit.
    let mut moved: Vec<(String, String)> = Vec::new();
    let mut kept = String::with_capacity(head.len() - runtime_at);
    for line in head[runtime_at..].split_inclusive('\n') {
        let bare = without_storage(line);
        let proto = !is_definition(bare)
            && bare.trim_end().ends_with(';')
            && function_name(bare).is_some_and(|n| definer.contains_key(n));
        if proto {
            moved.push((
                function_name(bare).unwrap_or_default().to_string(),
                line.to_string(),
            ));
        } else {
            kept.push_str(line);
        }
    }
    // A callback the head defines (`loft_init`) is called by the cold runtime: its definition is
    // the shared unit's, with external linkage, and every other unit keeps the runtime's prototype.
    let mut called_back = String::new();
    let mut rest_of_head = String::with_capacity(kept.len());
    let mut lines = kept.split_inclusive('\n');
    while let Some(line) = lines.next() {
        let bare = without_storage(line);
        let named = function_name(bare).is_some_and(|n| callbacks.contains(n));
        if named && !is_definition(bare) && !bare.trim_end().ends_with(';') {
            // a whole definition on its line
            called_back.push_str(bare);
        } else if named && is_definition(bare) {
            called_back.push_str(bare);
            for body in lines.by_ref() {
                called_back.push_str(body);
                if body.starts_with('}') {
                    break;
                }
            }
        } else {
            rest_of_head.push_str(line);
        }
    }
    let mut kept = rest_of_head;
    // A variable the head declares `static` is a copy in every unit: one unit's write is not
    // another's read.  Program state is declared through `LOFT_STATE` (one definition, `extern`
    // elsewhere); a mutable `static` left in the head stops the build here, loudly, rather than
    // leaving each unit its own copy.
    let mut unshared: Vec<String> = Vec::new();
    for line in hot.lines().chain(kept.lines()) {
        if let Some(rest) = line.strip_prefix("static ")
            && !rest.starts_with("const ")
            && !rest.starts_with("inline ")
            && function_name(rest).is_none()
            && let Some(name) = rest
                .split(['=', '[', ';'])
                .next()
                .and_then(|d| d.trim_end().rsplit([' ', '*']).next())
        {
            unshared.push(name.to_string());
        }
    }
    if let Some(name) = unshared.first() {
        let _ = writeln!(
            kept,
            "#error \"a split build cannot share the static variable `{name}`: declare it through LOFT_STATE\""
        );
    }
    let head_names = identifiers(&kept);
    // A function needs external linkage only when something outside its own unit names it: the
    // head, or another unit.  Every other keeps its storage, so the C compiler still inlines it
    // and drops what nothing calls, as in the one-unit build.
    let mut exported: HashSet<String> = HashSet::new();
    for (u, text) in bodies.iter().enumerate() {
        for n in identifiers(text) {
            if definer.get(n).is_some_and(|&home| home != u) {
                exported.insert(n.to_string());
            }
        }
    }
    exported.extend(
        definer
            .keys()
            .filter(|n| head_names.contains(n.as_str()) || callbacks.contains(n.as_str()))
            .cloned(),
    );
    // A function another unit names that the head declares nowhere (a helper the writer defined
    // before its callers, as the function-value calls are) is declared from its definition.
    let declared: HashSet<String> = moved.iter().map(|(n, _)| n.clone()).collect();
    let mut synthesized: Vec<(String, String)> = exported
        .iter()
        .filter(|n| !declared.contains(*n))
        .filter_map(|n| {
            let def = heads.get(n)?;
            let proto = format!("{};\n", def.trim_end().trim_end_matches('{').trim_end());
            Some((n.clone(), proto))
        })
        .collect();
    synthesized.sort();
    moved.extend(synthesized);
    let mut stay = String::new();
    moved.retain(|(n, line)| {
        let named = head_names.contains(n.as_str());
        if named {
            stay.push_str(line);
        }
        !named
    });
    let moved: Vec<(String, String)> = moved
        .into_iter()
        .map(|(n, line)| {
            let line = give_linkage(&line, &exported, true);
            (n, line)
        })
        .collect();
    let head = format!("{hot}{}{kept}", give_linkage(&stay, &exported, true));
    let mut out = String::new();
    let mut put = |name: &str, body: &str| {
        let body = give_linkage(body, &exported, false);
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
    put("shared", &format!("{called_back}{shared}"));
    for (i, body) in files.values().enumerate() {
        put(&format!("file{i}"), body);
    }
    // The cold runtime, alone: it names nothing the program wrote.
    out.push_str(SPLIT_UNIT);
    out.push_str("runtime*/\n");
    out.push_str(&cold);
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
        assert_eq!(parts.len(), 4, "{units}");
        assert!(parts[3].starts_with("runtime*/"));
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
        assert!(
            parts[1].contains("\nint64_t n_b(void);") && parts[2].contains("\nint64_t n_b(void);")
        );
        assert!(!parts[0].contains("n_b(void);") && parts[0].contains("\nint64_t n_a(void);"));
        assert!(
            parts
                .iter()
                .all(|p| !p.contains("static inline int64_t n_b"))
        );
        // the runtime before the end mark is untouched
        assert!(parts.iter().all(|p| p.contains("RT\nEND\n")));
    }

    #[test]
    fn the_cold_runtime_is_one_unit_and_the_hot_part_stays_in_every_unit() {
        let rt = "static void loft_init(loft_cx *cx);\n\
                  static inline int64_t loft_hot(int64_t a) { return a; }\n\
                  static int64_t loft_cold(int64_t a);\n\
                  static int64_t loft_cold(int64_t a) {\n    return loft_hot(a) + 1;\n}\n\
                  static const int loft_table[1] = { 0 };\n\
                  LOFT_HOT_INLINE int64_t loft_fast(void) {\n    return 2;\n}\n";
        let (hot, cold, callbacks) = split_runtime(rt);
        // a function the runtime declares and the program defines is external in both halves
        assert_eq!(callbacks, HashSet::from(["loft_init".to_string()]));
        assert!(
            hot.starts_with("void loft_init(loft_cx *cx);") && cold.starts_with("void loft_init(")
        );
        // the hot part keeps the inline helpers whole and declares the cold function
        assert!(hot.contains("static inline int64_t loft_hot(int64_t a) { return a; }"));
        assert!(hot.contains("LOFT_HOT_INLINE int64_t loft_fast(void) {\n    return 2;\n}"));
        assert!(hot.contains("\nint64_t loft_cold(int64_t a);\n"));
        assert!(!hot.contains("return loft_hot(a) + 1"), "{hot}");
        assert!(!hot.contains("static int64_t loft_cold"), "{hot}");
        // the cold unit defines it with external linkage, beside everything it calls
        assert!(
            cold.contains("\nint64_t loft_cold(int64_t a) {\n    return loft_hot(a) + 1;\n}\n")
        );
        assert!(cold.contains("static inline int64_t loft_hot"));
        assert!(!cold.contains("static int64_t loft_cold"), "{cold}");
        // a table is no function: every unit keeps its own
        assert!(
            hot.contains("static const int loft_table[1]")
                && cold.contains("static const int loft_table[1]")
        );
    }
}
