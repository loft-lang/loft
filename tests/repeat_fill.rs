// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `[x; n]` and the constant comprehension — the EMISSION pins.  A constant comprehension in a
//! struct FIELD lowers to the repeat literal (one template append and one `OpAppendCopy`
//! addressed to the FIELD, not the record), as a local's has since loft#884;
//! `LOFT_NO_FIELD_FILL=1` keeps the per-element loop.  The runtime fill behind the op is one
//! doubling block copy (`Stores::fill_from_template`); `LOFT_NO_BLOCK_REPEAT=1` restores the
//! per-element copy.  The cell corpus (`tests/scripts/a-repeated-element-fills-in-one-block.loft`)
//! says the VALUES hold on both backends under both switches; this pins what is emitted.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/a-repeated-element-fills-in-one-block.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_FIELD_FILL")
        .env_remove("LOFT_NO_BLOCK_REPEAT");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out =
        std::env::temp_dir().join(format!("loft_repeat_fill_{}_{tag}.rs", std::process::id()));
    let status = loft(
        &[
            "--native-emit",
            out.to_str().unwrap(),
            src.to_str().unwrap(),
        ],
        env,
    );
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

fn body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("\nfn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start + 1..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

fn run_ok(args: &[&str], env: &[(&str, &str)]) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let s = src.to_str().unwrap().to_string();
    let mut a: Vec<&str> = args.to_vec();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.trim_end().ends_with("ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_constant_comprehension_in_a_field_is_one_append_copy_addressed_to_the_field() {
    let rust = emit("field", &[]);
    let main = body(&rust, "n_main");
    // r5: `RfH { v: [for _i in 0..r5n { 3 }], w: [for _i in 0..r5n { 2.5 }] }` — two fills, each
    // handed the FIELD's own reference (the record plus the field's offset), never the record.
    let copies: Vec<&str> = main
        .match_indices("OpAppendCopy(cell, ")
        .map(|(i, _)| &main[i..i + 120])
        .collect();
    let to_field = copies
        .iter()
        .filter(|c| c.contains("var_r5") && c.contains("pos + ("))
        .count();
    assert!(
        to_field >= 2,
        "r5's two field fills should be append-copies addressed to the fields:\n{copies:?}"
    );
    assert!(
        !copies
            .iter()
            .any(|c| c.starts_with("OpAppendCopy(cell, var_r5,")),
        "a fill addressed to the RECORD (loft#892's shape):\n{copies:?}"
    );
}

#[test]
fn the_field_fill_switch_keeps_the_per_element_loop() {
    let rust = emit("field_off", &[("LOFT_NO_FIELD_FILL", "1")]);
    let main = body(&rust, "n_main");
    let field_copies = main
        .match_indices("OpAppendCopy(cell, ")
        .filter(|(i, _)| main[*i..*i + 120].contains("var_r5"))
        .count();
    assert_eq!(
        field_copies, 0,
        "the switch should keep r5's fields on the loop"
    );
    assert!(
        main.contains("For comprehension"),
        "no comprehension loop survived the switch"
    );
}

#[test]
fn the_values_hold_on_both_backends_under_both_switches_and_the_falsifiers() {
    run_ok(&["--interpret"], &[]);
    run_ok(
        &["--native"],
        &[
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_POISON_CLAIM", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
        ],
    );
    run_ok(&["--native"], &[("LOFT_NO_BLOCK_REPEAT", "1")]);
    run_ok(&["--interpret"], &[("LOFT_NO_BLOCK_REPEAT", "1")]);
    run_ok(&["--native"], &[("LOFT_NO_FIELD_FILL", "1")]);
}
