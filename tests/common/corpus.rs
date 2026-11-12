// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The language suite beside this repository (@PLN193), for the sweeps that walk "every
//! program": the IR codec round-trip, the ownership oracle's leak scan, the whole-corpus
//! interpreter run.  The corpus RUNNERS (`wrap`'s chunks, `native`'s chunks) do not use this —
//! the suite's own runner and the required `Language suite` check carry those programs — but
//! a sweep that asked a question of every program before the move still asks it of every
//! program.  `scripts/corpus.py` and `scripts/corpus_files.sh` are the same list for the
//! scripts; the three must agree.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

/// `$LOFT_SUITE_DIR`, else `../loft-suite` beside the repository root (the working directory
/// of a test binary); a relative value is taken from that root.
pub fn suite_dir() -> PathBuf {
    let d = std::env::var("LOFT_SUITE_DIR").unwrap_or_else(|_| "../loft-suite".to_string());
    PathBuf::from(d)
}

/// Every program of the suite, sorted: the rule cells (`rules/<chapter>/<rule>/*.loft`), then
/// the attic (`attic/scripts/*.loft`).  A file a sibling cell imports (`use <name>;`) is that
/// cell's library, not a program, and is left out: it may declare no entry point at all.  Empty without the checkout, and SAID on stderr, so a
/// sweep over 431 programs never reads as the sweep over 2300.
pub fn suite_programs() -> Vec<PathBuf> {
    let dir = suite_dir();
    if !fa::is_dir(dir.join("rules")) {
        eprintln!(
            "no language suite at {} — its programs are NOT in this sweep (LOFT_SUITE_DIR)",
            dir.display()
        );
        return Vec::new();
    }
    let mut out = Vec::new();
    for chapter in loft_files_or_dirs(&dir.join("rules")) {
        for rule in loft_files_or_dirs(&chapter) {
            let cells: Vec<PathBuf> = loft_files_or_dirs(&rule)
                .into_iter()
                .filter(|p| is_loft(p))
                .collect();
            let libraries = imported_names(&cells);
            out.extend(
                cells
                    .into_iter()
                    .filter(|p| fa::file_stem(p).is_none_or(|s| !libraries.contains(&s))),
            );
        }
    }
    out.sort();
    let mut attic: Vec<PathBuf> = loft_files_or_dirs(&dir.join("attic").join("scripts"))
        .into_iter()
        .filter(|p| is_loft(p))
        .collect();
    attic.sort();
    out.extend(attic);
    out
}

/// The `--lib <dir>` directories a program's `// @ARGS:` lines name (its first 20 lines).  A
/// relative directory is relative to where the program's own runner runs it: the language
/// suite's root for a suite cell (its runner's working directory), the repository root for
/// everything else.
pub fn args_lib_dirs(source: &str, file: &Path) -> Vec<String> {
    let suite = suite_dir();
    let in_suite = file.starts_with(&suite);
    let mut out = Vec::new();
    for line in source.lines().take(20) {
        if let Some(args) = line.trim().strip_prefix("// @ARGS:") {
            let mut tokens = args.split_whitespace();
            while let Some(tok) = tokens.next() {
                if tok == "--lib"
                    && let Some(dir) = tokens.next()
                {
                    if in_suite && !fa::is_absolute(dir) {
                        out.push(suite.join(dir).to_string_lossy().to_string());
                    } else {
                        out.push(dir.to_string());
                    }
                }
            }
        }
    }
    out
}

/// The names the files import with a top-level `use <name>;`.
fn imported_names(files: &[PathBuf]) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for f in files {
        let Ok(text) = fa::read_to_string(f) else {
            continue;
        };
        for line in text.lines() {
            if let Some(rest) = line.trim_start().strip_prefix("use ")
                && let Some(name) = rest.trim().strip_suffix(';')
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                out.insert(name.to_string());
            }
        }
    }
    out
}

fn loft_files_or_dirs(dir: &Path) -> Vec<PathBuf> {
    fa::read_dir(dir)
        .map(|entries| entries.into_iter().map(|e| e.os_spelling()).collect())
        .unwrap_or_default()
}

fn is_loft(p: &Path) -> bool {
    fa::is_file(p) && fa::has_extension(p, "loft")
}

/// A program a `[sandbox]` policy governs: an ACTIVE policy in the nearest `loft.toml` within
/// four directories above it — the walk `loft --tests` makes (`test_runner::sandbox_policy_for`).
/// Its admission refusals are the program's expectations, so a sweep that parses in-process
/// without applying the policy would read every one as unmatched; the suite's own runner, which
/// runs the CLI, is where such a cell is judged.
#[allow(dead_code)]
pub fn under_sandbox_policy(p: &Path) -> bool {
    let mut dir = p.parent();
    for _ in 0..4 {
        let Some(d) = dir else { return false };
        if let Ok(content) = fa::read_to_string(d.join("loft.toml"))
            && loft::sandbox::parse_sandbox_config(&content).is_active()
        {
            return true;
        }
        dir = d.parent();
    }
    false
}
