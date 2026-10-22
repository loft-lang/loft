// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Apart` instance 4 — the EMISSION pins.  A record of record vectors built in a
//! frame and handed only to the functions that copy it into a collection is an `Apr<n>`
//! value: its owners declare `__ap_<v>`, its callees are emitted as `__apr` twins, and the
//! store meets it at `apart_write_fresh` or `rotate_from_parts`.  A driver or a callee that
//! reads an element keeps the store form; `LOFT_NO_APART_RECORDS=1` emits none.  The plan is
//! made only where no live tier ships (`--native-release`), so every test runs that tier.  The
//! cell corpus (`tests/scripts/158-apart-records.loft`) says the VALUES hold; this pins what is
//! emitted.
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/158-apart-records.loft";

/// The twins: every callee the owners reach, and `noop`, which `push3`/`push2` call.
const TWINS: &[&str] = &["n_noop__apr", "n_push2__apr", "n_push3__apr", "n_keep__apr"];

/// Per driver: whether its `e` is apart.  a4 reads an element of `e`; a5's callee reads one.
const OWNERS: &[(&str, bool)] = &[
    ("n_a1", true),
    ("n_a2", true),
    ("n_a3", true),
    ("n_a4", false),
    ("n_a5", false),
    ("n_a6", true),
    ("n_a7", true),
];

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_APART_RECORDS")
        .env_remove("LOFT_NO_APART")
        .env_remove("LOFT_NO_HAND_OVER");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn cells() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(CELLS)
        .to_str()
        .unwrap()
        .to_string()
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    // One file per TEST: the tests run on threads of one process.
    let out = std::env::temp_dir().join(format!(
        "loft_apart_records_{}_{tag}.rs",
        std::process::id()
    ));
    let out_s = out.to_str().unwrap().to_string();
    let status = loft(
        &["--native-release", "--native-emit", &out_s, &cells()],
        env,
    );
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

fn run(args: &[&str], env: &[(&str, &str)]) -> String {
    let mut a: Vec<&str> = args.to_vec();
    let c = cells();
    a.push(&c);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.trim_end().ends_with("apart records ok"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

#[test]
fn the_cells_answer_the_interpreter_in_every_switch_state_under_the_falsifiers() {
    let oracle = run(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [
        None,
        Some(("LOFT_NO_APART_RECORDS", "1")),
        Some(("LOFT_NO_HAND_OVER", "1")),
    ] {
        for falsifier in [
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_POISON_CLAIM", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
        ] {
            let mut env = vec![falsifier];
            env.extend(switch);
            let got = run(&["--native-release"], &env);
            assert_eq!(
                got, oracle,
                "native under {env:?} must print what the interpreter printed"
            );
        }
    }
}

#[test]
fn each_callee_is_a_twin_and_each_driver_holds_what_was_predicted() {
    let rust = emit("on", &[]);
    for twin in TWINS {
        assert_eq!(
            rust.matches(&format!("\nfn {twin}(")).count(),
            1,
            "{twin} is emitted exactly once"
        );
    }
    assert!(
        !rust.contains("\nfn n_push3r__apr("),
        "push3r reads an element of its parameter: no twin"
    );
    for (name, apart) in OWNERS {
        let b = body(&rust, name);
        assert_eq!(
            b.contains("let mut __ap_e: "),
            *apart,
            "{name}: `e` apart\n{b}"
        );
    }
}

#[test]
fn the_store_meets_the_value_at_the_append_and_the_rotate() {
    let rust = emit("boundary", &[]);
    let push3 = body(&rust, "n_push3__apr");
    assert!(push3.contains("stores.rotate_from_parts("), "{push3}");
    assert!(push3.contains("stores.apart_write_fresh("), "{push3}");
    let keep = body(&rust, "n_keep__apr");
    assert!(keep.contains("stores.apart_write_fresh("), "{keep}");
}

#[test]
fn the_switch_emits_no_apart_record() {
    let rust = emit("off", &[("LOFT_NO_APART_RECORDS", "1")]);
    assert_eq!(rust.matches("__apr(").count(), 0);
    assert_eq!(rust.matches("let mut __ap_e: ").count(), 0);
}
