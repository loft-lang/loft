// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-FieldAppend` — the EMISSION pins.  A straight-line append to a struct's plain vector
//! field (`r.f += [x]` with no push header for `r.f`) is emitted as the field's own append,
//! the field's offset and the element's size resolved by the emitter; the prefill rides the
//! const argument (`<false>` where `(R-CompleteWrite)` covers the group).  The cell corpus
//! (`tests/scripts/a-field-append-outside-a-loop-lands-in-its-field.loft`) says the VALUES
//! hold; this pins what is emitted, and that `LOFT_NO_FIELD_APPEND=1` restores the templates.
use loft::file_access as fa;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-field-append-outside-a-loop-lands-in-its-field.loft";

/// Per function: `(no-prefill field appends, prefilled field appends, template mints)`.
const EXPECTED: &[(&str, usize, usize, usize)] = &[
    ("n_add_item", 1, 0, 0), // f1: the literal writes every field, the text included
    ("n_add_pair", 1, 0, 0), // f2 / f4: a whole-record copy covers a heap-free element
    ("n_add_k", 1, 0, 0),    // f3: a field of a field (`o.inner.its`)
    ("n_add_it", 1, 0, 0),   // f5: the parent may be an absent element
    ("n_add_box", 0, 1, 0),  // f7: a nested struct arriving by copy keeps the prefill
    ("n_main", 0, 2, 0),     // f6: a `vector<text>` field, its element a handle
];

fn emit(out: &Path, env: &[(&str, &str)]) -> String {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-release")
        .arg("--native-emit")
        .arg(out)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        fa::exists(out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    fa::read_to_string(out).expect("read the emitted Rust")
}

fn counts(rust: &str) -> HashMap<String, (usize, usize, usize)> {
    let mut map: HashMap<String, (usize, usize, usize)> = HashMap::new();
    let mut current = String::new();
    for line in rust.lines() {
        if let Some(rest) = line.strip_prefix("fn ")
            && let Some(paren) = rest.find('(')
        {
            current = rest[..paren].to_string();
        }
        let row = map.entry(current.clone()).or_default();
        row.0 += line.matches("field_vector_append::<false>(").count();
        row.1 += line.matches("field_vector_append::<true>(").count();
        row.2 += line.matches("OpNewRecord(").count() + line.matches("OpNewRecordNP(").count();
    }
    map
}

#[test]
fn each_cell_appends_through_its_field() {
    let out = std::env::temp_dir().join("loft_field_append_on.rs");
    let got = counts(&emit(&out, &[]));
    for (name, np, pre, tpl) in EXPECTED {
        assert_eq!(
            got.get(*name).copied().unwrap_or_default(),
            (*np, *pre, *tpl),
            "{name}: (no-prefill appends, prefilled appends, template mints)"
        );
    }
    let _ = fa::remove_file(&out);
}

#[test]
fn the_switch_restores_the_templates() {
    let out = std::env::temp_dir().join("loft_field_append_off.rs");
    let rust = emit(&out, &[("LOFT_NO_FIELD_APPEND", "1")]);
    assert!(
        !rust.contains("field_vector_append"),
        "LOFT_NO_FIELD_APPEND=1 left a field append in place"
    );
    let got = counts(&rust);
    for (name, np, pre, _) in EXPECTED {
        assert_eq!(
            got.get(*name).copied().unwrap_or_default().2,
            np + pre,
            "{name}: every field append is a template mint again"
        );
    }
    let _ = fa::remove_file(&out);
}
