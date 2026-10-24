// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Range`'s while-counter clause — the EMISSION pins.  The step of
//! `while i < E { …; i += 1 }` is emitted plain; a second step in one pass and an unranged
//! seed keep the checked template; `LOFT_NO_WHILE_COUNTER=1` emits every step checked.  The
//! cell corpus (`tests/scripts/a-while-counter-steps-plain-below-its-bound.loft`) says the
//! VALUES hold; this pins what is emitted.
use loft::file_access as fa;
use std::path::{Path, PathBuf};

const CELLS: &str = "tests/scripts/a-while-counter-steps-plain-below-its-bound.loft";
const PLAIN: &str = "var_i = ((var_i).wrapping_add(1_i64));";
const CHECKED: &str = "var_i = ops::op_add_int((var_i), (1_i64));";

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    // One file per TEST: the tests run on threads of one process.
    let out = std::env::temp_dir().join(format!("loft_whilectr_{}_{tag}.rs", std::process::id()));
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(&out)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS))
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_WHILE_COUNTER");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft");
    assert!(
        fa::exists(&out),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    let rust = fa::read_to_string(&out).expect("read the emitted Rust");
    let _ = fa::remove_file(&out);
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
fn a_while_counter_steps_plain() {
    let rust = emit("on", &[]);
    for f in ["n_c1", "n_c2", "n_c3", "n_c5"] {
        let b = body(&rust, f);
        assert!(b.contains(PLAIN) && !b.contains(CHECKED), "{f}:\n{b}");
    }
}

#[test]
fn a_second_step_or_an_unranged_seed_keeps_the_check() {
    let rust = emit("declines", &[]);
    for f in ["n_c6", "n_c7"] {
        let b = body(&rust, f);
        assert!(b.contains(CHECKED) && !b.contains(PLAIN), "{f}:\n{b}");
    }
}

#[test]
fn switched_off_every_step_is_checked() {
    let rust = emit("off", &[("LOFT_NO_WHILE_COUNTER", "1")]);
    let b = body(&rust, "n_c1");
    assert!(b.contains(CHECKED) && !b.contains(PLAIN), "{b}");
}
