// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-LoopRecord` — the EMISSION pins.  A plain no-heap record local declared and minted
//! by a literal INSIDE a loop keeps its store and its record across iterations: the local is
//! declared at the loop's prelude, the per-pass mint is a null-guarded first mint (a complete
//! literal needs nothing after it, a partial one only the prefill), the body's `Set(v, null)`
//! and end-of-body free are not emitted, and one free follows the loop.
//! `LOFT_NO_LOOP_RECORD=1` restores the per-pass free and fresh store.  The cell corpus
//! (`tests/scripts/157-loop-record.loft`) says the VALUES hold on both backends, in both
//! switch states and under the falsifiers; this pins what is emitted and runs it.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CELLS: &str = "tests/scripts/157-loop-record.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_LOOP_RECORD")
        .env_remove("LOFT_NO_LOOP_BUFFER_REUSE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    // One file per TEST: the tests run on threads of one process.
    let out =
        std::env::temp_dir().join(format!("loft_loop_record_{}_{tag}.rs", std::process::id()));
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
fn the_cells_hold_on_both_backends_in_both_switch_states_under_the_falsifiers() {
    run_ok(&["--interpret"], &[("LOFT_STRICT_STORES", "1")]);
    for switch in [None, Some(("LOFT_NO_LOOP_RECORD", "1"))] {
        for falsifier in [
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_POISON_CLAIM", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
        ] {
            let mut env = vec![falsifier];
            env.extend(switch);
            run_ok(&["--native"], &env);
        }
    }
}

#[test]
fn an_admitted_record_is_declared_at_the_prelude_kept_across_passes_and_freed_once_after() {
    let rust = emit("on", &[]);
    let main = body(&rust, "n_main");
    // l1–l7, l9 (two), l14, l15, l17 keep their store: twelve prelude declarations and twelve
    // frees after their loops; l8's `r` is returned, l10's owns text, l11's is copied,
    // l12's `o` carries a record field, l13's is appended, l16's is handed to a callee
    // whose return borrows that parameter.
    let kept = main
        .matches("//@FR-R-LoopRecord kept across iterations")
        .count();
    let freed = main
        .matches("/*@FR-R-LoopRecord freed after the loop*/")
        .count();
    assert_eq!(kept, 12, "prelude declarations:\n{main}");
    assert_eq!(freed, 12, "frees after the loop:\n{main}");
    // The per-pass mint is null-guarded and re-mints nothing after the first pass: every
    // literal here is a complete write (the parser spells an omitted field's declared default
    // as an explicit write — l15's `Ld { a: i }` writes `k = 5` itself), so the partial-literal
    // form that re-establishes defaults through `set_default_value` is the emitter's fallback
    // and no cell reaches it.
    assert_eq!(
        main.matches("/* @FR-R-LoopRecord kept record */").count(),
        12,
        "kept-record mints:\n{main}"
    );
    assert!(
        !main.contains("defaults re-established"),
        "every literal here is a complete write; the prefill form should not appear:\n{main}"
    );
    // A kept local's free never sits inside its loop body.
    let ret = body(&rust, "n_lr_ret_in_loop");
    assert!(
        !ret.contains("@FR-R-LoopRecord"),
        "a record returned from inside the loop must not be kept:\n{ret}"
    );
}

#[test]
fn the_switch_restores_the_per_pass_free_and_fresh_store() {
    // The emission embeds the loft SOURCE (`LOFT_SRC`), whose header names the rule, so the
    // pin looks for the emitter's own markers.
    let rust = emit("off", &[("LOFT_NO_LOOP_RECORD", "1")]);
    for marker in [
        "kept across iterations",
        "freed after the loop",
        "@FR-R-LoopRecord kept record",
    ] {
        assert!(
            !rust.contains(marker),
            "a kept record survived the switch ({marker})"
        );
    }
}

#[test]
fn the_declines_are_the_designed_ones() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let out = loft(
        &["--native-emit", "/dev/null", src.to_str().unwrap()],
        &[("LOFT_TRACE_LOOP_RECORD", "1")],
    );
    let trace = String::from_utf8_lossy(&out.stderr);
    assert!(
        trace.contains("declines: copied to another local"),
        "l11's copy must decline:\n{trace}"
    );
    assert!(
        trace.contains("declines: a native op takes it otherwise"),
        "l13's append must decline:\n{trace}"
    );
    assert!(
        trace.contains("declines: handed to a callee whose return borrows it"),
        "l16's hand-off to a borrowing callee must decline:\n{trace}"
    );
}
