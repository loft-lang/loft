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
//!
//! The HOISTED writers (`--native`'s loop forms: `vec_set_at`, `rec_set`, `push_windowed`,
//! `push_record_windowed`) read the lock state ONCE per loop — from the vector header, the
//! push window or the record address it was taken with — and a window over a locked store
//! has no capacity, so every push through it takes the runtime's refusing append.
//! Falsified against a build of c8ec0efba with the per-element checks deleted from those
//! writers: the `view`, `idxrec` and `index` cells changed the locked store (`v=77,77`,
//! `v=55,55`, `xs=[0,100,200]`).  The windowed cells do not move on that build — closing the
//! window refuses the length — so they guard the outcome, not the check.
//! Falsified again against the form where a locked header has no writable element and a
//! locked record address leaves `rec_set`'s fast path with the null one: a header bound
//! that ignores the lock fails the production, development and hoisted cells on `--native`
//! (`idxrec v=55,55`, `xs=[0,100,200]`, `loop` reached); a `rec_set` that ignores it fails
//! the hoisted `view` cell (`v=77,77`) and the development `view` cell.

use loft::file_access as fa;
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
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create tempdir");
    let script = dir.join(format!("{name}.loft"));
    fa::write(&script, source).expect("write script");
    let log_path = dir.join("log.txt");
    let conf = format!("[log]\nfile = {}\nlevel = info\n", log_path.display());
    fa::write(dir.join("log.conf"), conf).expect("write log.conf");
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
    let log = fa::read_to_string(&log_path).unwrap_or_default();
    let _ = fa::remove_dir_all(&dir);
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
    assert_eq!(
        stdout, UNCHANGED,
        "{backend}: a locked store changed\nstderr:\n{stderr}"
    );
    assert_eq!(
        code, 0,
        "{backend}: a production run must finish; stderr:\n{stderr}"
    );
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
const DEV_CELLS: [(&str, &str, &str); 6] = [
    ("scalar", "d.n = 99;", "write to a locked store"),
    ("text", "d.name = \"changed\";", "write to a locked store"),
    (
        "loop",
        "for i in 0..len(d.xs) { d.xs[i] = i * 100; }",
        "write to a locked store",
    ),
    (
        "from-end",
        "for i in 0..len(d.xs) { d.xs[i - len(d.xs)] = 9; }",
        "write to a locked store",
    ),
    (
        "view",
        "for r in d.recs { r.v = 77; }",
        "write to a locked store",
    ),
    ("constant", "NUMS += [3];", "write to a constant"),
];

/// A write PAST the end of a locked vector names no element (`@FR-H-WriteOOB`): it lands
/// nowhere and is no lock fault, so a development run does not halt on it.  The hoisted
/// writer sends a locked store and an absent element down the same slow path, which has to
/// keep telling them apart.
const DEV_PAST_END: &str = "for i in 0..len(d.xs) { d.xs[i + 7] = 9; }";

/// Bytes the program does not own: a development run halts on the write too.
const DEV_FOREIGN: &str = r#"fn main() {
  src: vector<u8> = [1, 2, 3];
  assert(write_bytes("locked_fm.tmp", src), "write");
  m = file_map("locked_fm.tmp") ?? [];
  m += [4 as u8];
  println("reached {m}");
}
"#;

/// One locked record and one write to it, then a line a halted run never prints.
fn dev_program(write: &str) -> String {
    format!(
        "struct E {{ k: integer, v: integer }}\n\
         struct D {{ n: integer, name: text, xs: vector<integer>, recs: vector<E> }}\n\
         NUMS: vector<integer> = [1, 2];\n\
         fn main() {{\n  d = D {{ n: 7, name: \"abc\", xs: [1, 2, 3], recs: [E {{ k: 1, v: 10 }}] }};\n  \
         d#lock = true;\n  {write}\n  \
         println(\"reached {{d.n}} {{d.name}} {{d.xs}} {{d.recs[0].v}} {{NUMS}}\");\n}}\n"
    )
}

fn development_halts_with_the_report(backend: &str) {
    let (stdout, stderr, code, _) = run("foreign", DEV_FOREIGN, backend, false);
    assert_ne!(
        code, 0,
        "{backend} foreign: a development run halts; stdout {stdout:?}"
    );
    assert!(
        !stdout.contains("reached"),
        "{backend} foreign: ran past the write"
    );
    assert!(
        stderr.contains("write to bytes the program does not own"),
        "{backend} foreign: the report; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "{backend} foreign: a crash:\n{stderr}"
    );
    for (name, write, report) in DEV_CELLS {
        let (stdout, stderr, code, _) = run(name, &dev_program(write), backend, false);
        assert_ne!(
            code, 0,
            "{backend} {name}: a development run halts; stdout {stdout:?}"
        );
        assert!(
            !stdout.contains("reached"),
            "{backend} {name}: ran past the write"
        );
        assert!(
            stderr.contains(report),
            "{backend} {name}: the report; stderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("panicked"),
            "{backend} {name}: a crash, not the report:\n{stderr}"
        );
    }
    let (stdout, stderr, code, _) = run("past-end", &dev_program(DEV_PAST_END), backend, false);
    assert_eq!(
        (stdout.as_str(), code),
        ("reached 7 abc [1,2,3] 10 [1,2]\n", 0),
        "{backend} past-end: a write to an absent element is no lock fault; stderr:\n{stderr}"
    );
}

#[test]
fn development_halts_on_a_locked_write_interpreted() {
    development_halts_with_the_report("--interpret");
}

#[test]
fn development_halts_on_a_locked_write_native() {
    development_halts_with_the_report("--native");
}

/// One cell per hoisted writer, each in the loop shape that reaches it on `--native`
/// ([`hoisted_loops_reach_the_writers_under_test`] pins that); every line prints the value
/// from BEFORE its write.
const HOISTED: &str = r#"struct E { k: integer, v: integer }
struct D { n: integer, xs: vector<integer>, recs: vector<E> }
fn view_loop(d: &D) { for r in d.recs { r.v = 77; } }
fn index_rec_loop(d: &D) { for i in 0..len(d.recs) { d.recs[i].v = 55; } }
fn index_loop(d: &D) { for i in 0..len(d.xs) { d.xs[i] = i * 100; } }
fn from_end_loop(d: &D) { for i in 0..len(d.xs) { d.xs[i - len(d.xs)] = i * 100 + 5; } }
fn past_end_loop(d: &D) { for i in 0..len(d.xs) { d.xs[i + 7] = 9; } }
fn main() {
  o = D { n: 7, xs: [1, 2, 3], recs: [E { k: 1, v: 10 }, E { k: 2, v: 20 }] };
  view_loop(o); index_loop(o); println("open view v={o.recs[0].v},{o.recs[1].v} index xs={o.xs}");
  index_rec_loop(o); from_end_loop(o); past_end_loop(o);
  println("open idxrec v={o.recs[0].v},{o.recs[1].v} from-end xs={o.xs}");
  d = D { n: 7, xs: [1, 2, 3], recs: [E { k: 1, v: 10 }, E { k: 2, v: 20 }] };
  d#lock = true;
  view_loop(d); println("view v={d.recs[0].v},{d.recs[1].v}");
  index_rec_loop(d); println("idxrec v={d.recs[0].v},{d.recs[1].v}");
  index_loop(d); println("index xs={d.xs}");
  from_end_loop(d); println("from-end xs={d.xs}");
  past_end_loop(d); println("past-end xs={d.xs}");
  ys = [1, 2, 3];
  ys#lock = true;
  for i in 0..3 { ys += [i * 11]; }
  println("push ys={ys}");
  ts = [E { k: 1, v: 10 }];
  ts#lock = true;
  for i in 0..3 { ts += [E { k: i + 20, v: i + 30 }]; }
  println("mint ts={len(ts)} v={ts[0].v}");
  println("done");
}
"#;

/// The two `open` lines are the same loops over an UNLOCKED record, so a writer that wrote
/// nothing at all could not pass for one that refused: `view` then `index` leave 77s and
/// `[0,100,200]`; `idxrec` then `from-end` (`xs[i - 3] = i * 100 + 5`) leave 55s and
/// `[5,105,205]`, and `past-end` writes no element.
const HOISTED_UNCHANGED: &str = "open view v=77,77 index xs=[0,100,200]\n\
     open idxrec v=55,55 from-end xs=[5,105,205]\n\
     view v=10,20\nidxrec v=10,20\nindex xs=[1,2,3]\nfrom-end xs=[1,2,3]\npast-end xs=[1,2,3]\n\
     push ys=[1,2,3]\nmint ts=1 v=10\ndone\n";

fn production_discards_every_hoisted_write(backend: &str) {
    let (stdout, stderr, code, log) = run("hoisted", HOISTED, backend, true);
    assert_eq!(
        stdout, HOISTED_UNCHANGED,
        "{backend}: a hoisted loop changed a locked store\nstderr:\n{stderr}"
    );
    assert_eq!(
        code, 0,
        "{backend}: a production run must finish; stderr:\n{stderr}"
    );
    assert!(
        log.contains("[write_to_locked_store]"),
        "{backend}: the discarded writes are logged; log:\n{log}"
    );
    assert!(!stderr.contains("panicked"), "{backend}: {stderr}");
}

#[test]
fn production_discards_every_hoisted_write_interpreted() {
    production_discards_every_hoisted_write("--interpret");
}

#[test]
fn production_discards_every_hoisted_write_native() {
    production_discards_every_hoisted_write("--native");
}

/// The cells above are only a guard of the hoisted writers while `--native` still emits
/// them for these loops: a shape that fell back to the runtime's per-write path would pass
/// for the wrong reason.  And `rec_set` takes the lock state held beside its address.
#[test]
fn hoisted_loops_reach_the_writers_under_test() {
    let dir = std::env::temp_dir().join(format!("loft_locked_emit_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create tempdir");
    let script = dir.join("hoisted.loft");
    fa::write(&script, HOISTED).expect("write script");
    let out = dir.join("hoisted.rs");
    let status = Command::new(loft_bin())
        .arg("--native-emit")
        .arg(&out)
        .arg("--lean")
        .arg(&script)
        .current_dir(&dir)
        .output()
        .expect("invoke loft");
    let rust = fa::read_to_string(&out).unwrap_or_default();
    let _ = fa::remove_dir_all(&dir);
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    for writer in [
        "vec_set_at::<",
        "push_windowed::<",
        "push_record_windowed::<",
    ] {
        assert!(rust.contains(writer), "no `{writer}` in the emitted loops");
    }
    assert!(
        rust.contains("rec_set::<i64>(__pa_") && rust.contains("let __pl_"),
        "`rec_set` is emitted with a lock state bound beside its address"
    );
    assert_eq!(
        rust.matches("rec_set::<").count(),
        rust.matches(", __pl_").count(),
        "every `rec_set` passes its `__pl_` lock state"
    );
}

/// The windowed shapes in a development run: a window over a locked store has no room, so
/// the first push or mint goes through the runtime's append, which halts with the report.
const DEV_WINDOWED: [(&str, &str); 2] = [
    (
        "wpush",
        "fn main() {\n  ys = [1, 2, 3];\n  ys#lock = true;\n  for i in 0..3 { ys += [i * 11]; }\n  println(\"reached {ys}\");\n}\n",
    ),
    (
        "wmint",
        "struct E { k: integer, v: integer }\nfn main() {\n  ts = [E { k: 1, v: 10 }];\n  ts#lock = true;\n  for i in 0..3 { ts += [E { k: i + 20, v: i + 30 }]; }\n  println(\"reached {len(ts)}\");\n}\n",
    ),
];

fn development_halts_in_a_windowed_loop(backend: &str) {
    for (name, source) in DEV_WINDOWED {
        let (stdout, stderr, code, _) = run(name, source, backend, false);
        assert_ne!(
            code, 0,
            "{backend} {name}: a development run halts; stdout {stdout:?}"
        );
        assert!(
            !stdout.contains("reached"),
            "{backend} {name}: ran past the write"
        );
        assert!(
            stderr.contains("locked store"),
            "{backend} {name}: the report; stderr:\n{stderr}"
        );
        assert!(
            !stderr.contains("panicked"),
            "{backend} {name}: a crash:\n{stderr}"
        );
    }
}

#[test]
fn development_halts_in_a_windowed_loop_interpreted() {
    development_halts_in_a_windowed_loop("--interpret");
}

#[test]
fn development_halts_in_a_windowed_loop_native() {
    development_halts_in_a_windowed_loop("--native");
}

/// The production log line gives the advice the store's lock calls for — the same advice the
/// development report gives for that write.  A mapped file cannot be unlocked: it was told
/// to `#lock = false`, a cure that does not exist, while the author's own `#lock` is the one
/// lock that advice is right for.
fn production_logs_the_advice_its_lock_calls_for(backend: &str) {
    let mapped = r#"
fn main() {
  src: vector<u8> = [1, 2, 3];
  assert(write_bytes("locked_advice_fm.tmp", src), "write");
  m = file_map("locked_advice_fm.tmp") ?? [];
  m[0] = 9 as u8;
  println("m={m}");
}
"#;
    let (stdout, stderr, code, log) = run("advice_mapped", mapped, backend, true);
    assert_eq!(
        (stdout.as_str(), code),
        ("m=[1,2,3]\n", 0),
        "{backend}: {stderr}"
    );
    assert!(
        log.contains("[write_to_locked_store]") && log.contains("copy them first"),
        "{backend}: a mapped file's discarded write names the copy cure; log:\n{log}"
    );
    assert!(
        !log.contains("#lock = false"),
        "{backend}: a mapped file cannot be unlocked; log:\n{log}"
    );

    let locked = r#"
struct B { n: integer }
fn main() {
  v = [B { n: 1 }];
  v#lock = true;
  v[0].n = 5;
  println("n={v[0].n}");
}
"#;
    let (stdout, stderr, code, log) = run("advice_locked", locked, backend, true);
    assert_eq!((stdout.as_str(), code), ("n=1\n", 0), "{backend}: {stderr}");
    assert!(
        log.contains("unlock it with `#lock = false`"),
        "{backend}: the author's own lock keeps its unlock cure; log:\n{log}"
    );
}

#[test]
fn production_logs_the_advice_its_lock_calls_for_interpreted() {
    production_logs_the_advice_its_lock_calls_for("--interpret");
}

#[test]
fn production_logs_the_advice_its_lock_calls_for_native() {
    production_logs_the_advice_its_lock_calls_for("--native");
}
