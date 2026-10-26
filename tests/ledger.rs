// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The test ledger (@PLN201 step 2, `src/ledger.rs`, `src/opt/ledger_sites.rs`): a run with
//! `LOFT_LEDGER=<file>` is the ordinary checked run, on either backend, and records per fault
//! site how often it ran and how often the processor's answer would have differed.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("loft_ledger_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&d);
    fa::create_dir_all(&d).expect("scratch dir");
    d
}

/// Run `prog` on `mode`, recording into `ledger` when given; answers stdout and stderr.
fn run(mode: &[&str], prog: &Path, ledger: Option<&Path>) -> (String, String) {
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.args(mode)
        .arg(prog)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("LOFT_TIMEOUT", "600")
        .env_remove("LOFT_LEDGER");
    if let Some(l) = ledger {
        cmd.env("LOFT_LEDGER", l);
    }
    let out = cmd.output().expect("invoke loft");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{mode:?} failed: {stdout}{stderr}");
    (stdout, stderr)
}

/// The ledger lines of the functions named `n_<name>`, as `name ordinal kind runs faults`.
fn lines_of(ledger: &Path, names: &[&str]) -> Vec<String> {
    let entries = loft::ledger::read(&ledger.to_string_lossy()).expect("a readable ledger");
    let mut out: Vec<String> = entries
        .iter()
        .filter(|e| names.iter().any(|n| e.function == format!("n_{n}")))
        .map(|e| {
            format!(
                "{} {} {} {} {}",
                &e.function[2..],
                e.ordinal,
                e.kind,
                e.runs,
                e.faults
            )
        })
        .collect();
    out.sort();
    out
}

const PROGRAM: &str = "fn mix(h: integer, x: integer) -> integer { h * 31 + x }
fn big(n: integer) -> integer { n * n }
fn share(a: integer, b: integer) -> integer { a / b }
fn never(n: integer) -> integer { n - 7 }
fn main() {
  h = 0;
  for x in 0..10 { h = mix(h, x) % 1000003; }
  m: integer = null;
  println(\"{h} {big(4000000000)} {m + 1} {share(7, 2)} {share(7, 0)} {-h}\");
}
";

/// Each site's counts, hand-computed: `mix`'s `+` and `*` run ten times and never fault;
/// `4000000000²` overflows; `m + 1` adds to a null, which the processor would not carry; the
/// division runs twice and faults once, on its zero divisor; `never`'s `-` never runs.  Both
/// backends write the same ledger, and the program answers what it answers without one.
#[test]
fn both_backends_record_the_same_counts_and_answer_as_a_checked_run() {
    let dir = scratch("counts");
    let prog = dir.join("p.loft");
    fa::write(&prog, PROGRAM).expect("write program");
    let (plain, _) = run(&["--interpret"], &prog, None);
    assert_eq!(
        plain, "364560 null null 3 null -364560\n",
        "the checked answers"
    );
    let want = vec![
        "big 0 mul 1 1",
        "mix 0 add 10 0",
        "mix 1 mul 10 0",
        "never 0 sub 0 0",
        "share 0 div 2 1",
    ];
    let mut files = Vec::new();
    for (mode, tag) in [("--interpret", "i"), ("--native", "n")] {
        let ledger = dir.join(format!("{tag}.ledger"));
        let (out, _) = run(&[mode], &prog, Some(&ledger));
        assert_eq!(
            out, plain,
            "{mode} under a ledger answers as the checked run"
        );
        assert_eq!(
            lines_of(&ledger, &["mix", "big", "share", "never"]),
            want,
            "{mode}"
        );
        let main = lines_of(&ledger, &["main"]);
        assert!(
            main.iter().any(|l| l.ends_with(" add 1 1")),
            "{mode}: `m + 1` on a null faults: {main:?}"
        );
        assert!(
            main.iter().any(|l| l.ends_with(" neg 1 0")),
            "{mode}: `-h` runs once: {main:?}"
        );
        files.push(fa::read_to_string(&ledger).expect("read ledger"));
    }
    assert_eq!(
        files[0], files[1],
        "the two backends' ledgers are the same file"
    );
    let _ = fa::remove_dir_all(&dir);
}

/// Runs merge: a second run, on the other backend, adds its counts to the first's.
#[test]
fn a_second_run_adds_its_counts() {
    let dir = scratch("merge");
    let prog = dir.join("p.loft");
    fa::write(&prog, PROGRAM).expect("write program");
    let ledger = dir.join("l.ledger");
    run(&["--interpret"], &prog, Some(&ledger));
    run(&["--native"], &prog, Some(&ledger));
    assert_eq!(
        lines_of(&ledger, &["mix", "big", "share"]),
        vec![
            "big 0 mul 2 2",
            "mix 0 add 20 0",
            "mix 1 mul 20 0",
            "share 0 div 4 2"
        ]
    );
    let _ = fa::remove_dir_all(&dir);
}

/// A site is named by what survives a rebuild of the same source: a function added above
/// another leaves that one's body hash alone, and an edit to a body changes its own.
#[test]
fn a_body_hash_moves_with_its_body_only() {
    let dir = scratch("hash");
    let prog = dir.join("p.loft");
    let hash_of = |src: &str, name: &str| -> u64 {
        fa::write(&prog, src).expect("write program");
        let ledger = dir.join("h.ledger");
        let _ = fa::remove_file(&ledger);
        run(&["--interpret"], &prog, Some(&ledger));
        loft::ledger::read(&ledger.to_string_lossy())
            .expect("ledger")
            .into_iter()
            .find(|e| e.function == format!("n_{name}"))
            .map(|e| e.body)
            .expect("the function's site")
    };
    let base = "fn f(a: integer) -> integer { a + 1 }\nfn main() { println(\"{f(1)}\"); }\n";
    let above = "fn g(a: integer) -> integer { a * 2 }\n\n\nfn f(a: integer) -> integer { a + 1 }\nfn main() { println(\"{f(1)} {g(2)}\"); }\n";
    let edited = "fn f(a: integer) -> integer { a + 2 }\nfn main() { println(\"{f(1)}\"); }\n";
    let h = hash_of(base, "f");
    assert_eq!(
        hash_of(above, "f"),
        h,
        "a function added above moves no hash"
    );
    assert_ne!(hash_of(edited, "f"), h, "an edited body has a new hash");
    let _ = fa::remove_dir_all(&dir);
}

/// A shipped build is never a ledger run, whatever the environment says: `--ship` with
/// `LOFT_LEDGER` set writes no ledger and answers the same.
#[test]
fn a_shipped_build_records_nothing() {
    let dir = scratch("ship");
    let prog = dir.join("p.loft");
    fa::write(&prog, PROGRAM).expect("write program");
    let ledger = dir.join("s.ledger");
    let (out, _) = run(&["--ship"], &prog, Some(&ledger));
    assert_eq!(out, "364560 null null 3 null -364560\n");
    assert!(!fa::exists(&ledger), "--ship wrote a ledger");
    let _ = fa::remove_dir_all(&dir);
}

/// `loft test` runs several programs in one process; each arms its own sites and all of them
/// reach the file.
#[test]
fn a_test_run_of_several_files_records_each() {
    let dir = scratch("tests");
    fa::write(
        dir.join("a.loft"),
        "fn add_a(x: integer) -> integer { x + 1 }\nfn test_a() { assert(add_a(1) == 2, \"a\"); }\n",
    )
    .expect("write a");
    fa::write(
        dir.join("b.loft"),
        "fn mul_b(x: integer) -> integer { x * 3 }\nfn test_b() { assert(mul_b(2) == 6, \"b\"); }\n",
    )
    .expect("write b");
    let ledger = dir.join("t.ledger");
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--tests")
        .arg(&dir)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("LOFT_TIMEOUT", "600")
        .env("LOFT_LEDGER", &ledger)
        .output()
        .expect("invoke loft --tests");
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        lines_of(&ledger, &["add_a", "mul_b"]),
        vec!["add_a 0 add 1 0", "mul_b 0 mul 1 0"]
    );
    let _ = fa::remove_dir_all(&dir);
}
