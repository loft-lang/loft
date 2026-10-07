// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 D1 — computed-index and custom-iterator de-null.
//!
//! Two source-level fixes that keep numeric code free of `?? d` friction under the null-flow model:
//!
//! * `index_provably_fit` now trusts an integer-arithmetic index built purely from constants and
//!   active loop vars (`m[k*4+row]` — the matrix-indexing contract), so the element types non-null
//!   and N-Prop does not spuriously nullify `sum += m[k*4+row] * …`.
//! * `for_type` strips the `?` from a custom iterator's `next(self) -> Item?`: null is the loop
//!   TERMINATOR (never delivered to the body), so the loop variable is the non-null `Item`.
//!
//! The negative control proves the index trust does NOT over-reach: an arithmetic index touching a
//! plain (non-loop, non-constant) variable stays `τ?`.

use loft::file_access as fa;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Returns `(success, stdout, stderr)`. `tag` keeps the temp script unique across parallel tests.
fn run(body: &str, backend: &str, nullflow: bool, tag: &str) -> (bool, String, String) {
    let script = std::env::temp_dir().join(format!("loft_nfidx_{}_{tag}.loft", std::process::id()));
    fa::write(&script, body).expect("write script");
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg(backend)
        .arg(&script)
        .current_dir(workspace_root())
        .env("LOFT_TIMEOUT", "120");
    // @PLN102 flip — the null-flow model is default-ON; the OFF case opts out with LOFT_NO_NULLFLOW.
    if nullflow {
        cmd.env_remove("LOFT_NO_NULLFLOW");
    } else {
        cmd.env("LOFT_NO_NULLFLOW", "1");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = fa::remove_file(&script);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

// `v[a*2+b]` with `a`,`b` loop vars over 0..2: indices 0,1,2,3 → sum 10+20+30+40 = 100.
const MATRIX_INDEX: &str = "fn main(){ v=[10,20,30,40]; sum=0;\n\
     for a in 0..2 { for b in 0..2 { sum = sum + v[a * 2 + b]; } }\n\
     print(\"r={sum}\\n\"); }\n";

// The negative control: the index arithmetic touches `w[0]`, a plain (untrusted) value, so the
// element stays `τ?` and N-Prop refuses the non-null accumulator under NULLFLOW.
// `sum` is DECLARED non-null so the store is the channel: an untrusted index reads `integer?`,
// the sum is `integer?` (N-Prop), and storing it into `sum: integer` is (N-Store)'s warning.
// An INFERRED `sum` would widen to `integer?` in silence (N-Join) and could not tell the
// over-reach from the rule.
const UNTRUSTED_INDEX: &str = "fn main(){ v=[10,20,30,40]; w=[1,1]; sum: integer = 0;\n\
     for a in 0..2 { sum = sum + v[a * 2 + w[0]]; }\n\
     print(\"r={sum}\\n\"); }\n";

// Custom iterator (I13 protocol): `next(self) -> integer?` returns null to STOP. The body only ever
// binds a present value → `total + x` must not nullify. Counter(3) yields 0,1,2 → total 3.
const CUSTOM_ITER: &str = "struct Counter { current: integer, limit: integer }\n\
     fn new_counter(limit: integer) -> Counter { Counter { current: 0, limit: limit } }\n\
     operator next(self: Counter) -> integer? { val = self.current; self.current = val + 1;\n\
         if val >= self.limit { return null; } val }\n\
     fn main(){ c = new_counter(3); total = 0; for x in c { total = total + x; }\n\
         print(\"r={total}\\n\"); }\n";

#[test]
fn matrix_index_non_null_interpret() {
    let (ok, out, err) = run(MATRIX_INDEX, "--interpret", true, "mi_i");
    assert!(
        ok,
        "ON: `v[a*2+b]` should stay non-null and compile; stderr: {err}"
    );
    assert!(out.contains("r=100"), "value: {out}");
}

#[test]
fn matrix_index_non_null_native() {
    let (ok, out, err) = run(MATRIX_INDEX, "--native", true, "mi_n");
    assert!(ok, "ON native: `v[a*2+b]` should compile; stderr: {err}");
    assert!(out.contains("r=100"), "native value: {out}");
}

#[test]
fn matrix_index_unchanged_off() {
    let (ok, out, _e) = run(MATRIX_INDEX, "--interpret", false, "mi_off");
    assert!(ok, "OFF must still run: {out}");
    assert!(out.contains("r=100"), "OFF value: {out}");
}

#[test]
fn untrusted_arith_index_stays_nullable() {
    // The trust must NOT reach a plain var: under NULLFLOW the untrusted index is `τ?`, so the
    // sum is `τ?` (N-Prop) and its store into the DECLARED non-null accumulator is reported —
    // (N-Store)'s WARNING at full width, the program running (@PLN153 phase 3b: it used to be
    // a hard error).  This is the over-reach guard: a trusted index would read non-null and
    // the store would be silent.
    let (ok, out, err) = run(UNTRUSTED_INDEX, "--interpret", true, "ui_i");
    assert!(
        ok,
        "ON: the reported store proceeds and the program runs; stderr: {err}"
    );
    assert!(out.contains("r=60"), "value: {out}");
    assert!(
        err.contains("a nullable `integer?` is stored into the local `sum`"),
        "expected the (N-Store) report on the non-null accumulator: {err}"
    );
}

#[test]
fn custom_iter_element_non_null_interpret() {
    let (ok, out, err) = run(CUSTOM_ITER, "--interpret", true, "ci_i");
    assert!(
        ok,
        "ON: the iterator element is non-null in the body; stderr: {err}"
    );
    assert!(out.contains("r=3"), "value: {out}");
}

#[test]
fn custom_iter_element_non_null_native() {
    let (ok, out, err) = run(CUSTOM_ITER, "--native", true, "ci_n");
    assert!(ok, "ON native: iterator element non-null; stderr: {err}");
    assert!(out.contains("r=3"), "native value: {out}");
}

#[test]
fn custom_iter_unchanged_off() {
    let (ok, out, _e) = run(CUSTOM_ITER, "--interpret", false, "ci_off");
    assert!(ok, "OFF must still run: {out}");
    assert!(out.contains("r=3"), "OFF value: {out}");
}
