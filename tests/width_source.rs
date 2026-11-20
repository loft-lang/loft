// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN217 — the C runtime names a loft `integer` and a loft `float` by the build's width, and
//! this test keeps what is not yet so from growing.
//!
//! The runtime is one source for the 64-bit build and the 32-bit one (`--width 32`).  A value
//! that is a loft `integer` — a length, an index, a count — is a `loft_int`, and a loft `float`
//! a `loft_float` (`src/c/loft_width.h`); what is 64 bits in every build is a `loft_word` or a
//! `loft_fword`.  A plain `int64_t` or `double` is neither: it is 64 bits on a 32-bit core
//! whether the value needs them or not, and where it is added, subtracted or compared no
//! library call betrays it, so neither the image gate nor the wide ledger sees it.  This count
//! is the one check that does.
//!
//! `tests/width_source.baseline` holds, per header, how many lines still name one.  A header
//! above its count fails: write the new code in the width header's names.  A header below its
//! count passes; record it with `LOFT_BLESS_WIDTH_SOURCE=1 cargo test --test width_source`.

use loft::file_access;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Does `line` name `word` as a whole identifier?
fn names(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(i, _)| {
        let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
        !line[..i].ends_with(ident) && !line[i + word.len()..].starts_with(ident)
    })
}

/// Per runtime header, the lines that name a plain `int64_t` or `double`.  The width header
/// itself is where the two are given their names.
fn counts() -> BTreeMap<String, usize> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("c");
    let mut out = BTreeMap::new();
    for entry in file_access::read_dir(&dir).expect("read src/c") {
        let path: PathBuf = entry.os_spelling();
        let Some(name) = file_access::file_name(&path) else {
            continue;
        };
        if !file_access::has_extension(&path, "h") || name == "loft_width.h" {
            continue;
        }
        let text = file_access::read_to_string(&path).expect("read a runtime header");
        let n = text
            .lines()
            .filter(|l| names(l, "int64_t") || names(l, "double"))
            .count();
        out.insert(name, n);
    }
    out
}

fn baseline_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("width_source.baseline")
}

#[test]
fn the_runtime_names_no_more_fixed_widths_than_its_baseline() {
    let now = counts();
    if std::env::var_os("LOFT_BLESS_WIDTH_SOURCE").is_some() {
        let mut text = String::from(
            "# header\tlines naming a plain int64_t or double — tests/width_source.rs; a count only goes down\n",
        );
        for (name, n) in &now {
            if *n > 0 {
                text.push_str(&format!("{name}\t{n}\n"));
            }
        }
        file_access::write(baseline_path(), text).expect("write the baseline");
        return;
    }
    let base: BTreeMap<String, usize> = file_access::read_to_string(baseline_path())
        .expect("tests/width_source.baseline")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let (name, n) = l.split_once('\t').expect("header<TAB>count");
            (name.to_string(), n.parse().expect("a count"))
        })
        .collect();
    let rose: Vec<String> = now
        .iter()
        .filter(|(name, n)| **n > base.get(*name).copied().unwrap_or(0))
        .map(|(name, n)| {
            format!(
                "src/c/{name}: {} -> {n}",
                base.get(name).copied().unwrap_or(0)
            )
        })
        .collect();
    assert!(
        rose.is_empty(),
        "the runtime names a plain `int64_t` or `double` on more lines than its baseline — a loft \
         `integer` is `loft_int`, a loft `float` is `loft_float`, and a value that is 64 bits in \
         every build is `loft_word` / `loft_fword` (src/c/loft_width.h):\n  {}",
        rose.join("\n  ")
    );
}

#[test]
fn a_fixed_width_is_found_as_a_whole_word() {
    assert!(names("static int64_t f(void) {", "int64_t"));
    assert!(names("    double d = 0;", "double"));
    assert!(!names("uint64_t u;", "int64_t"));
    assert!(!names("loft_double_x(1);", "double"));
    assert!(!names("my_int64_t_alias v;", "int64_t"));
}
