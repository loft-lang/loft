// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-H-Drop` for `File` — a `File` record's death releases its OS handle, wherever the
//! record lives: a local's store, a field of a record, an element of a collection, a value a
//! generator yields.  Measured as HANDLES: each cell runs its shape 1000 times under
//! `ulimit -n 256`, so a shape that keeps one handle per turn runs out after a few hundred
//! turns and its writes fail (`Too many open files`), while a shape that releases writes every
//! byte.  Each cell is its own program, so a leak in one cannot exhaust another's handles.
//!
//! The handle counts the `File` records that name it (`@FR-H-Lease`) and closes at the last
//! release; the release runs at a store's free, at a store re-initialised in place (a vector
//! buffer cleared at its root, a buffer re-minted by `OpDatabase`) and at a record's death
//! inside a store that lives on (`remove_claims`).  Before, a handle closed only when the
//! freed store's ROOT was the `File` itself, so every `File` in a vector, a record or a
//! generator's value stayed open until the process ended (loft#1896).
//!
//! Expected values by hand: every turn writes one byte `"a"` to each file the cell names, so
//! each file holds 1000 bytes; the copy cell writes two per turn, 2000.  The plain-local cell
//! is the control: it released before the fix and must still release after it.

use loft::file_access as fa;
use std::path::PathBuf;
use std::process::Command;

const TURNS: u32 = 1000;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Run `body` (the cell's functions, `turn(d)` among them) `TURNS` times on `backend` under a
/// 256-handle limit, then print the size of each file in `sizes`; answers stdout and stderr.
fn run_cell(name: &str, backend: &str, body: &str, sizes: &[&str]) -> (String, String) {
    let dir = std::env::temp_dir().join(format!(
        "loft-file-release-{name}-{}{}",
        backend.trim_start_matches('-'),
        std::process::id()
    ));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch dir");
    let d = dir.display().to_string();
    let report: Vec<String> = sizes
        .iter()
        .map(|f| format!("{{file(\"{d}/{f}\").size}}"))
        .collect();
    let program = format!(
        "{body}\nfn main() {{\n  for _ in 0..{TURNS} {{ turn(\"{d}\"); }}\n  println(\"{}\");\n}}\n",
        report.join(" ")
    );
    let file = dir.join(format!("{name}.loft"));
    fa::write(&file, &program).expect("write cell");
    let script = format!(
        "ulimit -n 256; exec {} {backend} {}",
        loft_bin().display(),
        file.display()
    );
    let out = Command::new("bash")
        .arg("-c")
        .arg(&script)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft under a handle limit");
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let mut stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        stderr = format!("[exit status: {}]\n{stderr}", out.status);
    }
    let _ = fa::remove_dir_all(&dir);
    (stdout, stderr)
}

fn assert_cell(name: &str, body: &str, sizes: &[&str], want: &str) {
    for backend in ["--interpret", "--native"] {
        let (stdout, stderr) = run_cell(name, backend, body, sizes);
        assert!(
            stdout == want && !stderr.contains("Too many open files"),
            "[{name} {backend}] want `{want}`, got `{stdout}`\nstderr (first lines):\n{}",
            stderr.lines().take(8).collect::<Vec<_>>().join("\n")
        );
    }
}

// ── The deaths: each must release ────────────────────────────────────────────────────────────

/// A vector a callee fills with a fresh `File`, bound by the caller and replaced every turn —
/// the callee re-mints the caller's buffer, so the old elements die in a store reset.
#[cfg_attr(windows, ignore = "no `ulimit -n` on Windows; the limit is the guard")]
#[test]
fn a_returned_vector_replaced_by_the_next_call() {
    assert_cell(
        "returned",
        "fn mk(p: text) -> vector<File> { [file(p)] }\n\
         fn turn(d: text) { fs = mk(\"{d}/f.txt\"); for g in fs { g += \"a\"; } }",
        &["f.txt"],
        "1000",
    );
}

/// A `File` local moved into the vector a callee returns (`[f]`), written there.
#[cfg_attr(windows, ignore = "no `ulimit -n` on Windows; the limit is the guard")]
#[test]
fn a_local_moved_into_a_returned_vector() {
    assert_cell(
        "moved",
        "fn mk(p: text) -> vector<File> { f = file(p); f += \"a\"; [f] }\n\
         fn turn(d: text) { fs = mk(\"{d}/f.txt\"); n = len(fs); assert(n == 1, \"one\"); }",
        &["f.txt"],
        "1000",
    );
}

/// A vector local that dies with its function — the store's free.
#[cfg_attr(windows, ignore = "no `ulimit -n` on Windows; the limit is the guard")]
#[test]
fn a_vector_local_at_its_scope_end() {
    assert_cell(
        "vector_local",
        "fn turn(d: text) { fs = [file(\"{d}/f.txt\")]; for g in fs { g += \"a\"; } }",
        &["f.txt"],
        "1000",
    );
}

/// A record local holding a `File` field.
#[cfg_attr(windows, ignore = "no `ulimit -n` on Windows; the limit is the guard")]
#[test]
fn a_record_field_at_its_scope_end() {
    assert_cell(
        "field",
        "struct Holder { h: File, tag: integer }\n\
         fn turn(d: text) { r = Holder { h: file(\"{d}/f.txt\"), tag: 1 }; x = r.h; x += \"a\"; }",
        &["f.txt"],
        "1000",
    );
}

/// A record holding a `vector<File>`.
#[cfg_attr(windows, ignore = "no `ulimit -n` on Windows; the limit is the guard")]
#[test]
fn a_vector_field_at_its_scope_end() {
    assert_cell(
        "vector_field",
        "struct Bag { fs: vector<File>, n: integer }\n\
         fn turn(d: text) { b = Bag { fs: [file(\"{d}/f.txt\")], n: 1 }; for g in b.fs { g += \"a\"; } }",
        &["f.txt"],
        "1000",
    );
}

/// A field's vector cleared while its record lives on — each element's death in
/// `remove_claims`.
#[cfg_attr(windows, ignore = "no `ulimit -n` on Windows; the limit is the guard")]
#[test]
fn a_vector_field_cleared() {
    assert_cell(
        "cleared",
        "struct Bag { fs: vector<File>, n: integer }\n\
         fn turn(d: text) {\n  b = Bag { fs: [], n: 0 };\n  b.fs += [file(\"{d}/f.txt\")];\n  \
         for g in b.fs { g += \"a\"; }\n  b.fs.clear();\n}",
        &["f.txt"],
        "1000",
    );
}

/// A `File` a generator yields, written by the consumer.
#[cfg_attr(windows, ignore = "no `ulimit -n` on Windows; the limit is the guard")]
#[test]
fn a_yielded_file() {
    assert_cell(
        "yielded",
        "fn gen(p: text) -> iterator<File> { yield file(p); }\n\
         fn turn(d: text) { for g in gen(\"{d}/f.txt\") { g += \"a\"; } }",
        &["f.txt"],
        "1000",
    );
}

/// A rebind displaces the `File` the local held.
#[cfg_attr(windows, ignore = "no `ulimit -n` on Windows; the limit is the guard")]
#[test]
fn a_rebind_displaces_its_file() {
    assert_cell(
        "rebind",
        "fn turn(d: text) { x = file(\"{d}/a.txt\"); x += \"a\"; x = file(\"{d}/b.txt\"); x += \"a\"; }",
        &["a.txt", "b.txt"],
        "1000 1000",
    );
}

/// A copy and its source, both dying with the function: two leases, both released.
#[cfg_attr(windows, ignore = "no `ulimit -n` on Windows; the limit is the guard")]
#[test]
fn a_copy_and_its_source_at_their_scope_end() {
    assert_cell(
        "copy",
        "fn turn(d: text) { f = file(\"{d}/f.txt\"); f += \"a\"; g = f; g += \"a\"; }",
        &["f.txt"],
        "2000",
    );
}

// ── The control: released before the fix, and still ─────────────────────────────────────────

/// A plain `File` local — the one shape whose handle always closed.
#[cfg_attr(windows, ignore = "no `ulimit -n` on Windows; the limit is the guard")]
#[test]
fn a_plain_local_at_its_scope_end() {
    assert_cell(
        "plain",
        "fn turn(d: text) { f = file(\"{d}/f.txt\"); f += \"a\"; }",
        &["f.txt"],
        "1000",
    );
}
