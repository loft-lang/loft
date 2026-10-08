// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I87 — Auto-use / lazy library triggers

//! Tier-0 lazy auto-`use`: a lightweight pre-scan of a loft source file for
//! `lib::…` references (lib_plans/59-lazy-stdlib).
//!
//! The parser loads explicit `use lib;` libraries at the TOP of each pass,
//! before any definition is created — which is the only place a library can be
//! loaded without the two-pass model later complaining "cannot redefine".  A
//! `lib::method` reference, however, is only discovered deep in a function
//! body.  This scan bridges the gap: run once over the file BEFORE the
//! definitions are parsed, collect every `lib::` prefix, and let the use-region
//! load those libraries — so `regex::matches(line)` works with no `use regex;`.
//!
//! Why a hand scan and not the lexer: it must see references inside
//! **format-string interpolations** (`"{regex::matches(line)}"` — the common
//! case), so it scans inside `{…}` within strings.  It deliberately stays
//! conservative: a missed reference just falls back to the normal "unknown"
//! error (the user adds `use`), and a stray hit on a name that is not a real
//! library is filtered out later by the resolver.  Two filters keep it tight:
//!
//!   * **lowercase only** — loft libraries are lowercase; types/enums are
//!     capitalised, so `Color::Red` / `Shape::Circle` are never candidates.
//!   * comments and plain (non-interpolated) string text are skipped.

/// Package names reserved for a language namespace — a library may not claim
/// one, because it resolves to a built-in namespace rather than a package
/// (@PLN13 / [C101](../doc/claude/DESIGN_DECISIONS.md)):
///
///   * `std` — the standard library. `std::name` is the stdlib's qualified form
///     and the escape hatch when a bare name is shadowed (by a user definition
///     or a `use lib::*` import — see C97/C98). It is already bound in `data.rs`
///     (`use_names["std"] = STD_SOURCE`), so a `std` library could never override
///     it — reserving the *name* keeps it from being claimed or confused.
///   * `core` — held for a possible future stdlib-core split (cf. Rust). Free to
///     reserve pre-freeze; a compat break to add after the contract-1 freeze.
///
/// Enforced at `loft new` (creation) and honoured by the registry submission
/// gate. This is the canonical list.
pub const RESERVED_PACKAGE_NAMES: &[&str] = &["std", "core"];

/// True when `name` is a [`RESERVED_PACKAGE_NAMES`] entry, i.e. a library may
/// not be named `name` because it collides with a language namespace.
#[must_use]
pub fn is_reserved_package_name(name: &str) -> bool {
    RESERVED_PACKAGE_NAMES.contains(&name)
}

/// True when `name` may be used as a package name: lowercase ASCII letters,
/// digits and `_`, and not empty.
///
/// Call this on any name that arrives from a **manifest** before using it — a
/// manifest is data, and on an installed or downloaded package it is data
/// somebody else wrote. Two things depend on it:
///
/// * the name becomes a directory component (`~/.loft/lib/<name>`), and the
///   character set here admits no `/`, `\`, or `.`, so a name cannot walk out
///   of the directory it is joined into;
/// * `use <name>` has to be able to reach the package afterwards, and that
///   spelling is a loft identifier.
///
/// The same rule therefore covers a `[library] native` stem, which becomes a
/// filename in a per-package directory.
#[must_use]
pub fn is_valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[inline]
fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

#[inline]
fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Collect the distinct lowercase `name` prefixes that appear as `name::` in
/// `content`, scanning code and format-string interpolations but skipping line
/// comments and plain string text.  Order-preserving, de-duplicated.
#[must_use]
pub fn scan_qualified_lib_refs(content: &str) -> Vec<String> {
    let b = content.as_bytes();
    let n = b.len();
    let mut refs: Vec<String> = Vec::new();
    let mut i = 0usize;
    // String state: when `in_string`, plain text is skipped; only code inside
    // `{…}` interpolation (brace_depth > 0) is scanned.
    let mut in_string = false;
    let mut brace_depth = 0usize;

    let record = |name: &str, refs: &mut Vec<String>| {
        // lowercase first char → library candidate (types are capitalised)
        if name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            && !refs.iter().any(|r| r == name)
        {
            refs.push(name.to_string());
        }
    };

    while i < n {
        let c = b[i];
        if in_string {
            match c {
                b'\\' => i += 2, // escape — skip the next byte
                b'{' => {
                    brace_depth += 1;
                    i += 1;
                }
                b'}' => {
                    brace_depth = brace_depth.saturating_sub(1);
                    i += 1;
                }
                b'"' if brace_depth == 0 => {
                    in_string = false;
                    i += 1;
                }
                _ if brace_depth > 0 && is_ident_start(c) => {
                    let start = i;
                    while i < n && is_ident_char(b[i]) {
                        i += 1;
                    }
                    if i + 1 < n && b[i] == b':' && b[i + 1] == b':' {
                        record(&content[start..i], &mut refs);
                    }
                }
                _ => i += 1,
            }
        } else {
            // line comment
            if c == b'/' && i + 1 < n && b[i + 1] == b'/' {
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if c == b'"' {
                in_string = true;
                brace_depth = 0;
                i += 1;
                continue;
            }
            if is_ident_start(c) {
                let start = i;
                while i < n && is_ident_char(b[i]) {
                    i += 1;
                }
                if i + 1 < n && b[i] == b':' && b[i + 1] == b':' {
                    record(&content[start..i], &mut refs);
                }
                continue;
            }
            i += 1;
        }
    }
    refs
}

/// Collect the distinct method names invoked as `.name(` in `content` —
/// the Tier-1 trigger candidates (`line.matches(…)` → `matches`).  Same
/// scanning discipline as [`scan_qualified_lib_refs`]: code + format-string
/// interpolations, skipping comments, plain string text, and `..` ranges.
/// Order-preserving, de-duplicated.  Over-collects (every `.method(` call,
/// including stdlib ones); the caller filters against the trigger map.
#[must_use]
pub fn scan_method_calls(content: &str) -> Vec<String> {
    let b = content.as_bytes();
    let n = b.len();
    let mut calls: Vec<String> = Vec::new();
    let mut i = 0usize;
    let mut in_string = false;
    let mut brace_depth = 0usize;

    let record = |name: &str, calls: &mut Vec<String>| {
        if !name.is_empty() && !calls.iter().any(|c| c == name) {
            calls.push(name.to_string());
        }
    };
    // A `.name(` whose `.` is not part of `..` (range) or a float.
    let try_method = |i: &mut usize, calls: &mut Vec<String>| {
        // at b[*i] == b'.'
        if *i + 1 < n && b[*i + 1] == b'.' {
            *i += 2; // `..` range — skip
            return;
        }
        let mut j = *i + 1;
        let start = j;
        while j < n && is_ident_char(b[j]) {
            j += 1;
        }
        if j > start && j < n && b[j] == b'(' {
            record(&content[start..j], calls);
        }
        *i = if j > start { j } else { *i + 1 };
    };

    while i < n {
        let c = b[i];
        if in_string {
            match c {
                b'\\' => i += 2,
                b'{' => {
                    brace_depth += 1;
                    i += 1;
                }
                b'}' => {
                    brace_depth = brace_depth.saturating_sub(1);
                    i += 1;
                }
                b'"' if brace_depth == 0 => {
                    in_string = false;
                    i += 1;
                }
                b'.' if brace_depth > 0 => try_method(&mut i, &mut calls),
                _ => i += 1,
            }
        } else {
            if c == b'/' && i + 1 < n && b[i + 1] == b'/' {
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if c == b'"' {
                in_string = true;
                brace_depth = 0;
                i += 1;
                continue;
            }
            if c == b'.' {
                try_method(&mut i, &mut calls);
                continue;
            }
            i += 1;
        }
    }
    calls
}

/// The CamelCase names a file DECLARES as types — `struct X`, `enum X`, `type X` at the top level
/// of the file — outside comments, strings and every `{ … }` block.  Read before pass 1 so a
/// program type that shares a standard-library type's name owns that name in its file from the
/// first line on (C101: the program's definition shadows the stdlib's wherever it is written),
/// not only below its declaration.  Order-preserving, de-duplicated.
#[must_use]
/// Type names a file REFERENCES, for the type half of the auto-`use` trigger surface
/// (`triggers.rs` § Type triggers): a capitalised identifier in a type position — after
/// `:`, `->`, `as` or `<`, or before `{` (a record literal) or `.Variant`.  Strings and
/// comments are skipped as `scan_method_calls` skips them.  A name a file DEFINES is still
/// reported (`c: Command` in the file that declares `Command`); the caller drops the names
/// it already knows, as it does for `lib::` references.  Deduplicated, in first-seen order.
pub fn scan_type_refs(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut types: Vec<String> = Vec::new();
    let mut i = 0usize;
    // The last non-blank bytes before the identifier, as a short text: `:`, `->`, `as`, `<`.
    let mut prev = String::new();
    while i < len {
        let ch = bytes[i];
        if ch == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if ch == b'"' || ch == b'`' {
            // A literal, with `{…}` interpolation nested inside a `"` one.
            let quote = ch;
            let mut depth = 0usize;
            i += 1;
            while i < len {
                match bytes[i] {
                    b'\\' if quote == b'"' => i += 1,
                    b'{' if quote == b'"' => depth += 1,
                    b'}' if quote == b'"' => depth = depth.saturating_sub(1),
                    byte if byte == quote && depth == 0 => break,
                    _ => {}
                }
                i += 1;
            }
            i += 1;
            prev.clear();
            continue;
        }
        if is_ident_start(ch) {
            let start = i;
            while i < len && is_ident_char(bytes[i]) {
                i += 1;
            }
            let word = &content[start..i];
            if word.as_bytes()[0].is_ascii_uppercase() {
                let mut after = i;
                while after < len && (bytes[after] == b' ' || bytes[after] == b'\t') {
                    after += 1;
                }
                let before = matches!(prev.as_str(), ":" | "->" | "as" | "<");
                let literal = after < len && bytes[after] == b'{';
                let variant = after + 1 < len
                    && bytes[after] == b'.'
                    && bytes[after + 1].is_ascii_uppercase();
                if (before || literal || variant) && !types.iter().any(|t| t == word) {
                    types.push(word.to_string());
                }
            }
            prev = word.to_string();
            continue;
        }
        if ch == b' ' || ch == b'\t' || ch == b'\n' || ch == b'\r' {
            i += 1;
            continue;
        }
        if ch == b'-' && i + 1 < len && bytes[i + 1] == b'>' {
            prev = "->".to_string();
            i += 2;
            continue;
        }
        prev = (ch as char).to_string();
        i += 1;
    }
    types
}

pub fn scan_type_declarations(content: &str) -> Vec<String> {
    let b = content.as_bytes();
    let n = b.len();
    let mut names: Vec<String> = Vec::new();
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut keyword_pending = false;
    while i < n {
        let c = b[i];
        if c == b'/' && i + 1 < n && b[i + 1] == b'/' {
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'"' {
            // A string literal, interpolations included, holds no declaration.
            i += 1;
            let mut inner = 0usize;
            while i < n {
                match b[i] {
                    b'\\' => i += 2,
                    b'{' => {
                        inner += 1;
                        i += 1;
                    }
                    b'}' => {
                        inner = inner.saturating_sub(1);
                        i += 1;
                    }
                    b'"' if inner == 0 => {
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            keyword_pending = false;
            continue;
        }
        // A character literal (`'{'`, `'\\''`) is no brace.
        if c == b'\'' {
            let close = if i + 1 < n && b[i + 1] == b'\\' {
                i + 3
            } else {
                i + 2
            };
            if close < n && b[close] == b'\'' {
                i = close + 1;
                keyword_pending = false;
                continue;
            }
        }
        match c {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if is_ident_start(c) {
            let start = i;
            while i < n && is_ident_char(b[i]) {
                i += 1;
            }
            let word = &content[start..i];
            if keyword_pending {
                if word.as_bytes()[0].is_ascii_uppercase() && !names.iter().any(|w| w == word) {
                    names.push(word.to_string());
                }
                keyword_pending = false;
            } else {
                keyword_pending = depth == 0 && matches!(word, "struct" | "enum" | "type");
            }
            continue;
        }
        if !c.is_ascii_whitespace() {
            keyword_pending = false;
        }
        i += 1;
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_top_level_type_declarations() {
        let src = "// struct Commented { }\nfn f() -> File { file(\"struct Quoted\") }\n\
                   pub struct File { name: text, open: character = '{' }\n\
                   enum Format { A, B }\nfn g() { struct_like = 1; }\ntype Meters = float;\n";
        assert_eq!(
            scan_type_declarations(src),
            vec!["File", "Format", "Meters"]
        );
    }

    #[test]
    fn finds_type_references_in_type_positions() {
        let src = "// c: Commented = 1\nfn f(a: Alpha, n: integer) -> Beta? { x = (\"g: Quoted\" as Gamma); \
                   d = Delta { k: 1 }; e = Epsilon.One; v: vector<Zeta> = []; lower {} \
                   println(\"{Eta.Two}\"); Theta }";
        assert_eq!(
            scan_type_refs(src),
            vec!["Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta"]
        );
    }

    #[test]
    fn finds_method_calls() {
        assert_eq!(
            scan_method_calls("fn f() { x = line.matches(\"a\"); n = v.len() }"),
            vec!["matches".to_string(), "len".to_string()]
        );
    }

    #[test]
    fn method_calls_in_format_string_and_skip_ranges() {
        assert_eq!(
            scan_method_calls("print(\"{s.matches(p)}\"); for i in 0..3 {}"),
            vec!["matches".to_string()]
        );
    }

    #[test]
    fn a_package_name_is_lowercase_ascii_digits_and_underscore() {
        for ok in [
            "goodpkg",
            "hex_world",
            "loft_graphics_native",
            "arcf",
            "loft_b3",
            "a",
        ] {
            assert!(is_valid_package_name(ok), "{ok} is a real package name");
        }
        // Every rejection here is also a path component that could leave the directory
        // it is joined into, or a spelling no `use` could reach.
        for bad in [
            "",
            "..",
            ".",
            "../../escaped",
            "a/b",
            "a\\b",
            "/etc",
            "Cargo",
            "my-pkg",
            "a b",
        ] {
            assert!(!is_valid_package_name(bad), "{bad:?} must be refused");
        }
    }

    #[test]
    fn finds_plain_code_reference() {
        assert_eq!(
            scan_qualified_lib_refs("fn f() { x = regex::find(\"a\", line) }"),
            vec!["regex".to_string()]
        );
    }

    #[test]
    fn finds_reference_inside_format_string() {
        // The common case: a `lib::` call inside `{…}` interpolation.
        assert_eq!(
            scan_qualified_lib_refs("print(\"hit at {regex::find(p, s)}\\n\")"),
            vec!["regex".to_string()]
        );
    }

    #[test]
    fn skips_plain_string_text_and_comments() {
        // `::` in plain (non-interpolated) text and in a comment are NOT refs.
        let unexpected = scan_qualified_lib_refs("// see regex::docs for more");
        assert!(unexpected.is_empty(), "{unexpected:?}");
        let unexpected = scan_qualified_lib_refs("s = \"path is a::b not a lib\"");
        assert!(unexpected.is_empty(), "{unexpected:?}");
    }

    #[test]
    fn skips_capitalised_enum_variants() {
        // `Enum::Variant` (capitalised) is a type, never a library.
        let unexpected = scan_qualified_lib_refs("match c { Color::Red => 1, Shape::Box => 2 }");
        assert!(unexpected.is_empty(), "{unexpected:?}");
    }

    #[test]
    fn dedups_and_keeps_order() {
        assert_eq!(
            scan_qualified_lib_refs("a::x; b::y; a::z"),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn mixed_code_and_interpolation() {
        let src = "fn f() {\n  if regex::matches(p, s) { print(\"{time::now()}\") }\n}";
        assert_eq!(
            scan_qualified_lib_refs(src),
            vec!["regex".to_string(), "time".to_string()]
        );
    }
}
