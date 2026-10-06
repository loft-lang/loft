// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-E-Report`'s dropped-write clause and `@FR-H-Write-Else`, with a logger attached, on
//! both backends: a write whose place names no record lands nowhere (`@FR-H-WriteNull`,
//! `@FR-H-WriteOOB`) and reports ONE Warn line — `index_out_of_bounds` where the index was
//! found out of range, `write_dropped` where the place was an absent key or a null view — and
//! a store with an `else` arm reports nothing, because the arm owns the failure.  Keeps the
//! "an unguarded dropped write logs one Warn line" clause of @C130.
//!
//! Every cell is its own program, so one cell's report cannot be another's: the logger keys
//! its rate limit by (file, line), and on `--native` the line is the running function's.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Run `body` inside `fn main` beside the shared declarations, from a fresh directory whose
/// `log.conf` names a known log file.  Answers (stdout, log).
fn run(name: &str, body: &str, backend: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!(
        "loft_dropped_{name}_{}_{}",
        backend.trim_start_matches('-'),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let source = format!(
        "struct P {{ x: integer, t: text }}\n\
         struct H {{ k: integer, v: integer }}\n\
         struct D {{ hs: hash<H[k]> }}\n\
         fn mk(x: integer) -> P {{ P {{ x: x, t: \"p{{x}}\" }} }}\n\
         fn main() {{\n\
         v = [1, 2, 3];\n\
         ps = [mk(1)];\n\
         d = D {{ hs: [H {{ k: 1, v: 10 }}] }};\n\
         q: P? = null;\n\
         {body}\n\
         println(\"v={{v}} ps={{len(ps)}} h={{len(d.hs)}} q={{q == null}}\");\n\
         }}\n"
    );
    let script = dir.join(format!("{name}.loft"));
    std::fs::write(&script, source).expect("write script");
    let log_path = dir.join("log.txt");
    let conf = format!("[log]\nfile = {}\nlevel = info\n", log_path.display());
    std::fs::write(dir.join("log.conf"), conf).expect("write log.conf");
    let out = Command::new(loft_bin())
        .arg(backend)
        .arg(&script)
        .current_dir(&dir)
        .env("LOFT_TIMEOUT", "240")
        .output()
        .expect("invoke loft");
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "{name} {backend}: exit {:?}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    (String::from_utf8_lossy(&out.stdout).into_owned(), log)
}

fn count(log: &str, label: &str) -> usize {
    log.lines()
        .filter(|l| l.contains(&format!("[{label}]")))
        .count()
}

/// (cell, the statements, index_out_of_bounds lines, write_dropped lines, stdout)
const CELLS: &[(&str, &str, usize, usize, &str)] = &[
    // An index out of range reports where it was found, once — the write through the
    // already-reported null adds nothing.
    ("element", "v[5] = 9;", 1, 0, "v=[1,2,3] ps=1 h=1 q=true"),
    ("field", "ps[4].x = 9;", 1, 0, "v=[1,2,3] ps=1 h=1 q=true"),
    (
        "record",
        "ps[4] = mk(9);",
        1,
        0,
        "v=[1,2,3] ps=1 h=1 q=true",
    ),
    // An absent key and a null view are found by no reporting access: the write reports.
    ("key", "d.hs[2].v = 9;", 0, 1, "v=[1,2,3] ps=1 h=1 q=true"),
    ("view", "q.x = 9;", 0, 1, "v=[1,2,3] ps=1 h=1 q=true"),
    (
        "view-text",
        "q.t = \"z\";",
        0,
        1,
        "v=[1,2,3] ps=1 h=1 q=true",
    ),
    // Two misses past the end in a loop: one line each (the hoisted writer on --native).
    (
        "loop",
        "for i in 0..5 { v[i] = i * 10; }",
        2,
        0,
        "v=[0,10,20] ps=1 h=1 q=true",
    ),
    // The guarded twin of every cell above reports nothing: its arm is the report.
    (
        "guarded",
        "m = 0; v[5] = 9 else { m += 1 }; ps[4].x = 9 else { m += 1 }; \
         ps[4] = mk(9) else { m += 1 }; d.hs[2].v = 9 else { m += 1 }; \
         q.x = 9 else { m += 1 }; q.t = \"z\" else { m += 1 }; \
         for i in 0..5 { v[i] = i * 10 else { m += 1 } } \
         assert(m == 8, \"every arm ran: {m}\");",
        0,
        0,
        "v=[0,10,20] ps=1 h=1 q=true",
    ),
    // A write that lands reports nothing, guarded or not.
    (
        "landed",
        "v[1] = 9; ps[0].x = 9; d.hs[1].v = 9 else { v[0] = 0 };",
        0,
        0,
        "v=[1,9,3] ps=1 h=1 q=true",
    ),
];

fn check(backend: &str) {
    for (name, body, oob, dropped, want) in CELLS {
        let (out, log) = run(name, body, backend);
        assert_eq!(out.trim(), *want, "{name} {backend}: stdout");
        assert_eq!(
            count(&log, "index_out_of_bounds"),
            *oob,
            "{name} {backend}: log\n{log}"
        );
        assert_eq!(
            count(&log, "write_dropped"),
            *dropped,
            "{name} {backend}: log\n{log}"
        );
    }
}

#[test]
fn each_unguarded_dropped_write_reports_once_and_a_guarded_one_never_interpreted() {
    check("--interpret");
}

#[test]
fn each_unguarded_dropped_write_reports_once_and_a_guarded_one_never_native() {
    check("--native");
}

/// A thousand dropped writes at one statement are the logger's to thin, not the runtime's:
/// five lines and a suppression count, never a thousand lines.  The count line arrives only
/// when the window closes, so the cell asserts the ceiling.
#[test]
fn a_thousand_dropped_writes_at_one_place_are_rate_limited() {
    for backend in ["--interpret", "--native"] {
        let (_, log) = run("thousand", "for _ in 0..1000 { q.x = 9; }", backend);
        let n = count(&log, "write_dropped");
        assert!((1..=5).contains(&n), "{backend}: {n} lines\n{log}");
    }
}
