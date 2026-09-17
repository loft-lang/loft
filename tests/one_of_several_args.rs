// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! loft#1550 (`@FR-O-Oracle`, `@FR-B-Copy`) — a callee whose return may hand back ANY of
//! several arguments has no single witness, so the bind COPIES on every run.  The guard
//! (`tests/scripts/1550-a-view-of-one-of-several-arguments-is-copied.loft`) holds the values;
//! this pins the lowering: no runtime guard against one argument on either backend.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/164-activation-arena/bytecode-comparisons/1550-two-borrow-join-cells.loft";

fn cells() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS)
}

fn loft() -> Command {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.env("LOFT_TIMEOUT", "120");
    cmd
}

/// The first listing of `fn <name>(` up to the next top-level `fn`.
fn section<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} is not in the listing"));
    let rest = &text[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

/// The bytecode listing of `name`, from `byte-code for …:name(` to its `Return`.
fn bytecode<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(&format!(":{name}("))
        .unwrap_or_else(|| panic!("no bytecode for {name}"));
    let rest = &text[start..];
    let end = rest.find("Return(").map_or(rest.len(), |i| i + 7);
    &rest[..end]
}

#[test]
fn the_interpreter_copies_without_a_witness() {
    let out = loft()
        .arg("introspect")
        .arg(cells())
        .output()
        .expect("spawn loft introspect");
    let text = String::from_utf8_lossy(&out.stdout);
    let pick = bytecode(&text, "n_pick_one");
    assert!(
        pick.contains("CopyRefOrNull"),
        "pick_one: the first bind copies\n{pick}"
    );
    assert!(
        !pick.contains("BindOrCopy"),
        "pick_one: no runtime guard against one argument\n{pick}"
    );
}

#[test]
fn native_copies_without_a_witness() {
    let out = std::env::temp_dir().join("loft_one_of_several_args.rs");
    let status = loft()
        .arg("--native-emit")
        .arg(&out)
        .arg(cells())
        .output()
        .expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = std::fs::read_to_string(&out).expect("read the emitted Rust");
    let pick = section(&rust, "n_pick_one");
    assert!(
        pick.contains("if _src.store_nr == u16::MAX || _src.store_nr == _dst.store_nr {"),
        "pick_one: the plain copy-or-adopt split"
    );
    assert!(
        !pick.contains("_src.store_nr != var_cv.store_nr"),
        "pick_one: no witness compare against one argument"
    );
    let _ = std::fs::remove_file(&out);
}
