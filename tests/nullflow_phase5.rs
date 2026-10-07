// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 null-flow Phase 5.3 — the GENERAL null-propagation algorithm.
//!
//! A null-transparent stdlib scalar fn (`abs`/`min`/`max`/`clamp`/`floor`/…) called with a
//! nullable arg types its result `τ?` and is wrapped in a runtime guard (`if any arg null →
//! null`). This replaces the hand-written `min`/`max`/`clamp` `τ?` overloads. `min`/`max`/`clamp`
//! propagated null already (DN3, default-on), so their guard runs in ALL modes; `abs`/etc. are
//! the new `LOFT_NULLFLOW` behaviour. The guard's correctness matters most for integer `max`/`abs`,
//! whose raw body would NOT propagate the sentinel (`max(null,5)` would wrongly give 5).

use loft::file_access as fa;

mod common;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `(success, stdout, warning_count)`.  The count is loft's OWN warnings — see
/// `common::loft_warnings` for why rustc's must not count.
fn run(body: &str, backend: &str, nullflow: bool, tag: &str) -> (bool, String, usize) {
    let name = format!("loft_nf5_{}_{tag}.loft", std::process::id());
    let script = std::env::temp_dir().join(&name);
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
    let stderr = String::from_utf8_lossy(&out.stderr);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        common::loft_warnings(&stderr, &name),
    )
}

// max(null, 5): the raw body gives 5; the guard must give null. Defended `?? -1` so the run
// observes null vs a wrong value, and works in BOTH modes (min/max propagate always).
const MAX_NULL: &str = "fn main(){ y=0; n=10/y; print(\"r={max(n, 5) ?? -1}\\n\"); }\n";
const ABS_NULL: &str =
    "struct S{f:integer} fn main(){ y=0; n=10/y; s=S{f:0}; s.f=abs(n); print(\"r={s.f}\\n\"); }\n";

#[test]
fn max_null_propagates_off_via_guard() {
    let (ok, out, _w) = run(MAX_NULL, "--interpret", false, "max_off");
    assert!(ok, "{out}");
    assert!(
        out.contains("r=-1"),
        "OFF: max(null,5) must be null (guard), not 5: {out}"
    );
}
#[test]
fn max_null_propagates_on_interpret() {
    let (ok, out, _w) = run(MAX_NULL, "--interpret", true, "max_on_i");
    assert!(ok, "{out}");
    assert!(out.contains("r=-1"), "ON: max(null,5) must be null: {out}");
}
#[test]
fn max_null_propagates_on_native() {
    let (ok, out, _w) = run(MAX_NULL, "--native", true, "max_on_n");
    assert!(ok, "{out}");
    assert!(
        out.contains("r=-1"),
        "ON native: max(null,5) must be null: {out}"
    );
}
#[test]
fn abs_nullable_off_launders_no_warning() {
    let (ok, _o, warns) = run(ABS_NULL, "--interpret", false, "abs_off");
    assert!(ok);
    assert_eq!(
        warns, 0,
        "OFF: abs(nullable) is not yet propagated (unchanged)"
    );
}
#[test]
fn abs_nullable_on_propagates_and_guards_interpret() {
    let (ok, out, warns) = run(ABS_NULL, "--interpret", true, "abs_on_i");
    assert!(ok, "{out}");
    // ONE (N-Store) warning: abs(n)'s `integer?` result into the non-null field `s.f`.
    // The nullable ARG `n` into abs is NOT flagged — `abs` is NULL-TRANSPARENT (@PLN102
    // #583 gate-2: abs/min/max/clamp/floor/… propagate null by design, so passing a
    // nullable to them is legitimate). The store of the propagated null is the violation.
    assert_eq!(
        warns, 1,
        "ON: abs(nullable) warns at the STORE only (abs is null-transparent): {out}"
    );
    assert!(
        out.contains("r=null"),
        "ON: abs(null) must be null (guard, not overflow): {out}"
    );
}
#[test]
fn abs_nullable_on_propagates_and_guards_native() {
    let (ok, out, warns) = run(ABS_NULL, "--native", true, "abs_on_n");
    assert!(ok, "{out}");
    assert_eq!(
        warns, 1,
        "ON native: store only (abs is null-transparent): {out}"
    );
    assert!(
        out.contains("r=null"),
        "ON native: abs(null) must be null: {out}"
    );
}
#[test]
fn non_null_args_unaffected() {
    let (ok, out, warns) = run(
        "fn main(){ print(\"r={max(3, 7)}\\n\"); }",
        "--interpret",
        true,
        "nn",
    );
    assert!(ok, "{out}");
    assert_eq!(warns, 0);
    assert!(out.contains("r=7"));
}

// #534 / @PLN102 H4 regression — format-interpolating a `text?`-returning call must
// emit the `&*` deref on native (an owned `Str`/`String` carrying the null sentinel →
// `&str`), or native codegen fails E0308 `expected &str, found Str`. `format_text`
// renders the sentinel as `null`, so the deref is null-safe: the value renders its
// text, or `null` when null, IDENTICALLY on both backends. Surfaced by `content() ->
// text?`; the fix (peel `Optional` in `generation/text.rs::format_text`) is general.
const TEXT_OPT_FMT: &str = "fn maybe(x: boolean) -> text? { if x { return \"hello\" } null }\n\
     fn main(){ print(\"a={maybe(true)}\\nb={maybe(false)}\\n\"); }\n";

#[test]
fn text_opt_format_parity_interpret_and_native() {
    let (oi, i, _) = run(TEXT_OPT_FMT, "--interpret", true, "txtopt_i");
    let (on, n, _) = run(TEXT_OPT_FMT, "--native", true, "txtopt_n");
    assert!(oi, "interp run failed: {i}");
    assert!(
        on,
        "native run failed (text? format must emit the &* deref): {n}"
    );
    assert!(
        i.contains("a=hello") && i.contains("b=null"),
        "text? interpolation must render text then null: {i}"
    );
    assert_eq!(i, n, "text? format must be byte-identical interp vs native");
}
