// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 case B (soften-nullflow-discharge.md) — the sign/lower-bound lattice that lets a
//! domain-fault op with a PROVABLY in-domain argument type non-null, so no `??` discharge is
//! forced.  Opt-in behind `LOFT_MATH_DOMAIN` (default off) until the B5 flip, so this drives
//! the binary as a subprocess with/without the flag.  The soundness half — unprovable args
//! MUST stay `float?` even under the flag — is the load-bearing assertion (a wrong non-null
//! proof would store a runtime null into a non-null slot).

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Compile `src` on the interpreter with the domain lattice on (`domain_on`, the DEFAULT) or
/// opted out via `LOFT_NO_MATH_DOMAIN`. Returns `(compiled_ok, stdout+stderr)`.
fn compile(tag: &str, src: &str, domain_on: bool) -> (bool, String) {
    let path = std::env::temp_dir().join(format!("loft_mathdom_{}_{tag}.loft", std::process::id()));
    std::fs::write(&path, src).expect("write temp");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg(&path)
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    if domain_on {
        cmd.env_remove("LOFT_NO_MATH_DOMAIN");
    } else {
        cmd.env("LOFT_NO_MATH_DOMAIN", "1");
    }
    let out = cmd.output().expect("invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), combined)
}

// The channel every cell below reads.  A `float?` argument stored into a DECLARED `x: float`
// is `(N-Store)`'s WARNING at full width — the program compiles and runs, and the report names
// the local (@PLN153 phase 3b; it used to be a hard error, and `ok` alone measured it).  So a
// "stays `float?`" cell asserts the report on its local, and a "softens" cell asserts its
// ABSENCE — without the second half every softening cell reads green vacuously now.
const REPORT: &str = " is stored into the local `";

fn reported(diag: &str, v: &str) -> bool {
    diag.contains(&format!("a nullable `float?`{REPORT}{v}`"))
}

// Provably in-domain EXPRESSION arguments: square, sum-of-squares, max-with-positive, abs,
// pow-of-nonneg-base, ln-of-positive.  Forced (float?) by default, softened under the flag.
const PROVABLE: &str = "\
fn f(x: float, y: float) {
  p1: float = sqrt(x * x + y * y);
  p2: float = sqrt(max(x, 0.01));
  p3: float = sqrt(abs(x));
  p4: float = pow(abs(x), 2.4);
  p5: float = ln(max(x, 0.01));
}
fn main() { }
";

// Unprovable arguments — each MUST stay float? even under the flag (soundness controls):
// unknown sign, distinct-operand product, subtraction, and ln of a merely-NonNeg value.
const UNPROVABLE: &str = "\
fn f(x: float, y: float) {
  n1: float = sqrt(x);
  n2: float = sqrt(x * y);
  n3: float = sqrt(x - 1.0);
  n4: float = ln(max(x, 0.0));
}
fn main() { }
";

// `@FR-N-Domain`'s GUARD licence for this family (loft#1450's walk, D-Domain-Guard).  The rule
// promises ONE elision — "provably in-domain (constant / range / GUARD)" — over three families,
// and the math row was the only one without its guard: `if x >= 0.0 { sqrt(x) }` typed `float?`
// where `if d != 0 { a / d }` and `if i < len(v) { v[i] }` both took theirs.  A comparison
// against zero now contributes a `Sign` for the slot, so the guard composes with the expression
// lattice above rather than sitting beside it.
const GUARDED: &str = "\
fn f(x: float, y: float) {
  if x >= 0.0 { g1: float = sqrt(x); }
  if y > 0.0 { g2: float = ln(y); }
}
fn clause(x: float) { if x < 0.0 { return; } g3: float = sqrt(x); }
fn main() { }
";

// The soundness half, and the half that decides whether the widening was safe: a guard that
// proves the WRONG fact, or proves it on the WRONG side, must leave the arg `float?`.  Each is
// a cell a wrong implementation passes: `u1` takes the else of `x >= 0.0` (so `x < 0`), `u2`
// proves non-zero rather than a sign, `u3` bounds only from above, `u4` proves `NonNeg` where
// `ln` needs strictly `Pos`, and `u5` reassigns the slot after the guard.
const GUARD_UNSOUND: &str = "\
fn a(x: float) { if x >= 0.0 { } else { u1: float = sqrt(x); } }
fn b(x: float) { if x != 0.0 { u2: float = sqrt(x); } }
fn c(x: float) { if x < 5.0 { u3: float = sqrt(x); } }
fn d(x: float) { if x >= 0.0 { u4: float = ln(x); } }
fn e(x: float) { if x >= 0.0 { x = 0.0 - 1.0; u5: float = sqrt(x); } }
fn main() { }
";

#[test]
fn math_domain_takes_the_guard_licence() {
    let (ok, diag) = compile("guarded", GUARDED, true);
    assert!(
        ok && !diag.contains(REPORT),
        "a zero-comparison guard must prove the arg in-domain, as it already does for the \
         divisor and index families; diag={diag}"
    );
}

#[test]
fn math_domain_guard_proves_only_what_it_proves() {
    // The cell the diagnostic was ABOUT.  A widening that removes a diagnostic is measured
    // here, not only where the diagnostic was noise — the divisor's truthy arm was written and
    // reverted the same hour for exactly this reason.
    let (ok, diag) = compile("guard_unsound", GUARD_UNSOUND, true);
    assert!(
        ok,
        "the reported stores proceed and the program runs; diag={diag}"
    );
    for v in ["u1", "u2", "u3", "u4", "u5"] {
        assert!(
            reported(&diag, v),
            "a guard that does not prove the domain must leave the arg float? — {v} must be \
             reported; diag={diag}"
        );
    }
}

#[test]
fn math_domain_opt_out_keeps_expression_args_forced() {
    // `LOFT_NO_MATH_DOMAIN` reverts to the constant-only elision — the expression args are
    // forced back to `float?` (the escape hatch behaves).
    let (ok, diag) = compile("optout", PROVABLE, false);
    assert!(
        ok,
        "the reported store proceeds and the program runs; diag={diag}"
    );
    for v in ["p1", "p2", "p3", "p4", "p5"] {
        assert!(
            reported(&diag, v),
            "under LOFT_NO_MATH_DOMAIN a non-constant fault-op arg must stay float? (forced) — \
             {v} must be reported; diag={diag}"
        );
    }
}

#[test]
fn math_domain_softens_provable_args_by_default() {
    // Default (B5 flipped default-on): a provably in-domain arg types non-null, no ?? forced.
    let (ok, diag) = compile("provable", PROVABLE, true);
    assert!(
        ok && !diag.contains(REPORT),
        "a provably in-domain arg must type non-null by default (no ?? forced, nothing reported); diag={diag}"
    );
}

#[test]
fn math_domain_keeps_unprovable_args_nullable() {
    let (ok, diag) = compile("unprovable", UNPROVABLE, true);
    assert!(
        ok,
        "the reported stores proceed and the program runs; diag={diag}"
    );
    for v in ["n1", "n2", "n3", "n4"] {
        assert!(
            reported(&diag, v),
            "unprovable args must stay float? even under LOFT_MATH_DOMAIN (soundness): control \
             {v} must be reported; diag={diag}"
        );
    }
}

// Two-sided domain [-1, 1] for asin/acos: sin/cos outputs, clamp, and the min/max clamp idiom.
const TWO_SIDED: &str = "\
fn f(x: float) {
  q1: float = asin(sin(x));
  q2: float = acos(cos(x));
  q3: float = asin(clamp(x, -1.0, 1.0));
  q4: float = asin(min(max(x, -1.0), 1.0));
}
fn main() { }
";

// asin/acos controls — only a LOWER bound (m2), an unbounded arg (m1), or arithmetic past the
// interval (m3) — each must stay float? (the sign lattice's one-sided bound is not enough).
const TWO_SIDED_CTRL: &str = "\
fn f(x: float) {
  m1: float = asin(x);
  m2: float = asin(max(x, -1.0));
  m3: float = acos(sin(x) + 0.5);
}
fn main() { }
";

#[test]
fn math_domain_two_sided_asin_acos() {
    let (ok, diag) = compile("asin_pos", TWO_SIDED, true);
    assert!(
        ok && !diag.contains(REPORT),
        "provably-in-[-1,1] asin/acos args must soften (nothing reported); diag={diag}"
    );
    let (ok2, diag2) = compile("asin_ctrl", TWO_SIDED_CTRL, true);
    assert!(
        ok2,
        "the reported stores proceed and the program runs; diag={diag2}"
    );
    for v in ["m1", "m2", "m3"] {
        assert!(
            reported(&diag2, v),
            "an unbounded / one-sided asin/acos arg must stay float?: control {v} must be \
             reported; diag={diag2}"
        );
    }
}

// @PLN102 case-C residual: the call-valued consts PI/E (OpMathPiFloat/OpMathEFloat) const-fold,
// so a divisor or fault-op arg written with them is proven non-zero / in-domain. Always on (the
// constant + divisor paths, not gated by the flag).
const PI_CONST: &str = "\
fn f(x: float) {
  a: float = x / PI;
  b: float = x / (2.0 * E);
  c: float = sqrt(PI);
}
fn main() { }
";

#[test]
fn math_domain_folds_call_valued_pi_e_consts() {
    let (ok, diag) = compile("pi", PI_CONST, true);
    assert!(
        ok && !diag.contains(REPORT),
        "PI/E as a divisor or fault-op arg must const-fold to non-null (nothing reported); diag={diag}"
    );
    // control: a genuine variable divisor stays float? — reported on the local it lands in
    let (ok2, diag2) = compile(
        "pivar",
        "fn f(x: float, d: float) { z: float = x / d; }\nfn main() { }\n",
        true,
    );
    assert!(
        ok2 && reported(&diag2, "z"),
        "a variable divisor must stay float? and be reported at `z`; diag={diag2}"
    );
}
