// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 gate-2 residual — the `?? null` typing soundness fix (`operators.rs`,
//! `build_null_coalesce_default` + `qq_null_typing_enabled`).
//!
//! `a ?? b` yields `a` when non-null else `b`, so it can STILL be null exactly when the FALLBACK
//! `b` is nullable (a bare `null` literal, or a `τ?`-typed expression). Before this fix the result
//! peeled unconditionally to the non-null base, so `y: integer = x ?? null` was accepted and a
//! non-null slot held the null sentinel — the "null in a non-null slot" incoherence the null-model
//! gate exists to remove. The fix re-marks the result `τ?` when the fallback can be null; the
//! N-Store check at the consuming slot then rejects.
//!
//! Locks: (1) FIRE — `?? null` / `?? <nullableVar>` into a non-null slot now REJECTS (and the gate
//! `LOFT_NO_QQ_NULL=1` restores the old accept, proving it's the fix and not a pre-existing reject);
//! (2) OK — a NON-null fallback still discharges to the non-null base (the common `?? default`), a
//! `τ?` slot accepts, and an inferred bind types `τ?`; (3) both backends agree.

use loft::file_access as fa;
use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Compile+run `body`; return `(compiled_and_ran_ok, stdout)`. `qq_fix` selects the fix (default
/// ON) vs the opt-out (`LOFT_NO_QQ_NULL=1`, the pre-fix behaviour).
fn run(body: &str, backend: &str, qq_fix: bool, tag: &str) -> (bool, String) {
    let script = std::env::temp_dir().join(format!("loft_qq_{}_{tag}.loft", std::process::id()));
    fa::write(&script, body).expect("write script");
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(&script)
        .current_dir(workspace_root())
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1");
    if qq_fix {
        cmd.env_remove("LOFT_NO_QQ_NULL");
    } else {
        cmd.env("LOFT_NO_QQ_NULL", "1");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = fa::remove_file(&script);
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

// The unsound stores — a nullable fallback flowing into a non-null slot.
const NULL_LIT_INTO_NONNULL: &str =
    "fn main() {\n  x = \"z\" as integer?;\n  y: integer = x ?? null;\n  print(\"{y}\");\n}\n";
const NULLABLE_VAR_INTO_NONNULL: &str = "fn maybe() -> integer? { \"z\" as integer? }\n\
fn main() {\n  x = \"z\" as integer?;\n  y: integer = x ?? maybe();\n  print(\"{y}\");\n}\n";

// The forms that must stay legal.
const NONNULL_FALLBACK: &str =
    "fn main() {\n  x = \"z\" as integer?;\n  y: integer = x ?? 0;\n  print(\"y={y}\");\n}\n";
const NULLABLE_SLOT: &str =
    "fn main() {\n  x = \"z\" as integer?;\n  y: integer? = x ?? null;\n  print(\"y={y}\");\n}\n";
const CHAIN_TO_NONNULL: &str = "fn main() {\n  x = \"z\" as integer?;\n  a = 10; b = 0;\n\
  y: integer = x ?? (a / b) ?? 7;\n  print(\"y={y}\");\n}\n";

// ── FIRE: the unsound store is now reported (both backends), gate restores the old silence ──────
//
// The coalesce with a nullable fallback types `integer?`, and that `integer?` into the DECLARED
// `y: integer` is `(N-Store)`'s WARNING at full width — the store proceeds and `y` holds the null
// (@PLN153 phase 3b: it used to be a hard error).  The channel is the diagnostic naming the slot,
// not the exit code.

const NSTORE_REPORT: &str = "a nullable `integer?` is stored into the local `y`";

fn assert_reported(body: &str, tag: &str) {
    for backend in ["--interpret", "--native"] {
        let (ok_fix, out_fix) = run(body, backend, true, &format!("{tag}_fix_{backend}"));
        assert!(
            ok_fix && out_fix.contains(NSTORE_REPORT),
            "[{backend}] `{tag}`: a nullable `??` fallback into a non-null slot must be REPORTED \
             and the program must run; got ok={ok_fix}, output:\n{out_fix}"
        );
        // The gate opts out → the coalesce types non-null again and nothing is reported, proving
        // this is the fix's report, not a pre-existing one from an unrelated rule.
        let (ok_off, out_off) = run(body, backend, false, &format!("{tag}_off_{backend}"));
        assert!(
            ok_off && !out_off.contains(NSTORE_REPORT),
            "[{backend}] `{tag}`: LOFT_NO_QQ_NULL must restore the pre-fix silence; output:\n{out_off}"
        );
    }
}

#[test]
fn null_literal_into_nonnull_is_reported() {
    assert_reported(NULL_LIT_INTO_NONNULL, "null_lit");
}

#[test]
fn nullable_var_into_nonnull_is_reported() {
    assert_reported(NULLABLE_VAR_INTO_NONNULL, "nullable_var");
}

// ── OK: non-null fallback discharges; nullable slot / inferred bind accept ────────────────────────

#[test]
fn nonnull_fallback_still_discharges() {
    for backend in ["--interpret", "--native"] {
        let (ok, out) = run(NONNULL_FALLBACK, backend, true, &format!("nn_{backend}"));
        assert!(
            ok,
            "[{backend}] a non-null `??` fallback must stay non-null"
        );
        assert!(out.contains("y=0"), "[{backend}] expected y=0, got {out:?}");
    }
}

#[test]
fn nullable_slot_accepts() {
    for backend in ["--interpret", "--native"] {
        let (ok, out) = run(NULLABLE_SLOT, backend, true, &format!("slot_{backend}"));
        assert!(ok, "[{backend}] `?? null` into a `τ?` slot must compile");
        assert!(
            out.contains("y=null"),
            "[{backend}] expected y=null, got {out:?}"
        );
    }
}

#[test]
fn chain_discharges_on_last_fallback() {
    // `x ?? (a/b) ?? 7`: the intermediate fallback is nullable but the LAST is not, so the whole
    // chain discharges to non-null — the fix must not over-reject a chain that ends non-null.
    for backend in ["--interpret", "--native"] {
        let (ok, out) = run(CHAIN_TO_NONNULL, backend, true, &format!("chain_{backend}"));
        assert!(
            ok,
            "[{backend}] a `??` chain ending non-null must discharge"
        );
        assert!(out.contains("y=7"), "[{backend}] expected y=7, got {out:?}");
    }
}
