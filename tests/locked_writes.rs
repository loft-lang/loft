// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-H-WriteLocked` in both run modes, on both backends: a write to a LOCKED store — the
//! author's `#lock`, a constant, or bytes the program does not own (`file_map`) — never
//! changes the store.  A development run halts with the report; a production run logs the
//! write, DISCARDS it and continues on the old bytes (DESIGN_DECISIONS.md C80: nothing
//! stops a production program).  Keeps @C80 and the lock-fault clause of @C130: a lock
//! fault is not a dropped write, so unguarded it halts development rather than logging.
//!
//! @falsified-at: 1f91d81f0 — the same cells on that build: every production cell halted
//!   (exit 1, the development report), `d.name = …` and `d.name += …` crashed with a Rust
//!   panic (`Claim on read-only store`) in both modes, and the hoisted loop write on
//!   `--native` changed the locked vector silently (`xs=[0,100,200]`, exit 0).

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Run `source` from a fresh directory whose `log.conf` names a known log file.
/// Answers (stdout, stderr, exit code, log).
fn run(name: &str, source: &str, backend: &str, production: bool) -> (String, String, i32, String) {
    let dir = std::env::temp_dir().join(format!(
        "loft_locked_{name}_{}_{}",
        backend.trim_start_matches('-'),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let script = dir.join(format!("{name}.loft"));
    std::fs::write(&script, source).expect("write script");
    let log_path = dir.join("log.txt");
    let conf = format!("[log]\nfile = {}\nlevel = info\n", log_path.display());
    std::fs::write(dir.join("log.conf"), conf).expect("write log.conf");
    let mut cmd = Command::new(loft_bin());
    if production {
        cmd.arg("--production");
    }
    let out = cmd
        .arg(backend)
        .arg(&script)
        .current_dir(&dir)
        .env("LOFT_TIMEOUT", "240")
        .output()
        .expect("invoke loft");
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
        log,
    )
}

/// Every write kind the matrix found a distinct route for, against every lock origin.  Each
/// line prints the value AFTER its write, and every expected value is the value BEFORE it.
const EVERY_ROUTE: &str = r#"struct In { m: integer }
struct E { k: integer, v: integer }
struct H { k: integer, v: integer }
struct D { n: integer, f: float, b: boolean, name: text, xs: vector<integer>, inner: In, recs: vector<E>, hs: hash<H[k]> }
NUMS: vector<integer> = [1, 2];
fn main() {
  d = D { n: 7, f: 1.5, b: true, name: "abc", xs: [1, 2, 3], inner: In { m: 4 }, recs: [E { k: 1, v: 10 }], hs: [H { k: 5, v: 50 }] };
  d#lock = true;
  d.n = 99; println("n={d.n}");
  d.f = 9.25; println("f={d.f}");
  d.b = false; println("b={d.b}");
  d.name = "changed"; println("name={d.name}");
  d.name += "def"; println("name={d.name}");
  d.xs += [4]; println("xs={d.xs}");
  d.xs[1] = 20; println("xs={d.xs}");
  d.xs.remove(0); println("xs={d.xs}");
  for i in 0..len(d.xs) { d.xs[i] = i * 100; }
  println("xs={d.xs}");
  d.inner.m = 44; println("m={d.inner.m}");
  d.recs[0].v = 11; println("v={d.recs[0].v}");
  d.hs += [H { k: 6, v: 60 }]; println("hs={len(d.hs)}");
  d.hs[5] = null; println("hs={len(d.hs)}");
  NUMS += [3]; println("nums={NUMS}");
  NUMS[0] = 9; println("nums={NUMS}");
  src: vector<u8> = [1, 2, 3];
  assert(write_bytes("locked_fm.tmp", src), "write");
  m = file_map("locked_fm.tmp") ?? [];
  m[1] = 20 as u8; println("m={m}");
  m += [4 as u8]; println("m={m}");
  for i in 0..len(m) { m[i] = (i * 10) as u8? ?? 0; }
  println("m={m}");
  println("done");
}
"#;

const UNCHANGED: &str = "n=7\nf=1.5\nb=true\nname=abc\nname=abc\nxs=[1,2,3]\nxs=[1,2,3]\n\
xs=[1,2,3]\nxs=[1,2,3]\nm=4\nv=10\nhs=1\nhs=1\nnums=[1,2]\nnums=[1,2]\nm=[1,2,3]\nm=[1,2,3]\n\
m=[1,2,3]\ndone\n";

fn production_discards_every_route(backend: &str) {
    let (stdout, stderr, code, log) = run("every_route", EVERY_ROUTE, backend, true);
    assert_eq!(stdout, UNCHANGED, "{backend}: a locked store changed\nstderr:\n{stderr}");
    assert_eq!(code, 0, "{backend}: a production run must finish; stderr:\n{stderr}");
    assert!(
        log.contains("[write_to_locked_store]"),
        "{backend}: the discarded writes are logged; log:\n{log}"
    );
    assert!(!stderr.contains("panicked"), "{backend}: {stderr}");
}

#[test]
fn production_discards_every_locked_write_interpreted() {
    production_discards_every_route("--interpret");
}

#[test]
fn production_discards_every_locked_write_native() {
    production_discards_every_route("--native");
}

/// One write per program, since a development run stops at the first.  The three cells
/// are the three shapes the matrix saw fail differently: a scalar field, a text field (a
/// claim — it crashed), and a hoisted loop (native wrote through it silently).
const DEV_CELLS: [(&str, &str, &str); 4] = [
    ("scalar", "d.n = 99;", "write to a locked store"),
    ("text", "d.name = \"changed\";", "write to a locked store"),
    ("loop", "for i in 0..len(d.xs) { d.xs[i] = i * 100; }", "write to a locked store"),
    ("constant", "NUMS += [3];", "write to a constant"),
];

/// Bytes the program does not own: a development run halts on the write too.
const DEV_FOREIGN: &str = r#"fn main() {
  src: vector<u8> = [1, 2, 3];
  assert(write_bytes("locked_fm.tmp", src), "write");
  m = file_map("locked_fm.tmp") ?? [];
  m += [4 as u8];
  println("reached {m}");
}
"#;

fn development_halts_with_the_report(backend: &str) {
    let (stdout, stderr, code, _) = run("foreign", DEV_FOREIGN, backend, false);
    assert_ne!(code, 0, "{backend} foreign: a development run halts; stdout {stdout:?}");
    assert!(!stdout.contains("reached"), "{backend} foreign: ran past the write");
    assert!(
        stderr.contains("write to bytes the program does not own"),
        "{backend} foreign: the report; stderr:\n{stderr}"
    );
    assert!(!stderr.contains("panicked"), "{backend} foreign: a crash:\n{stderr}");
    for (name, write, report) in DEV_CELLS {
        let source = format!(
            "struct D {{ n: integer, name: text, xs: vector<integer> }}\n\
             NUMS: vector<integer> = [1, 2];\n\
             fn main() {{\n  d = D {{ n: 7, name: \"abc\", xs: [1, 2, 3] }};\n  d#lock = true;\n  \
             {write}\n  println(\"reached {{d.n}} {{d.name}} {{d.xs}} {{NUMS}}\");\n}}\n"
        );
        let (stdout, stderr, code, _) = run(name, &source, backend, false);
        assert_ne!(code, 0, "{backend} {name}: a development run halts; stdout {stdout:?}");
        assert!(!stdout.contains("reached"), "{backend} {name}: ran past the write");
        assert!(stderr.contains(report), "{backend} {name}: the report; stderr:\n{stderr}");
        assert!(!stderr.contains("panicked"), "{backend} {name}: a crash, not the report:\n{stderr}");
    }
}

#[test]
fn development_halts_on_a_locked_write_interpreted() {
    development_halts_with_the_report("--interpret");
}

#[test]
fn development_halts_on_a_locked_write_native() {
    development_halts_with_the_report("--native");
}
