// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN157 § V (loft#1426) — the positive control for the record-buffer reuse gate.
//!
//! A call's hidden `__ref_N` buffer is allocated ONCE so a callee that builds its return
//! into it reuses one record per call SITE instead of minting a store per CALL. The buffer
//! then outlives the call, which is only sound where the RESULT's free is guarded against
//! it — `scopes`'s `witness_buffer`, whose `OpFreeRefIfDistinct(v, __ref_N)` declines
//! exactly when the callee handed the buffer back.
//!
//! The gate that enforces it cannot be scored by the corpus, because both forms answer the
//! same numbers: an unsound reuse frees the buffer's store and writes it again, and the
//! bytes are still readable, so every assertion in
//! `tests/scripts/157-a-struct-return-builds-into-the-callers-buffer.loft` passes either
//! way. `LOFT_STRICT_STORES=1` is the channel that sees it, and
//! `LOFT_NO_RETBUF_WITNESS_GATE=1` is the build that produces it — allocate every buffer,
//! not only the guarded ones.
//!
//! So this is the A/B on one binary, and it asserts BOTH directions: the shipped gate is
//! clean, and the gate removed is not. Without the second half the first proves nothing.

use std::path::PathBuf;
use std::process::Command;

/// The escaping shape the gate refuses: the result of a call in a loop is APPENDED to a
/// vector, so it lands in a `__lift_N` temp whose free is a plain `OpFreeRef`. Reusing a
/// buffer there releases the buffer's store, and the next turn of the loop writes a record
/// that is back in the pool.
const ESCAPING: &str = "\
struct S { a: integer, b: integer }
fn mk(u: integer) -> S { S { a: u, b: u * 2 } }
fn main() {
  keep: vector<S> = [];
  for i in 0..8 { keep += [mk(i)]; }
  println(\"{len(keep)}\");
}
";

/// The second shape the gate refuses: the result local is REASSIGNED inside the loop.  The
/// set lowering frees the store `v` displaces as an owned one, which under reuse is the
/// buffer's — the next call writes a store that is back in the pool.
const DISPLACED: &str = "\
struct S { a: integer, b: integer }
fn mk(u: integer) -> S { S { a: u, b: u * 2 } }
fn main() {
  other = S { a: 7, b: 8 };
  t = 0;
  for i in 0..8 { v = mk(i); v = other; t += v.a; }
  println(\"{t}\");
}
";

/// Both directions run with the § V-d append-in-place lowering OFF: that lowering builds
/// `keep += [mk(i)]`'s element in place — no lift temp, no buffer, nothing to free — so
/// with it on the escaping shape has no use-after-free to report under the control and
/// the gate would read as inert.  What this test pins is the LIFT path, which V-d leaves
/// to every element it does not qualify (an NRVO callee, a nullable return, a record
/// with a collection field).
fn run(src: &std::path::Path, ungated: bool) -> String {
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--interpret")
        .arg(src)
        .env("LOFT_STRICT_STORES", "1")
        .env("LOFT_NO_APPEND_IN_PLACE", "1")
        .env("LOFT_TIMEOUT", "60");
    if ungated {
        cmd.env("LOFT_NO_RETBUF_WITNESS_GATE", "1");
    }
    let out = cmd.output().expect("spawn loft");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn the_buffer_reuse_gate_is_what_keeps_an_escaping_result_alive() {
    both_directions("escaping", ESCAPING);
}

#[test]
fn the_buffer_reuse_gate_is_what_keeps_a_reassigned_result_alive() {
    both_directions("displaced", DISPLACED);
}

fn both_directions(name: &str, probe: &str) {
    let src = std::env::temp_dir().join(format!("loft_retbuf_reuse_{name}.loft"));
    std::fs::write(&src, probe).expect("write probe");

    let shipped = run(&src, false);
    let ungated = run(&src, true);
    let _ = std::fs::remove_file(&src);

    assert!(
        !shipped.contains("strict-store"),
        "the shipped gate must leave the {name} result alone:\n{shipped}"
    );
    assert!(
        ungated.contains("USE AFTER FREE"),
        "the gate is INERT on the {name} shape — reusing every buffer, guarded or not, no \
         longer reports the use-after-free this gate exists to refuse, so its silence on the \
         corpus is not evidence:\n{ungated}"
    );
}
