// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `LOFT_RELEASE_PASS_PROBE=1` — the MEASUREMENT instrument for the eventual release build
//! pass for games (DESIGN_DECISIONS.md C120, NATIVE.md § Optimisation tiers): every
//! integer `+`, `-`, `*` and non-literal division emits the processor's wrapping operator
//! and every float comparison the plain one, so a row's time under it is the ceiling its
//! checked build is measured against.  This pins that the probe changes the EMISSION and
//! only under the switch: off, the default build is untouched (the checked helpers stand);
//! on, no sentinel-aware integer helper remains and the null test itself does.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-al-loop-buffer-cells.loft";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = std::env::temp_dir().join(format!(
        "loft_release_pass_probe_{}_{tag}.rs",
        std::process::id()
    ));
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("LOFT_RELEASE_PASS_PROBE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = std::fs::read_to_string(&out).expect("read the emitted Rust");
    let _ = std::fs::remove_file(&out);
    rust
}

/// The emitted body of one function, up to the next top-level `fn`.
fn body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

#[test]
fn the_default_build_keeps_every_checked_helper() {
    let rust = emit("off", &[]);
    let l1 = body(&rust, "n_l1");
    // `10 * i + j` over a loop variable and a counter: the proof gives the checked
    // helper, never the wrapping operator.
    assert!(
        l1.contains("ops::op_mul_long_nn(") || l1.contains("ops::op_mul_int("),
        "l1's multiply is a checked helper by default"
    );
    assert!(
        !l1.contains(".wrapping_mul(") && !l1.contains(".wrapping_add("),
        "no wrapping operator is emitted without the probe"
    );
}

#[test]
fn the_probe_emits_the_processors_arithmetic_and_keeps_the_null_test() {
    let rust = emit("on", &[("LOFT_RELEASE_PASS_PROBE", "1")]);
    let l1 = body(&rust, "n_l1");
    assert!(
        l1.contains(".wrapping_mul(") && l1.contains(".wrapping_add("),
        "under the probe the arithmetic is the processor's"
    );
    assert!(
        !l1.contains("ops::op_mul_int(")
            && !l1.contains("ops::op_add_int(")
            && !l1.contains("ops::op_mul_long_nn(")
            && !l1.contains("ops::op_add_long_nn("),
        "under the probe no sentinel-aware integer helper remains"
    );
    // The `?? -1` on an absent element is the language's null semantics, not fault
    // protection: its test stays under the probe.
    assert!(
        l1.contains("op_conv_bool_from_int(")
            || l1.contains(".rec == 0")
            || l1.contains("i64::MIN"),
        "the null test of the discharge stays"
    );
}
