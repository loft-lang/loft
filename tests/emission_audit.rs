// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-r (loft#1426) — the EMISSION AUDIT: the emitted Rust of every hoist-family cell
//! corpus and of the in-repo drawing bench is checked against the rewrite rules'
//! assumptions (`formal/rewrites.md`: R-State one holder per path per frame, R-Refresh no
//! mover on a held path, R-Inputs a twin handed only live holders) by
//! `scripts/emission_audit.py`, at emission time, before any run.  A clean audit that
//! resolves no holder is vacuous, so each corpus must also bind at least one; and the audit
//! must be able to FAIL, so a hand-made double holder is fed to it and must be refused.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run the audit; answers `(exit ok, summary line, full output)`.
fn audit(rust: &Path) -> (bool, String, String) {
    let out = loft::platform::process::harness_command("python3")
        .arg(root().join("scripts/emission_audit.py"))
        .arg(rust)
        .current_dir(root())
        .output()
        .expect("spawn the emission audit");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let summary = text
        .lines()
        .find(|l| l.starts_with("emission audit:"))
        .unwrap_or("")
        .to_string();
    (out.status.success(), summary, text)
}

/// A holder dies with the scope that bound it, also where one line closes that scope and opens
/// the next (`} else {`): a mover in the `else` arm of `@FR-R-PushFill`'s run clause is no
/// mover on a held path, while the same mover inside the arm still is.
#[test]
fn a_holder_ends_at_the_else_that_closes_its_arm() {
    let out = std::env::temp_dir().join("loft_emission_audit_else.rs");
    let rust = [
        "fn n_run(cell: &std::cell::UnsafeCell<Stores>, mut var_buf: DbRef) {",
        "  if !(var_buf).is_null() { //@FR-R-PushFill run of 2",
        "    let mut __ph_1 = vector::push_header(&(var_buf), &stores.allocations); //@FR-R-PushFill run header",
        "    let mut __pw_1 = vector::push_window(&__ph_1, 1_u32, &stores.allocations); //@FR-R-PushFill push window, run of 2",
        "    vector::pre_alloc_vector(&(var_buf), 1_u32, 1_u32, &mut stores.allocations);",
        "  } else {",
        "    vector::pre_alloc_vector(&(var_buf), 1_u32, 1_u32, &mut stores.allocations);",
        "  }",
        "}",
    ]
    .join("\n");
    fa::write(&out, rust).expect("write the hand-made emission");
    let (ok, _, text) = audit(&out);
    let found: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("VIOLATION"))
        .collect();
    assert!(
        !ok && found.len() == 1 && found[0].contains("n_run:5:") && found[0].contains("R-Refresh"),
        "the mover inside the arm, and only it, must be refused:\n{text}"
    );
    let _ = fa::remove_file(&out);
}
