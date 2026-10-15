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
/// the attic (`attic/scripts/*.loft`).  Empty without the checkout, and SAID on stderr, so a
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
            out.extend(loft_files_or_dirs(&rule).into_iter().filter(|p| is_loft(p)));
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

fn loft_files_or_dirs(dir: &Path) -> Vec<PathBuf> {
    fa::read_dir(dir)
        .map(|entries| entries.into_iter().map(|e| e.os_spelling()).collect())
        .unwrap_or_default()
}

fn is_loft(p: &Path) -> bool {
    fa::is_file(p) && fa::has_extension(p, "loft")
}
