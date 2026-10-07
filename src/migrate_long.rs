// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F48 — loft CLI (--migrate-long source rewriter)

//! `--migrate-long` — source rewriter for the C54 deprecation of `long`.
//!
//! Walks `.loft` source files and rewrites:
//! - `long` as a standalone word (type position, variant body) → `integer`.
//! - Integer literals with `l` / `L` suffix (`42l`, `1000L`) → plain
//!   (`42`, `1000`).  Post-C54.A both produce i64 values.
//!
//! **Preserves**:
//! - Identifiers containing `long` (e.g. `long_value`, `longitude`) —
//!   matched via word boundaries.
//! - String and backtick literals — scanned for, skipped entirely.
//! - Line comments (`// …`) and the `#rust"..."` annotation bodies —
//!   conservative skip-through since an unnoticed substitution inside
//!   a `#rust` string would break native codegen.
//!
//! Usage: `loft --migrate-long <path-or-dir>` (or `--dry-run` to print
//! diffs without writing).

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Rewrite a single source string.  Returns the transformed source.
#[must_use]
pub fn rewrite_source(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Pass through string literals (", `, ') unchanged.
        if c == '"' || c == '`' || c == '\'' {
            let quote = c;
            out.push(c);
            i += 1;
            while i < chars.len() {
                out.push(chars[i]);
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    out.push(chars[i]);
                    i += 1;
                    continue;
                }
                if chars[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // Pass through single-line comments (// ...).
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            while i < chars.len() && chars[i] != '\n' {
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        // `#rust "..."` annotation — skip through its opening quote +
        // contents.  Already covered by the string-literal branch above
        // when the `#rust` sequence feeds into `"` — so nothing special
        // needed here; the standard string-literal pass-through handles it.

        // Integer literal with `l`/`L` suffix: rewrite NNN[lL] → NNN.
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                i += 1;
            }
            // Check if followed by 'l' or 'L' AND not continuing into a word.
            if i < chars.len() && (chars[i] == 'l' || chars[i] == 'L') {
                let is_suffix = i + 1 == chars.len() || !is_word_continuation(chars[i + 1]);
                if is_suffix {
                    // Emit digits (including any underscores), skip the suffix.
                    for ch in &chars[start..i] {
                        out.push(*ch);
                    }
                    i += 1;
                    continue;
                }
            }
            for ch in &chars[start..i] {
                out.push(*ch);
            }
            continue;
        }
        // Identifier start — check if it's the `long` keyword.
        if is_word_start(c) {
            let start = i;
            while i < chars.len() && is_word_continuation(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if word == "long" {
                out.push_str("integer");
            } else {
                out.push_str(&word);
            }
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

fn is_word_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_word_continuation(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Rewrite every `.loft` file under `path` (file or directory).  Returns
/// the count of (files_scanned, files_modified).  If `dry_run`, no files
/// are written; diffs are printed to stdout instead.
///
/// # Errors
/// Returns the first I/O error encountered.
pub fn migrate_path(path: &Path, dry_run: bool) -> io::Result<(usize, usize)> {
    let mut scanned = 0usize;
    let mut modified = 0usize;
    let meta = fs::metadata(path)?;
    if meta.is_file() {
        if path.extension().and_then(|e| e.to_str()) == Some("loft") {
            let (s, m) = migrate_file(path, dry_run)?;
            scanned += s;
            modified += m;
        }
    } else if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_dir() {
                let (s, m) = migrate_path(&p, dry_run)?;
                scanned += s;
                modified += m;
            } else if p.extension().and_then(|e| e.to_str()) == Some("loft") {
                let (s, m) = migrate_file(&p, dry_run)?;
                scanned += s;
                modified += m;
            }
        }
    }
    Ok((scanned, modified))
}

fn migrate_file(path: &Path, dry_run: bool) -> io::Result<(usize, usize)> {
    let src = fs::read_to_string(path)?;
    let out = rewrite_source(&src);
    if out == src {
        return Ok((1, 0));
    }
    if dry_run {
        let stdout = io::stdout();
        let mut h = stdout.lock();
        writeln!(h, "--- {} (would rewrite)", path.display())?;
        for (i, (b, a)) in src.lines().zip(out.lines()).enumerate() {
            if b != a {
                writeln!(h, "  {:>4}: - {}", i + 1, b)?;
                writeln!(h, "  {:>4}: + {}", i + 1, a)?;
            }
        }
    } else {
        fs::write(path, &out)?;
    }
    Ok((1, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_long_type_keyword() {
        let src = "fn foo(x: long) -> long { x }";
        let out = rewrite_source(src);
        assert_eq!(out, "fn foo(x: integer) -> integer { x }");
    }

    #[test]
    fn rewrite_literal_suffix() {
        let src = "let x = 42l; let y = 1000L;";
        let out = rewrite_source(src);
        assert_eq!(out, "let x = 42; let y = 1000;");
    }

    #[test]
    fn preserve_long_in_identifier() {
        let src = "fn long_value(x: integer) -> integer { x }";
        let out = rewrite_source(src);
        assert_eq!(out, "fn long_value(x: integer) -> integer { x }");
    }

    #[test]
    fn preserve_l_suffix_inside_identifier() {
        // `x5l` is not a literal — no suffix replacement.
        let src = "let x5l = 10;";
        let out = rewrite_source(src);
        assert_eq!(out, "let x5l = 10;");
    }

    #[test]
    fn preserve_inside_string() {
        let src = r#"println("long is 42l");"#;
        let out = rewrite_source(src);
        assert_eq!(out, r#"println("long is 42l");"#);
    }

    #[test]
    fn preserve_inside_comment() {
        let src = "// long is 42l\nfn foo() {}";
        let out = rewrite_source(src);
        assert_eq!(out, "// long is 42l\nfn foo() {}");
    }

    #[test]
    fn rewrite_mixed() {
        let src = "fn sum(a: long, b: long) -> long { a + b + 42l }";
        let out = rewrite_source(src);
        assert_eq!(
            out,
            "fn sum(a: integer, b: integer) -> integer { a + b + 42 }"
        );
    }

    #[test]
    fn preserve_underscore_separators_in_literal() {
        let src = "let x = 1_000_000l;";
        let out = rewrite_source(src);
        assert_eq!(out, "let x = 1_000_000;");
    }

    #[test]
    fn preserve_longitude_identifier() {
        let src = "longitude = 42.5;";
        let out = rewrite_source(src);
        assert_eq!(out, "longitude = 42.5;");
    }
}
