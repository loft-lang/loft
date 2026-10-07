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

/// Does the last name end in `.ext`?  Asked of the parts: `PathText::extension` reads to the
/// text guard as `std::path`'s.
fn has_extension(p: &PathText, ext: &str) -> bool {
    p.parts()
        .last()
        .and_then(|n| n.rsplit_once('.'))
        .is_some_and(|(stem, e)| !stem.is_empty() && e == ext)
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
            } else if has_extension(&p, "rs")
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
        if !has_extension(&p, "yml") {
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
    let make = file_access::read_to_string(root.join("Makefile")).unwrap_or_default();
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

// ── D2/D3 — the scripts' Windows hazards ───────────────────────────────────────────────────
// Until a script is ported to loft (@PLN179), what makes it fail on Windows only falls.
// Python (D2): a text `open` / `read_text` / `write_text` without an encoding (cp1252 on a
// Windows runner whenever PYTHONUTF8 is not set), and a path split by hand on `/`.  Shell (D3):
// a tool Git Bash does not have (`flock`, `pgrep`, `pkill`, `xvfb-run`, `valgrind`), a
// hard-coded `/tmp/`, and GNU's `sed -i`.

const SCRIPTS_BASELINE: &str = "src/platform_census_scripts.baseline";

/// The code part of a script line: what precedes a comment that starts the line or follows
/// whitespace.
fn script_code(line: &str) -> &str {
    let t = line.trim_start();
    if t.starts_with('#') {
        return "";
    }
    line.find(" #").map_or(line, |at| &line[..at])
}

fn word_at(code: &str, at: usize, len: usize) -> bool {
    let b = code.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.';
    (at == 0 || !ident(b[at - 1])) && b.get(at + len).is_none_or(|c| !ident(*c))
}

fn python_hazard(line: &str) -> bool {
    let code = script_code(line);
    if code.contains("encoding") {
        return code.contains("split('/')") || code.contains("split(\"/\")");
    }
    let binary = [
        "'rb'", "\"rb\"", "'wb'", "\"wb\"", "'ab'", "\"ab\"", "'r+b'", "\"r+b\"",
    ]
    .iter()
    .any(|m| code.contains(m));
    let bare_open = code
        .match_indices("open(")
        .any(|(at, _)| at == 0 || !matches!(code.as_bytes()[at - 1], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.'));
    (bare_open && !binary)
        || code.contains(".read_text(")
        || code.contains(".write_text(")
        || code.contains("split('/')")
        || code.contains("split(\"/\")")
}

fn shell_hazard(line: &str) -> bool {
    let code = script_code(line);
    ["flock", "pgrep", "pkill", "xvfb-run", "valgrind"]
        .iter()
        .any(|w| {
            code.match_indices(w)
                .any(|(at, _)| word_at(code, at, w.len()))
        })
        || code.contains("/tmp/")
        || code.contains("sed -i")
}

/// Hazard lines per tracked script (`.py`, `.sh`) under `root`.
fn measure_scripts(root: &PathText) -> BTreeMap<String, usize> {
    const SKIP: &[&str] = &[
        ".git",
        "target",
        ".claude/worktrees",
        "tests/fixtures",
        "node_modules",
    ];
    let mut out = BTreeMap::new();
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        for p in file_access::read_dir(&d).unwrap_or_default() {
            let rel = p
                .relative_to(root)
                .map(|r| r.portable())
                .unwrap_or_default();
            if SKIP.iter().any(|s| rel == *s) {
                continue;
            }
            if file_access::is_dir(&p) && !file_access::is_symlink(&p) {
                stack.push(p);
                continue;
            }
            let hazard: fn(&str) -> bool = if has_extension(&p, "py") {
                python_hazard
            } else if has_extension(&p, "sh") {
                shell_hazard
            } else {
                continue;
            };
            let n = file_access::read_to_string(&p)
                .unwrap_or_default()
                .lines()
                .filter(|l| hazard(l))
                .count();
            if n > 0 {
                out.insert(rel, n);
            }
        }
    }
    out
}

#[test]
fn script_hazards_only_fall() {
    let root = PathText::host(env!("CARGO_MANIFEST_DIR"));
    let now = measure_scripts(&root);
    let base_path = root.join(SCRIPTS_BASELINE);
    let base = read_baseline(&base_path);
    let bless = std::env::var_os("LOFT_BLESS_PLATFORM_CENSUS").is_some();
    let rose: Vec<String> = now
        .iter()
        .filter(|(f, n)| **n > base.get(*f).copied().unwrap_or(0))
        .map(|(f, n)| format!("{f}: {} -> {n}", base.get(f).copied().unwrap_or(0)))
        .collect();
    assert!(
        rose.is_empty() || (bless && base.is_empty()),
        "a script gained a Windows hazard (@PLN184 D2/D3): Python — give `open` / `read_text` / \
         `write_text` an `encoding=\"utf-8\"` and join paths with `os.path`/`pathlib`; shell — no \
         `flock` / `pgrep` / `pkill` / `xvfb-run` / `valgrind` / hard-coded `/tmp/` / `sed -i` \
         (or port the script to loft, @PLN179):\n  {}",
        rose.join("\n  ")
    );
    let fell: Vec<String> = base
        .iter()
        .filter(|(f, n)| now.get(*f).copied().unwrap_or(0) < **n)
        .map(|(f, n)| format!("{f}: {n} -> {}", now.get(f).copied().unwrap_or(0)))
        .collect();
    if bless {
        let mut text = String::from(
            "# Windows hazards per script (@PLN184 D2 Python, D3 shell).  Only shrinks: see\n\
             # src/platform_census.rs.  Format: <lines> <file>\n",
        );
        for (f, n) in &now {
            let _ = writeln!(text, "{n} {f}");
        }
        file_access::write(&base_path, text).expect("write baseline");
        return;
    }
    assert!(
        fell.is_empty(),
        "script hazards FELL — lock the progress in: \
         LOFT_BLESS_PLATFORM_CENSUS=1 cargo test --lib platform_census\n  {}",
        fell.join("\n  ")
    );
}

#[test]
fn the_script_census_sees_a_hazard_and_nothing_else() {
    assert!(python_hazard("    with open(p) as f:"));
    assert!(python_hazard("text = Path(p).read_text()"));
    assert!(python_hazard("parts = rel.split('/')"));
    assert!(!python_hazard("with open(p, encoding='utf-8') as f:"));
    assert!(!python_hazard("with open(p, 'rb') as f:"));
    assert!(!python_hazard("data = gzip.open(p)"));
    assert!(!python_hazard("# open(p) in a comment"));
    assert!(shell_hazard("flock -n 9 || exit 1"));
    assert!(shell_hazard("out=/tmp/loft_x"));
    assert!(shell_hazard("sed -i 's/a/b/' f"));
    assert!(!shell_hazard("# uses flock"));
    assert!(!shell_hazard("echo nopgrepx"));
    assert!(!shell_hazard("make valgrind-check-target"));
}
