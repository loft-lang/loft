// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-BoundedNest` — the EMISSION pins.  An innermost counted loop that accumulates
//! `?`-discharged element products gets a guard (`__nb_N`) and a plain-operator arm, with
//! the checked loop as its `else`; the element bounds (`__bd_N`) are derived at the
//! outermost loop whose body leaves the vector alone, never at the nest's own prelude; a
//! loop whose body writes the vector holds no bound and the nest under it declines;
//! `LOFT_NO_BOUNDED_NEST=1` restores the checked form everywhere.  Step 2: where every read's
//! index chain is affine in the counter the guard also proves both range ends in `[0, len)`
//! and the arm reads RAW through the held base with no null select; `LOFT_NO_NEST_RAW_READS=1`
//! keeps step 1's bounds-tested arm.  The cell corpus
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
fn an_affine_nest_reads_raw_through_the_base_and_a_non_affine_one_keeps_the_checked_read() {
    let rust = emit("raw", &[]);
    let taps = body(&rust, "n_bn_taps");
    let arm = &taps[taps.find("plain nest").unwrap()..taps.find("plain nest*/").unwrap()];
    assert!(
        arm.contains(".read_unaligned()"),
        "the tap's plain arm does not read raw:\n{arm}"
    );
    assert!(
        !arm.contains("get_elem_at"),
        "the tap's plain arm keeps a bounds-tested read:\n{arm}"
    );
    assert!(
        !arm.contains("op_conv_bool_from_int"),
        "the tap's plain arm keeps the null select:\n{arm}"
    );
    // The guard proves both ends of every read's index in range against the header's length.
    assert!(
        taps.contains("let __ln_0: i64 = i64::from(__vh_"),
        "no in-range clause:\n{taps}"
    );
    assert!(
        taps.contains("__xh_1 >= __ln_1 { return None; }"),
        "the second read's range test is missing:\n{taps}"
    );
    // n21 (`a[x * x]`) names the counter twice: not affine, so its arm keeps the checked read.
    let main = body(&rust, "n_main");
    let arms: Vec<&str> = main.split("plain nest\n").skip(1).collect();
    assert!(
        arms.iter()
            .any(|a| a.contains("get_elem_at") && !a.contains("read_unaligned")),
        "no plain arm kept its checked read — n21's non-affine chain should have"
    );
}

#[test]
fn the_raw_read_switch_restores_the_bounds_tested_arm() {
    let rust = emit("raw_off", &[("LOFT_NO_NEST_RAW_READS", "1")]);
    let taps = body(&rust, "n_bn_taps");
    assert!(
        taps.contains("let __nb_"),
        "the nest itself should still be guarded"
    );
    assert!(
        !taps.contains(".read_unaligned()"),
        "a raw read survived the switch:\n{taps}"
    );
    assert!(
        !taps.contains("let __ln_0"),
        "the range clause survived the switch"
    );
}

#[test]
fn a_loop_that_writes_the_vector_holds_no_bound_and_its_nest_declines() {
    // n16/n17: `a16[0] = …` inside the enclosing loop.  Its nest stays checked.
    let rust = emit("written", &[]);
    let main = body(&rust, "n_main");
    // main's admitted nests are n13, n15 and the step-2 cells n18–n23 — eight — and n16's and
    // n17's are NOT among them, because the loop enclosing each writes `a16`/`a17`.
    let guards = main.matches("let __nb_").count();
    assert_eq!(
        guards, 8,
        "main should guard exactly its eight admitted nests:\n{main}"
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
    run_ok(&["--native"], &[("LOFT_NO_NEST_RAW_READS", "1")]);
    run_ok(
        &["--native"],
        &[
            ("LOFT_HOIST_VERIFY", "1"),
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            // A raw read one past the end lands in the claim's zeroed slack and would answer
            // the checked read's 0 by luck; poisoned, n19 can only pass through the checked loop.
            ("LOFT_POISON_CLAIM", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
        ],
    );
}
