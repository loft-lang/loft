// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! @PLN184's ratchets on what is still platform-specific (B1, C1, D1).  Each is a committed
//! count that may only fall, so the Windows parity work is locked in as it lands.
//!
//! - **B1 — `src/`:** a platform gate (`cfg(unix)`, `cfg(windows)`, `cfg!(windows)`, a
//!   `target_os` of an operating system, a `target_family`) outside `platform` and
//!   `file_access` is a difference handled at a call site.  It belongs in `platform`, which
//!   hides it behind one routine both platforms implement.
//! - **C1 — `tests/`:** the same gates in a test compile it out on one platform or return
//!   early there, so the test passes without testing.  Each one becomes a test that runs on
//!   Windows, or an exemption with a substitute.
//! - **D1 — scripts:** every workflow and the Makefile set `PYTHONUTF8=1`, so a Python script
//!   reads UTF-8 on a Windows runner (whose default is cp1252) as it does on Linux.
//!
//! A count that RISES fails.  A count that FALLS fails too, until the baseline is lowered with
//! it: `LOFT_BLESS_PLATFORM_CENSUS=1 cargo test --lib platform_census`.  Bless refuses while
//! any count is above its baseline.

use crate::file_access::{self, PathText};
use std::collections::BTreeMap;
use std::fmt::Write as _;

const BASELINE: &str = "src/platform_census.baseline";

/// Where a gate is not counted: the modules whose job is to hide a platform difference (a
/// gate there is the design), and the library fixtures.
const HIDES_THE_PLATFORM: &[&str] = &[
    "src/platform.rs",
    "src/file_access",
    "src/platform_census.rs",
    // Library crates the tests build: their platform code is the library's own, and a
    // library's CI proves it.
    "tests/fixtures",
];

/// The operating-system words a gate names.  `wasm32`/`wasi` are targets of the
/// browser and WASI backends, not of a host platform, and are not counted.
const PLATFORM_WORDS: &[&str] = &[
    "unix",
    "windows",
    "target_family = \"unix\"",
    "target_family = \"windows\"",
    "target_os = \"macos\"",
    "target_os = \"linux\"",
    "target_os = \"windows\"",
    "target_os = \"ios\"",
    "target_os = \"android\"",
    "target_os = \"freebsd\"",
];

/// Is `word` at `at` in `code` a whole word (`unix`, not `unix_time`)?
fn whole_word(code: &str, at: usize, word: &str) -> bool {
    let b = code.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    (at == 0 || !ident(b[at - 1])) && b.get(at + word.len()).is_none_or(|c| !ident(*c))
}

/// Does this line gate code on the platform?  Only its code is read: a comment that names
/// a gate is not one.
fn line_gates(line: &str) -> bool {
    let code = line.split("//").next().unwrap_or("");
    let has_cfg = ["cfg(", "cfg!(", "cfg_attr("]
        .iter()
        .any(|c| code.contains(c));
    has_cfg
        && PLATFORM_WORDS.iter().any(|w| {
            code.match_indices(w)
                .any(|(at, _)| w.contains('"') || whole_word(code, at, w))
        })
}

/// Gate lines per `.rs` file under `dir`, outside the modules that hide the platform.
fn measure(root: &PathText, dir: &str, into: &mut BTreeMap<String, usize>) {
    let mut stack = vec![root.join(dir)];
    while let Some(d) = stack.pop() {
        let Ok(entries) = file_access::read_dir(&d) else {
            continue;
        };
        for p in entries {
            let rel = p
                .relative_to(root)
                .map(|r| r.portable())
                .unwrap_or_default();
            if HIDES_THE_PLATFORM.contains(&rel.as_str()) {
                continue;
            }
            if file_access::is_dir(&p) {
                stack.push(p);
            } else if p.portable().ends_with(".rs")
                && let Ok(text) = file_access::read_to_string(&p)
            {
                let n = text.lines().filter(|l| line_gates(l)).count();
                if n > 0 {
                    into.insert(rel, n);
                }
            }
        }
    }
}

/// D1: the workflows and the Makefile that do NOT set `PYTHONUTF8=1` for every job.
fn without_utf8_python(root: &PathText) -> Vec<String> {
    let mut missing = Vec::new();
    let workflows = root.join(".github/workflows");
    for p in file_access::read_dir(&workflows).unwrap_or_default() {
        if !p.portable().ends_with(".yml") {
            continue;
        }
        let text = file_access::read_to_string(&p).unwrap_or_default();
        // The top-level `env:` block: the lines after `env:` up to the next top-level key.
        let set = text
            .lines()
            .skip_while(|l| *l != "env:")
            .skip(1)
            .take_while(|l| l.is_empty() || l.starts_with(' '))
            .any(|l| l.trim() == "PYTHONUTF8: '1'");
        if !set {
            missing.push(p.parts().last().cloned().unwrap_or_default());
        }
    }
    let make = file_access::read_to_string(&root.join("Makefile")).unwrap_or_default();
    if !make.lines().any(|l| l == "export PYTHONUTF8 := 1") {
        missing.push("Makefile".to_string());
    }
    missing.sort();
    missing
}

fn read_baseline(path: &PathText) -> BTreeMap<String, usize> {
    file_access::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (n, file) = l.split_once(' ')?;
            Some((file.to_string(), n.parse().ok()?))
        })
        .collect()
}

fn write_baseline(path: &PathText, now: &BTreeMap<String, usize>) {
    let mut out = String::from(
        "# Platform gates per file outside platform / file_access: src/ is @PLN184 B1, tests/ is C1.\n\
         # Only shrinks: see src/platform_census.rs.  Format: <lines> <file>\n",
    );
    for (f, n) in now {
        let _ = writeln!(out, "{n} {f}");
    }
    file_access::write(path, out).expect("write baseline");
}

#[test]
fn platform_gates_only_fall() {
    let root = PathText::host(env!("CARGO_MANIFEST_DIR"));
    let mut now = BTreeMap::new();
    measure(&root, "src", &mut now);
    measure(&root, "tests", &mut now);
    let base_path = root.join(BASELINE);
    let base = read_baseline(&base_path);
    let bless = std::env::var_os("LOFT_BLESS_PLATFORM_CENSUS").is_some();
    let rose: Vec<String> = now
        .iter()
        .filter(|(f, n)| **n > base.get(*f).copied().unwrap_or(0))
        .map(|(f, n)| format!("{f}: {} -> {n}", base.get(f).copied().unwrap_or(0)))
        .collect();
    assert!(
        rose.is_empty() || (bless && base.is_empty()),
        "a platform gate was ADDED outside `platform` / `file_access`.  In src/, move the \
         difference into `platform` (src/platform.rs) behind one routine both platforms \
         implement; in tests/, make the test run on Windows, or give it an exemption with its \
         substitute (@PLN184 B1/C1):\n  {}",
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
        "platform gates FELL — lock the progress in: \
         LOFT_BLESS_PLATFORM_CENSUS=1 cargo test --lib platform_census\n  {}",
        fell.join("\n  ")
    );
}

#[test]
fn every_python_reads_utf8() {
    let root = PathText::host(env!("CARGO_MANIFEST_DIR"));
    let missing = without_utf8_python(&root);
    assert!(
        missing.is_empty(),
        "these do not set PYTHONUTF8=1 for every job, so a Python script they run decodes \
         files as cp1252 on Windows (@PLN184 D1) — add `PYTHONUTF8: '1'` to the top-level \
         `env:` (or `export PYTHONUTF8 := 1`):\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn the_census_sees_a_gate_and_nothing_else() {
    assert!(line_gates("#[cfg(unix)]"));
    assert!(line_gates("    #[cfg(not(windows))]"));
    assert!(line_gates("    if cfg!(windows) { return; }"));
    assert!(line_gates(r#"#[cfg(target_os = "macos")]"#));
    assert!(line_gates(r#"#[cfg_attr(windows, ignore = "no fork")]"#));
    assert!(line_gates(
        "#[cfg(any(unix, target_family = \"wasm\"))] // x"
    ));
    assert!(!line_gates(r#"#[cfg(target_family = "wasm")]"#));
    assert!(!line_gates("// a `cfg(unix)` handler"));
    assert!(!line_gates("/// gated `#[cfg(windows)]`"));
    assert!(!line_gates(r#"#[cfg(target_arch = "wasm32")]"#));
    assert!(!line_gates(
        r#"#[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]"#
    ));
    assert!(!line_gates("let unix_time = now();"));
    assert!(!line_gates("cfg(unix_like_feature)"));
}
