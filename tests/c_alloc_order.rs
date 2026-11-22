// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The C runtime's store events, read off `-DLOFT_ALLOC_TRACE`, in the order the store model
//! gives them (`doc/claude/STORE_MODEL.md`, D38).  The values pass on either order, so only the
//! trace sees a difference.  `scripts/c_alloc_diff.py` sets a whole program's events beside the
//! interpreter's; these pin two of them by hand.
use loft::file_access as fa;
use loft::platform::process::harness_command;
use std::path::PathBuf;

fn have_cc() -> bool {
    harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The program built as C with the allocation trace, run in a scratch directory: its output and
/// the store events after the program starts (`M`).
fn traced(name: &str, src: &str) -> (String, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("loft-c-alloc-{}-{name}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let prog = dir.join(format!("{name}.loft"));
    fa::write(&prog, src).expect("write the program");
    let c: PathBuf = dir.join(format!("{name}.c"));
    let emit = harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg("--emit-c")
        .arg(&c)
        .arg(&prog)
        .output()
        .expect("run loft --emit-c");
    assert!(
        emit.status.success(),
        "{name}: not carried: {}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let links: Vec<String> = fa::read_to_string(&c)
        .expect("read the C")
        .lines()
        .filter_map(|l| l.strip_prefix("/* link: ")?.strip_suffix(" */"))
        .map(str::to_string)
        .chain(
            loft::platform::c_system_link_libs()
                .iter()
                .map(|l| (*l).to_string()),
        )
        .collect();
    let exe = dir.join(name);
    let cc = harness_command("cc")
        .args(["-std=c11", "-O1", "-DLOFT_ALLOC_TRACE"])
        .arg(&c)
        .arg("-o")
        .arg(&exe)
        .arg("-lm")
        .args(&links)
        .output()
        .expect("run cc");
    assert!(
        cc.status.success(),
        "{name}: cc rejected the C:\n{}",
        String::from_utf8_lossy(&cc.stderr)
    );
    let trace = dir.join(format!("{name}.trace"));
    let out = harness_command(&exe)
        .current_dir(&dir)
        .env("LOFT_ALLOC_TRACE", &trace)
        .output()
        .expect("run the program");
    let events = fa::read_to_string(&trace)
        .expect("read the trace")
        .lines()
        .skip_while(|l| *l != "M")
        .skip(1)
        .map(str::to_string)
        .collect();
    (String::from_utf8_lossy(&out.stdout).into_owned(), events)
}

fn at(events: &[String], line: &str) -> usize {
    events
        .iter()
        .position(|e| e == line)
        .unwrap_or_else(|| panic!("no `{line}` in {events:?}"))
}

/// A rebind whose value reads nothing the variable holds frees the displaced store BEFORE the
/// value is built (`bind_lowering::re_bind`'s `DisplacedFree::Pre`), as the bytecode writer
/// does: the new store is made after the old one is gone, so the two are never live together.
///
/// @falsified-at: none — the order predates the guard on the interpreter, so the receipt is a
///   SABOTAGE (2026-11-21): with `displacing_rebind`'s `Pre` arm struck (`src/c/body.rs`), the
///   C stashes the old store and frees it after the call.
///   CHANNEL: the trace's event order.  WITNESS: `N 2 100` before `F 1`.
///   HOLDS: `3` and `6 p2` on either order.
#[test]
fn a_rebind_frees_the_displaced_store_before_the_value_is_built() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let (out, ev) = traced(
        "rebind",
        r#"
struct P { a: integer, n: text }
fn mk(k: integer) -> P { P { a: k * 3, n: "p{k}" } }
fn main() {
  x = mk(1);
  println("{x.a}");
  x = mk(2);
  println("{x.a} {x.n}");
}
"#,
    );
    assert_eq!(out, "3\n6 p2\n");
    assert!(
        at(&ev, "F 1") < at(&ev, "N 2 100"),
        "the displaced store is freed first: {ev:?}"
    );
}

/// A store freed whole only closes the `File` records it holds (`Stores::release_file_leases`):
/// no record of it is deleted first, the file's name included.
///
/// @falsified-at: none — the interpreter never deleted here, so the receipt is a SABOTAGE
///   (2026-11-21): with `loft_release_file_leases` walking `loft_remove_claims_mode` again
///   (`src/c/loft_io.h`), each `File` store deletes its name's record before its free.
///   CHANNEL: the trace's `D` events.  WITNESS: `D 2 7` before `F 2`.
///   HOLDS: `3 abc` either way.
#[test]
fn a_freed_store_closes_its_files_and_deletes_no_record() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let (out, ev) = traced(
        "files",
        r#"
fn main() {
  f = file("c_alloc_order.txt");
  f.write("abc");
  g = file("c_alloc_order.txt");
  println("{g.size} {g.content()}");
  delete("c_alloc_order.txt");
}
"#,
    );
    assert_eq!(out, "3 abc\n");
    let frees = ev.iter().filter(|e| e.starts_with("F ")).count();
    assert_eq!(frees, 2, "both file stores are freed: {ev:?}");
    assert!(
        !ev.iter().any(|e| e.starts_with("D ")),
        "no record is deleted: {ev:?}"
    );
}

/// `@FR-Col-Capacity` through a push window: a loop's `v += [x]` reserves its one element
/// exactly while the vector has no record (`OpPreAllocVector`), as the bytecode does, and the
/// second push takes the growth step — the window does not skip the reservation and claim the
/// growth step at once.
///
/// @falsified-at: none — the bytecode always reserved here, so the receipt is a SABOTAGE
///   (2026-11-21): with the window's `OpPreAllocVector` emitted as `((void) 0)` again
///   (`windowed` in `src/c/body.rs`), the vector's first claim is the growth step.
///   CHANNEL: the trace's claim sizes.  WITNESS: `C 1 12 3` for `C 1 2 3`.
///   HOLDS: `20 57` on either form.
#[test]
fn a_windowed_push_keeps_the_reservation_before_the_growth_step() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let (out, ev) = traced(
        "window",
        r#"
fn main() {
  out: vector<integer> = [];
  i = 0;
  while i < 20 { out += [i * 3]; i = i + 1; }
  println("{len(out)} {out[19]?}");
}
"#,
    );
    assert_eq!(out, "20 57\n");
    assert!(
        at(&ev, "C 1 2 3") < at(&ev, "R 1 3 12 3"),
        "one element reserved, then the growth step: {ev:?}"
    );
}

/// The store a call through a function value makes for a vector answer claims its root as any
/// vector's store does (`State::hidden_return_buffer`, `loft_database_wrapper`): the wrapper's
/// payload and its header word, two words.  With `R-PushFill` off, the 5 elements take the
/// growth step (12 words), so the store's 100 words are 14 % used on both backends.
///
/// @falsified-at: none — a SABOTAGE (2026-11-22): with the byte size passed to
///   `Stores::database` as words again (`hidden_return_buffer`), the interpreter's root claims
///   4 words.  CHANNEL: `store_memory()`'s use of the store.  WITNESS: `used  16%` on
///   `--interpret` against 14 % on `--native`.  HOLDS: `5` either way.
#[test]
fn a_function_values_vector_buffer_claims_its_root_as_any_vector_does() {
    let dir = std::env::temp_dir().join(format!("loft-fnbuf-{}", std::process::id()));
    fa::create_dir_all(&dir).expect("scratch dir");
    let prog = dir.join("fnbuf.loft");
    fa::write(
        &prog,
        r#"
fn grown(seed: integer, n: integer) -> vector<integer> {
  out: vector<integer> = [];
  for i in 0..n { out += [seed + i]; }
  out
}
fn main() {
  f = grown;
  g = f(1, 5);
  println("{len(g)}");
  println(store_memory());
}
"#,
    )
    .expect("write the program");
    let modes: &[&str] = if have_cc() {
        &["--interpret", "--native"]
    } else {
        &["--interpret"]
    };
    for mode in modes {
        let out = harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(mode)
            .arg(&prog)
            .env("LOFT_NO_PUSH_FILL", "1")
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run loft");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.starts_with("5\n"), "{mode}: {text}");
        let line = text
            .lines()
            .find(|l| l.contains("type main_vector<integer>"))
            .unwrap_or_else(|| panic!("{mode}: no vector store in {text}"));
        let used: String = line
            .split("used")
            .nth(1)
            .and_then(|r| r.split('%').next())
            .unwrap_or_default()
            .trim()
            .to_string();
        assert_eq!(used, "14", "{mode}: {line}");
    }
}

/// loft#2610 — a call through a function value answering a vector is carried by every backend:
/// the C unit writes the type table that call makes the answer's buffer through.
#[test]
fn a_function_value_answering_a_vector_is_carried_by_every_backend() {
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/a-function-value-answering-a-vector-is-carried-by-every-backend.loft");
    let modes: &[&str] = if have_cc() {
        &["--interpret", "--native"]
    } else {
        &["--interpret"]
    };
    for mode in modes {
        let out = harness_command(env!("CARGO_BIN_EXE_loft"))
            .arg(mode)
            .arg(&script)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run loft");
        assert!(
            out.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// `scripts/c_alloc_diff.py`'s verdicts on hand-made traces (`--self-test`): what it may accept
/// as order alone or as a `par` region's machinery (STORE_MODEL D45, D46) and what it must
/// reject — work between two regions, a store claimed differently, leaked or freed twice after
/// one, a truncated trace, claims in each other's places.
///
/// @falsified-at: none — the receipt is the region check an adversarial review broke
///   (2026-11-22, the script at 039169797): 7 of the 14 verdicts wrong.
///   CHANNEL: the self-test's exit status.
#[test]
fn the_allocation_comparison_holds_its_verdicts() {
    let out = harness_command("python3")
        .arg("scripts/c_alloc_diff.py")
        .arg("--self-test")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output();
    let Ok(out) = out else {
        eprintln!("skipped: no python3 on PATH");
        return;
    };
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}
