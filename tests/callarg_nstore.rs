// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 gate-2 residual — the call-arg N-Store hole (`process_call_args` +
//! `callarg_nstore_enabled`).
//!
//! The `(N-Store)` teeth used to sit only on the LOCAL-slot / field / return / index store sites,
//! so a nullable `τ?` passed into a non-null PARAMETER slipped through `convert` (which leniently
//! peels the `Optional`) — `takes(x)` with `x: integer?` silently bound `null` into `n: integer`.
//! The fix runs the same `n_store_violation` at the param-binding chokepoint, with the identical
//! Phase-1 warn/error split: a non-narrow scalar/heap param WARNS (the null binds), a NARROW width
//! hard-errors. Null-transparent fns (`min`/`max`/`clamp`/`abs`/…) are exempt — they PROPAGATE
//! null by design; operators dodge the path via the nullable-op swap.
//!
//! Locks: (1) FIRE — a nullable arg into a non-null user param warns (and `LOFT_NO_CALLARG_NSTORE`
//! restores the old silent accept, proving it is the fix); (2) narrow param hard-errors; (3) OK —
//! a discharged arg, a `τ?` param, and a null-transparent fn stay silent; (4) both backends.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `(compiled_and_ran_ok, callarg_nstore_warning_count, stdout)`. `fix` selects the fix (default
/// ON) vs the opt-out (`LOFT_NO_CALLARG_NSTORE=1`).
fn run(body: &str, backend: &str, fix: bool, tag: &str) -> (bool, usize, String) {
    let script = std::env::temp_dir().join(format!("loft_ca_{}_{tag}.loft", std::process::id()));
    std::fs::write(&script, body).expect("write script");
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(&script)
        .current_dir(workspace_root())
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_CACHE", "1");
    if fix {
        cmd.env_remove("LOFT_NO_CALLARG_NSTORE");
    } else {
        cmd.env("LOFT_NO_CALLARG_NSTORE", "1");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let warns = stderr.matches("is stored into parameter").count();
    (
        out.status.success(),
        warns,
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

const NULLABLE_INTO_NONNULL: &str = "fn takes(n: integer) -> integer { n + 1 }\n\
fn main() {\n  x = \"z\" as integer?;\n  a = takes(x);\n  print(\"a={a}\");\n}\n";
const NARROW_PARAM: &str = "fn takes(n: u8) -> integer { n as integer }\n\
fn main() {\n  x = \"z\" as integer?;\n  r = takes(x);\n  print(\"{r}\");\n}\n";
const DISCHARGED: &str = "fn takes(n: integer) -> integer { n + 1 }\n\
fn main() {\n  x = \"z\" as integer?;\n  a = takes(x ?? 5);\n  print(\"a={a}\");\n}\n";
const NULLABLE_PARAM: &str = "fn takes(n: integer?) -> integer { n ?? 0 }\n\
fn main() {\n  x = \"z\" as integer?;\n  a = takes(x);\n  print(\"a={a}\");\n}\n";
const NULL_TRANSPARENT: &str =
    "fn main() {\n  x = \"z\" as integer?;\n  c = max(5, x);\n  print(\"c={c}\");\n}\n";

// ── FIRE: a nullable arg into a non-null user param warns; the gate restores the silent accept ──

fn assert_fires(backend: &str) {
    let (ok, warns, _out) = run(
        NULLABLE_INTO_NONNULL,
        backend,
        true,
        &format!("fire_{backend}"),
    );
    assert!(
        ok,
        "[{backend}] a non-narrow param warns (Phase-1 split), the call still runs"
    );
    assert_eq!(
        warns, 1,
        "[{backend}] exactly one call-arg N-Store warning (the nullable `x` into `takes`)"
    );
    // Opt-out → no call-arg warning (proves the warning is the fix's, not a pre-existing one).
    let (_ok, warns_off, _) = run(
        NULLABLE_INTO_NONNULL,
        backend,
        false,
        &format!("off_{backend}"),
    );
    assert_eq!(
        warns_off, 0,
        "[{backend}] LOFT_NO_CALLARG_NSTORE must restore the pre-fix silent accept"
    );
}

#[test]
fn fires_interpret() {
    assert_fires("--interpret");
}

// @speed 1.0
#[test]
fn fires_native() {
    assert_fires("--native");
}

// ── loft#1413: a `&τ?` parameter is nullable, so the store it asks for must not be reported ──
//
// `τ?` has three spellings at this gate. Two were handled: `Type::Optional`, and the synthetic
// `__nullable<S>` an inline slot holds (loft#1123). The third is a `&` parameter, which carries
// its nullability INSIDE the reference — `&integer?` is `RefVar(Optional(Integer))` — so asking
// `Type::Optional` of the outer type answered about the REFERENCE, not about the slot the store
// lands in. Every correct call handing a `τ?` to a `&τ?` parameter warned that the value
// "becomes null there" in "the non-null type `&integer?`", a message naming a nullable type
// non-null. `warning` is the tier that gates library CI, so a library taking a `&τ?` parameter
// failed its own CI on correct code.
//
// The pair is what carries the claim: the `&τ?` cases must be SILENT, and the `&τ` case must
// still warn — a peel that silenced both would have removed the rule instead of spelling it.

const REF_NULLABLE_PARAM: &str = "fn bump(n: &integer?) { n = (n ?? 0) + 1; }\n\
fn main() {\n  x: integer? = 5;\n  bump(x);\n  print(\"x={x}\");\n}\n";
const REF_NULLABLE_TEXT: &str = "fn setb(s: &text?) { s = \"b\"; }\n\
fn main() {\n  t: text? = \"a\";\n  setb(t);\n  print(\"t={t}\");\n}\n";
const REF_NONNULL_PARAM: &str = "fn bump(n: &integer) { n = n + 1; }\n\
fn main() {\n  x = \"z\" as integer?;\n  bump(x);\n  print(\"x={x}\");\n}\n";

#[test]
fn a_nullable_argument_into_a_nullable_reference_parameter_is_silent() {
    for backend in ["--interpret", "--native"] {
        for (body, tag, want) in [
            (REF_NULLABLE_PARAM, "ref_opt_int", "x=6"),
            (REF_NULLABLE_TEXT, "ref_opt_text", "t=b"),
        ] {
            let (ok, warns, out) = run(body, backend, true, &format!("{tag}_{backend}"));
            assert!(ok, "[{backend}] {tag}: a `&τ?` parameter must compile");
            assert_eq!(
                warns, 0,
                "[{backend}] {tag}: a nullable argument into a `&τ?` parameter is the store the \
                 signature asks for — it must not be reported (loft#1413)"
            );
            assert!(
                out.contains(want),
                "[{backend}] {tag}: expected {want:?}, got {out:?} — the write must still reach \
                 the caller, so the silence is not bought by dropping the store"
            );
        }
    }
}

#[test]
fn a_nullable_argument_into_a_non_null_reference_parameter_still_warns() {
    for backend in ["--interpret", "--native"] {
        let (_ok, warns, _out) = run(
            REF_NONNULL_PARAM,
            backend,
            true,
            &format!("ref_nonnull_{backend}"),
        );
        assert_eq!(
            warns, 1,
            "[{backend}] a nullable argument into a `&integer` parameter DOES become null there \
             — peeling the `RefVar` must not silence the rule, only ask it of the pointee"
        );
    }
}

// ── Narrow param hard-errors (no room for a null sentinel) ────────────────────────────────────

#[test]
fn narrow_param_hard_errors() {
    for backend in ["--interpret", "--native"] {
        let (ok, _w, _o) = run(NARROW_PARAM, backend, true, &format!("narrow_{backend}"));
        assert!(
            !ok,
            "[{backend}] a nullable into a NARROW (u8) param must be a hard compile error"
        );
    }
}

// ── #585: guard-clause fall-through narrowing suppresses the false positive ───────────────────
//
// `if !s { return } … sink(s)` proves `s` non-null on the fall-through (the null case already
// left the block), exactly like the positive `if s { sink(s) }`. The lint used to narrow only the
// positive THEN branch, so the idiomatic guard clause false-positived. These lock BOTH directions:
// the four guard shapes (exit kind × null-test) go silent, and the conservative boundary (a body
// that can fall through, a `!= null` guard whose fall-through is the NULL case, a guard nested in
// another conditional) must STILL warn.

const G_RETURN_BANG: &str = "fn sink(s: text) -> integer { len(s) }\n\
fn f(s: text?) -> integer { if !s { return 0; } sink(s) }\n\
fn main() { x = f(\"hi\"); print(\"x={x}\"); }\n";
const G_RETURN_EQNULL: &str = "fn sink(s: text) -> integer { len(s) }\n\
fn f(s: text?) -> integer { if s == null { return 0; } sink(s) }\n\
fn main() { x = f(\"hi\"); print(\"x={x}\"); }\n";
const G_BREAK_BANG: &str = "fn sink(s: text) -> integer { len(s) }\n\
fn f(s: text?) -> integer { t = 0; for i in 0..3 { if !s { break; } t = t + sink(s); } t }\n\
fn main() { x = f(\"hi\"); print(\"x={x}\"); }\n";
const G_CONTINUE_BANG: &str = "fn sink(s: text) -> integer { len(s) }\n\
fn f(s: text?) -> integer { t = 0; for i in 0..3 { if !s { continue; } t = t + sink(s); } t }\n\
fn main() { x = f(\"hi\"); print(\"x={x}\"); }\n";

const W_BODY_NOEXIT: &str = "fn sink(s: text) -> integer { len(s) }\n\
fn f(s: text?) -> integer { if !s { print(\"z\"); } sink(s) }\n\
fn main() { x = f(\"hi\"); print(\"x={x}\"); }\n";
const W_NENULL_GUARD: &str = "fn sink(s: text) -> integer { len(s) }\n\
fn f(s: text?) -> integer { if s != null { return 0; } sink(s) }\n\
fn main() { x = f(\"hi\"); print(\"x={x}\"); }\n";
const W_NESTED_COND: &str = "fn sink(s: text) -> integer { len(s) }\n\
fn f(s: text?, b: boolean) -> integer { if b { if !s { return 0; } } sink(s) }\n\
fn main() { x = f(\"hi\", true); print(\"x={x}\"); }\n";

const GUARD_SILENT: &[(&str, &str)] = &[
    (G_RETURN_BANG, "g_return_bang"),
    (G_RETURN_EQNULL, "g_return_eqnull"),
    (G_BREAK_BANG, "g_break_bang"),
    (G_CONTINUE_BANG, "g_continue_bang"),
];
const GUARD_STILL_WARNS: &[(&str, &str)] = &[
    (W_BODY_NOEXIT, "w_body_noexit"),
    (W_NENULL_GUARD, "w_nenull_guard"),
    (W_NESTED_COND, "w_nested_cond"),
];

fn assert_guard_585(backend: &str) {
    for (body, tag) in GUARD_SILENT {
        let (ok, warns, _o) = run(body, backend, true, &format!("{tag}_{backend}"));
        assert!(ok, "[{backend}] `{tag}` must compile and run");
        assert_eq!(
            warns, 0,
            "[{backend}] `{tag}`: a guard-clause fall-through proves the var non-null — no N-Store warning (#585)"
        );
    }
    for (body, tag) in GUARD_STILL_WARNS {
        let (ok, warns, _o) = run(body, backend, true, &format!("{tag}_{backend}"));
        assert!(ok, "[{backend}] `{tag}` must compile and run");
        assert_eq!(
            warns, 1,
            "[{backend}] `{tag}`: the var may still be null here — the N-Store warning must still fire"
        );
    }
}

#[test]
fn guard_clause_narrows_interpret_585() {
    assert_guard_585("--interpret");
}

// @speed 3.1
#[test]
fn guard_clause_narrows_native_585() {
    assert_guard_585("--native");
}

// ── OK: discharged arg, τ? param, null-transparent fn — all silent ────────────────────────────

// @speed 1.6
#[test]
fn legal_forms_stay_silent() {
    for (body, tag) in [
        (DISCHARGED, "discharged"),
        (NULLABLE_PARAM, "nullable_param"),
        (NULL_TRANSPARENT, "null_transparent"),
    ] {
        for backend in ["--interpret", "--native"] {
            let (ok, warns, _o) = run(body, backend, true, &format!("{tag}_{backend}"));
            assert!(ok, "[{backend}] `{tag}` must compile");
            assert_eq!(
                warns, 0,
                "[{backend}] `{tag}` must emit no call-arg N-Store warning"
            );
        }
    }
}
