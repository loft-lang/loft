// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Header`'s KEPT clause — the EMISSION pins.  A vector field of a record parameter
//! that a chain of callees walks (`m.cs`) has one holder: bound at entry by the body that
//! reads it twice or more, handed down by `&mut` to each callee's `__kh` twin, and derived
//! again after an append to it and after a plain call that may change it.  A body that
//! appends inside a loop, or calls an appender inside an expression, keeps nothing.
//! `LOFT_NO_KEPT_HEADER=1` restores the per-walk derivation.  The cells
//! (`tests/scripts/a-kept-vector-field-follows-its-appends-across-calls.loft`) say the VALUES
//! hold on both backends, in both switch states and under the falsifiers; this pins what is
//! emitted.
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/a-kept-vector-field-follows-its-appends-across-calls.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_KEPT_HEADER")
        .env_remove("LOFT_NO_VECTOR_HOIST")
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
        std::env::temp_dir().join(format!("loft_kept_header_{}_{tag}.rs", std::process::id()));
    let status = loft(
        &["--native-emit", out.to_str().unwrap(), src.to_str().unwrap()],
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

fn run_ok(args: &[&str], env: &[(&str, &str)]) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut a: Vec<&str> = args.to_vec();
    let s = src.to_str().unwrap().to_string();
    a.push(&s);
    let out = loft(&a, env);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.trim_end().ends_with("g10 24"),
        "{args:?} {env:?}: exit {:?}\nstdout: {stdout}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// @C122 — the contract is what a program computes, so the clause may be on or off: every
/// cell answers the same in both switch states, on both backends, under the falsifiers.
#[test]
fn the_cells_hold_on_both_backends_in_both_switch_states_under_the_falsifiers() {
    run_ok(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [None, Some(("LOFT_NO_KEPT_HEADER", "1"))] {
        for falsifier in [
            ("LOFT_HOIST_VERIFY", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
        ] {
            let mut env = vec![falsifier];
            env.extend(switch);
            run_ok(&["--native"], &env);
        }
    }
}

#[test]
fn a_kept_field_is_held_once_and_handed_down_the_chain() {
    let rust = emit("on", &[]);
    // `run` walks through `step` four times a pass: it binds the holder and hands it down.
    let run = body(&rust, "n_run");
    assert!(
        run.contains("//@FR-R-Header kept clause\n") && run.contains("n_step__kh(cell,"),
        "run binds the holder and calls step's kept twin:\n{run}"
    );
    // The twins hand the holder on, never deriving it.
    let step = body(&rust, "n_step__kh");
    assert!(
        step.contains("n_log__kh(cell,") && step.contains("n_bump__kh(cell,"),
        "step's twin hands the holder to log's and bump's twins:\n{step}"
    );
    assert!(
        !step.contains("vector::vec_header("),
        "a twin derives no header of its own:\n{step}"
    );
    // ensure's twin derives it again after its append's two halves, and nowhere else.
    let ensure = body(&rust, "n_ensure__kh");
    assert_eq!(
        ensure.matches("//@FR-R-Header kept clause: refreshed").count(),
        2,
        "ensure's twin refreshes after the append and after its finish:\n{ensure}"
    );
    // view_then calls via_view, which appends through a VIEW of the field and keeps nothing:
    // its plain call is followed by a refresh.
    let view = body(&rust, "n_view_then");
    let at = view.find("n_via_view").expect("view_then calls via_view");
    assert!(
        view[at..].contains("//@FR-R-Header kept clause: refreshed"),
        "the plain call to via_view is followed by a refresh:\n{view}"
    );
}

#[test]
fn a_body_whose_changes_no_holder_follows_keeps_nothing() {
    let rust = emit("decline", &[]);
    for f in ["n_fill_loop", "n_nested", "n_self_len", "n_via_view", "n_reset"] {
        let b = body(&rust, f);
        assert!(
            !b.contains("__kh"),
            "{f} appends inside a loop, calls an appender inside an expression or writes the field another way — it holds nothing:\n{b}"
        );
    }
}

#[test]
fn the_switch_restores_the_per_walk_derivation() {
    let rust = emit("off", &[("LOFT_NO_KEPT_HEADER", "1")]);
    assert!(
        !rust.contains("__kh"),
        "LOFT_NO_KEPT_HEADER=1 emits no holder and no kept twin"
    );
}
