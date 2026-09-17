// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-BoundedNest` — the EMISSION pins.  An innermost counted loop that accumulates
//! `?`-discharged element products gets a guard (`__nb_N`) and a plain-operator arm, with
//! the checked loop as its `else`; the element bounds (`__bd_N`) are derived at the
//! outermost loop whose body leaves the vector alone, never at the nest's own prelude; a
//! loop whose body writes the vector holds no bound and the nest under it declines;
//! `LOFT_NO_BOUNDED_NEST=1` restores the checked form everywhere.  The cell corpus
//! (`tests/scripts/157-bounded-nest.loft`) says the VALUES hold on both backends, in both
//! switch states and under `LOFT_HOIST_VERIFY=1`; this pins what is emitted and runs it.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/157-bounded-nest.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_BOUNDED_NEST")
        .env_remove("LOFT_HOIST_VERIFY");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    // One file per TEST: the tests run on threads of one process.
    let out =
        std::env::temp_dir().join(format!("loft_bounded_nest_{}_{tag}.rs", std::process::id()));
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

/// The emitted body of one function, up to the next top-level `fn`.
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
    let mut a: Vec<&str> = args.to_vec();
    let s = src.to_str().unwrap().to_string();
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
fn the_tap_nest_is_guarded_and_runs_plain_with_the_checked_loop_as_its_else() {
    let rust = emit("tap", &[]);
    let taps = body(&rust, "n_bn_taps");
    assert!(taps.contains("let __nb_"), "no guard in n_bn_taps:\n{taps}");
    assert!(
        taps.contains("plain nest"),
        "no plain arm in n_bn_taps:\n{taps}"
    );
    assert!(
        taps.contains("/* checked nest */"),
        "no checked arm in n_bn_taps:\n{taps}"
    );
    // The accumulate and the index chain are plain inside the arm …
    assert!(
        taps.contains(".wrapping_mul("),
        "the plain arm keeps a checked multiply:\n{taps}"
    );
    assert!(
        taps.contains(".wrapping_add("),
        "the plain arm keeps a checked add:\n{taps}"
    );
    // … and the guard proves the whole chain, the reads' bounds and the accumulate.
    assert!(
        taps.contains("checked_mul(__tb)?.checked_add(__ab)"),
        "the accumulate bound is missing:\n{taps}"
    );
    assert!(
        taps.contains("let __rb_0: i64 = (__bd_"),
        "the element bounds are not read:\n{taps}"
    );
    // The bounds are derived at the OUTER loops, before the nest's own prelude.
    let guard = taps.find("let __nb_").unwrap();
    let bound = taps.find("abs_bound_i64(").unwrap();
    assert!(
        bound < guard,
        "the element bound is derived after the guard that reads it:\n{taps}"
    );
}

#[test]
fn a_loop_that_writes_the_vector_holds_no_bound_and_its_nest_declines() {
    // n16/n17: `a16[0] = …` inside the enclosing loop.  Its nest stays checked.
    let rust = emit("written", &[]);
    let main = body(&rust, "n_main");
    // The two admitted main-level nests (n13, n15) are guarded; n16's and n17's are not:
    // count the guards against the admissions the trace names for main.
    let guards = main.matches("let __nb_").count();
    assert_eq!(
        guards, 2,
        "main should guard exactly its two admitted nests:\n{main}"
    );
}

#[test]
fn the_switch_restores_the_checked_form_everywhere() {
    let rust = emit("off", &[("LOFT_NO_BOUNDED_NEST", "1")]);
    assert!(!rust.contains("__nb_"), "a guard survived the switch");
    assert!(
        !rust.contains("abs_bound_i64"),
        "a bound survived the switch"
    );
    assert!(
        !rust.contains("plain nest"),
        "a plain arm survived the switch"
    );
}

#[test]
fn the_values_hold_on_both_backends_in_both_switch_states_and_under_verify() {
    run_ok(&["--interpret"], &[]);
    run_ok(&["--native-release"], &[]);
    run_ok(&["--native"], &[("LOFT_NO_BOUNDED_NEST", "1")]);
    run_ok(
        &["--native"],
        &[
            ("LOFT_HOIST_VERIFY", "1"),
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
        ],
    );
}
