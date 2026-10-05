// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! The rule's guard: compiler code reaches the file system, and reasons about path names,
//! only through `file_access`.
//!
//! Every line in `src/` outside this module that touches the file system directly — a
//! `std::fs` call, a `File::open`, `Path::exists()`, `canonicalize`, a separator handled by
//! hand — is counted per file against `direct.baseline`.  A file whose count RISES fails:
//! the new access goes through `file_access` instead.  A file whose count FALLS fails too,
//! until the baseline is lowered with it — `LOFT_BLESS_FILE_ACCESS=1 cargo test --lib
//! file_access::guard` — so the migration's progress is locked in and never given back.
//! Bless refuses while any count is above its baseline.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// What counts as reaching the file system directly.  `fs::` alone catches the calls a
/// `use std::fs;` leaves unqualified.
const PATTERNS: &[&str] = &[
    "std::fs",
    "fs::",
    "File::open",
    "File::create",
    "OpenOptions",
    ".canonicalize(",
    ".exists()",
    ".is_file()",
    ".is_dir()",
    ".read_dir(",
    ".metadata()",
    ".symlink_metadata(",
    "MAIN_SEPARATOR",
    r#"replace('\\', "/")"#,
    // Path-NAME logic through `std::path`: the name operations `file_access` answers by
    // the host flavor's rules.  `.parent()` is left out — a tree's or a scope's parent is
    // spelled the same, and a count that cannot tell them apart would not be honest.
    ".file_name()",
    ".file_stem()",
    ".extension()",
    ".with_extension(",
    ".set_extension(",
    ".with_file_name(",
    ".has_root()",
    ".is_relative()",
];

const BASELINE: &str = "src/file_access/direct.baseline";

/// Is byte `at` of `line` inside a string literal?  Quote parity before it, skipping
/// escaped quotes and a `'"'` character literal — enough for the emitter's one-line
/// `"…std::fs::…"` strings, which are generated code and not the compiler's own access.
fn in_string(line: &str, at: usize) -> bool {
    let b = line.as_bytes();
    let mut inside = false;
    let mut i = 0;
    while i < at {
        match b[i] {
            b'\\' if inside => i += 1,
            b'\'' if !inside && b.get(i + 1) == Some(&b'"') && b.get(i + 2) == Some(&b'\'') => {
                i += 2;
            }
            b'"' => inside = !inside,
            _ => {}
        }
        i += 1;
    }
    inside
}

fn line_accesses(line: &str) -> bool {
    let code = line.trim_start();
    if code.starts_with("//") {
        return false;
    }
    PATTERNS.iter().any(|pat| {
        line.match_indices(pat).any(|(at, _)| {
            // `fs::` must not be the tail of a longer path or name (`std::fs::` is
            // matched by its own pattern; `my_fs::` is not the file system).
            let bare_ok = *pat != "fs::"
                || at == 0
                || !matches!(line.as_bytes()[at - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b':');
            bare_ok && !in_string(line, at)
        })
    })
}

/// Direct-access line counts per `src/` file outside this module.
fn measure(root: &std::path::Path) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let p = entry.path();
            let rel = p
                .strip_prefix(root)
                .map(super::portable)
                .unwrap_or_default();
            if p.is_dir() {
                if rel != "src/file_access" {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "rs")
                && let Ok(text) = std::fs::read_to_string(&p)
            {
                let n = text.lines().filter(|l| line_accesses(l)).count();
                if n > 0 {
                    counts.insert(rel, n);
                }
            }
        }
    }
    counts
}

fn read_baseline(path: &std::path::Path) -> BTreeMap<String, usize> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (n, file) = l.split_once(' ')?;
            Some((file.to_string(), n.parse().ok()?))
        })
        .collect()
}

fn write_baseline(path: &std::path::Path, now: &BTreeMap<String, usize>) {
    let mut out = String::from(
        "# Direct file-system access per compiler file, outside src/file_access/.\n\
         # Only shrinks: see src/file_access/guard.rs.  Format: <lines> <file>\n",
    );
    for (f, n) in now {
        let _ = writeln!(out, "{n} {f}");
    }
    std::fs::write(path, out).expect("write baseline");
}

#[test]
fn compiler_code_reaches_the_file_system_only_through_file_access() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let now = measure(root);
    let base_path = root.join(BASELINE);
    let base = read_baseline(&base_path);
    let bless = std::env::var_os("LOFT_BLESS_FILE_ACCESS").is_some();
    if bless && !base_path.exists() {
        write_baseline(&base_path, &now);
        return;
    }
    let rose: Vec<String> = now
        .iter()
        .filter(|(f, n)| **n > base.get(*f).copied().unwrap_or(0))
        .map(|(f, n)| format!("{f}: {} -> {n}", base.get(f).copied().unwrap_or(0)))
        .collect();
    assert!(
        rose.is_empty(),
        "direct file-system access ROSE outside `file_access` — build a `PathText` and call \
         `file_access::read_to_string` / `write` / `exists` / … instead (src/file_access/mod.rs):\n  {}",
        rose.join("\n  ")
    );
    let fell: Vec<String> = base
        .iter()
        .filter(|(f, n)| now.get(*f).copied().unwrap_or(0) < **n)
        .map(|(f, n)| format!("{f}: {n} -> {}", now.get(f).copied().unwrap_or(0)))
        .collect();
    if bless {
        write_baseline(&base_path, &now);
        return;
    }
    assert!(
        fell.is_empty(),
        "direct file-system access FELL — lock the progress in: \
         LOFT_BLESS_FILE_ACCESS=1 cargo test --lib file_access::guard\n  {}",
        fell.join("\n  ")
    );
}

#[test]
fn the_guard_sees_what_it_must_and_nothing_else() {
    assert!(line_accesses("    let t = std::fs::read_to_string(&p)?;"));
    assert!(line_accesses("    fs::write(&p, x)?;"));
    assert!(line_accesses("    if p.exists() {"));
    assert!(line_accesses(r#"    s.replace('\\', "/")"#));
    assert!(line_accesses(
        "    let ext = p.extension().unwrap_or_default();"
    ));
    assert!(!line_accesses("    let up = scope.parent();"));
    assert!(!line_accesses("    // std::fs::write is not called here"));
    assert!(!line_accesses(
        r#"    out.push_str("std::fs::write(&p, b)?;");"#
    ));
    assert!(!line_accesses("    my_fs::write(x);"));
    assert!(!line_accesses("    let q = path_text.portable();"));
}
