// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `(R-CSplit)` (@PLN212): each `tests/c_split/*.loft` cell, built by the C backend with a
//! budget small enough that every function it holds is split, answers its hand-computed
//! values (the cell asserts them), exits 0 and frees every store — and the trace shows each
//! function the cell names really was split, so no cell passes by staying whole.  The same
//! cell runs on the interpreter as the control.
//!
//! @falsified-at: 26197a482 — scored by hand against two defects planted in `src/c/split.rs`.
//!   CHANNEL: value (each cell's own assertion), on `--native-c`.
//!   ARMED: LOFT_C_SPLIT=4 (set by the test; without it nothing splits).
//!   WITNESS: (a) every part works on a COPY of the frame, so its writes never reach the
//!   function: all five cells fail — `members` answers `integer 0`, `liveness` `the view
//!   writes its slot: null null`, `stores` `items 0`.  (b) a part's `return` answers 0, so the
//!   function runs on past it: `boundary` fails with `tail ` (empty where `7-7-7-!` is due).
//!   HOLDS: the interpreter run passes on both trees, and no run reports a leak.

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn cell(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/c_split/{name}.loft"))
}

fn have_cc() -> bool {
    loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Run `name` on the interpreter and split on the C backend; `split` names the functions the
/// trace must report as split into parts.
fn check(name: &str, split: &[&str]) {
    let want = format!("{name} ok\n");
    let i = loft::platform::process::harness_command(loft_bin())
        .args(["--interpret", "--no-warnings"])
        .arg(cell(name))
        .output()
        .expect("loft runs");
    assert_eq!(
        String::from_utf8_lossy(&i.stdout),
        want,
        "{name} on the interpreter"
    );
    assert!(i.status.success(), "{name} on the interpreter: {i:?}");
    if !have_cc() {
        return;
    }
    let c = loft::platform::process::harness_command(loft_bin())
        .args(["--native-c", "--no-warnings"])
        .arg(cell(name))
        .env("LOFT_C_SPLIT", "4")
        .env("LOFT_TRACE_C_SPLIT", "1")
        .env("LOFT_NATIVE_LEAK_CHECK", "1")
        .env_remove("LOFT_NO_C_SPLIT")
        .output()
        .expect("loft runs");
    let err = String::from_utf8_lossy(&c.stderr);
    assert_eq!(
        String::from_utf8_lossy(&c.stdout),
        want,
        "{name} split: {err}"
    );
    assert!(c.status.success(), "{name} split: {err}");
    assert!(
        !err.contains("stores not freed"),
        "{name} split leaks: {err}"
    );
    for f in split {
        let line = format!("[c-split] {f}__fr: ");
        assert!(
            err.lines()
                .any(|l| l.starts_with(&line) && l.contains(" parts (")),
            "{name}: `{f}` was not split: {err}"
        );
    }
}

/// The C `args` write for `prog`, the split switches cleared and `env` set.
fn emitted(prog: &std::path::Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let out = prog.with_extension(format!("{}.c", args.len() * 10 + env.len()));
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.args(["--native-c", "--no-warnings"])
        .args(args)
        .arg("--native-emit")
        .arg(&out)
        .arg(prog)
        .env_remove("LOFT_C_SPLIT")
        .env_remove("LOFT_NO_C_SPLIT");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let r = cmd.output().expect("loft runs");
    assert!(r.status.success(), "{args:?} {env:?}: {r:?}");
    std::fs::read_to_string(&out).expect("the unit is written")
}

/// @PLN212 P2: a semantics build splits an over-budget function by default (the MSVC sweep's
/// `cl` time stays bounded without a switch), and a shipped build (`--ship`) never does — not
/// even under a switch that asks — so a shipped artefact's C is the C it was before the split.
///
/// @falsified-at: 4bce889d7 — by hand.  The default-on assertion fails on the tree before it
/// (no `__part` without `LOFT_C_SPLIT`); with `functions_whole` planted to answer false, the
/// `--ship` assertion fails ("a shipped unit keeps its functions whole").
#[test]
fn a_semantics_build_splits_by_default_and_a_shipped_one_never() {
    let dir = std::env::temp_dir().join(format!("loft-c-split-tier-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A `main` far over the default budget of runtime-call sites: each line appends to a text
    // and checks its length.
    let mut src = String::from("fn main() {\n    s = \"\";\n");
    for i in 0..400 {
        src += &format!("    s += \"{i}\";\n    assert(len(s) > {i}, \"grew\");\n");
    }
    src += "    println(\"{len(s)}\");\n}\n";
    let prog = dir.join("large.loft");
    std::fs::write(&prog, src).unwrap();
    assert!(
        emitted(&prog, &[], &[]).contains("n_main__fr__part"),
        "the default build splits"
    );
    assert!(
        !emitted(&prog, &["--ship"], &[]).contains("__part"),
        "a shipped unit keeps its functions whole"
    );
    assert!(
        !emitted(&prog, &["--ship"], &[("LOFT_C_SPLIT", "4")]).contains("__part"),
        "no switch splits a shipped unit"
    );
    assert!(
        !emitted(&prog, &[], &[("LOFT_NO_C_SPLIT", "1")]).contains("__part"),
        "LOFT_NO_C_SPLIT keeps the semantics build whole"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_member_shape_crosses_from_part_to_part() {
    check("members", &["n_main"]);
}

#[test]
fn a_view_written_through_in_a_later_part_writes_its_slot() {
    check("liveness", &["n_main"]);
}

#[test]
fn stores_made_in_one_part_are_freed_in_another() {
    check("stores", &["n_main", "n_grow", "n_build"]);
}

#[test]
fn a_loop_match_block_value_return_and_generator_answer_as_whole() {
    check("control", &["n_main", "n_looped", "n_chosen"]);
}

#[test]
fn a_split_before_the_tail_value_and_around_an_early_return() {
    check("boundary", &["n_main", "n_tail", "n_early"]);
}
