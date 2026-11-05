// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-LoopRecord` — the EMISSION pins.  A plain no-heap record local declared and minted
//! by a literal INSIDE a loop keeps its store and its record across iterations, decided and
//! applied in the IR (`opt::loop_record`): the local is declared before the outermost loop,
//! the per-pass mint is a null-guarded first mint (`if OpRefIsNull(v) { mint }`, the block
//! named `loop record mint`), the body's `Set(v, null)` and end-of-body free go, and one free
//! follows the loop.  The pins count the IR pass's trace and read the IR it leaves.
//! `LOFT_NO_LOOP_RECORD=1` restores the per-pass free and fresh store.  The cell corpus
//! (`tests/scripts/157-loop-record.loft`) says the VALUES hold on both backends, in both
//! switch states and under the falsifiers; this pins what is emitted and runs it.
use loft::file_access as fa;
use std::path::{Path, PathBuf};
use std::process::Output;

const CELLS: &str = "tests/scripts/157-loop-record.loft";

fn loft(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd =
        loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.args(args)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_LOOP_RECORD")
        .env_remove("LOFT_NO_LOOP_BUFFER_REUSE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn loft")
}

/// The records the IR pass kept in function `name` of `src` (its `LOFT_TRACE_IR_LOOP_RECORD`
/// lines), and the IR it left (`LOFT_LOG=static`).
fn ir_of(src: &Path, name: &str, env: &[(&str, &str)]) -> (Vec<String>, String) {
    let mut env: Vec<(&str, &str)> = env.to_vec();
    env.extend([
        ("LOFT_TRACE_IR_LOOP_RECORD", "1"),
        ("LOFT_NO_CACHE", "1"),
        ("LOFT_LOG", "static"),
    ]);
    let out = loft(&["--interpret", src.to_str().unwrap()], &env);
    let err =
        String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    let kept: Vec<String> = err
        .lines()
        .filter_map(|l| l.strip_prefix(&format!("[ir-loop-record] fn={name} ")))
        .filter(|l| l.contains("kept across the passes"))
        .map(|l| l.split(' ').next().unwrap_or("").to_string())
        .collect();
    let start = err.find(&format!("fn {name}(")).unwrap_or(0);
    let rest = &err[start..];
    let end = rest[3..].find("\nfn ").map_or(rest.len(), |i| i + 3);
    (kept, rest[..end].to_string())
}

fn emit(tag: &str, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    // One file per TEST: the tests run on threads of one process.
    let out =
        std::env::temp_dir().join(format!("loft_loop_record_{}_{tag}.rs", std::process::id()));
    let status = loft(
        &[
            "--native-rust",
            "--native-emit",
            out.to_str().unwrap(),
            src.to_str().unwrap(),
        ],
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
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    // l1–l7, l9 (two), l14, l15, l17 keep their store: twelve records; l8's `r` is returned,
    // l10's owns text, l11's is copied, l12's `o` carries a record field, l13's is appended,
    // l16's is handed to a callee whose return borrows that parameter.
    let (kept, _) = ir_of(&src, "n_main", &[]);
    assert_eq!(kept.len(), 12, "kept records: {kept:?}");
    // The per-pass mint is null-guarded: every literal here is a complete write (the parser
    // spells an omitted field's declared default as an explicit write — l15's `Ld { a: i }`
    // writes `k = 5` itself), so each kept record's mint is one `loop record mint` block.
    let rust = emit("on", &[]);
    let main = body(&rust, "n_main");
    assert_eq!(
        main.matches("//loop record mint").count(),
        12,
        "kept-record mints:\n{main}"
    );
    // A record returned from inside the loop is not kept.
    let (ret, _) = ir_of(&src, "n_lr_ret_in_loop", &[]);
    assert!(
        ret.is_empty(),
        "a record returned from inside the loop must not be kept: {ret:?}"
    );
}

#[test]
fn the_switch_restores_the_per_pass_free_and_fresh_store() {
    // The emission embeds the loft SOURCE (`LOFT_SRC`), whose header names the rule, so the
    // pin looks for the emitter's own markers.
    let rust = emit("off", &[("LOFT_NO_LOOP_RECORD", "1")]);
    assert!(
        !rust.contains("//loop record mint"),
        "a kept record survived the switch"
    );
}

#[test]
fn the_declines_are_the_designed_ones() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    // The IR pass asks the analysis for every backend; the interpreter's run is the cheapest.
    let out = loft(
        &["--interpret", src.to_str().unwrap()],
        &[("LOFT_TRACE_LOOP_RECORD", "1"), ("LOFT_NO_CACHE", "1")],
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

const NEST: &str = "tests/scripts/a-loop-record-is-kept-across-its-whole-nest.loft";

#[test]
fn a_record_in_a_nest_is_kept_across_the_outermost_loop() {
    // The kept record's `= null` declaration precedes the FIRST `loop` of `n1` (the outer `i`
    // loop); under `LOFT_NO_LOOP_RECORD_NEST=1` it stands between the two loops.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(NEST);
    let decl_vs_loop = |env: &[(&str, &str)]| -> (usize, usize) {
        let (kept, ir) = ir_of(&src, "n_n1", env);
        let v = kept.first().expect("n1 keeps a record").clone();
        let decl = ir
            .find(&format!("{v}("))
            .and_then(|i| ir[i..].find("= null;").map(|j| i + j))
            .expect("the kept record's declaration");
        (decl, ir.find("loop {").expect("a loop"))
    };
    let (decl, first_loop) = decl_vs_loop(&[]);
    assert!(decl < first_loop, "declared before the outer loop");
    let (decl, first_loop) = decl_vs_loop(&[("LOFT_NO_LOOP_RECORD_NEST", "1")]);
    assert!(decl > first_loop, "declared inside the outer loop");
}
