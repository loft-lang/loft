// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 — rename on top of the workspace reverse index (`plan_rename` /
// `prepare_rename` / `is_valid_identifier`).  SAFE by default: refuses stdlib
// symbols, stdlib-file edits, and invalid new names.

use loft::file_access as fa;
use std::path::PathBuf;

use loft::lsp::{WorkspaceIndex, is_valid_identifier, plan_rename, prepare_rename};

fn ws(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    fa::create_dir_all(&dir).unwrap();
    fa::write(
        dir.join("a.loft"),
        "fn area(w: integer) -> integer { w * w }\n",
    )
    .unwrap();
    fa::write(dir.join("b.loft"), "fn main() { print(area(3)) }\n").unwrap();
    dir
}

#[test]
fn valid_identifier_guard() {
    for (n, ok) in [
        ("foo", true),
        ("_x", true),
        ("a1", true),
        ("3bad", false),
        ("a b", false),
        ("fn", false), // keyword
        ("", false),
    ] {
        assert_eq!(is_valid_identifier(n), ok, "is_valid_identifier({n:?})");
    }
}

#[test]
fn rename_a_user_symbol_and_refuse_the_unsafe() {
    let dir = ws("renlib");
    let wi = WorkspaceIndex::build(dir.to_str().unwrap());

    // A user global renames across files (def in a.loft + call in b.loft).
    let refs = plan_rename("area", "zone", &wi, &[], "default").expect("`area` is renamable");
    assert_eq!(refs.len(), 2, "def + call: {refs:?}");

    // A standard-library symbol is refused.
    assert!(
        plan_rename("print", "foo", &wi, &[], "default").is_err(),
        "a stdlib symbol must be refused"
    );
    // An invalid new name is refused.
    assert!(
        plan_rename("area", "3bad", &wi, &[], "default").is_err(),
        "an invalid new name must be refused"
    );
    // A name with no references is refused.
    assert!(
        plan_rename("nowhere_symbol", "x", &wi, &[], "default").is_err(),
        "no references must be refused"
    );
}

#[test]
fn prepare_rename_gates_stdlib_symbols() {
    let dir = ws("renprep");
    let a = fa::read_to_string(dir.join("a.loft")).unwrap();
    // `area` at col 4 is renamable → its 0-based span (cols 3..7).
    assert_eq!(
        prepare_rename(&a, 1, 4, "default"),
        Some(("area".to_string(), 3, 7))
    );
    // `print` (a stdlib fn) in b.loft is NOT renamable → the editor offers no box.
    let b = fa::read_to_string(dir.join("b.loft")).unwrap();
    assert!(
        prepare_rename(&b, 1, 13, "default").is_none(),
        "a stdlib symbol is not renamable"
    );
}
