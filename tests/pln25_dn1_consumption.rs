// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN25 DN1 read-consumption regression (both backends).
//!
//! Exercises the `text?` READ-consumption paths surfaced by the web `try_recv`
//! consumer sweep — method call, single char-index, open-range slice, and format
//! interpolation on a nullable text after a null-check. Under `LOFT_PLN25_DN1=1`
//! the value is `Optional(Text)` and each read must peel to its base (the parser
//! index/format peels + the native `&str` borrow peel). The output must match the
//! gate-OFF run of the same script byte-for-byte, on BOTH the interpreter and the
//! `--native` backend.
//!
//! The gate-OFF path is covered by the normal script sweep (the file self-asserts
//! `total == 246`); this binary drives the gate-ON path a subprocess at a time so
//! the `LOFT_PLN25_DN1` `OnceLock` starts fresh for each run.

use loft::file_access as fa;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run_dn1(backend: &str) -> String {
    let script = workspace_root().join("tests/scripts/25-nullable-read-consumption.loft");
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(&script)
        .current_dir(workspace_root())
        .env("LOFT_PLN25_DN1", "1")
        // rustc can hang on the native path; bound it (0 = off is the default).
        .env("LOFT_TIMEOUT", "180")
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "DN1 {backend} run failed (exit {:?}); stdout={stdout:?}; stderr={stderr:?}",
        out.status.code()
    );
    stdout
}

/// The expected output — identical across gate-OFF/DN1 × interpret/native.
const EXPECTED: &str =
    "frame MAP:hello\nframe MAP:hello\nframe MAP:hello\nnull fmt: null\ntotal=246\n";

#[test]
fn dn1_text_read_consumption_interpret() {
    let stdout = run_dn1("--interpret");
    assert_eq!(
        stdout, EXPECTED,
        "DN1 interpret output must match the gate-OFF byte-identical baseline"
    );
}

#[test]
fn dn1_text_read_consumption_native() {
    let stdout = run_dn1("--native");
    assert_eq!(
        stdout, EXPECTED,
        "DN1 native output must match the gate-OFF byte-identical baseline"
    );
}

/// Run a self-asserting script under `LOFT_PLN25_DN1=1` and require a clean exit
/// (exit 0 = every internal `assert` passed). Used for the scripts whose value is
/// their own asserts rather than a stdout signature.
fn dn1_script_exits_zero(backend: &str, rel_path: &str) {
    let script = workspace_root().join(rel_path);
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(&script)
        .current_dir(workspace_root())
        .env("LOFT_PLN25_DN1", "1")
        .env("LOFT_TIMEOUT", "180")
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "DN1 {backend} {rel_path} failed (exit {:?}); stdout={stdout:?}; stderr={stderr:?}",
        out.status.code()
    );
}

/// @PLN25 scalar-nullable-vector-element slice: `vector<τ?>` for a scalar τ must
/// store/read/iterate the typed-sentinel null under DN1, on both backends. These
/// scripts self-assert; the flagship + three-state-boolean were previously blocked
/// on this slice and now go green.
const DN1_SVEC_SCRIPTS: &[&str] = &[
    "tests/scripts/25-nullable-vector-scalar-element.loft",
    "tests/scripts/25-nullable-sequences.loft",
    "tests/scripts/292-pln17-three-state-boolean.loft",
    // @PLN25 `character?` native null-sentinel wrap (Call / Var / Block / TupleGet).
    "tests/scripts/25-nullable-character.loft",
    // @PLN25 full-range nullable narrow-int FIELD in a vector<struct> (native
    // struct-size / element-stride) — the previously native-blocked 389-h6 + 407.
    "tests/scripts/25-nullable-narrow-field-vector.loft",
    "tests/scripts/389-h6-nullable-full-range-narrow.loft",
    "tests/scripts/407-cluster-d-null-sentinel-roundtrip.loft",
];

#[test]
fn dn1_scalar_vector_element_interpret() {
    for s in DN1_SVEC_SCRIPTS {
        dn1_script_exits_zero("--interpret", s);
    }
}

#[test]
fn dn1_scalar_vector_element_native() {
    for s in DN1_SVEC_SCRIPTS {
        dn1_script_exits_zero("--native", s);
    }
}

/// Run a self-asserting script under `LOFT_INDEX_DEV=1` (the index-flip dev gate;
/// DN1 is default-on, so the gate constructs `Optional` element types for a
/// not-provably-fit `v[i]`) and require a clean exit. Used for the index-flip
/// F1a slices whose value is their own asserts.
fn index_dev_script_exits_zero(backend: &str, rel_path: &str) {
    let script = workspace_root().join(rel_path);
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(&script)
        .current_dir(workspace_root())
        .env("LOFT_INDEX_DEV", "1")
        .env("LOFT_TIMEOUT", "180")
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "index-flip {backend} {rel_path} failed (exit {:?}); stdout={stdout:?}; stderr={stderr:?}",
        out.status.code()
    );
}

/// @PLN25 index-flip F1a Step 1 — copy-elision must peel `Optional` when reading a
/// nullable element borrower's deps, so a mutated/escaping `e = v[i]` (typing
/// `Item?`) KEEPS the copy instead of silently leaking the mutation to the source.
/// Guards the silent copy-elision wrong-answer on both backends.
#[test]
fn index_dev_elision_borrower_interpret() {
    index_dev_script_exits_zero(
        "--interpret",
        "tests/scripts/25-index-elision-borrower.loft",
    );
}

#[test]
fn index_dev_elision_borrower_native() {
    index_dev_script_exits_zero("--native", "tests/scripts/25-index-elision-borrower.loft");
}

/// @PLN25 DN6-adjacent — a bare `null` into a declared non-null local names the real fix
/// (`τ?`) and NEVER suggests `as`, which would launder the null into the non-null slot (the
/// DN5 hole).  `(N-Store)`'s split holds at the local as everywhere else (D-types-7): a
/// full-width `integer` WARNS and the program runs, a narrow `u8` is REFUSED.
#[test]
fn dn1_null_local_message_names_optional_not_as() {
    let dir = std::env::temp_dir();
    for (name, decl, fix, refused) in [
        (
            "pln25_null_local_msg.loft",
            "a: integer = null;",
            "declare it `integer?`",
            false,
        ),
        (
            "pln25_null_local_msg_u8.loft",
            "a: u8 = null;",
            "declare it `u8?`",
            true,
        ),
    ] {
        let src = dir.join(name);
        fa::write(&src, format!("fn t() {{ {decl} }}\nfn main() {{ }}\n"))
            .expect("write temp source");
        let out = loft::platform::process::harness_command(loft_bin())
            .arg("--interpret")
            .arg(&src)
            .current_dir(workspace_root())
            .env("LOFT_PLN25_DN1", "1")
            .output()
            .expect("failed to invoke loft binary");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            !out.status.success(),
            refused,
            "`{decl}`: refused must be {refused} (N-Store's width split); stderr={stderr:?}"
        );
        assert!(
            stderr.contains(fix),
            "`{decl}`: the message must name the `?` fix; got: {stderr:?}"
        );
        // Must not be the generic type-mismatch message, and — the actual DN5 hazard — must
        // never SUGGEST an `as` cast ("… or cast with 'as'").
        assert!(
            !stderr.contains("use a new variable name"),
            "`{decl}`: must be the nullability-specific message; got: {stderr:?}"
        );
        assert!(
            !stderr.contains("or cast with"),
            "`{decl}`: must NOT suggest an `as` cast (the DN5 laundering hole); got: {stderr:?}"
        );
        let _ = fa::remove_file(&src);
    }
}
