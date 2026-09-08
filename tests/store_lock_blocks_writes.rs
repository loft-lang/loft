// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A locked store refuses every write, and an unlocked one accepts them.
//!
//! `#lock = true` is offered as a debugging tripwire: the Store Locks chapter says any
//! write to a locked store panics immediately, wherever it happens. Nothing in the suite
//! held that up — the chapter and `tests/scripts/59-locks.loft` between them set the flag,
//! read it back and read THROUGH it, and neither ever wrote. A guard that only sets a flag
//! passes on a build where the flag does nothing.
//!
//! It cannot be a `.loft` cell: the refusal HALTS the program, so the only harness that can
//! score it is one that runs the program and reads its exit. `@EXPECT_FAIL` is the nearest
//! annotation and TOLERATES a halt rather than requiring one, so it passes just as happily
//! on a build where the lock is inert.
//!
//! loft#1405 changed what the halt SAYS, not that it halts. The author's own `#lock` now
//! reaches them as a rendered loft fault instead of `Store::addr_mut`'s `assert!` — the
//! three refusal cells below are exactly the three user-reachable shapes, so they are this
//! file's evidence for that change and were re-pointed at the new text with it. The
//! properties the new fault owns and this file does not check — that it exits 1 rather
//! than a panic's 101, that it names no `src/store.rs`, and that both backends render it
//! identically — are guarded in `panic_halts_both_backends.rs` beside `panic` and `assert`.
//!
//! Every case is checked on BOTH backends. [`an_unlocked_store_takes_the_same_write`] is
//! the control: without it, a lock that blocked writes by breaking writes in general would
//! pass every other case here.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn write_temp(tag: &str, src: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("loft_store_lock_{tag}_{}.loft", std::process::id()));
    std::fs::write(&path, src).expect("write probe");
    path
}

/// Run `src` on `backend`; return `(exited_ok, stdout + stderr)`.
fn run(backend: &str, tag: &str, src: &str) -> (bool, String) {
    let path = write_temp(&format!("{tag}_{backend}"), src);
    let out = Command::new(loft_bin())
        .arg(backend)
        .arg(&path)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

const HEAD: &str = "struct LKC { value: integer }\n\
                    fn lk_bump(self: LKC) { self.value += 1 }\n";

/// The write must stop the program, and it must stop it for the stated reason: a run that
/// aborted on anything else would satisfy a bare exit check.
fn assert_write_is_refused(tag: &str, body: &str) {
    for backend in ["--interpret", "--native"] {
        let (ok, log) = run(backend, tag, &format!("{HEAD}fn main() {{\n{body}}}\n"));
        assert!(
            !ok,
            "[{backend}/{tag}] a write to a locked store must stop the program\n{log}"
        );
        assert!(
            log.contains("locked store"),
            "[{backend}/{tag}] it must stop ON THE WRITE, naming the locked store\n{log}"
        );
        assert!(
            !log.contains("src/store.rs"),
            "[{backend}/{tag}] the author is being shown loft's internals (loft#1405)\n{log}"
        );
    }
}

fn assert_runs_green(tag: &str, body: &str, marker: &str) {
    for backend in ["--interpret", "--native"] {
        let (ok, log) = run(backend, tag, &format!("{HEAD}fn main() {{\n{body}}}\n"));
        assert!(
            ok && log.contains(marker),
            "[{backend}/{tag}] expected a green run printing {marker:?}\n{log}"
        );
    }
}

#[test]
fn a_locked_store_refuses_a_direct_field_write() {
    assert_write_is_refused(
        "direct",
        "  d = LKC { value: 5 };\n  d#lock = true;\n  d.value = 77;\n  print(\"{d.value}\");\n",
    );
}

/// The lock lives on the STORE, so it does not stop at the function that set it — this is
/// the half that makes it useful for finding a mutation you cannot locate by reading.
#[test]
fn a_locked_store_refuses_a_write_from_a_called_function() {
    assert_write_is_refused(
        "callee",
        "  d = LKC { value: 5 };\n  d#lock = true;\n  lk_bump(d);\n  print(\"{d.value}\");\n",
    );
}

#[test]
fn a_locked_vector_refuses_an_element_write() {
    for backend in ["--interpret", "--native"] {
        let (ok, log) = run(
            backend,
            "vector",
            "fn main() {\n  v = [1, 2, 3];\n  v#lock = true;\n  v[0] = 9;\n  print(\"{v[0]}\");\n}\n",
        );
        assert!(
            !ok && log.contains("locked store"),
            "[{backend}] a locked vector must refuse an element write\n{log}"
        );
    }
}

/// Reads are the half a lock must NOT block, or it would be unusable as a tripwire.
#[test]
fn a_locked_store_still_reads() {
    assert_runs_green(
        "read",
        "  d = LKC { value: 5 };\n  d#lock = true;\n  print(\"read {d.value}\");\n",
        "read 5",
    );
}

/// The control. Same program, same write, no lock — so a build that refused this write for
/// any other reason cannot pass the cases above by accident.
#[test]
fn an_unlocked_store_takes_the_same_write() {
    assert_runs_green(
        "control",
        "  d = LKC { value: 5 };\n  lk_bump(d);\n  d.value = 77;\n  print(\"wrote {d.value}\");\n",
        "wrote 77",
    );
}

/// Setting the flag back to false lifts the guard — the chapter now says so, and a lock you
/// cannot lift is a different feature from the one it describes.
#[test]
fn unlocking_lets_the_write_through_again() {
    assert_runs_green(
        "unlock",
        "  d = LKC { value: 5 };\n  d#lock = true;\n  d#lock = false;\n  lk_bump(d);\n  print(\"wrote {d.value}\");\n",
        "wrote 6",
    );
}
