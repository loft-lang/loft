// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN157 § V-o (loft#1426) — a call to a stdlib ONE-OP wrapper (`len(v)` is
//! `OpLengthVector(v)`, `sqrt(x)` is `OpMathFuncFloat(9, x)`) is emitted as the op itself on
//! the native backend, so the registry's header-aware emitters can serve it.
//!
//! The cell corpus (`bytecode-comparisons/V-o-wrapper-op-cells.loft`) can only say the VALUES
//! hold, because the wrapper's body IS the op.  This pins the EMISSION: no wrapper CALL is
//! left (a definition may stay), `exp` — whose body converts a constant — stays a call, a
//! `len` inside a hoisted loop and after a view binding reads the header's length, and the
//! switch (`LOFT_NO_WRAPPER_INLINE=1`) restores the calls.  Read off `--native-emit`.
use std::path::{Path, PathBuf};
use std::process::Command;

const CELLS: &str =
    "doc/claude/plans/157-native-4x-drawing/bytecode-comparisons/V-o-wrapper-op-cells.loft";

fn emit(out: &Path, env: &[(&str, &str)]) -> String {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(CELLS);
    let mut cmd = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")));
    cmd.arg("--native-emit")
        .arg(out)
        .arg(&src)
        .env("LOFT_TIMEOUT", "120");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.output().expect("spawn loft --native-emit");
    assert!(
        out.exists(),
        "no Rust emitted (exit {:?}): {}",
        status.status,
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read_to_string(out).expect("read the emitted Rust")
}

/// A CALL to `name` — its definition spells `(cell: &`, a call `(cell,`.
fn calls(rust: &str, name: &str) -> usize {
    rust.matches(&format!("{name}(cell,")).count()
}

/// The body of one emitted function.
fn body<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("{name} was not emitted"));
    let rest = &rust[start..];
    let end = rest[1..].find("\nfn ").map_or(rest.len(), |i| i + 1);
    &rest[..end]
}

#[test]
fn a_wrapper_call_is_its_op_and_the_header_serves_its_length() {
    let out = std::env::temp_dir().join("loft_wrapper_op_on.rs");
    let rust = emit(&out, &[]);
    // A call whose arguments are all LEAVES (a variable, a literal) is its op …
    for (name, leaf_call) in [
        ("t_6vector_len", "t_6vector_len(cell, var_"),
        ("t_5float_sin", "t_5float_sin(cell, "),
        ("t_5float_sqrt", "t_5float_sqrt(cell, var_"),
        ("t_5float_atan2", "t_5float_atan2(cell, "),
    ] {
        assert_eq!(
            rust.matches(leaf_call).count(),
            0,
            "{name} with leaf arguments must be emitted as its op, not called"
        );
    }
    // … and one whose argument is an EXPRESSION stays a call, because the op's operands are
    // emitted from a fresh list the pre-evaluation map cannot see: `len(cv.data)` (a field
    // path) in c2's print, `sqrt(-1.0)` (a negation) in c6.
    assert_eq!(
        calls(&rust, "t_6vector_len"),
        1,
        "`len(cv.data)` keeps its call"
    );
    assert_eq!(
        calls(&rust, "t_5float_sqrt"),
        1,
        "`sqrt(-1.0)` keeps its call"
    );
    assert_eq!(
        calls(&rust, "t_5float_exp"),
        1,
        "`exp` converts a constant in its body and is not a one-op wrapper: it stays a call"
    );
    // A USER one-op function is not a wrapper: its call is observable (the live tier's
    // flip, the shadow call stack), so `dbl(21)` in c11 stays a call.
    assert_eq!(
        calls(&rust, "n_dbl"),
        1,
        "a user function whose body is one op keeps its call"
    );
    for name in ["n_h1", "n_setp"] {
        let b = body(&rust, name);
        assert!(
            b.contains("__vh_1.len")
                && !b.contains("length_vector")
                && !b.contains("t_6vector_len"),
            "{name}: `len` must read the hoisted header's length:\n{b}"
        );
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_switch_restores_the_calls() {
    let out = std::env::temp_dir().join("loft_wrapper_op_off.rs");
    let rust = emit(&out, &[("LOFT_NO_WRAPPER_INLINE", "1")]);
    assert!(
        rust.matches("t_6vector_len(cell, var_").count() >= 2
            && rust.matches("t_5float_sin(cell, ").count() >= 1,
        "LOFT_NO_WRAPPER_INLINE=1 must call the wrappers again"
    );
    let _ = std::fs::remove_file(&out);
}
