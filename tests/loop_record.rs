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
            "--native",
            "--lean", // the CLOSED program (NATIVE.md § Optimisation tiers)
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
        {
            // Every detector at once: one build per switch state, each detector still on
            // (falsifying a guard is a one-time receipt, not a cost of every run).
            let mut env = vec![
                ("LOFT_STRICT_STORES", "1"),
                ("LOFT_POISON", "1"),
                ("LOFT_POISON_CLAIM", "1"),
                ("LOFT_NATIVE_LEAK_CHECK", "1"),
            ];
            env.extend(switch);
            run_ok(&["--native"], &env);
        }
    }
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
    // The IR's value records are held off: with them l16's callee answers a tuple, which
    // borrows nothing, and the borrowing hand-off this pins has no subject.
    let out = loft(
        &["--interpret", src.to_str().unwrap()],
        &[
            ("LOFT_TRACE_LOOP_RECORD", "1"),
            ("LOFT_NO_CACHE", "1"),
            ("LOFT_NO_IR_VALUE_RECORD", "1"),
        ],
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

/// loft#2696 — an early `return S { … }` (`@FR-R-ValueRecord` `Object` leaf) after a kept loop
/// record's mint (`@FR-R-LoopRecord`) evaluates the literal's fields while the loop record is
/// still live, frees the loop record's store, and only then resets the local to `null`.
#[test]
fn an_early_value_record_return_after_a_mint_reads_and_frees_the_loop_record() {
    let dir = std::env::temp_dir().join(format!("loft_2696_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("mkdir");
    let file = dir.join("early_return_loop_record.loft");
    fa::write(
        &file,
        "\
struct P { v: integer }
struct Pair { a: integer, b: integer }
struct Refill { v: integer, xs: vector<integer> }

fn mk_p(n: integer) -> P { P { v: n } }

fn c1(k: integer) -> P {
  for i in 0..3 {
    _x = P { v: i + 20 };
    if i == k { return P { v: 32 }; }
  }
  P { v: 34 }
}

fn c2(k: integer) -> P {
  for i in 0..3 {
    x = P { v: i + 20 };
    if i == k { return P { v: x.v + 1 }; }
  }
  P { v: 34 }
}

fn c3(k: integer) -> Pair {
  for i in 0..3 {
    x = Pair { a: i + 10, b: i + 20 };
    if i == k { return Pair { b: x.a + 1, a: x.b + 2 }; }
  }
  Pair { a: 1, b: 2 }
}

fn c4(k: integer) -> P? {
  for i in 0..3 {
    x = P { v: i + 20 };
    if i == k {
      if k == 1 { return null; }
      return P { v: x.v + 5 };
    }
  }
  P { v: 99 }
}

fn c5(k: integer) -> P {
  for i in 0..3 {
    x = P { v: i + 20 };
    if i == k { return mk_p(x.v + 3); }
  }
  P { v: 50 }
}

fn c6(ps: const vector<P>, k: integer) -> P {
  for i in 0..3 {
    x = P { v: i };
    if i == k { return ps[x.v]; }
  }
  P { v: 60 }
}

fn c7(k: integer) -> P {
  for i in 0..3 {
    x = Refill { v: i + 10, xs: [1, 2, 3] };
    if i == k { return P { v: x.v + len(x.xs) }; }
  }
  P { v: 70 }
}

fn c8(k: integer) -> P {
  for o in 0..2 {
    for i in 0..3 {
      x = P { v: o * 10 + i };
      y = Pair { a: i, b: o };
      if o * 3 + i == k { return P { v: x.v + y.a + y.b + 1 }; }
    }
  }
  P { v: 80 }
}

fn test_c1() { t = 0; for k in 0..4 { t += c1(k).v; } assert(t == 32 * 3 + 34, \"c1: {t}\"); }
fn test_c2() { t = 0; for k in 0..4 { t += c2(k).v; } assert(t == 21 + 22 + 23 + 34, \"c2: {t}\"); }
fn test_c3() {
  a = 0; b = 0;
  for k in 0..4 { r = c3(k); a += r.a; b += r.b; }
  assert(a == 22 + 23 + 24 + 1 && b == 11 + 12 + 13 + 2, \"c3: {a},{b}\");
}
fn test_c4() {
  t = 0; n = 0;
  for k in 0..4 {
    r = c4(k);
    if r { t += r.v; } else { n += 1; }
  }
  assert(t == 25 + 27 + 99 && n == 1, \"c4: {t},{n}\");
}
fn test_c5() { t = 0; for k in 0..4 { t += c5(k).v; } assert(t == 23 + 24 + 25 + 50, \"c5: {t}\"); }
fn test_c6() {
  ps = [P { v: 7 }, P { v: 8 }, P { v: 9 }];
  t = 0;
  for k in 0..4 { t += c6(ps, k).v; }
  assert(t == 7 + 8 + 9 + 60, \"c6: {t}\");
}
fn test_c7() { t = 0; for k in 0..4 { t += c7(k).v; } assert(t == 13 + 14 + 15 + 70, \"c7: {t}\"); }
fn test_c8() { t = 0; for k in 0..7 { t += c8(k).v; } assert(t == 1 + 3 + 5 + 12 + 14 + 16 + 80, \"c8: {t}\"); }
",
    )
    .expect("write test");
    let path = file.to_str().unwrap();

    let interp = loft(
        &["--tests", "--interpret", path],
        &[("LOFT_LEAK_CHECK", "1"), ("LOFT_STRICT_STORES", "1")],
    );
    assert!(
        interp.status.success(),
        "--tests --interpret failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&interp.stdout),
        String::from_utf8_lossy(&interp.stderr)
    );

    for switch in [
        None,
        Some(("LOFT_NO_IR_VALUE_RECORD", "1")),
        Some(("LOFT_NO_LOOP_RECORD", "1")),
    ] {
        let mut env = vec![
            ("LOFT_STRICT_STORES", "1"),
            ("LOFT_POISON", "1"),
            ("LOFT_POISON_CLAIM", "1"),
            ("LOFT_NATIVE_LEAK_CHECK", "1"),
        ];
        env.extend(switch);
        let out = loft(&["--tests", "--native", path], &env);
        assert!(
            out.status.success(),
            "--tests --native {switch:?} failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let _ = fa::remove_dir_all(&dir);
}
